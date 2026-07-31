use crate::config::{AppConfig, DnsProtocol, DnsStrategy, RuleAction, RuleType, Subscription, SubscriptionType};
use crate::dns::cache::DnsCache;
use crate::dns::ecs;
use crate::dns::pool::DnsConnectionPool;
use crate::dns::DnsQueryLog;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;
use tracing::{info, warn, debug};
use trust_dns_client::op::Message;
use trust_dns_client::rr::{Record, RecordType, RData};
use trust_dns_proto::serialize::binary::{BinDecodable, BinEncodable};

pub struct DnsHandler {
    config: Arc<Mutex<AppConfig>>,
    cache: Arc<DnsCache>,
    logs: Arc<Mutex<Vec<DnsQueryLog>>>,
    stats: Arc<Mutex<QueryStats>>,
    log_id_counter: Arc<Mutex<u64>>,
    strategy_index: Arc<Mutex<usize>>,
    http_client: Arc<reqwest::Client>,
    blocklist: Arc<Mutex<HashSet<String>>>,  // 黑名单域名集合
    geosite_map: Arc<Mutex<HashMap<String, String>>>,  // 域名 -> 目标分组
    server_latency: Arc<Mutex<HashMap<String, ServerLatency>>>,  // 服务器延迟统计
    public_ip: Arc<Mutex<Option<IpAddr>>>,  // 自动获取的公网 IP
    public_ip_last_update: Arc<Mutex<Option<Instant>>>,  // 上次更新时间
    traffic_stats: Arc<Mutex<TrafficStatsCollector>>,  // 流量统计收集器
    /// 上游连接池（DoT 长连接 + UDP socket 复用）
    pool: Arc<DnsConnectionPool>,
    /// 请求合并：等待中的查询 (cache_key -> 追随者列表)
    pending_queries: Arc<AsyncMutex<HashMap<String, PendingQueryState>>>,
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
    pub time: String,      // "HH:MM" 格式
    pub total: u64,        // 总查询数
    pub blocked: u64,      // 阻止数
    pub cached: u64,       // 缓存命中数
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
    pub range: String,     // "0-10ms", "10-50ms", etc.
    pub count: u64,
}

/// 流量统计数据（返回给前端）
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrafficStats {
    pub timeline: Vec<TimeBucket>,           // 时间线数据
    pub top_domains: Vec<DomainStat>,        // Top 10 域名
    pub latency_dist: Vec<LatencyDistribution>, // 延迟分布
    pub total_queries: u64,
    pub queries_per_second: f64,
}

/// 时间序列数据收集器
#[derive(Default)]
pub struct TrafficStatsCollector {
    // 按分钟统计的时间线 (格式: "HH:MM" -> (total, blocked, cached))
    pub minute_buckets: BTreeMap<String, (u64, u64, u64)>,
    // 域名计数
    pub domain_counts: HashMap<String, u64>,
    // 延迟分布
    pub latency_buckets: [u64; 6], // 0-10, 10-50, 50-100, 100-200, 200-500, 500+
    // 启动时间
    pub start_time: Option<Instant>,
}

impl DnsHandler {
    pub fn new(config: AppConfig, cache: Arc<DnsCache>) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .pool_max_idle_per_host(8)
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .unwrap_or_default();

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
            8,                              // 每主机最多 8 个空闲连接
            Duration::from_secs(120),       // 空闲连接 120 秒过期
        ));

        // 加载已有的订阅规则
        let handler = Self {
            config: Arc::new(Mutex::new(config.clone())),
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
        };

        // 初始化黑名单和域名路由
        handler.init_blocklist(&config.subscriptions);
        handler.init_geosite(&config.subscriptions);
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
        map.clear();
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
            config.subscriptions
                .iter()
                .filter(|s| s.enabled)
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
                sub.last_updated = Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
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
                    if first == "0.0.0.0" || first == "127.0.0.1" || first == "::1" || first == "::" {
                        let domain = parts[1].to_lowercase().trim_end_matches('.').to_string();
                        if !domain.is_empty() && domain.contains('.') && !domain.starts_with('#') {
                            return Some(domain);
                        }
                    }
                }

                // 检查是否是纯域名格式 (AdGuard格式: ||domain.com^)
                let line = line.trim();
                if line.starts_with("||") && line.ends_with('^') {
                    let domain = line[2..line.len()-1].to_lowercase();
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

    pub async fn handle_query(
        &self,
        query_bytes: &[u8],
        _src_addr: SocketAddr,
    ) -> Option<Vec<u8>> {
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
                return Some(self.create_blocked_response(&query, &config.proxy.listen_address));
            }
        }

        // ① 自定义规则（最高优先级，覆盖一切订阅）
        let rule_result = self.check_rules(&query_name);
        let is_whitelisted = rule_result.as_ref().map_or(false, |(action, _, _)| *action == RuleAction::Allow);
        let mut forward_group: Option<String> = match rule_result {
            Some((RuleAction::Allow, _, _)) => {
                // 白名单规则，跳过黑名单检查，直接放行
                debug!("白名单放行: {}", query_name);
                None
            }
            Some((RuleAction::Block, _, _)) | Some((RuleAction::BlockNull, _, _)) => {
                let config = self.config.lock().unwrap();
                self.record_blocked(&query_name, &format!("{:?}", query_type), "rule", start);
                return Some(self.create_blocked_response(&query, &config.proxy.listen_address));
            }
            Some((RuleAction::BlockNxdomain, _, _)) => {
                self.record_blocked(&query_name, &format!("{:?}", query_type), "rule:nxdomain", start);
                return Some(self.create_nxdomain_response(&query));
            }
            Some((RuleAction::Forward, _, ref target)) => target.clone(),
            _ => None,
        };

        // ② 黑名单订阅（自定义规则未命中或为白名单时跳过）
        if !is_whitelisted && forward_group.is_none() && self.is_blocked(&query_name) {
            let config = self.config.lock().unwrap();
            self.record_blocked(&query_name, &format!("{:?}", query_type), "blocklist", start);
            return Some(self.create_blocked_response(&query, &config.proxy.listen_address));
        }

        // ③ 缓存
        let cache_key = format!("{}:{:?}", query_name, query_type);
        if let Some(cached) = self.cache.get(&cache_key) {
            self.record_cached(&query_name, &format!("{:?}", query_type), start);
            return cached.to_bytes().ok();
        }

        // ④ 请求合并：避免相同域名+类型的并发查询重复请求上游
        //    使用 Leader-Follower 模式：第一个查询成为 Leader 执行实际转发，
        //    后续相同查询成为 Follower，等待 Leader 的结果
        {
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
                        // 超时或领导者失败，向上返回 None
                        return None;
                    }
                }
            } else {
                // 成为领导者，注册进行中查询
                pending.insert(cache_key.clone(), PendingQueryState {
                    waiters: Vec::new(),
                    started_at: Instant::now(),
                });
            }
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

        // 根据策略选择DNS服务器并转发（如有指定分组则过滤）
        let (response, server_name) = if let Some(ref group) = forward_group {
            self.forward_with_strategy_for_group(query_bytes, group).await
        } else {
            self.forward_with_strategy(query_bytes).await
        };

        if let Some(response_bytes) = &response {
            let response_ip = Message::from_bytes(response_bytes)
                .ok()
                .and_then(|m| {
                    m.answers().first().and_then(|a| {
                        a.data().and_then(|d| {
                            d.to_string()
                                .split_whitespace()
                                .last()
                                .map(String::from)
                        })
                    })
                })
                .unwrap_or_else(|| "-".to_string());

            let latency = start.elapsed().as_millis() as u64;

            // 缓存响应
            if let Ok(response_msg) = Message::from_bytes(response_bytes) {
                let ttl = Duration::from_secs(self.config.lock().unwrap().proxy.cache_ttl);
                self.cache.put(cache_key.clone(), response_msg, ttl);
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
        self.notify_pending(&cache_key, response.clone()).await;

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
                RuleType::Blocklist => false, // 黑名单通过is_blocked检查
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
            warn!("分组 '{}' 没有启用的DNS服务器，回退到默认策略", group);
            return self.forward_with_strategy(query_bytes).await;
        }

        self.do_forward(query_bytes, &strategy, &group_servers).await
    }

    // 根据策略转发请求（使用全部启用的服务器）
    async fn forward_with_strategy(
        &self,
        query_bytes: &[u8],
    ) -> (Option<Vec<u8>>, String) {
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
            warn!("没有启用的DNS服务器");
            return (None, "none".to_string());
        }

        self.do_forward(query_bytes, &strategy, &enabled_servers).await
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
                let futures: Vec<std::pin::Pin<Box<dyn futures::Future<Output = Option<(Vec<u8>, String, Instant)>> + Send>>> = servers
                    .iter()
                    .map(|s| {
                        let s = s.clone();
                        let bytes = query_bytes.to_vec();
                        Box::pin(async move {
                            let start = Instant::now();
                            let result = self.forward_to_server(&bytes, &s).await;
                            result.map(|r| (r, s.name.clone(), start))
                        }) as std::pin::Pin<Box<dyn futures::Future<Output = Option<(Vec<u8>, String, Instant)>> + Send>>
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
                let futures: Vec<std::pin::Pin<Box<dyn futures::Future<Output = Option<(Vec<u8>, String, Instant)>> + Send>>> = servers
                    .iter()
                    .map(|s| {
                        let s = s.clone();
                        let bytes = query_bytes.to_vec();
                        Box::pin(async move {
                            let start = Instant::now();
                            let result = self.forward_to_server(&bytes, &s).await;
                            result.map(|r| (r, s.name.clone(), start))
                        }) as std::pin::Pin<Box<dyn futures::Future<Output = Option<(Vec<u8>, String, Instant)>> + Send>>
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
    fn get_fastest_server(&self, servers: &[crate::config::DnsServer]) -> Option<crate::config::DnsServer> {
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

        match server.protocol {
            DnsProtocol::Udp | DnsProtocol::Tcp => {
                self.forward_udp(&query_with_ecs, &server.ip, server.port).await
            }
            DnsProtocol::Doh => {
                let url = server
                    .doh_url
                    .as_deref()
                    .unwrap_or("https://cloudflare-dns.com/dns-query");
                self.forward_doh(&query_with_ecs, url).await
            }
            DnsProtocol::Dot => {
                // DoT 默认端口为 853
                let port = if server.port == 53 { 853 } else { server.port };
                self.forward_dot(&query_with_ecs, &server.ip, port).await
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

    async fn forward_udp(
        &self,
        query_bytes: &[u8],
        ip: &str,
        port: u16,
    ) -> Option<Vec<u8>> {
        let addr = format!("{}:{}", ip, port);
        self.pool.udp_query(&addr, query_bytes).await
    }

    async fn forward_doh(&self, query_bytes: &[u8], url: &str) -> Option<Vec<u8>> {
        let response = self.http_client
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
    async fn forward_dot(
        &self,
        query_bytes: &[u8],
        ip: &str,
        port: u16,
    ) -> Option<Vec<u8>> {
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

    // 创建阻止响应（返回0.0.0.0）
    fn create_blocked_response(&self, query: &Message, _listen_addr: &str) -> Vec<u8> {
        let mut response = Message::new();
        response.set_id(query.id());
        response.set_response_code(trust_dns_proto::op::ResponseCode::NoError);

        if let Some(q) = query.queries().first() {
            response.add_query(q.clone());

            // 添加A记录指向0.0.0.0
            let record = Record::from_rdata(
                q.name().clone(),
                300,  // TTL 300秒
                RData::A(trust_dns_client::rr::rdata::A(std::net::Ipv4Addr::new(0, 0, 0, 0))),
            );
            response.add_answer(record);
        }

        response.to_bytes().unwrap_or_default()
    }

    // 创建NXDOMAIN响应
    fn create_nxdomain_response(&self, query: &Message) -> Vec<u8> {
        let mut response = Message::new();
        response.set_id(query.id());
        response.set_response_code(trust_dns_proto::op::ResponseCode::NXDomain);

        if let Some(q) = query.queries().first() {
            response.add_query(q.clone());
        }

        response.to_bytes().unwrap_or_default()
    }

    fn record_success(&self, domain: &str, qtype: &str, response: &str, upstream: &str, latency: u64, group: &str) {
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

        if let Ok(mut logs) = self.logs.lock() {
            logs.insert(0, log);
            if logs.len() > 1000 {
                logs.truncate(1000);
            }
        }

        // 更新流量统计
        if let Ok(mut traffic) = self.traffic_stats.lock() {
            traffic.record_to_bucket(false, false);
            traffic.record_domain(domain);
            traffic.record_latency(latency);
        }

        info!("DNS查询: {} {} -> {} via {} ({}ms)", domain, qtype, response, upstream, latency);
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

        if let Ok(mut logs) = self.logs.lock() {
            logs.insert(0, log);
        }

        // 更新流量统计
        if let Ok(mut traffic) = self.traffic_stats.lock() {
            traffic.record_to_bucket(true, false);
            traffic.record_domain(domain);
        }

        info!("DNS阻止: {} {}", domain, qtype);
    }

    fn record_cached(&self, domain: &str, qtype: &str, start: Instant) {
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
            response: "cached".to_string(),
            upstream: "cache".to_string(),
            latency_ms: start.elapsed().as_millis() as u64,
            action: "cached".to_string(),
            group: String::new(),
        };

        if let Ok(mut logs) = self.logs.lock() {
            logs.insert(0, log);
        }

        // 更新流量统计
        if let Ok(mut traffic) = self.traffic_stats.lock() {
            traffic.record_to_bucket(false, true);
            traffic.record_domain(domain);
            traffic.record_latency(start.elapsed().as_millis() as u64);
        }
    }

    pub fn get_logs(&self) -> Vec<DnsQueryLog> {
        self.logs.lock().unwrap().clone()
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

    /// 获取流量统计数据
    pub fn get_traffic_stats(&self) -> TrafficStats {
        let traffic = self.traffic_stats.lock().unwrap();
        let stats = self.stats.lock().unwrap();

        // 构建时间线数据
        let timeline: Vec<TimeBucket> = traffic.minute_buckets
            .iter()
            .map(|(time, (total, blocked, cached))| TimeBucket {
                time: time.clone(),
                total: *total,
                blocked: *blocked,
                cached: *cached,
            })
            .collect();

        // 构建 Top 10 域名
        let mut domain_vec: Vec<(String, u64)> = traffic.domain_counts
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        domain_vec.sort_by(|a, b| b.1.cmp(&a.1));
        let top_domains: Vec<DomainStat> = domain_vec
            .into_iter()
            .take(10)
            .map(|(domain, count)| DomainStat { domain, count })
            .collect();

        // 构建延迟分布
        let latency_ranges = ["0-10ms", "10-50ms", "50-100ms", "100-200ms", "200-500ms", "500ms+"];
        let latency_dist: Vec<LatencyDistribution> = latency_ranges
            .iter()
            .zip(traffic.latency_buckets.iter())
            .map(|(range, &count)| LatencyDistribution {
                range: range.to_string(),
                count,
            })
            .collect();

        // 计算 QPS
        let elapsed_secs = traffic.start_time
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

        if let Ok(mut logs) = self.logs.lock() {
            logs.insert(0, log);
            if logs.len() > 1000 {
                logs.truncate(1000);
            }
        }

        // 更新流量统计（合并的请求不计入延迟分布）
        if let Ok(mut traffic) = self.traffic_stats.lock() {
            traffic.record_to_bucket(false, false);
            traffic.record_domain(domain);
        }
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
    /// 记录查询到时间桶
    fn record_to_bucket(&mut self, is_blocked: bool, is_cached: bool) {
        let time_key = chrono::Local::now().format("%H:%M").to_string();
        let entry = self.minute_buckets.entry(time_key).or_insert((0, 0, 0));
        entry.0 += 1; // total
        if is_blocked {
            entry.1 += 1; // blocked
        }
        if is_cached {
            entry.2 += 1; // cached
        }

        // 只保留最近 60 分钟的数据
        let cutoff = (chrono::Local::now() - chrono::Duration::minutes(60))
            .format("%H:%M")
            .to_string();
        while let Some(first_key) = self.minute_buckets.keys().next().cloned() {
            if first_key < cutoff {
                self.minute_buckets.remove(&first_key);
            } else {
                break;
            }
        }
    }

    /// 记录域名查询
    fn record_domain(&mut self, domain: &str) {
        *self.domain_counts.entry(domain.to_string()).or_insert(0) += 1;

        // 只保留 Top 100 域名，避免内存溢出
        if self.domain_counts.len() > 100 {
            // 找到最小计数并移除
            if let Some(min_domain) = self.domain_counts
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
