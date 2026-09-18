use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use trust_dns_proto::op::Message;

/// 取响应中可缓存记录的最小 TTL
///
/// 只统计答案段与权威段：附加段多是胶水记录，不应缩短整条响应的缓存寿命。
/// 返回 None 表示响应里没有可缓存的记录。
///
/// 注意 OPT 记录不在这些段里 —— trust-dns 把它单独存放在 `edns` 字段，
/// OPT 的「TTL」字段实际承载扩展 RCODE 与 DO 位，绝不能被当成 TTL 参与计算。
pub fn min_record_ttl(message: &Message) -> Option<u32> {
    message
        .answers()
        .iter()
        .chain(message.name_servers().iter())
        .map(|record| record.ttl())
        .min()
}

/// 计算实际缓存时长
///
/// 缓存寿命由权威服务器给出的 TTL 决定，配置项只作为上限，避免超长缓存。
/// `cache_ttl` 设为 0 时表示不设上限，直接使用响应中的原始 TTL（使用果冻解析时建议保持为 0）。
/// TTL 为 0 表示不可缓存（RFC 2181 §8）。
pub fn effective_cache_ttl(message: &Message, cap: Duration) -> Option<Duration> {
    let ttl = min_record_ttl(message)?;
    if ttl == 0 {
        return None;
    }
    if cap.is_zero() {
        // cache_ttl = 0 表示无上限，完全信任权威服务器给出的 TTL
        Some(Duration::from_secs(u64::from(ttl)))
    } else {
        Some(Duration::from_secs(u64::from(ttl)).min(cap))
    }
}

/// 按驻留时间扣减响应中所有记录的 TTL
///
/// 命中缓存时若原样回带缓存那一刻的 TTL，下游会以为记录永远新鲜：
/// 客户端不会过期、上游变更也永远不会被感知。
fn apply_elapsed_ttl(message: &mut Message, elapsed_secs: u32) {
    for record in message.answers_mut().iter_mut() {
        record.set_ttl(record.ttl().saturating_sub(elapsed_secs));
    }
    for record in message.name_servers_mut().iter_mut() {
        record.set_ttl(record.ttl().saturating_sub(elapsed_secs));
    }
    // OPT 不在附加段（单独存于 edns 字段），这里不会破坏扩展 RCODE 与 DO 位
    for record in message.additionals_mut().iter_mut() {
        record.set_ttl(record.ttl().saturating_sub(elapsed_secs));
    }
}

/// 把响应中各记录的 TTL 截断到上限以内
///
/// 上限来自配置项，而不是条目寿命。条目寿命取整条响应的最小 TTL，
/// 若拿它去截断每条记录，会把 TTL 较长的记录（同响应里常见的 CDN CNAME）
/// 一并压到最小值，下游因此过早丢弃仍然有效的记录并反复回查。
/// 只按配置上限收敛：既保留各记录自身的权威 TTL，又避免超长 TTL 传下去。
/// `cap_secs` 为 0 时表示不设上限，跳过截断（此时完全信任权威服务器的 TTL）。
fn clamp_record_ttls(message: &mut Message, cap_secs: u32) {
    if cap_secs == 0 {
        return;
    }
    let clamp = |records: &mut Vec<trust_dns_proto::rr::Record>| {
        for record in records.iter_mut() {
            if record.ttl() > cap_secs {
                record.set_ttl(cap_secs);
            }
        }
    };

    clamp(message.answers_mut());
    clamp(message.name_servers_mut());
    clamp(message.additionals_mut());
}

/// 缓存条目
#[derive(Clone)]
pub struct CacheEntry {
    pub response: Message,
    pub created_at: Instant,
    pub ttl: Duration,
}

/// 缓存统计信息
#[derive(Debug, Clone, serde::Serialize)]
pub struct CacheStats {
    /// 总查询次数
    pub total_queries: u64,
    /// 缓存命中次数
    pub cache_hits: u64,
    /// 缓存未命中次数
    pub cache_misses: u64,
    /// 命中率 (0.0 - 1.0)
    pub hit_rate: f64,
    /// 当前缓存大小
    pub current_size: usize,
    /// 最大缓存容量
    pub max_size: usize,
}

/// LRU DNS 缓存
pub struct DnsCache {
    entries: Mutex<LruCache<String, CacheEntry>>,
    max_size: usize,
    default_ttl: Duration,
    // 统计信息
    total_queries: Mutex<u64>,
    cache_hits: Mutex<u64>,
    cache_misses: Mutex<u64>,
}

impl DnsCache {
    pub fn new(max_size: usize, default_ttl: Duration) -> Self {
        let capacity =
            NonZeroUsize::new(max_size.max(1)).unwrap_or(NonZeroUsize::new(1000).unwrap());
        Self {
            entries: Mutex::new(LruCache::new(capacity)),
            max_size,
            default_ttl,
            total_queries: Mutex::new(0),
            cache_hits: Mutex::new(0),
            cache_misses: Mutex::new(0),
        }
    }

    /// 从缓存获取条目
    ///
    /// 命中时回带的 TTL 会按驻留时间扣减，过期条目直接移除。
    pub fn get(&self, key: &str) -> Option<Message> {
        // 更新统计
        {
            let mut total = self.total_queries.lock().unwrap();
            *total += 1;
        }

        let mut entries = self.entries.lock().ok()?;

        let hit = match entries.get(key) {
            Some(entry) => {
                let elapsed = entry.created_at.elapsed();
                if elapsed < entry.ttl {
                    Some((entry.response.clone(), elapsed))
                } else {
                    None
                }
            }
            None => None,
        };

        match hit {
            Some((mut response, elapsed)) => {
                {
                    let mut hits = self.cache_hits.lock().unwrap();
                    *hits += 1;
                }
                apply_elapsed_ttl(&mut response, elapsed.as_secs().min(u32::MAX as u64) as u32);
                Some(response)
            }
            None => {
                // 过期条目与不存在的 key 都会走到这里，pop 对后者是空操作
                entries.pop(key);
                drop(entries);

                let mut misses = self.cache_misses.lock().unwrap();
                *misses += 1;
                None
            }
        }
    }

    /// 存入缓存
    ///
    /// - `lifetime`：条目寿命，决定这条响应在本进程里驻留多久
    /// - `ttl_cap`：回带 TTL 的上限，写入时据此收敛各记录的 TTL
    ///
    /// 两者分开：寿命取整条响应的最小 TTL（保证不过期），而回带 TTL 只受配置上限
    /// 约束（保留每条记录自身的权威 TTL）。
    pub fn put(&self, key: String, mut response: Message, lifetime: Duration, ttl_cap: Duration) {
        if let Ok(mut entries) = self.entries.lock() {
            clamp_record_ttls(&mut response, ttl_cap.as_secs().min(u32::MAX as u64) as u32);
            entries.put(
                key,
                CacheEntry {
                    response,
                    created_at: Instant::now(),
                    ttl: lifetime,
                },
            );
        }
    }

    /// 使用默认 TTL 存入缓存
    pub fn put_with_default_ttl(&self, key: String, response: Message) {
        self.put(key, response, self.default_ttl, self.default_ttl);
    }

    /// 清空缓存
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
        // 重置统计
        if let Ok(mut total) = self.total_queries.lock() {
            *total = 0;
        }
        if let Ok(mut hits) = self.cache_hits.lock() {
            *hits = 0;
        }
        if let Ok(mut misses) = self.cache_misses.lock() {
            *misses = 0;
        }
    }

    /// 获取当前缓存大小
    pub fn size(&self) -> usize {
        self.entries.lock().map(|e| e.len()).unwrap_or(0)
    }

    /// 获取缓存统计信息
    pub fn get_stats(&self) -> CacheStats {
        let total = *self.total_queries.lock().unwrap();
        let hits = *self.cache_hits.lock().unwrap();
        let misses = *self.cache_misses.lock().unwrap();
        let current_size = self.size();

        let hit_rate = if total > 0 {
            hits as f64 / total as f64
        } else {
            0.0
        };

        CacheStats {
            total_queries: total,
            cache_hits: hits,
            cache_misses: misses,
            hit_rate,
            current_size,
            max_size: self.max_size,
        }
    }

    /// 清理过期条目
    pub fn cleanup_expired(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            let now = Instant::now();
            let mut expired_keys = Vec::new();

            // 找出所有过期的 key
            for (key, entry) in entries.iter() {
                if now.duration_since(entry.created_at) >= entry.ttl {
                    expired_keys.push(key.clone());
                }
            }

            // 移除过期条目
            for key in expired_keys {
                entries.pop(&key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trust_dns_client::rr::{Name, RData, Record, RecordType};
    use trust_dns_proto::op::{Edns, MessageType, Query};

    fn build_response(domain: &str, ttl: u32) -> Message {
        let mut message = Message::new();
        message.set_id(0x1234);
        message.set_message_type(MessageType::Response);
        message.set_recursion_available(true);

        let mut query = Query::new();
        query.set_name(Name::from_ascii(domain).expect("合法域名"));
        query.set_query_type(RecordType::A);
        message.add_query(query);
        message.add_answer(Record::from_rdata(
            Name::from_ascii(domain).expect("合法域名"),
            ttl,
            RData::A(trust_dns_client::rr::rdata::A(std::net::Ipv4Addr::new(
                1, 2, 3, 4,
            ))),
        ));

        message
    }

    #[test]
    fn cache_lifetime_follows_the_record_ttl_when_below_cap() {
        let response = build_response("short.example.com", 30);
        let ttl = effective_cache_ttl(&response, Duration::from_secs(300));

        assert_eq!(
            ttl,
            Some(Duration::from_secs(30)),
            "记录 TTL 小于配置上限时应以记录 TTL 为准，否则会被缓存过久导致解析到旧 IP"
        );
    }

    #[test]
    fn configured_ttl_caps_the_cache_lifetime() {
        let response = build_response("long.example.com", 86400);
        let ttl = effective_cache_ttl(&response, Duration::from_secs(300));

        assert_eq!(
            ttl,
            Some(Duration::from_secs(300)),
            "记录 TTL 超过配置上限时应被配置值截断"
        );
    }

    #[test]
    fn zero_ttl_response_is_not_cached() {
        let response = build_response("no-cache.example.com", 0);

        assert_eq!(
            effective_cache_ttl(&response, Duration::from_secs(300)),
            None,
            "TTL 为 0 表示不可缓存（RFC 2181 §8）"
        );
    }

    #[test]
    fn response_without_records_is_not_cached() {
        let mut response = Message::new();
        response.set_message_type(MessageType::Response);

        assert_eq!(
            effective_cache_ttl(&response, Duration::from_secs(300)),
            None,
            "没有记录就没有 TTL 依据，不应缓存"
        );
    }

    #[test]
    fn min_record_ttl_takes_the_smallest_across_answers_and_authorities() {
        let mut response = build_response("mixed.example.com", 600);
        response.add_name_server(Record::from_rdata(
            Name::from_ascii("example.com").expect("合法域名"),
            120,
            RData::A(trust_dns_client::rr::rdata::A(std::net::Ipv4Addr::new(
                5, 6, 7, 8,
            ))),
        ));

        assert_eq!(
            min_record_ttl(&response),
            Some(120),
            "整条响应的寿命由最小 TTL 决定"
        );
    }

    /// 命中缓存时要按驻留时间扣减 TTL
    fn wind_back(cache: &DnsCache, key: &str, seconds: u64) {
        let mut entries = cache.entries.lock().unwrap();
        let entry = entries.get_mut(key).expect("条目应存在");
        entry.created_at = Instant::now() - Duration::from_secs(seconds);
    }

    #[test]
    fn cache_hit_returns_decremented_ttl() {
        let cache = DnsCache::new(16, Duration::from_secs(300));
        cache.put(
            "k".to_string(),
            build_response("ttl.example.com", 100),
            Duration::from_secs(100),
            Duration::from_secs(300),
        );

        wind_back(&cache, "k", 40);

        let served = cache.get("k").expect("应命中");
        assert_eq!(
            served.answers()[0].ttl(),
            60,
            "回带 TTL 必须是剩余时间，否则下游永远看不到记录过期"
        );
    }

    /// 配置上限必须约束「告诉下游能缓存多久」
    #[test]
    fn stored_ttl_is_clamped_to_the_configured_cap() {
        let cache = DnsCache::new(16, Duration::from_secs(300));
        cache.put(
            "k".to_string(),
            build_response("long.example.com", 86400),
            Duration::from_secs(300),
            Duration::from_secs(300),
        );

        let served = cache.get("k").expect("应命中");
        assert_eq!(
            served.answers()[0].ttl(),
            300,
            "权威 TTL 长达一天时，不能原样告知下游，否则上限形同虚设"
        );
    }

    /// TTL 较长的记录不能被同响应里的最小 TTL 压掉
    ///
    /// 条目寿命取最小 TTL（短 TTL 记录先过期），但 CNAME 这类长 TTL 记录
    /// 仍应保有自己的权威 TTL，否则下游会过早丢弃仍然有效的记录并反复回查。
    #[test]
    fn longer_ttl_records_keep_their_own_ttl() {
        let mut response = build_response("cdn.example.com", 11);
        response.add_answer(Record::from_rdata(
            Name::from_ascii("cdn.example.com").expect("合法域名"),
            174,
            RData::CNAME(trust_dns_client::rr::rdata::CNAME(
                Name::from_ascii("edge.example.net").expect("合法域名"),
            )),
        ));

        let lifetime = effective_cache_ttl(&response, Duration::from_secs(300)).expect("可缓存");
        assert_eq!(lifetime, Duration::from_secs(11), "寿命应取最小 TTL");

        let cache = DnsCache::new(16, Duration::from_secs(300));
        cache.put(
            "k".to_string(),
            response,
            lifetime,
            Duration::from_secs(300),
        );

        let served = cache.get("k").expect("应命中");
        let cname_ttl = served
            .answers()
            .iter()
            .find(|record| record.record_type() == RecordType::CNAME)
            .expect("应保留 CNAME 记录")
            .ttl();
        assert_eq!(
            cname_ttl, 174,
            "CNAME 的权威 TTL 不应被同响应里的最小 TTL 压低"
        );
    }

    /// 小于上限的记录 TTL 不应被改写
    #[test]
    fn stored_ttl_below_the_cap_is_preserved() {
        let cache = DnsCache::new(16, Duration::from_secs(300));
        cache.put(
            "k".to_string(),
            build_response("short.example.com", 30),
            Duration::from_secs(30),
            Duration::from_secs(300),
        );

        let served = cache.get("k").expect("应命中");
        assert_eq!(served.answers()[0].ttl(), 30);
    }

    #[test]
    fn cache_entry_expires_once_its_lifetime_passes() {
        let cache = DnsCache::new(16, Duration::from_secs(300));
        cache.put(
            "k".to_string(),
            build_response("ttl.example.com", 30),
            Duration::from_secs(30),
            Duration::from_secs(300),
        );

        wind_back(&cache, "k", 31);

        assert!(cache.get("k").is_none(), "超过缓存寿命后不应再命中");
    }

    /// 扣减 TTL 时绝不能碰 OPT 记录：它的「TTL」字段承载扩展 RCODE 与 DO 位
    #[test]
    fn decrementing_ttl_does_not_corrupt_the_opt_record() {
        let mut response = build_response("edns.example.com", 100);
        let mut edns = Edns::new();
        edns.set_max_payload(4096);
        edns.set_dnssec_ok(true);
        response.set_edns(edns);

        apply_elapsed_ttl(&mut response, 40);

        let edns = response.extensions().as_ref().expect("OPT 必须仍在");
        assert_eq!(edns.max_payload(), 4096, "OPT 载荷尺寸不得被 TTL 扣减破坏");
        assert!(edns.dnssec_ok(), "DO 位不得被 TTL 扣减破坏");
        assert_eq!(response.answers()[0].ttl(), 60);
    }

    #[test]
    fn ttl_decrement_saturates_instead_of_underflowing() {
        let mut response = build_response("old.example.com", 10);

        // 驻留时间超过记录 TTL 时不得回绕成巨大数值
        apply_elapsed_ttl(&mut response, 9999);

        assert_eq!(response.answers()[0].ttl(), 0);
    }
}
