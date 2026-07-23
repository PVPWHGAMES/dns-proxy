use crate::config::{AppConfig, DnsProtocol, DnsStrategy, RuleAction, RuleType, Subscription, SubscriptionType};
use crate::dns::cache::DnsCache;
use crate::dns::DnsQueryLog;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tracing::{info, warn};
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
}

#[derive(Default, Clone)]
pub struct QueryStats {
    pub total_queries: u64,
    pub blocked_queries: u64,
    pub cached_queries: u64,
    pub total_latency_ms: u64,
}

impl DnsHandler {
    pub fn new(config: AppConfig, cache: Arc<DnsCache>) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .pool_max_idle_per_host(4)
            .build()
            .unwrap_or_default();

        let blocklist = Arc::new(Mutex::new(HashSet::new()));
        let geosite_map = Arc::new(Mutex::new(HashMap::new()));

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
        let mut forward_group: Option<String> = match rule_result {
            Some((RuleAction::Block, _, _)) | Some((RuleAction::BlockNull, _, _)) => {
                let config = self.config.lock().unwrap();
                self.record_blocked(&query_name, &format!("{:?}", query_type), "rule", start);
                return Some(self.create_blocked_response(&query, &config.proxy.listen_address));
            }
            Some((RuleAction::BlockNxdomain, _, _)) => {
                self.record_blocked(&query_name, &format!("{:?}", query_type), "rule:nxdomain", start);
                return Some(self.create_nxdomain_response(&query));
            }
            Some((RuleAction::Forward, _, target)) => target,
            _ => None,
        };

        // ② 黑名单订阅（自定义规则未命中时生效）
        if forward_group.is_none() && self.is_blocked(&query_name) {
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

        // 自定义规则未指定分组时，检查 geosite 域名路由
        if forward_group.is_none() {
            forward_group = self.check_geosite(&query_name);
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
            let cache_key = format!("{}:{:?}", query_name, query_type);
            if let Ok(response_msg) = Message::from_bytes(response_bytes) {
                let ttl = Duration::from_secs(self.config.lock().unwrap().proxy.cache_ttl);
                self.cache.put(cache_key, response_msg, ttl);
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
                    if let Some(response) = self.forward_to_server(query_bytes, server).await {
                        return (Some(response), server.name.clone());
                    }
                }
                (None, "none".to_string())
            }
            DnsStrategy::Fastest => {
                // 真正的最快策略：返回第一个成功的响应
                let futures: Vec<std::pin::Pin<Box<dyn futures::Future<Output = Option<(Vec<u8>, String)>> + Send>>> = servers
                    .iter()
                    .map(|s| {
                        let s = s.clone();
                        let bytes = query_bytes.to_vec();
                        Box::pin(async move {
                            let result = self.forward_to_server(&bytes, &s).await;
                            result.map(|r| (r, s.name.clone()))
                        }) as std::pin::Pin<Box<dyn futures::Future<Output = Option<(Vec<u8>, String)>> + Send>>
                    })
                    .collect();

                // 使用 select_all 返回第一个完成的 future
                let (result, _index, _remaining) = futures::future::select_all(futures).await;
                match result {
                    Some((resp, name)) => (Some(resp), name),
                    None => (None, "none".to_string()),
                }
            }
            DnsStrategy::Parallel => {
                // 并行策略：同时发送到所有服务器，返回第一个成功的响应
                let futures: Vec<std::pin::Pin<Box<dyn futures::Future<Output = Option<(Vec<u8>, String)>> + Send>>> = servers
                    .iter()
                    .map(|s| {
                        let s = s.clone();
                        let bytes = query_bytes.to_vec();
                        Box::pin(async move {
                            let result = self.forward_to_server(&bytes, &s).await;
                            result.map(|r| (r, s.name.clone()))
                        }) as std::pin::Pin<Box<dyn futures::Future<Output = Option<(Vec<u8>, String)>> + Send>>
                    })
                    .collect();

                // 使用 select_all 返回第一个完成的 future
                let (result, _index, _remaining) = futures::future::select_all(futures).await;
                match result {
                    Some((resp, name)) => (Some(resp), name),
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
                let result = self.forward_to_server(query_bytes, server).await;
                (result, name)
            }
        }
    }

    async fn forward_to_server(
        &self,
        query_bytes: &[u8],
        server: &crate::config::DnsServer,
    ) -> Option<Vec<u8>> {
        match server.protocol {
            DnsProtocol::Udp | DnsProtocol::Tcp => {
                self.forward_udp(query_bytes, &server.ip, server.port).await
            }
            DnsProtocol::Doh => {
                let url = server
                    .doh_url
                    .as_deref()
                    .unwrap_or("https://cloudflare-dns.com/dns-query");
                self.forward_doh(query_bytes, url).await
            }
            DnsProtocol::Dot => {
                self.forward_udp(query_bytes, &server.ip, server.port).await
            }
        }
    }

    async fn forward_udp(
        &self,
        query_bytes: &[u8],
        ip: &str,
        port: u16,
    ) -> Option<Vec<u8>> {
        let addr = format!("{}:{}", ip, port);
        let socket = UdpSocket::bind("0.0.0.0:0").await.ok()?;
        socket.connect(&addr).await.ok()?;

        socket.send(query_bytes).await.ok()?;

        let mut buf = vec![0u8; 512];
        let timeout = Duration::from_secs(2);

        match tokio::time::timeout(timeout, socket.recv(&mut buf)).await {
            Ok(Ok(len)) => Some(buf[..len].to_vec()),
            _ => {
                warn!("转发DNS查询超时: {}", addr);
                None
            }
        }
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

    pub async fn get_config(&self) -> AppConfig {
        self.config.lock().unwrap().clone()
    }
}
