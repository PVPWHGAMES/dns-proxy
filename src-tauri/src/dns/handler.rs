use crate::config::{
    AppConfig, DnsProtocol, DnsStrategy, RuleAction, RuleType, Subscription, SubscriptionType,
};
use crate::dns::cache::{effective_cache_ttl, DnsCache};
use crate::dns::ecs;
use crate::dns::pool::DnsConnectionPool;
use crate::dns::DnsQueryLog;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::{IpAddr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;
use tracing::{debug, info, warn};
use trust_dns_client::op::Message;
use trust_dns_client::rr::{RData, Record, RecordType};
use trust_dns_proto::op::{Edns, MessageType, ResponseCode};
use trust_dns_proto::serialize::binary::{BinDecodable, BinEncodable};

/// 是否启用 GeoSite 域名路由
///
/// 开关定义在 `config` 里（它是产品级决定，同时要驱动配置清理），这里只做转发引用。
/// 停用不只是省内存：命中 proxy 列表的域名会被判为「代理请求」，从而**跳过缓存与
/// 请求合并**、每次都问上游；上游分组为空时又回退到默认策略，等于白白浪费了缓存。
const GEOSITE_ROUTING_ENABLED: bool = crate::config::GEOSITE_ROUTING_ENABLED;

/// 查询日志保留上限，超出后丢弃最旧记录
///
/// 日志页按页翻查，缓冲区就是可翻的历史范围。单条日志约几百字节，
/// 一万条的量级在 3MB 上下，对桌面程序可以接受；再往上就得落盘检索了。
const MAX_LOG_ENTRIES: usize = 10000;

/// 发往上游的查询统一声明的 EDNS0 载荷尺寸
///
/// 客户端在 UDP 上声明的尺寸只约束「客户端 ↔ 本代理」这一段，不应传导到上游。
/// 若原样转发，上游会按 512 截断并返回 TC=1；客户端随后按规范改用 TCP 重试时，
/// 本代理又以同样尺寸问上游，于是在 TCP 上再次拿到 TC=1 —— 而 TCP 的 TC 是终态，
/// 客户端没有更高一级可升级，该域名就彻底解析不了。
const UPSTREAM_EDNS_PAYLOAD: u16 = 4096;

/// 把回包规范化成「这次查询的应答」
///
/// 两件事必须在这里统一兜住：
/// 1. 事务 ID 必须是本次查询的 ID。缓存命中与请求合并返回的都是别的查询产生的
///    响应字节，直接下发会被客户端判为非法报文（Windows 解析器报 Bad DNS packet
///    并丢弃），表现为「首次能解析、之后整个缓存 TTL 内都失败」。
/// 2. 客户端没声明 EDNS0 时不得回带 OPT 记录（RFC 6891 §7），而本代理为了向
///    上游取全量答案会主动加上 OPT，因此需要按客户端情况摘掉。
fn normalize_response_for_client(query_bytes: &[u8], response: Vec<u8>) -> Vec<u8> {
    let Some(query_id) = query_bytes
        .get(0..2)
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
    else {
        return response;
    };

    let query = Message::from_bytes(query_bytes);
    let client_had_edns = query
        .as_ref()
        .map(|message| message.extensions().is_some())
        .unwrap_or(true);

    let id_matches = response
        .get(0..2)
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]) == query_id)
        .unwrap_or(true);

    // 常见路径：EDNS0 客户端且 ID 已一致，无需重新编码上游原包
    if id_matches && client_had_edns {
        return response;
    }

    let Ok(mut message) = Message::from_bytes(&response) else {
        return response;
    };

    message.set_id(query_id);
    if !client_had_edns {
        *message.extensions_mut() = None;
    }

    message.to_bytes().unwrap_or(response)
}

/// 让发往上游的查询声明足够大的 EDNS0 载荷尺寸
///
/// 客户端已声明不小于该值时不重新编码，直接原样转发。
fn prepare_upstream_query(query_bytes: &[u8]) -> Vec<u8> {
    let Ok(mut message) = Message::from_bytes(query_bytes) else {
        return query_bytes.to_vec();
    };

    if message.extensions().is_some() {
        let Some(edns) = message.extensions_mut() else {
            return query_bytes.to_vec();
        };
        if edns.max_payload() >= UPSTREAM_EDNS_PAYLOAD {
            return query_bytes.to_vec();
        }
        // 只改尺寸，DO 位与已有 EDNS 选项（例如 ECS）保持不变
        edns.set_max_payload(UPSTREAM_EDNS_PAYLOAD);
    } else {
        let mut edns = Edns::new();
        edns.set_max_payload(UPSTREAM_EDNS_PAYLOAD);
        message.set_edns(edns);
    }

    message.to_bytes().unwrap_or_else(|_| query_bytes.to_vec())
}

pub struct DnsHandler {
    config: Arc<Mutex<AppConfig>>,
    cache: Arc<DnsCache>,
    logs: Arc<Mutex<Vec<DnsQueryLog>>>,
    stats: Arc<Mutex<QueryStats>>,
    log_id_counter: Arc<Mutex<u64>>,
    strategy_index: Arc<Mutex<usize>>,
    http_client: Arc<reqwest::Client>,
    blocklist: Arc<Mutex<HashSet<String>>>, // 黑名单域名集合
    geosite_map: Arc<Mutex<HashMap<String, String>>>, // 域名 -> 目标分组
    server_latency: Arc<Mutex<HashMap<String, ServerLatency>>>, // 服务器延迟统计
    public_ip: Arc<Mutex<Option<IpAddr>>>,  // 自动获取的公网 IP
    public_ip_last_update: Arc<Mutex<Option<Instant>>>, // 上次更新时间
    traffic_stats: Arc<Mutex<TrafficStatsCollector>>, // 流量统计收集器
    /// 上游连接池（DoT 长连接 + UDP socket 复用）
    pool: Arc<DnsConnectionPool>,
    /// 请求合并：等待中的查询 (cache_key -> 追随者列表)
    pending_queries: Arc<AsyncMutex<HashMap<String, PendingQueryState>>>,
    /// 已告警过的问题标识，避免每次查询重复刷同一条告警
    warned_messages: Arc<Mutex<HashSet<String>>>,
}

/// 请求合并的进行中查询状态
struct PendingQueryState {
    /// 等待此查询结果的追随者
    waiters: Vec<tokio::sync::oneshot::Sender<Option<Vec<u8>>>>,
    /// 查询开始时间（用于清理过期条目）
    started_at: Instant,
}

#[derive(Clone, Default)]
struct ServerLatency {
    avg_latency_ms: u64,
    success_count: u64,
    fail_count: u64,
    last_latency_ms: Option<u64>,
}

#[derive(Default, Clone)]
pub struct QueryStats {
    pub total_queries: u64,
    pub blocked_queries: u64,
    pub cached_queries: u64,
    pub total_latency_ms: u64,
}

/// 时间桶统计（每分钟）
#[derive(Debug, Clone, serde::Serialize)]
pub struct TimeBucket {
    pub time: String, // "HH:MM" 格式
    pub total: u64,   // 总查询数
    pub blocked: u64, // 阻止数
    pub cached: u64,  // 缓存命中数
}

/// 域名统计
#[derive(Debug, Clone, serde::Serialize)]
pub struct DomainStat {
    pub domain: String,
    pub count: u64,
}

/// 延迟分布
#[derive(Debug, Clone, serde::Serialize)]
pub struct LatencyDistribution {
    pub range: String, // "0-10ms", "10-50ms", etc.
    pub count: u64,
}

/// 流量统计数据（返回给前端）
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrafficStats {
    pub timeline: Vec<TimeBucket>,              // 时间线数据
    pub top_domains: Vec<DomainStat>,           // Top 10 域名
    pub latency_dist: Vec<LatencyDistribution>, // 延迟分布
    pub total_queries: u64,
    pub queries_per_second: f64,
}

/// 时间线保留的分钟数
pub const TIMELINE_MINUTES: i64 = 60;

/// 时间序列数据收集器
#[derive(Default)]
pub struct TrafficStatsCollector {
    /// 按「当天第几分钟」统计的时间线 -> (total, blocked, cached)
    ///
    /// 用分钟序号而不是 "HH:MM" 字符串：字符串只能做字典序比较，
    /// 跨零点时 "00:05" < "23:30" 会把当天刚产生的桶全部当成过期数据删掉，
    /// 导致 0 点到 1 点之间趋势图恒为空。
    pub minute_buckets: BTreeMap<u32, (u64, u64, u64)>,
    // 域名计数
    pub domain_counts: HashMap<String, u64>,
    // 延迟分布
    pub latency_buckets: [u64; 6], // 0-10, 10-50, 50-100, 100-200, 200-500, 500+
    // 启动时间
    pub start_time: Option<Instant>,
}

/// 查询日志的筛选条件
///
/// 过滤必须放在后端：分页总数要是「整段缓冲区里命中的条数」。
/// 若只在前端过滤当前页，页数会按当前页的命中数算，翻页越翻越乱，
/// 且明明存在的记录会因为不在当前页而搜不到。
#[derive(Default, Clone)]
pub struct LogFilter {
    /// 域名关键字（大小写不敏感），空串表示不筛
    pub keyword: String,
    /// 查询状态，None 表示不筛
    pub action: Option<String>,
    /// 查询类型，None 表示不筛
    pub query_type: Option<String>,
}

impl LogFilter {
    fn matches(&self, log: &DnsQueryLog) -> bool {
        if !self.keyword.is_empty() && !log.domain.to_lowercase().contains(&self.keyword) {
            return false;
        }
        if let Some(action) = &self.action {
            if log.action != *action {
                return false;
            }
        }
        if let Some(query_type) = &self.query_type {
            if log.query_type != *query_type {
                return false;
            }
        }
        true
    }
}

/// 日志分页结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct LogPage {
    /// 命中筛选条件的总条数（按整段缓冲区统计）
    pub total: usize,
    /// 缓冲区当前保留的条数
    pub retained: usize,
    /// 缓冲区容量上限，即「最多保留最近多少条」
    pub capacity: usize,
    /// 当前页数据（新到旧）
    pub logs: Vec<DnsQueryLog>,
}

impl DnsHandler {
    /// 构造处理器
    ///
    /// `config` 是**共享**的配置实例，与服务器和界面状态同一份，构造期不再克隆。
    /// 过去这里写的是 `Arc::new(Mutex::new(config.clone()))`：既把按值传入的参数
    /// 白克隆一次，又让同一份配置在内存里存在多份，而订阅规则动辄十几万条。
    pub fn new(config: Arc<Mutex<AppConfig>>, cache: Arc<DnsCache>) -> Self {
        let mut http_client_builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .pool_max_idle_per_host(8)
            .pool_idle_timeout(Duration::from_secs(90));

        // 在系统 DNS 切换到本地代理前固定 DoH 上游地址，避免解析 DoH 主机名时递归回自身。
        // 借用一段短锁读出上游列表与引导服务器，读完立刻释放，不把锁带进后面的 await。
        let (upstream, bootstrap_servers) = {
            let guard = config
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            (guard.upstream.clone(), guard.proxy.bootstrap_dns.clone())
        };

        for server in upstream
            .iter()
            .filter(|server| server.enabled && server.protocol == DnsProtocol::Doh)
        {
            let Some(doh_url) = server.doh_url.as_deref() else {
                continue;
            };
            let Ok(url) = reqwest::Url::parse(doh_url) else {
                warn!("DoH URL 无效，无法预解析: {}", doh_url);
                continue;
            };
            let Some(host) = url.host_str() else {
                warn!("DoH URL 缺少主机名，无法预解析: {}", doh_url);
                continue;
            };
            if host.parse::<IpAddr>().is_ok() {
                continue;
            }

            let port = url.port_or_known_default().unwrap_or(443);
            let bootstrap_addr = server
                .ip
                .parse::<IpAddr>()
                .ok()
                .map(|ip| SocketAddr::new(ip, port))
                .or_else(|| {
                    // 先直连配置的引导服务器：系统 DNS 此时很可能已指向本程序，
                    // 走系统解析器就会递归回自身，这正是过去解析不出来的原因。
                    crate::dns::bootstrap::resolve_ipv4_default(host, &bootstrap_servers)
                        .map(|ip| SocketAddr::new(IpAddr::V4(ip), port))
                })
                .or_else(|| {
                    // 兜底才交给系统解析器
                    (host, port)
                        .to_socket_addrs()
                        .ok()
                        .and_then(|mut addresses| addresses.next())
                });

            if let Some(addr) = bootstrap_addr {
                http_client_builder = http_client_builder.resolve(host, addr);
                info!("DoH bootstrap 地址已固定: {} -> {}", host, addr);
            } else {
                warn!(
                    "无法解析 DoH bootstrap 地址，可能触发本地 DNS 递归: {}（可在设置里配置引导解析服务器）",
                    doh_url
                );
            }
        }

        let http_client = http_client_builder.build().unwrap_or_default();

        let blocklist = Arc::new(Mutex::new(HashSet::new()));
        let geosite_map = Arc::new(Mutex::new(HashMap::new()));

        let server_latency = Arc::new(Mutex::new(HashMap::new()));
        let public_ip = Arc::new(Mutex::new(None));
        let public_ip_last_update = Arc::new(Mutex::new(None));

        // 初始化流量统计收集器
        let mut traffic_collector = TrafficStatsCollector::default();
        traffic_collector.start_time = Some(Instant::now());

        // 创建上游连接池
        let pool = Arc::new(DnsConnectionPool::new(
            8,                        // 每主机最多 8 个空闲连接
            Duration::from_secs(120), // 空闲连接 120 秒过期
        ));

        // 加载已有的订阅规则
        let handler = Self {
            config,
            cache,
            logs: Arc::new(Mutex::new(Vec::new())),
            stats: Arc::new(Mutex::new(QueryStats::default())),
            log_id_counter: Arc::new(Mutex::new(1)),
            strategy_index: Arc::new(Mutex::new(0)),
            http_client: Arc::new(http_client),
            blocklist,
            geosite_map,
            server_latency,
            public_ip,
            public_ip_last_update,
            traffic_stats: Arc::new(Mutex::new(traffic_collector)),
            pool,
            pending_queries: Arc::new(AsyncMutex::new(HashMap::new())),
            warned_messages: Arc::new(Mutex::new(HashSet::new())),
        };

        // 初始化黑名单和域名路由：直接借用已持有的共享配置，不再克隆订阅规则
        {
            let guard = handler
                .config
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            handler.init_blocklist(&guard.subscriptions);
            handler.init_geosite(&guard.subscriptions);
        }

        handler
    }

    // 初始化黑名单
    fn init_blocklist(&self, subscriptions: &[Subscription]) {
        let mut blocklist = self.blocklist.lock().unwrap();
        for sub in subscriptions {
            if sub.enabled && sub.sub_type == SubscriptionType::Blocklist {
                for rule in &sub.rules {
                    blocklist.insert(rule.clone());
                }
            }
        }
        info!("已加载 {} 条黑名单规则", blocklist.len());
    }

    // 初始化域名路由表
    fn init_geosite(&self, subscriptions: &[Subscription]) {
        let mut map = self.geosite_map.lock().unwrap();
        // 先清空：停用期间不能让上一次运行残留的规则继续影响判断
        map.clear();

        if !GEOSITE_ROUTING_ENABLED {
            info!("域名路由规则已停用，全部走直连");
            return;
        }

        for sub in subscriptions {
            if sub.enabled && sub.sub_type == SubscriptionType::Geosite {
                if let Some(ref group) = sub.target_group {
                    for rule in &sub.rules {
                        map.insert(rule.clone(), group.clone());
                    }
                }
            }
        }
        info!("已加载 {} 条域名路由规则", map.len());
    }

    // 更新服务器延迟统计
    fn update_server_latency(&self, server_name: &str, latency_ms: u64, success: bool) {
        let mut latency_map = self.server_latency.lock().unwrap();
        let entry = latency_map.entry(server_name.to_string()).or_default();

        if success {
            entry.success_count += 1;
            entry.last_latency_ms = Some(latency_ms);
            // 使用指数移动平均计算平均延迟
            if entry.avg_latency_ms == 0 {
                entry.avg_latency_ms = latency_ms;
            } else {
                entry.avg_latency_ms = (entry.avg_latency_ms * 7 + latency_ms * 3) / 10;
            }
        } else {
            entry.fail_count += 1;
        }
    }

    // 获取服务器的历史延迟
    fn get_server_latency(&self, server_name: &str) -> Option<u64> {
        let latency_map = self.server_latency.lock().unwrap();
        latency_map.get(server_name).and_then(|l| {
            if l.success_count > 0 {
                Some(l.avg_latency_ms)
            } else {
                None
            }
        })
    }

    // 更新订阅
    pub async fn update_subscriptions(&self) {
        // 先获取需要更新的订阅URL
        let enabled_subs: Vec<(String, String)> = {
            let config = self.config.lock().unwrap();
            config
                .subscriptions
                .iter()
                .filter(|s| s.enabled)
                // 路由停用时不再下载 geosite 列表：十几万条规则既不参与匹配，
                // 每次拉取还要重写整份配置，纯属浪费
                .filter(|s| GEOSITE_ROUTING_ENABLED || s.sub_type != SubscriptionType::Geosite)
                .map(|s| (s.name.clone(), s.url.clone()))
                .collect()
        };

        // 获取每个订阅的规则
        let mut results: Vec<(String, Vec<String>)> = Vec::new();
        for (name, url) in enabled_subs {
            match self.fetch_subscription(&url).await {
                Ok(rules) => {
                    info!("更新订阅 {}: {} 条规则", name, rules.len());
                    results.push((name, rules));
                }
                Err(e) => {
                    warn!("更新订阅 {} 失败: {}", name, e);
                }
            }
        }

        // 更新配置
        let mut config = self.config.lock().unwrap();
        for (name, rules) in results {
            if let Some(sub) = config.subscriptions.iter_mut().find(|s| s.name == name) {
                sub.rules = rules;
                sub.last_updated =
                    Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
            }
        }

        // 保存配置
        if let Err(e) = config.save() {
            warn!("保存配置失败: {}", e);
        }

        // 重新加载黑名单和域名路由
        self.init_blocklist(&config.subscriptions);
        self.init_geosite(&config.subscriptions);
    }

    // 获取订阅内容
    async fn fetch_subscription(&self, url: &str) -> anyhow::Result<Vec<String>> {
        let response = self.http_client.get(url).send().await?;
        let text = response.text().await?;

        let rules: Vec<String> = text
            .lines()
            .map(|line| line.trim())
            .filter(|line| {
                // 过滤注释和空行
                !line.is_empty()
                    && !line.starts_with('#')
                    && !line.starts_with('!')
                    && !line.starts_with('[')
                    && !line.starts_with("//")
            })
            .filter_map(|line| {
                // 解析hosts格式: 0.0.0.0 domain.com 或 127.0.0.1 domain.com
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let first = parts[0];
                    if first == "0.0.0.0" || first == "127.0.0.1" || first == "::1" || first == "::"
                    {
                        let domain = parts[1].to_lowercase().trim_end_matches('.').to_string();
                        if !domain.is_empty() && domain.contains('.') && !domain.starts_with('#') {
                            return Some(domain);
                        }
                    }
                }

                // 检查是否是纯域名格式 (AdGuard格式: ||domain.com^)
                let line = line.trim();
                if line.starts_with("||") && line.ends_with('^') {
                    let domain = line[2..line.len() - 1].to_lowercase();
                    if domain.contains('.') {
                        return Some(domain);
                    }
                }

                // 检查是否是纯域名格式
                if line.contains('.') && !line.contains(' ') && !line.contains('/') {
                    let domain = line.to_lowercase().trim_end_matches('.').to_string();
                    if !domain.is_empty() && !domain.starts_with('#') {
                        return Some(domain);
                    }
                }

                None
            })
            .collect();

        Ok(rules)
    }

    /// 处理一条 DNS 查询
    ///
    /// 所有回包（缓存命中、请求合并、上游转发、本地拦截）都经过
    /// `normalize_response_for_client`，保证事务 ID 与 EDNS0 形态对得上客户端。
    pub async fn handle_query(&self, query_bytes: &[u8]) -> Option<Vec<u8>> {
        let response = self.handle_query_inner(query_bytes).await?;
        Some(normalize_response_for_client(query_bytes, response))
    }

    async fn handle_query_inner(&self, query_bytes: &[u8]) -> Option<Vec<u8>> {
        let start = Instant::now();

        let query = match Message::from_bytes(query_bytes) {
            Ok(q) => q,
            Err(e) => {
                warn!("解析DNS请求失败: {}", e);
                return None;
            }
        };

        let query_name = query
            .queries()
            .first()
            .map(|q| q.name().to_string().trim_end_matches('.').to_lowercase())
            .unwrap_or_default();

        let query_type = query
            .queries()
            .first()
            .map(|q| q.query_type())
            .unwrap_or(RecordType::A);

        // 检查阻止IPv6
        {
            let config = self.config.lock().unwrap();
            if config.proxy.block_ipv6 && query_type == RecordType::AAAA {
                self.record_blocked(&query_name, "AAAA", "-", start);
                // 回 NODATA 而非黑洞地址：客户端看到「没有 AAAA 记录」会立刻改用 IPv4
                return Some(self.create_nodata_response(&query));
            }
        }

        // ① 自定义规则（最高优先级，覆盖一切订阅）
        let rule_result = self.check_rules(&query_name);
        let is_whitelisted = rule_result
            .as_ref()
            .map_or(false, |(action, _, _)| *action == RuleAction::Allow);
        let mut forward_group: Option<String> = match rule_result {
            Some((RuleAction::Allow, _, _)) => {
                // 白名单规则，跳过黑名单检查，直接放行
                debug!("白名单放行: {}", query_name);
                None
            }
            Some((RuleAction::Block, _, _)) | Some((RuleAction::BlockNull, _, _)) => {
                self.record_blocked(&query_name, &format!("{:?}", query_type), "rule", start);
                return Some(self.create_blocked_response(&query));
            }
            Some((RuleAction::BlockNxdomain, _, _)) => {
                self.record_blocked(
                    &query_name,
                    &format!("{:?}", query_type),
                    "rule:nxdomain",
                    start,
                );
                return Some(self.create_nxdomain_response(&query));
            }
            Some((RuleAction::Forward, _, ref target)) => target.clone(),
            _ => None,
        };

        // ② 黑名单订阅（自定义规则未命中或为白名单时跳过）
        if !is_whitelisted && forward_group.is_none() && self.is_blocked(&query_name) {
            self.record_blocked(
                &query_name,
                &format!("{:?}", query_type),
                "blocklist",
                start,
            );
            return Some(self.create_blocked_response(&query));
        }

        // 自定义规则未指定分组时，检查 geosite 域名路由
        if forward_group.is_none() {
            forward_group = self.check_geosite(&query_name);
        }

        // 仍未命中时，使用默认分组
        if forward_group.is_none() {
            let default_group = self.config.lock().unwrap().proxy.default_group.clone();
            if !default_group.is_empty() {
                forward_group = Some(default_group);
            }
        }

        let is_proxy_request = forward_group.as_deref() == Some("proxy");
        let cache_key = format!("{}:{:?}", query_name, query_type);

        // 代理请求交给代理软件每次重新解析，不读取缓存
        if !is_proxy_request {
            if let Some(cached) = self.cache.get(&cache_key) {
                // 缓存命中也要记录真实目标 IP，便于日志直接看出访问去向
                let cached_ip = cached
                    .answers()
                    .first()
                    .and_then(|a| {
                        a.data()
                            .and_then(|d| d.to_string().split_whitespace().last().map(String::from))
                    })
                    .unwrap_or_else(|| "cached".to_string());
                self.record_cached(&query_name, &format!("{:?}", query_type), &cached_ip, start);
                return cached.to_bytes().ok();
            }
        }

        // 直连请求合并并发查询，代理请求每次都交给代理软件
        if !is_proxy_request {
            let mut pending = self.pending_queries.lock().await;
            // 顺便清理过期条目（超过 10 秒未完成）
            pending.retain(|_, state| state.started_at.elapsed() < Duration::from_secs(10));

            if let Some(state) = pending.get_mut(&cache_key) {
                // 已有进行中的查询，成为追随者
                let (tx, rx) = tokio::sync::oneshot::channel();
                state.waiters.push(tx);
                drop(pending);

                // 等待领导者完成（5 秒超时）
                match tokio::time::timeout(Duration::from_secs(5), rx).await {
                    Ok(Ok(Some(response))) => {
                        self.record_coalesced(&query_name, &format!("{:?}", query_type), start);
                        return Some(response);
                    }
                    _ => {
                        // 超时或领导者失败，向上返回 None。
                        // 这条查询同样是「收到了但没答复」，计入失败，
                        // 否则并发等待超时的请求会完全不出现在统计与日志里。
                        self.record_failure(
                            &query_name,
                            &format!("{:?}", query_type),
                            "coalesced-timeout",
                            start,
                        );
                        return None;
                    }
                }
            } else {
                // 成为领导者，注册进行中查询
                pending.insert(
                    cache_key.clone(),
                    PendingQueryState {
                        waiters: Vec::new(),
                        started_at: Instant::now(),
                    },
                );
            }
        }

        // 根据策略选择DNS服务器并转发（如有指定分组则过滤）
        let (response, server_name) = if let Some(ref group) = forward_group {
            self.forward_with_strategy_for_group(query_bytes, group)
                .await
        } else {
            self.forward_with_strategy(query_bytes).await
        };

        if let Some(response_bytes) = &response {
            let response_ip = Message::from_bytes(response_bytes)
                .ok()
                .and_then(|m| {
                    m.answers().first().and_then(|a| {
                        a.data()
                            .and_then(|d| d.to_string().split_whitespace().last().map(String::from))
                    })
                })
                .unwrap_or_else(|| "-".to_string());

            let latency = start.elapsed().as_millis() as u64;

            // 代理请求不写入缓存，避免污染直连结果
            if !is_proxy_request {
                if let Ok(response_msg) = Message::from_bytes(response_bytes) {
                    // 缓存寿命取记录自身 TTL，配置项只当上限；TTL 为 0 的响应不缓存
                    let cap = Duration::from_secs(self.config.lock().unwrap().proxy.cache_ttl);
                    match effective_cache_ttl(&response_msg, cap) {
                        Some(lifetime) => {
                            self.cache
                                .put(cache_key.clone(), response_msg, lifetime, cap)
                        }
                        None => debug!("响应无可缓存记录或 TTL 为 0，跳过缓存: {}", query_name),
                    }
                }
            }

            self.record_success(
                &query_name,
                &format!("{:?}", query_type),
                &response_ip,
                &server_name,
                latency,
                forward_group.as_deref().unwrap_or("default"),
            );
        }

        // 通知所有等待中的追随者
        if !is_proxy_request {
            self.notify_pending(&cache_key, response.clone()).await;
        }

        // 上游 DNS 全部失败时记录失败日志并返回 SERVFAIL 响应，
        // 避免上游不可用时让客户端一直等到超时。
        //
        // 代理请求（proxy 分组）不回 SERVFAIL，等代理软件自行处理；但它同样是一次
        // 收到却没答复的查询，必须计入统计，否则「总查询数」会整类漏掉 ——
        // 代理分组没有可用上游时尤其明显。
        if response.is_none() {
            self.record_failure(&query_name, &format!("{:?}", query_type), "none", start);
            if !is_proxy_request {
                return Some(self.create_servfail_response(&query));
            }
        }

        response
    }

    // 检查是否在黑名单中
    fn is_blocked(&self, domain: &str) -> bool {
        let blocklist = self.blocklist.lock().unwrap();

        // 精确匹配
        if blocklist.contains(domain) {
            return true;
        }

        // 检查父域名
        let parts: Vec<&str> = domain.split('.').collect();
        for i in 1..parts.len() {
            let parent = parts[i..].join(".");
            if blocklist.contains(&parent) {
                return true;
            }
        }

        false
    }

    // 检查域名是否匹配 geosite 路由表，返回目标分组
    fn check_geosite(&self, domain: &str) -> Option<String> {
        // 停用期间表恒为空，这里显式短路，语义比依赖「表恰好是空的」更清楚
        if !GEOSITE_ROUTING_ENABLED {
            return None;
        }

        let map = self.geosite_map.lock().unwrap();

        // 精确匹配
        if let Some(group) = map.get(domain) {
            return Some(group.clone());
        }

        // 检查父域名
        let parts: Vec<&str> = domain.split('.').collect();
        for i in 1..parts.len() {
            let parent = parts[i..].join(".");
            if let Some(group) = map.get(&parent) {
                return Some(group.clone());
            }
        }

        None
    }

    // 检查自定义规则，返回 (action, rule_name, target_group)
    fn check_rules(&self, domain: &str) -> Option<(RuleAction, String, Option<String>)> {
        let config = self.config.lock().unwrap();
        let mut rules: Vec<_> = config.rules.iter().filter(|r| r.enabled).collect();
        rules.sort_by_key(|r| r.priority);

        for rule in rules {
            let matched = match rule.rule_type {
                RuleType::Exact => domain == rule.pattern.to_lowercase(),
                RuleType::Wildcard => {
                    let pattern = rule.pattern.replace("*", "").to_lowercase();
                    domain.ends_with(&pattern) || domain == pattern.trim_end_matches('.')
                }
                RuleType::Regex => regex::Regex::new(&rule.pattern)
                    .map(|re| re.is_match(domain))
                    .unwrap_or(false),
                // 黑名单类型规则自身就描述一个要拦的域名（含其子域名）。
                // 订阅里的黑名单走 is_blocked 查表，这条用于用户手写的单条规则。
                RuleType::Blocklist => {
                    let pattern = rule.pattern.to_lowercase();
                    let pattern = pattern.trim_end_matches('.');
                    !pattern.is_empty()
                        && (domain == pattern || domain.ends_with(&format!(".{}", pattern)))
                }
            };

            if matched {
                return Some((rule.action.clone(), rule.name.clone(), rule.target.clone()));
            }
        }

        None
    }

    // 根据策略转发请求（使用指定分组的服务器）
    async fn forward_with_strategy_for_group(
        &self,
        query_bytes: &[u8],
        group: &str,
    ) -> (Option<Vec<u8>>, String) {
        let (strategy, group_servers) = {
            let config = self.config.lock().unwrap();
            let servers: Vec<_> = config
                .upstream
                .iter()
                .filter(|s| s.enabled && s.group == group)
                .cloned()
                .collect();
            (config.strategy.clone(), servers)
        };

        if group_servers.is_empty() {
            self.warn_once(
                &format!("group_without_server:{}", group),
                &format!("分组 '{}' 没有启用的DNS服务器，回退到默认策略", group),
            );
            return self.forward_with_strategy(query_bytes).await;
        }

        self.do_forward(query_bytes, &strategy, &group_servers)
            .await
    }

    // 根据策略转发请求（使用全部启用的服务器）
    async fn forward_with_strategy(&self, query_bytes: &[u8]) -> (Option<Vec<u8>>, String) {
        let (strategy, enabled_servers) = {
            let config = self.config.lock().unwrap();
            let enabled: Vec<_> = config
                .upstream
                .iter()
                .filter(|s| s.enabled)
                .cloned()
                .collect();
            (config.strategy.clone(), enabled)
        };

        if enabled_servers.is_empty() {
            self.warn_once("no_enabled_server", "没有启用的DNS服务器");
            return (None, "none".to_string());
        }

        self.do_forward(query_bytes, &strategy, &enabled_servers)
            .await
    }

    // 实际转发逻辑（供 forward_with_strategy 和 forward_with_strategy_for_group 共用）
    async fn do_forward(
        &self,
        query_bytes: &[u8],
        strategy: &DnsStrategy,
        servers: &[crate::config::DnsServer],
    ) -> (Option<Vec<u8>>, String) {
        match strategy {
            DnsStrategy::Sequential => {
                for server in servers {
                    let start = Instant::now();
                    if let Some(response) = self.forward_to_server(query_bytes, server).await {
                        let latency = start.elapsed().as_millis() as u64;
                        self.update_server_latency(&server.name, latency, true);
                        return (Some(response), server.name.clone());
                    } else {
                        let latency = start.elapsed().as_millis() as u64;
                        self.update_server_latency(&server.name, latency, false);
                    }
                }
                (None, "none".to_string())
            }
            DnsStrategy::Fastest => {
                // 智能最快策略：优先使用历史延迟最低的服务器
                let fastest_server = self.get_fastest_server(servers);
                if let Some(server) = fastest_server {
                    let start = Instant::now();
                    if let Some(response) = self.forward_to_server(query_bytes, &server).await {
                        let latency = start.elapsed().as_millis() as u64;
                        self.update_server_latency(&server.name, latency, true);
                        return (Some(response), server.name.clone());
                    } else {
                        let latency = start.elapsed().as_millis() as u64;
                        self.update_server_latency(&server.name, latency, false);
                    }
                }

                // 回退到并发查询
                let futures: Vec<
                    std::pin::Pin<
                        Box<
                            dyn futures::Future<Output = Option<(Vec<u8>, String, Instant)>> + Send,
                        >,
                    >,
                > = servers
                    .iter()
                    .map(|s| {
                        let s = s.clone();
                        let bytes = query_bytes.to_vec();
                        Box::pin(async move {
                            let start = Instant::now();
                            let result = self.forward_to_server(&bytes, &s).await;
                            result.map(|r| (r, s.name.clone(), start))
                        })
                            as std::pin::Pin<
                                Box<
                                    dyn futures::Future<Output = Option<(Vec<u8>, String, Instant)>>
                                        + Send,
                                >,
                            >
                    })
                    .collect();

                let (result, _index, _remaining) = futures::future::select_all(futures).await;
                match result {
                    Some((resp, name, start)) => {
                        let latency = start.elapsed().as_millis() as u64;
                        self.update_server_latency(&name, latency, true);
                        (Some(resp), name)
                    }
                    None => (None, "none".to_string()),
                }
            }
            DnsStrategy::Parallel => {
                // 并行策略：同时发送到所有服务器，返回第一个成功的响应
                let futures: Vec<
                    std::pin::Pin<
                        Box<
                            dyn futures::Future<Output = Option<(Vec<u8>, String, Instant)>> + Send,
                        >,
                    >,
                > = servers
                    .iter()
                    .map(|s| {
                        let s = s.clone();
                        let bytes = query_bytes.to_vec();
                        Box::pin(async move {
                            let start = Instant::now();
                            let result = self.forward_to_server(&bytes, &s).await;
                            result.map(|r| (r, s.name.clone(), start))
                        })
                            as std::pin::Pin<
                                Box<
                                    dyn futures::Future<Output = Option<(Vec<u8>, String, Instant)>>
                                        + Send,
                                >,
                            >
                    })
                    .collect();

                let (result, _index, _remaining) = futures::future::select_all(futures).await;
                match result {
                    Some((resp, name, start)) => {
                        let latency = start.elapsed().as_millis() as u64;
                        self.update_server_latency(&name, latency, true);
                        (Some(resp), name)
                    }
                    None => (None, "none".to_string()),
                }
            }
            DnsStrategy::LoadBalance => {
                let index = {
                    let mut idx = self.strategy_index.lock().unwrap();
                    let i = *idx % servers.len();
                    *idx = i + 1;
                    i
                };

                let server = &servers[index];
                let name = server.name.clone();
                let start = Instant::now();
                let result = self.forward_to_server(query_bytes, server).await;
                let latency = start.elapsed().as_millis() as u64;
                self.update_server_latency(&name, latency, result.is_some());
                (result, name)
            }
        }
    }

    // 获取历史延迟最低的服务器
    fn get_fastest_server(
        &self,
        servers: &[crate::config::DnsServer],
    ) -> Option<crate::config::DnsServer> {
        let latency_map = self.server_latency.lock().unwrap();

        let mut best_server: Option<crate::config::DnsServer> = None;
        let mut best_latency = u64::MAX;

        for server in servers {
            if let Some(latency_entry) = latency_map.get(&server.name) {
                if latency_entry.success_count > 0 && latency_entry.avg_latency_ms < best_latency {
                    best_latency = latency_entry.avg_latency_ms;
                    best_server = Some(server.clone());
                }
            }
        }

        best_server
    }

    async fn forward_to_server(
        &self,
        query_bytes: &[u8],
        server: &crate::config::DnsServer,
    ) -> Option<Vec<u8>> {
        // 获取 ECS 配置并注入 ECS 信息
        let query_with_ecs = self.maybe_inject_ecs(query_bytes).await;
        // 再统一把上游查询的 EDNS0 尺寸抬到足够大，避免上游按客户端的小尺寸截断
        let upstream_query = prepare_upstream_query(&query_with_ecs);

        match server.protocol {
            DnsProtocol::Udp | DnsProtocol::Tcp => {
                self.forward_udp(&upstream_query, &server.ip, server.port)
                    .await
            }
            DnsProtocol::Doh => {
                let url = server
                    .doh_url
                    .as_deref()
                    .unwrap_or("https://cloudflare-dns.com/dns-query");
                self.forward_doh(&upstream_query, url).await
            }
            DnsProtocol::Dot => {
                // DoT 默认端口为 853
                let port = if server.port == 53 { 853 } else { server.port };
                self.forward_dot(&upstream_query, &server.ip, port).await
            }
        }
    }

    /// 如果启用了 ECS，则在 DNS 查询中注入 ECS 信息
    async fn maybe_inject_ecs(&self, query_bytes: &[u8]) -> Vec<u8> {
        // 克隆 ECS 配置，避免持有 MutexGuard 跨越 await
        let ecs_config = {
            let config = self.config.lock().unwrap();
            config.ecs.clone()
        };

        if !ecs_config.enabled {
            return query_bytes.to_vec();
        }

        // 获取客户端 IP：优先使用配置的 IP，否则自动获取公网 IP
        let client_ip = if let Some(ref ip_str) = ecs_config.client_ip {
            match ip_str.parse::<IpAddr>() {
                Ok(ip) => ip,
                Err(_) => {
                    warn!("无效的 ECS 客户端 IP: {}", ip_str);
                    return query_bytes.to_vec();
                }
            }
        } else {
            // 自动获取公网 IP（带缓存，每5分钟更新一次）
            match self.get_or_fetch_public_ip().await {
                Some(ip) => ip,
                None => {
                    warn!("无法获取公网 IP，跳过 ECS 注入");
                    return query_bytes.to_vec();
                }
            }
        };

        // 根据 IP 类型选择掩码
        let source_mask = match client_ip {
            IpAddr::V4(_) => ecs_config.ipv4_source_mask,
            IpAddr::V6(_) => ecs_config.ipv6_source_mask,
        };

        debug!("注入 ECS: client_ip={}, mask=/{}/", client_ip, source_mask);
        ecs::inject_ecs(query_bytes, client_ip, source_mask)
    }

    /// 获取公网 IP（异步版本，带缓存）
    async fn get_or_fetch_public_ip(&self) -> Option<IpAddr> {
        // 检查缓存是否有效（5分钟内）
        {
            let last_update = self.public_ip_last_update.lock().unwrap();
            if let Some(last) = *last_update {
                if last.elapsed() < Duration::from_secs(300) {
                    let ip = self.public_ip.lock().unwrap();
                    return *ip;
                }
            }
        }

        // 缓存过期，需要更新
        let ip = fetch_public_ip(&self.http_client).await;
        if let Some(ip) = ip {
            let mut cached_ip = self.public_ip.lock().unwrap();
            *cached_ip = Some(ip);
            let mut last_update = self.public_ip_last_update.lock().unwrap();
            *last_update = Some(Instant::now());
            info!("自动获取公网 IP: {}", ip);
            Some(ip)
        } else {
            // 获取失败，返回缓存的 IP（如果有）
            let cached_ip = self.public_ip.lock().unwrap();
            *cached_ip
        }
    }

    /// 获取公网 IP（同步版本，带缓存）
    /// 异步更新公网 IP（可在后台定期调用）
    pub async fn update_public_ip(&self) {
        let ip = fetch_public_ip(&self.http_client).await;
        if let Some(ip) = ip {
            let mut cached_ip = self.public_ip.lock().unwrap();
            *cached_ip = Some(ip);
            let mut last_update = self.public_ip_last_update.lock().unwrap();
            *last_update = Some(Instant::now());
            info!("更新公网 IP: {}", ip);
        }
    }

    async fn forward_udp(&self, query_bytes: &[u8], ip: &str, port: u16) -> Option<Vec<u8>> {
        let addr = format!("{}:{}", ip, port);
        self.pool.udp_query(&addr, query_bytes).await
    }

    async fn forward_doh(&self, query_bytes: &[u8], url: &str) -> Option<Vec<u8>> {
        let response = self
            .http_client
            .post(url)
            .header("Content-Type", "application/dns-message")
            .header("Accept", "application/dns-message")
            .body(query_bytes.to_vec())
            .send()
            .await
            .ok()?;

        if response.status().is_success() {
            response.bytes().await.ok().map(|b| b.to_vec())
        } else {
            warn!("DoH请求失败: {} {}", url, response.status());
            None
        }
    }

    /// DoT (DNS-over-TLS) 转发（使用连接池复用 TLS 连接）
    async fn forward_dot(&self, query_bytes: &[u8], ip: &str, port: u16) -> Option<Vec<u8>> {
        let addr = format!("{}:{}", ip, port);

        // 首次尝试：从连接池获取连接
        // TODO: DoT 应使用主机名而非 IP 进行 TLS SNI 验证，需要在配置中添加 dot_hostname 字段
        if let Some(mut tls) = self.pool.acquire_dot(&addr, ip).await {
            if let Some(response) = self.do_dot_query(&mut tls, query_bytes).await {
                self.pool.release_dot(&addr, tls);
                return Some(response);
            }
            // 池连接已失效，丢弃后创建新连接
        }

        // 二次尝试：创建全新连接
        let mut tls = self.pool.acquire_dot(&addr, ip).await?;
        let response = self.do_dot_query(&mut tls, query_bytes).await;
        if response.is_some() {
            self.pool.release_dot(&addr, tls);
        }
        // 失败则丢弃连接（不归还）
        response
    }

    /// DoT 查询核心：在已建立的 TLS 连接上执行一次 DNS 查询
    async fn do_dot_query(
        &self,
        tls: &mut tokio_rustls::client::TlsStream<tokio::net::TcpStream>,
        query_bytes: &[u8],
    ) -> Option<Vec<u8>> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // 发送 DNS 查询（DoT 使用 TCP 格式：2字节长度前缀 + 查询数据）
        let len = (query_bytes.len() as u16).to_be_bytes();
        tls.write_all(&len).await.ok()?;
        tls.write_all(query_bytes).await.ok()?;

        // 读取响应长度
        let mut len_buf = [0u8; 2];
        tls.read_exact(&mut len_buf).await.ok()?;
        let resp_len = u16::from_be_bytes(len_buf) as usize;

        if resp_len > 4096 {
            warn!("[连接池] DoT 响应长度异常: {}", resp_len);
            return None;
        }

        // 读取响应数据
        let mut resp_buf = vec![0u8; resp_len];
        match tokio::time::timeout(Duration::from_secs(3), tls.read_exact(&mut resp_buf)).await {
            Ok(Ok(_)) => Some(resp_buf),
            Ok(Err(e)) => {
                warn!("[连接池] DoT 读取响应失败: {}", e);
                None
            }
            Err(_) => {
                warn!("[连接池] DoT 读取响应超时");
                None
            }
        }
    }

    /// 构造响应骨架：回显查询段并置 QR=1
    ///
    /// `Message::new()` 默认是 Query 报文（QR=0），直接下发会被客户端
    /// 当成非法响应丢弃，因此所有自造响应都必须经过这里设置响应类型。
    fn build_response(&self, query: &Message, code: ResponseCode) -> Message {
        let mut response = Message::new();
        response.set_id(query.id());
        response.set_message_type(MessageType::Response);
        response.set_op_code(query.op_code());
        response.set_recursion_desired(query.recursion_desired());
        response.set_recursion_available(true);
        response.set_response_code(code);

        if let Some(q) = query.queries().first() {
            response.add_query(q.clone());
        }

        response
    }

    /// 创建广告/规则拦截响应
    ///
    /// 按查询类型返回黑洞地址：A 回 0.0.0.0、AAAA 回 ::，让应用立刻连接失败；
    /// 其它类型没有黑洞语义，回空答案（NODATA）。统一用 NOERROR，
    /// 避免与「域名真的不存在」在客户端负缓存里混为一谈。
    fn create_blocked_response(&self, query: &Message) -> Vec<u8> {
        let mut response = self.build_response(query, ResponseCode::NoError);

        if let Some(q) = query.queries().first() {
            let blackhole = match q.query_type() {
                RecordType::A => Some(RData::A(trust_dns_client::rr::rdata::A(
                    std::net::Ipv4Addr::UNSPECIFIED,
                ))),
                RecordType::AAAA => Some(RData::AAAA(trust_dns_client::rr::rdata::AAAA(
                    Ipv6Addr::UNSPECIFIED,
                ))),
                _ => None,
            };

            if let Some(rdata) = blackhole {
                response.add_answer(Record::from_rdata(q.name().clone(), 300, rdata));
            }
        }

        response.to_bytes().unwrap_or_default()
    }

    /// 创建 NODATA 响应（NOERROR + 空答案）
    ///
    /// 用于「这个类型的记录不该存在」的场景，例如屏蔽 IPv6 时的 AAAA 查询：
    /// 客户端读到空答案会立刻改用 IPv4，而不是去连一个黑洞地址。
    fn create_nodata_response(&self, query: &Message) -> Vec<u8> {
        self.build_response(query, ResponseCode::NoError)
            .to_bytes()
            .unwrap_or_default()
    }

    /// 创建 NXDOMAIN 响应
    fn create_nxdomain_response(&self, query: &Message) -> Vec<u8> {
        self.build_response(query, ResponseCode::NXDomain)
            .to_bytes()
            .unwrap_or_default()
    }

    /// 创建 SERVFAIL 响应
    fn create_servfail_response(&self, query: &Message) -> Vec<u8> {
        self.build_response(query, ResponseCode::ServFail)
            .to_bytes()
            .unwrap_or_default()
    }

    fn record_success(
        &self,
        domain: &str,
        qtype: &str,
        response: &str,
        upstream: &str,
        latency: u64,
        group: &str,
    ) {
        let mut stats = self.stats.lock().unwrap();
        stats.total_queries += 1;
        stats.total_latency_ms += latency;

        let mut counter = self.log_id_counter.lock().unwrap();
        let id = *counter;
        *counter += 1;

        let log = DnsQueryLog {
            id,
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            domain: domain.to_string(),
            query_type: qtype.to_string(),
            response: response.to_string(),
            upstream: upstream.to_string(),
            latency_ms: latency,
            action: "success".to_string(),
            group: group.to_string(),
        };

        self.push_log(log);

        // 更新流量统计
        if let Ok(mut traffic) = self.traffic_stats.lock() {
            traffic.record_to_bucket(false, false);
            traffic.record_domain(domain);
            traffic.record_latency(latency);
        }

        info!(
            "DNS查询: {} {} -> {} via {} ({}ms)",
            domain, qtype, response, upstream, latency
        );
    }

    fn record_blocked(&self, domain: &str, qtype: &str, upstream: &str, start: Instant) {
        let mut stats = self.stats.lock().unwrap();
        stats.total_queries += 1;
        stats.blocked_queries += 1;

        let mut counter = self.log_id_counter.lock().unwrap();
        let id = *counter;
        *counter += 1;

        let log = DnsQueryLog {
            id,
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            domain: domain.to_string(),
            query_type: qtype.to_string(),
            response: "0.0.0.0".to_string(),
            upstream: upstream.to_string(),
            latency_ms: start.elapsed().as_millis() as u64,
            action: "blocked".to_string(),
            group: String::new(),
        };

        self.push_log(log);

        // 更新流量统计
        if let Ok(mut traffic) = self.traffic_stats.lock() {
            traffic.record_to_bucket(true, false);
            traffic.record_domain(domain);
        }
    }

    fn record_failure(&self, domain: &str, qtype: &str, upstream: &str, start: Instant) {
        let mut stats = self.stats.lock().unwrap();
        stats.total_queries += 1;

        let mut counter = self.log_id_counter.lock().unwrap();
        let id = *counter;
        *counter += 1;

        let log = DnsQueryLog {
            id,
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            domain: domain.to_string(),
            query_type: qtype.to_string(),
            response: "servfail".to_string(),
            upstream: upstream.to_string(),
            latency_ms: start.elapsed().as_millis() as u64,
            action: "failed".to_string(),
            group: String::new(),
        };

        self.push_log(log);

        // 失败请求仍计入流量统计，不计入 blocked/cached。
        if let Ok(mut traffic) = self.traffic_stats.lock() {
            traffic.record_to_bucket(false, false);
            traffic.record_domain(domain);
        }

        warn!("DNS查询失败: {} {} via {}", domain, qtype, upstream);
    }

    fn record_cached(&self, domain: &str, qtype: &str, response: &str, start: Instant) {
        let mut stats = self.stats.lock().unwrap();
        stats.total_queries += 1;
        stats.cached_queries += 1;

        let mut counter = self.log_id_counter.lock().unwrap();
        let id = *counter;
        *counter += 1;

        let log = DnsQueryLog {
            id,
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            domain: domain.to_string(),
            query_type: qtype.to_string(),
            response: response.to_string(),
            upstream: "cache".to_string(),
            latency_ms: start.elapsed().as_millis() as u64,
            action: "cached".to_string(),
            group: String::new(),
        };

        self.push_log(log);

        // 更新流量统计
        if let Ok(mut traffic) = self.traffic_stats.lock() {
            traffic.record_to_bucket(false, true);
            traffic.record_domain(domain);
            traffic.record_latency(start.elapsed().as_millis() as u64);
        }

        // 缓存命中此前不写文件日志，排查时看起来像「查询没进来」
        info!(
            "DNS查询(缓存命中): {} {} -> {} ({}ms)",
            domain,
            qtype,
            response,
            start.elapsed().as_millis()
        );
    }

    /// 写入查询日志
    ///
    /// 所有日志路径统一走这里，保证截断逻辑不会被漏掉导致日志无界增长。
    fn push_log(&self, log: DnsQueryLog) {
        if let Ok(mut logs) = self.logs.lock() {
            logs.insert(0, log);
            if logs.len() > MAX_LOG_ENTRIES {
                logs.truncate(MAX_LOG_ENTRIES);
            }
        }
    }

    /// 取最近 limit 条查询日志（新到旧），供前端首次加载与仪表盘使用
    pub fn get_logs(&self, limit: usize) -> Vec<DnsQueryLog> {
        let logs = self.logs.lock().unwrap();
        let count = limit.min(logs.len());
        logs[..count].to_vec()
    }

    /// 取 id 大于 since_id 的日志（旧到新），供前端增量刷新
    pub fn get_logs_since(&self, since_id: u64) -> Vec<DnsQueryLog> {
        let logs = self.logs.lock().unwrap();
        // 日志按 id 降序存放，从最新的开始取到已见过的 id 为止
        let mut fresh: Vec<DnsQueryLog> = logs
            .iter()
            .take_while(|log| log.id > since_id)
            .cloned()
            .collect();
        fresh.reverse();
        fresh
    }

    /// 按条件分页取日志（新到旧）
    ///
    /// 一次遍历同时得到「命中总数」和「当前页数据」：总数必须先于分页窗口统计，
    /// 前端才能据它算出正确页数，翻到末页不会出现空白页。
    pub fn get_logs_page(&self, offset: usize, limit: usize, filter: &LogFilter) -> LogPage {
        let logs = self.logs.lock().unwrap();
        let mut total = 0usize;
        let mut page: Vec<DnsQueryLog> = Vec::new();

        for log in logs.iter() {
            if !filter.matches(log) {
                continue;
            }
            if total >= offset && page.len() < limit {
                page.push(log.clone());
            }
            total += 1;
        }

        LogPage {
            total,
            retained: logs.len(),
            capacity: MAX_LOG_ENTRIES,
            logs: page,
        }
    }

    pub fn get_stats(&self) -> (u64, u64, u64, f64) {
        let stats = self.stats.lock().unwrap();
        let avg_latency = if stats.total_queries > 0 {
            stats.total_latency_ms as f64 / stats.total_queries as f64
        } else {
            0.0
        };
        (
            stats.total_queries,
            stats.blocked_queries,
            stats.cached_queries,
            avg_latency,
        )
    }

    pub fn clear_logs(&self) {
        if let Ok(mut logs) = self.logs.lock() {
            logs.clear();
        }
    }

    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    pub async fn get_config(&self) -> AppConfig {
        self.config.lock().unwrap().clone()
    }

    /// 同一问题只记录一次告警
    ///
    /// 这类问题（例如某分组没有可用上游）会在每次查询时重复出现，
    /// 不去重会把日志刷爆；配置变更会重建 handler，标识随之清空。
    fn warn_once(&self, key: &str, message: &str) {
        let mut warned = self.warned_messages.lock().unwrap();
        if warned.insert(key.to_string()) {
            warn!("{}", message);
        }
    }

    /// 获取流量统计数据
    pub fn get_traffic_stats(&self) -> TrafficStats {
        // 锁顺序必须与 record_success 等保持一致（stats -> traffic），
        // 否则与 record_* 并发时会死锁。
        let stats = self.stats.lock().unwrap();
        let traffic = self.traffic_stats.lock().unwrap();

        // 构建时间线：固定最近 60 分钟，按时间正序
        //
        // 固定长度有两个好处：图表横轴稳定；这个接口被首页每 2 秒轮询一次，
        // 返回时长固定才不会随时间线性增长。
        // 逐分钟向前查表而不是直接遍历 map，跨零点时顺序同样正确。
        let now_minute = TrafficStatsCollector::minute_of_day() as i64;
        let timeline: Vec<TimeBucket> = (0..TIMELINE_MINUTES)
            .rev()
            .map(|offset| {
                let minute = (now_minute - offset).rem_euclid(24 * 60) as u32;
                let (total, blocked, cached) = traffic
                    .minute_buckets
                    .get(&minute)
                    .copied()
                    .unwrap_or((0, 0, 0));
                TimeBucket {
                    time: format!("{:02}:{:02}", minute / 60, minute % 60),
                    total,
                    blocked,
                    cached,
                }
            })
            .collect();

        // 构建 Top 10 域名
        //
        // 次数相同时按域名升序兜底：domain_counts 是 HashMap，迭代顺序每次随机，
        // 只按次数排序会让并列域名在图表里反复跳位（首页每 2 秒刷新一次）。
        let mut domain_vec: Vec<(String, u64)> = traffic
            .domain_counts
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        domain_vec.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let top_domains: Vec<DomainStat> = domain_vec
            .into_iter()
            .take(10)
            .map(|(domain, count)| DomainStat { domain, count })
            .collect();

        // 构建延迟分布
        let latency_ranges = [
            "0-10ms",
            "10-50ms",
            "50-100ms",
            "100-200ms",
            "200-500ms",
            "500ms+",
        ];
        let latency_dist: Vec<LatencyDistribution> = latency_ranges
            .iter()
            .zip(traffic.latency_buckets.iter())
            .map(|(range, &count)| LatencyDistribution {
                range: range.to_string(),
                count,
            })
            .collect();

        // 计算 QPS
        let elapsed_secs = traffic
            .start_time
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(1.0);
        let qps = if elapsed_secs > 0.0 {
            stats.total_queries as f64 / elapsed_secs
        } else {
            0.0
        };

        TrafficStats {
            timeline,
            top_domains,
            latency_dist,
            total_queries: stats.total_queries,
            queries_per_second: qps,
        }
    }

    /// 获取缓存统计信息
    pub fn get_cache_stats(&self) -> crate::dns::cache::CacheStats {
        self.cache.get_stats()
    }

    /// 清理过期缓存
    pub fn cleanup_expired_cache(&self) {
        self.cache.cleanup_expired();
    }

    /// 获取连接池统计信息
    pub fn get_pool_stats(&self) -> crate::dns::pool::PoolStats {
        self.pool.get_stats()
    }

    /// 清理过期的空闲连接（由后台定时任务调用）
    pub fn cleanup_idle_connections(&self) {
        self.pool.cleanup_idle();
    }

    /// 记录合并请求（请求合并命中）
    fn record_coalesced(&self, domain: &str, qtype: &str, start: Instant) {
        let mut stats = self.stats.lock().unwrap();
        stats.total_queries += 1;

        let mut counter = self.log_id_counter.lock().unwrap();
        let id = *counter;
        *counter += 1;

        let log = DnsQueryLog {
            id,
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            domain: domain.to_string(),
            query_type: qtype.to_string(),
            response: "coalesced".to_string(),
            upstream: "coalesced".to_string(),
            latency_ms: start.elapsed().as_millis() as u64,
            action: "coalesced".to_string(),
            group: String::new(),
        };

        self.push_log(log);

        // 更新流量统计（合并的请求不计入延迟分布）
        if let Ok(mut traffic) = self.traffic_stats.lock() {
            traffic.record_to_bucket(false, false);
            traffic.record_domain(domain);
        }

        // 追随者的响应来自领导者的上游查询，同样需要留痕
        info!(
            "DNS查询(请求合并): {} {} 由同域名的并发查询代为完成 ({}ms)",
            domain,
            qtype,
            start.elapsed().as_millis()
        );
    }

    /// 通知所有等待中的追随者（领导者完成查询后调用）
    async fn notify_pending(&self, cache_key: &str, response: Option<Vec<u8>>) {
        if let Some(state) = self.pending_queries.lock().await.remove(cache_key) {
            if !state.waiters.is_empty() {
                debug!(
                    "请求合并: {} 通知 {} 个追随者 (成功={})",
                    cache_key,
                    state.waiters.len(),
                    response.is_some()
                );
            }
            for waiter in state.waiters {
                let _ = waiter.send(response.clone());
            }
        }
    }
}

// TrafficStatsCollector 实现
impl TrafficStatsCollector {
    /// 当前时刻对应的「当天第几分钟」
    fn minute_of_day() -> u32 {
        use chrono::Timelike;
        let now = chrono::Local::now();
        now.hour() * 60 + now.minute()
    }

    /// 记录查询到时间桶
    fn record_to_bucket(&mut self, is_blocked: bool, is_cached: bool) {
        let now_minute = Self::minute_of_day();
        let entry = self.minute_buckets.entry(now_minute).or_insert((0, 0, 0));
        entry.0 += 1; // total
        if is_blocked {
            entry.1 += 1; // blocked
        }
        if is_cached {
            entry.2 += 1; // cached
        }

        // 按真实时间差裁剪，跨零点用回绕取模
        self.minute_buckets.retain(|minute, _| {
            let elapsed = (now_minute as i64 - *minute as i64).rem_euclid(24 * 60);
            elapsed < TIMELINE_MINUTES
        });
    }

    /// 记录域名查询
    fn record_domain(&mut self, domain: &str) {
        *self.domain_counts.entry(domain.to_string()).or_insert(0) += 1;

        // 只保留 Top 100 域名，避免内存溢出
        if self.domain_counts.len() > 100 {
            // 找到最小计数并移除
            if let Some(min_domain) = self
                .domain_counts
                .iter()
                .min_by_key(|(_, count)| *count)
                .map(|(domain, _)| domain.clone())
            {
                self.domain_counts.remove(&min_domain);
            }
        }
    }

    /// 记录延迟
    fn record_latency(&mut self, latency_ms: u64) {
        let bucket = match latency_ms {
            0..=10 => 0,
            11..=50 => 1,
            51..=100 => 2,
            101..=200 => 3,
            201..=500 => 4,
            _ => 5,
        };
        self.latency_buckets[bucket] += 1;
    }
}

/// 从公共服务获取公网 IP
async fn fetch_public_ip(client: &reqwest::Client) -> Option<IpAddr> {
    // 尝试多个服务，提高成功率
    let services = [
        "https://api.ipify.org",
        "https://ip.sb",
        "https://ifconfig.me/ip",
        "https://icanhazip.com",
        "https://checkip.amazonaws.com",
    ];

    for service in &services {
        match tokio::time::timeout(Duration::from_secs(3), client.get(*service).send()).await {
            Ok(Ok(resp)) => {
                if let Ok(text) = resp.text().await {
                    let ip_str = text.trim();
                    if let Ok(ip) = ip_str.parse::<IpAddr>() {
                        debug!("从 {} 获取到公网 IP: {}", service, ip);
                        return Some(ip);
                    }
                }
            }
            _ => continue,
        }
    }

    warn!("所有公网 IP 服务均不可用");
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_cached_does_not_deadlock() {
        let handler = Arc::new(DnsHandler::new(
            Arc::new(Mutex::new(AppConfig::default())),
            Arc::new(DnsCache::new(16, Duration::from_secs(300))),
        ));
        let worker = handler.clone();
        let (tx, rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            worker.record_cached("example.com", "A", "1.2.3.4", Instant::now());
            tx.send(()).unwrap();
        });

        rx.recv_timeout(Duration::from_millis(250))
            .expect("缓存命中记录不应阻塞");
        assert_eq!(handler.get_stats().2, 1);
    }

    #[test]
    fn log_buffer_is_capped_on_every_record_path() {
        let handler = DnsHandler::new(
            Arc::new(Mutex::new(AppConfig::default())),
            Arc::new(DnsCache::new(16, Duration::from_secs(300))),
        );
        let start = Instant::now();

        // 缓存命中与阻止路径过去缺少截断，这里确认都不会突破上限
        for _ in 0..(MAX_LOG_ENTRIES + 50) {
            handler.record_cached("cache.example", "A", "1.2.3.4", start);
            handler.record_blocked("blocked.example", "A", "blocklist", start);
        }

        assert_eq!(handler.get_logs(MAX_LOG_ENTRIES * 2).len(), MAX_LOG_ENTRIES);
    }

    #[test]
    fn incremental_logs_only_return_newer_entries() {
        let handler = DnsHandler::new(
            Arc::new(Mutex::new(AppConfig::default())),
            Arc::new(DnsCache::new(16, Duration::from_secs(300))),
        );
        let start = Instant::now();
        handler.record_cached("first.example", "A", "1.2.3.4", start);
        let cursor = handler.get_logs(1)[0].id;
        handler.record_cached("second.example", "A", "1.2.3.4", start);

        let fresh = handler.get_logs_since(cursor);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].domain, "second.example");
    }

    #[test]
    fn recent_logs_are_limited_by_requested_count() {
        let handler = DnsHandler::new(
            Arc::new(Mutex::new(AppConfig::default())),
            Arc::new(DnsCache::new(16, Duration::from_secs(300))),
        );
        let start = Instant::now();
        for _ in 0..20 {
            handler.record_cached("limit.example", "A", "1.2.3.4", start);
        }

        assert_eq!(handler.get_logs(10).len(), 10);
        // 请求条数超过实际条数时不应报错
        assert_eq!(handler.get_logs(1000).len(), 20);
    }

    /// 分页取日志：总数覆盖整段缓冲区，末页只剩余数条，翻页不重叠
    #[test]
    fn paged_logs_cover_the_whole_buffer() {
        let handler = handler();
        let start = Instant::now();
        for index in 0..26 {
            handler.record_cached(&format!("cache{}.example", index), "A", "1.2.3.4", start);
        }

        let first = handler.get_logs_page(0, 10, &LogFilter::default());
        assert_eq!(first.total, 26, "总数必须是整段缓冲区里的条数");
        assert_eq!(first.retained, 26);
        assert_eq!(first.capacity, MAX_LOG_ENTRIES);
        assert_eq!(first.logs.len(), 10);
        // 日志按 id 降序存放：第 1 页首条是最新的那条
        assert_eq!(first.logs[0].domain, "cache25.example");

        let second = handler.get_logs_page(10, 10, &LogFilter::default());
        assert_eq!(second.logs.len(), 10);
        assert_eq!(
            second.logs[0].id + 10,
            first.logs[0].id,
            "相邻页必须严格错开，不能重复同一条"
        );

        let last = handler.get_logs_page(20, 10, &LogFilter::default());
        assert_eq!(last.logs.len(), 6, "末页只剩余数条");
        // 越过末页时只返回空页，不应报错
        assert!(handler
            .get_logs_page(100, 10, &LogFilter::default())
            .logs
            .is_empty());
    }

    /// 筛选在后端做：总数是整段缓冲区里命中的条数，而不是当前页里的条数
    #[test]
    fn paged_logs_respect_filters() {
        let handler = handler();
        let start = Instant::now();
        for index in 0..26 {
            handler.record_cached(&format!("cache{}.example", index), "A", "1.2.3.4", start);
        }
        handler.record_blocked("ads.example", "A", "blocklist", start);

        let by_action = handler.get_logs_page(
            0,
            10,
            &LogFilter {
                action: Some("blocked".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(by_action.total, 1, "命中数要按整段缓冲区统计");
        assert_eq!(by_action.logs[0].domain, "ads.example");

        // 关键字调用方统一转小写（命令层做），这里按小写匹配；
        // cache1 命中 cache1 与 cache10..cache19
        let by_keyword = handler.get_logs_page(
            0,
            5,
            &LogFilter {
                keyword: "cache1".to_string(),
                ..Default::default()
            },
        );
        assert_eq!(by_keyword.total, 11);
        assert_eq!(by_keyword.logs.len(), 5, "分页窗口仍然要生效");

        let by_type = handler.get_logs_page(
            0,
            10,
            &LogFilter {
                query_type: Some("AAAA".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(by_type.total, 0);
        assert!(by_type.logs.is_empty());
    }

    /// 代理分组的查询拿不到上游答复时也要计入统计
    ///
    /// 过去这条路径既不记失败也不记账（`!is_proxy_request` 直接把整条查询放过），
    /// 于是代理分组没有可用上游期间，仪表盘的「总查询数」会停着不动。
    #[tokio::test]
    async fn proxy_request_without_answer_is_counted_as_failure() {
        let mut config = AppConfig::default();
        // 默认分组设为 proxy，即可让它被判为「代理请求」；
        // 上游清空，让转发立刻返回「没有可用服务器」，测试不碰真实网络
        config.proxy.default_group = "proxy".to_string();
        config.upstream.clear();
        let handler = DnsHandler::new(
            Arc::new(Mutex::new(config)),
            Arc::new(DnsCache::new(16, Duration::from_secs(300))),
        );

        let query = build_query("proxy.example.com", RecordType::A)
            .to_bytes()
            .unwrap();
        // 代理请求失败时不回 SERVFAIL（交给代理软件），但仍要留痕
        assert!(handler.handle_query(&query).await.is_none());

        let (total, _, _, _) = handler.get_stats();
        assert_eq!(total, 1, "代理请求失败必须计入总查询数");
        let logs = handler.get_logs_since(0);
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].action, "failed");
    }

    #[test]
    fn warn_once_records_each_key_only_once() {
        let handler = DnsHandler::new(
            Arc::new(Mutex::new(AppConfig::default())),
            Arc::new(DnsCache::new(16, Duration::from_secs(300))),
        );

        handler.warn_once("group_without_server:proxy", "分组没有可用服务器");
        handler.warn_once("group_without_server:proxy", "分组没有可用服务器");
        handler.warn_once("no_enabled_server", "没有启用的DNS服务器");

        assert_eq!(handler.warned_messages.lock().unwrap().len(), 2);
    }

    /// 构造一条指定类型的查询报文
    fn build_query(domain: &str, query_type: RecordType) -> Message {
        let mut message = Message::new();
        message.set_id(0x4321);
        message.set_message_type(MessageType::Query);
        message.set_recursion_desired(true);

        let mut query = trust_dns_proto::op::Query::new();
        query.set_name(trust_dns_client::rr::Name::from_ascii(domain).expect("合法域名"));
        query.set_query_type(query_type);
        message.add_query(query);

        message
    }

    fn handler() -> DnsHandler {
        DnsHandler::new(
            Arc::new(Mutex::new(AppConfig::default())),
            Arc::new(DnsCache::new(16, Duration::from_secs(300))),
        )
    }

    /// 首页趋势图的横轴必须固定为 60 个点，否则每 2 秒轮询的返回体
    /// 会随运行时间线性增长，图表也会越画越密
    #[test]
    fn timeline_always_has_exactly_sixty_buckets() {
        let handler = handler();
        handler.record_success("a.example.com", "A", "1.1.1.1", "阿里 DNS", 5, "domestic");

        let stats = handler.get_traffic_stats();

        assert_eq!(
            stats.timeline.len(),
            TIMELINE_MINUTES as usize,
            "时间线长度必须固定"
        );
        // 最后一格是当前分钟，且记录了刚才那次查询
        let last = stats.timeline.last().expect("应有最后一格");
        assert_eq!(last.total, 1, "刚记录的查询应落在当前分钟");
    }

    /// 趋势图的数据来源：域名排行要按次数降序，延迟分布要能对上总数
    #[test]
    fn traffic_stats_feed_the_dashboard_charts() {
        let handler = handler();

        for _ in 0..3 {
            handler.record_success("hot.example.com", "A", "1.1.1.1", "阿里 DNS", 5, "domestic");
        }
        handler.record_success(
            "cold.example.com",
            "A",
            "2.2.2.2",
            "阿里 DNS",
            120,
            "domestic",
        );
        handler.record_blocked("ads.example.com", "A", "blocklist", Instant::now());

        let stats = handler.get_traffic_stats();

        // 按次数降序；cold 与 ads 都是 1 次，此时按域名升序兜底。
        // 必须断言完整顺序：HashMap 迭代顺序随机，只按次数排会让并列域名
        // 在每次刷新之间跳位，而这个接口被首页每 2 秒轮询一次。
        let rank: Vec<(&str, u64)> = stats
            .top_domains
            .iter()
            .map(|item| (item.domain.as_str(), item.count))
            .collect();
        assert_eq!(
            rank,
            vec![
                ("hot.example.com", 3),
                ("ads.example.com", 1),
                ("cold.example.com", 1),
            ],
            "并列次数的域名必须按域名稳定排序"
        );

        // 延迟分布：5ms 落 0-10ms 桶 3 次，120ms 落 100-200ms 桶 1 次
        assert_eq!(stats.latency_dist.len(), 6, "延迟分布应有 6 个区间");
        assert_eq!(stats.latency_dist[0].range, "0-10ms");
        assert_eq!(stats.latency_dist[0].count, 3);
        assert_eq!(stats.latency_dist[3].range, "100-200ms");
        assert_eq!(stats.latency_dist[3].count, 1);

        assert_eq!(stats.total_queries, 5);
        assert!(stats.queries_per_second >= 0.0);
    }

    /// 跨零点时不能把当天刚产生的桶当成过期数据删掉
    ///
    /// 旧实现用 "HH:MM" 做字典序比较，00:05 < 23:30，于是 0 点到 1 点之间
    /// 每次记录都会立刻清空时间线，趋势图恒为空。
    #[test]
    fn minute_bucket_pruning_survives_midnight_wrap() {
        let mut collector = TrafficStatsCollector::default();

        // 模拟 23:58 与 23:59 的历史桶，以及新一天 00:01 的记录
        collector.minute_buckets.insert(23 * 60 + 58, (7, 1, 2));
        collector.minute_buckets.insert(23 * 60 + 59, (9, 0, 3));
        collector.minute_buckets.insert(1, (1, 0, 0));

        // 以 00:01 为当前时刻做一次裁剪
        let now_minute = 1i64;
        collector.minute_buckets.retain(|minute, _| {
            let elapsed = (now_minute - *minute as i64).rem_euclid(24 * 60);
            elapsed < TIMELINE_MINUTES
        });

        assert_eq!(
            collector.minute_buckets.get(&1).copied(),
            Some((1, 0, 0)),
            "当天新产生的桶不能被删掉"
        );
        assert!(
            collector.minute_buckets.contains_key(&(23 * 60 + 59)),
            "跨零点时前一分钟仍属于最近 60 分钟，应保留"
        );
    }

    #[test]
    fn traffic_stats_spans_midnight_in_chronological_order() {
        let mut collector = TrafficStatsCollector::default();
        collector.start_time = Some(Instant::now());
        // 昨天 23:59 与今天 00:00 各一条
        collector.minute_buckets.insert(23 * 60 + 59, (5, 0, 0));
        collector.minute_buckets.insert(0, (6, 0, 0));

        // 以 00:00 为当前时刻构造时间线，跨越零点
        let now_minute = 0i64;
        let timeline: Vec<(String, u64)> = (0..TIMELINE_MINUTES)
            .rev()
            .map(|offset| {
                let minute = (now_minute - offset).rem_euclid(24 * 60) as u32;
                let (total, _, _) = collector
                    .minute_buckets
                    .get(&minute)
                    .copied()
                    .unwrap_or((0, 0, 0));
                (format!("{:02}:{:02}", minute / 60, minute % 60), total)
            })
            .collect();

        assert_eq!(timeline.len(), TIMELINE_MINUTES as usize);
        // 倒数第二格应是昨天 23:59，排在今天 00:00 之前
        assert_eq!(timeline[timeline.len() - 2], ("23:59".to_string(), 5));
        assert_eq!(timeline[timeline.len() - 1], ("00:00".to_string(), 6));
    }

    /// 停用期间 geosite 订阅里的规则不得参与分流
    ///
    /// 这条同时锁住两个后果：域名不会被路由到（可能为空的）proxy 分组，
    /// 也就不会被判为「代理请求」而跳过缓存。
    #[test]
    fn disabled_geosite_rules_do_not_route() {
        let mut config = AppConfig::default();
        config.subscriptions = vec![Subscription {
            name: "Proxy 需代理域名".to_string(),
            url: String::new(),
            enabled: true,
            rules: vec!["routed.example.com".to_string()],
            last_updated: None,
            sub_type: SubscriptionType::Geosite,
            target_group: Some("proxy".to_string()),
        }];

        let handler = DnsHandler::new(
            Arc::new(Mutex::new(config)),
            Arc::new(DnsCache::new(16, Duration::from_secs(300))),
        );

        assert!(
            !GEOSITE_ROUTING_ENABLED,
            "当前阶段约定全部走直连，若改为 true 请同步更新本用例与规则页"
        );
        assert_eq!(
            handler.check_geosite("routed.example.com"),
            None,
            "域名路由停用期间不得把域名分流到其它分组"
        );
        assert_eq!(
            handler.check_geosite("sub.routed.example.com"),
            None,
            "父域名匹配同样不得生效"
        );
    }

    /// 黑名单不受域名路由停用影响，广告过滤必须照常工作
    #[test]
    fn blocklist_still_applies_while_geosite_is_disabled() {
        let mut config = AppConfig::default();
        config.subscriptions = vec![
            Subscription {
                name: "广告拦截域名".to_string(),
                url: String::new(),
                enabled: true,
                rules: vec!["ads.example.com".to_string()],
                last_updated: None,
                sub_type: SubscriptionType::Blocklist,
                target_group: None,
            },
            Subscription {
                name: "Proxy 需代理域名".to_string(),
                url: String::new(),
                enabled: true,
                rules: vec!["normal.example.com".to_string()],
                last_updated: None,
                sub_type: SubscriptionType::Geosite,
                target_group: Some("proxy".to_string()),
            },
        ];

        let handler = DnsHandler::new(
            Arc::new(Mutex::new(config)),
            Arc::new(DnsCache::new(16, Duration::from_secs(300))),
        );

        assert!(
            handler.is_blocked("ads.example.com"),
            "广告过滤订阅必须仍然生效"
        );
        assert!(
            handler.is_blocked("sub.ads.example.com"),
            "黑名单的父域名匹配必须仍然生效"
        );
        assert!(!handler.is_blocked("normal.example.com"));
    }

    /// 自造响应必须是合法响应报文，否则客户端会当垃圾包丢掉
    #[test]
    fn self_built_responses_set_the_qr_bit() {
        let handler = handler();
        let query = build_query("ads.example.com", RecordType::A);

        for bytes in [
            handler.create_blocked_response(&query),
            handler.create_nodata_response(&query),
            handler.create_nxdomain_response(&query),
            handler.create_servfail_response(&query),
        ] {
            let response = Message::from_bytes(&bytes).expect("响应必须可解析");
            assert_eq!(
                response.message_type(),
                MessageType::Response,
                "自造响应必须置 QR=1，Message::new() 默认是 Query"
            );
            assert_eq!(response.id(), 0x4321, "必须保留原始事务 ID");
            assert_eq!(response.queries().len(), 1, "必须回显查询段");
        }
    }

    #[test]
    fn blocked_a_query_returns_blackhole_address() {
        let handler = handler();
        let bytes = handler.create_blocked_response(&build_query("ads.example.com", RecordType::A));
        let response = Message::from_bytes(&bytes).unwrap();

        assert_eq!(response.response_code(), ResponseCode::NoError);
        assert_eq!(response.answers().len(), 1);
        assert_eq!(
            response.answers()[0].data().map(|d| d.to_string()),
            Some("0.0.0.0".to_string()),
            "A 查询应返回 0.0.0.0"
        );
    }

    #[test]
    fn blocked_aaaa_query_returns_blackhole_address_instead_of_an_a_record() {
        let handler = handler();
        let bytes =
            handler.create_blocked_response(&build_query("ads.example.com", RecordType::AAAA));
        let response = Message::from_bytes(&bytes).unwrap();

        assert_eq!(response.answers().len(), 1);
        assert_eq!(
            response.answers()[0].record_type(),
            RecordType::AAAA,
            "AAAA 查询不能拿到 A 记录"
        );
        assert_eq!(
            response.answers()[0].data().map(|d| d.to_string()),
            Some("::".to_string())
        );
    }

    #[test]
    fn blocked_query_of_other_types_returns_empty_answer() {
        let handler = handler();
        let bytes =
            handler.create_blocked_response(&build_query("ads.example.com", RecordType::MX));
        let response = Message::from_bytes(&bytes).unwrap();

        assert_eq!(response.response_code(), ResponseCode::NoError);
        assert!(
            response.answers().is_empty(),
            "没有黑洞语义的类型应回空答案（NODATA）"
        );
    }

    #[test]
    fn blocklist_rule_type_matches_domain_and_subdomains() {
        let mut config = AppConfig::default();
        config.rules = vec![crate::config::Rule {
            name: "拦截示例".to_string(),
            pattern: "ads.example.com".to_string(),
            rule_type: RuleType::Blocklist,
            action: RuleAction::Block,
            target: None,
            enabled: true,
            priority: 1,
        }];
        let handler = DnsHandler::new(
            Arc::new(Mutex::new(config)),
            Arc::new(DnsCache::new(16, Duration::from_secs(300))),
        );

        assert!(
            handler.check_rules("ads.example.com").is_some(),
            "应匹配域名本身"
        );
        assert!(
            handler.check_rules("cdn.ads.example.com").is_some(),
            "应匹配子域名"
        );
        assert!(
            handler.check_rules("notads.example.com").is_none(),
            "不应误伤同后缀但不同标签的域名"
        );
        assert!(
            handler.check_rules("example.com").is_none(),
            "不应把父域名一起拦掉"
        );
    }

    /// 构造一条查询，可指定 ID 与 EDNS0 载荷尺寸
    fn build_query_with_id(domain: &str, id: u16, udp_payload: Option<u16>) -> Vec<u8> {
        let mut message = build_query(domain, RecordType::A);
        message.set_id(id);

        if let Some(payload) = udp_payload {
            let mut edns = Edns::new();
            edns.set_max_payload(payload);
            edns.set_dnssec_ok(true);
            message.set_edns(edns);
        }

        message.to_bytes().expect("查询序列化失败")
    }

    /// 构造一条「别的查询」产生的响应，模拟缓存命中与请求合并借用的字节
    fn foreign_response(domain: &str, foreign_id: u16) -> Vec<u8> {
        let mut message = Message::new();
        message.set_id(foreign_id);
        message.set_message_type(MessageType::Response);
        message.set_recursion_available(true);

        let mut query = trust_dns_proto::op::Query::new();
        query.set_name(trust_dns_client::rr::Name::from_ascii(domain).expect("合法域名"));
        query.set_query_type(RecordType::A);
        message.add_query(query);
        message.add_answer(Record::from_rdata(
            trust_dns_client::rr::Name::from_ascii(domain).expect("合法域名"),
            300,
            RData::A(trust_dns_client::rr::rdata::A(std::net::Ipv4Addr::new(
                1, 2, 3, 4,
            ))),
        ));

        message.to_bytes().expect("响应序列化失败")
    }

    /// 缓存命中与请求合并返回的是别的查询的响应字节，必须重写成当前查询的 ID，
    /// 否则客户端会判为非法报文（Windows 解析器报 Bad DNS packet 并丢弃）
    #[test]
    fn borrowed_response_gets_the_current_query_id() {
        let query_bytes = build_query_with_id("cached.example.com", 0xbeef, None);
        let response = foreign_response("cached.example.com", 0x0abc);

        let normalized = normalize_response_for_client(&query_bytes, response);
        let message = Message::from_bytes(&normalized).expect("规范化后必须可解析");

        assert_eq!(
            message.id(),
            0xbeef,
            "回包事务 ID 必须是本次查询的 ID，而不是被借用响应的旧 ID"
        );
        assert_eq!(message.answers().len(), 1, "答案内容不应被改动");
    }

    /// 上游查询必须向上游声明 4096，而不是沿用客户端在 UDP 上声明的小尺寸
    #[test]
    fn upstream_query_raises_advertised_payload_size() {
        let client_query = build_query_with_id("example.com", 0x1234, Some(512));

        let upstream = prepare_upstream_query(&client_query);
        let message = Message::from_bytes(&upstream).expect("上游查询必须可解析");
        let edns = message.extensions().as_ref().expect("应保留 EDNS0");

        assert_eq!(
            edns.max_payload(),
            UPSTREAM_EDNS_PAYLOAD,
            "传给上游的尺寸必须是 4096，否则上游按 512 截断、TCP 重试也会拿到 TC"
        );
        assert!(edns.dnssec_ok(), "抬高尺寸不应丢掉客户端的 DO 位");
    }

    /// 客户端没带 EDNS0 时，上游查询要补上 OPT，才能向支持 EDNS0 的上游取全量答案
    #[test]
    fn upstream_query_adds_edns_when_client_omitted_it() {
        let client_query = build_query_with_id("example.com", 0x1234, None);

        let upstream = prepare_upstream_query(&client_query);
        let message = Message::from_bytes(&upstream).expect("上游查询必须可解析");
        let edns = message.extensions().as_ref().expect("应补上 EDNS0");

        assert_eq!(edns.max_payload(), UPSTREAM_EDNS_PAYLOAD);
    }

    /// 客户端已声明不小于上限时不应重新编码，原样透传
    #[test]
    fn upstream_query_is_untouched_when_client_already_advertises_enough() {
        let client_query = build_query_with_id("example.com", 0x1234, Some(4096));

        assert_eq!(
            prepare_upstream_query(&client_query),
            client_query,
            "尺寸已够大时不应改动报文"
        );
    }

    /// RFC 6891 §7：请求方没带 OPT 时不得回带 OPT。本代理为取全量会主动加 OPT，
    /// 因此回给客户端的报文必须按客户端情况摘掉
    #[test]
    fn opt_is_stripped_for_clients_that_did_not_use_edns() {
        let query_bytes = build_query_with_id("example.com", 0x1234, None);

        // 用一条带上游 OPT 的响应模拟上游回包
        let mut response_message =
            Message::from_bytes(&foreign_response("example.com", 0x1234)).expect("响应可解析");
        let mut edns = Edns::new();
        edns.set_max_payload(UPSTREAM_EDNS_PAYLOAD);
        response_message.set_edns(edns);
        let response_with_opt = response_message.to_bytes().unwrap();

        let normalized = normalize_response_for_client(&query_bytes, response_with_opt);
        let message = Message::from_bytes(&normalized).expect("规范化后必须可解析");

        assert!(
            message.extensions().is_none(),
            "客户端没用 EDNS0，回包不应带 OPT"
        );
    }

    /// EDNS0 客户端应保留上游回带的 OPT，不能一并摘掉
    #[test]
    fn opt_is_kept_for_edns_clients() {
        let query_bytes = build_query_with_id("example.com", 0x1234, Some(4096));
        let mut response_message =
            Message::from_bytes(&foreign_response("example.com", 0x1234)).expect("响应可解析");
        let mut edns = Edns::new();
        edns.set_max_payload(UPSTREAM_EDNS_PAYLOAD);
        response_message.set_edns(edns);
        let response_with_opt = response_message.to_bytes().unwrap();

        let normalized = normalize_response_for_client(&query_bytes, response_with_opt);
        let message = Message::from_bytes(&normalized).expect("规范化后必须可解析");

        assert!(
            message.extensions().is_some(),
            "EDNS0 客户端的回包应保留 OPT"
        );
    }

    /// ID 已一致且客户端使用 EDNS0 时应走快路径，不重新编码上游原包
    #[test]
    fn matching_response_for_edns_client_is_passed_through_unchanged() {
        let query_bytes = build_query_with_id("example.com", 0x1234, Some(4096));
        let response = foreign_response("example.com", 0x1234);

        assert_eq!(
            normalize_response_for_client(&query_bytes, response.clone()),
            response,
            "无需改写时不应触碰上游原包"
        );
    }
}
