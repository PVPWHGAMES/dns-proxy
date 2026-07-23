use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use trust_dns_proto::op::Message;

#[derive(Clone)]
pub struct CacheEntry {
    pub response: Message,
    pub created_at: Instant,
    pub ttl: Duration,
}

pub struct DnsCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
    max_size: usize,
}

impl DnsCache {
    pub fn new(max_size: usize, _default_ttl: Duration) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            max_size,
        }
    }

    pub fn get(&self, key: &str) -> Option<Message> {
        let mut entries = self.entries.lock().ok()?;
        if let Some(entry) = entries.get(key) {
            if entry.created_at.elapsed() < entry.ttl {
                return Some(entry.response.clone());
            } else {
                entries.remove(key);
            }
        }
        None
    }

    pub fn put(&self, key: String, response: Message, ttl: Duration) {
        if let Ok(mut entries) = self.entries.lock() {
            // 如果缓存已满，移除最旧的条目
            if entries.len() >= self.max_size {
                if let Some(oldest_key) = entries
                    .iter()
                    .min_by_key(|(_, v)| v.created_at)
                    .map(|(k, _)| k.clone())
                {
                    entries.remove(&oldest_key);
                }
            }
            entries.insert(
                key,
                CacheEntry {
                    response,
                    created_at: Instant::now(),
                    ttl,
                },
            );
        }
    }

    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }

    pub fn size(&self) -> usize {
        self.entries.lock().map(|e| e.len()).unwrap_or(0)
    }
}
