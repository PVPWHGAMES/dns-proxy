pub mod server;
pub mod handler;
pub mod cache;
pub mod ecs;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsQueryLog {
    pub id: u64,
    pub timestamp: String,
    pub domain: String,
    pub query_type: String,
    pub response: String,
    pub upstream: String,
    pub latency_ms: u64,
    pub action: String,
    #[serde(default)]
    pub group: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsStats {
    pub total_queries: u64,
    pub blocked_queries: u64,
    pub cached_queries: u64,
    pub avg_latency: f64,
    pub is_running: bool,
}

// 重新导出流量统计类型
pub use handler::{TrafficStats, TimeBucket, DomainStat, LatencyDistribution};

// 重新导出缓存统计类型
pub use cache::CacheStats;
