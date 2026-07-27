use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use trust_dns_proto::op::Message;

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
        let capacity = NonZeroUsize::new(max_size.max(1)).unwrap_or(NonZeroUsize::new(1000).unwrap());
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
    pub fn get(&self, key: &str) -> Option<Message> {
        // 更新统计
        {
            let mut total = self.total_queries.lock().unwrap();
            *total += 1;
        }

        let mut entries = self.entries.lock().ok()?;
        if let Some(entry) = entries.get(key) {
            // 检查是否过期
            if entry.created_at.elapsed() < entry.ttl {
                // 更新命中统计
                let mut hits = self.cache_hits.lock().unwrap();
                *hits += 1;
                return Some(entry.response.clone());
            } else {
                // 过期条目，移除
                entries.pop(key);
            }
        }

        // 未命中
        let mut misses = self.cache_misses.lock().unwrap();
        *misses += 1;
        None
    }

    /// 存入缓存
    pub fn put(&self, key: String, response: Message, ttl: Duration) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.put(
                key,
                CacheEntry {
                    response,
                    created_at: Instant::now(),
                    ttl,
                },
            );
        }
    }

    /// 使用默认 TTL 存入缓存
    pub fn put_with_default_ttl(&self, key: String, response: Message) {
        self.put(key, response, self.default_ttl);
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
