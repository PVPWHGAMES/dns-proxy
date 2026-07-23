use crate::tun::TunConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerGroup {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub proxy: ProxyConfig,
    pub upstream: Vec<DnsServer>,
    pub rules: Vec<Rule>,
    pub subscriptions: Vec<Subscription>,
    pub subscription_update_interval: u64,  // 订阅更新间隔（分钟）
    pub latency_test_interval: u64,  // 延迟测试间隔（秒），0表示禁用
    pub log: LogConfig,
    pub strategy: DnsStrategy,
    #[serde(default = "default_server_groups")]
    pub server_groups: Vec<ServerGroup>,
    #[serde(default)]
    pub tun: TunConfig,
}

fn default_server_groups() -> Vec<ServerGroup> {
    vec![
        ServerGroup { name: "default".to_string(), description: "默认组".to_string() },
        ServerGroup { name: "domestic".to_string(), description: "国内域名".to_string() },
        ServerGroup { name: "foreign".to_string(), description: "国外域名".to_string() },
        ServerGroup { name: "proxy".to_string(), description: "代理 (ClashVerge)".to_string() },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub listen_address: String,
    pub listen_port: u16,
    pub protocol: String,
    pub cache_size: usize,
    pub cache_ttl: u64,
    pub auto_start: bool,
    pub block_ipv6: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsServer {
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub enabled: bool,
    pub protocol: DnsProtocol,
    pub doh_url: Option<String>,
    #[serde(default = "default_group")]
    pub group: String,
}

fn default_group() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DnsProtocol {
    Udp,
    Tcp,
    Doh,
    Dot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DnsStrategy {
    Sequential,
    Fastest,
    LoadBalance,
    Parallel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    pub pattern: String,
    pub rule_type: RuleType,
    pub action: RuleAction,
    pub target: Option<String>,
    pub enabled: bool,
    pub priority: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RuleType {
    Exact,
    Wildcard,
    Regex,
    Blocklist,  // 黑名单模式
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Forward,
    Block,
    BlockNull,   // 返回0.0.0.0
    BlockNxdomain, // 返回NXDOMAIN
    Cache,
}

// 订阅类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionType {
    Blocklist,  // 黑名单（广告拦截）
    Geosite,    // 域名路由（国内外分流）
}

// 订阅
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub rules: Vec<String>,  // 缓存的规则列表
    pub last_updated: Option<String>,
    #[serde(default = "default_sub_type")]
    pub sub_type: SubscriptionType,
    #[serde(default)]
    pub target_group: Option<String>,  // geosite 类型的目标服务器组
}

fn default_sub_type() -> SubscriptionType {
    SubscriptionType::Blocklist
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    pub level: String,
    pub file: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            proxy: ProxyConfig {
                listen_address: "0.0.0.0".to_string(),
                listen_port: 53,
                protocol: "both".to_string(),
                cache_size: 1000,
                cache_ttl: 300,
                auto_start: false,
                block_ipv6: false,
            },
            upstream: vec![
                DnsServer {
                    name: "阿里 DNS".to_string(),
                    ip: "223.5.5.5".to_string(),
                    port: 53,
                    enabled: true,
                    protocol: DnsProtocol::Udp,
                    doh_url: Some("https://dns.alidns.com/dns-query".to_string()),
                    group: "domestic".to_string(),
                },
                DnsServer {
                    name: "Cloudflare".to_string(),
                    ip: "1.1.1.1".to_string(),
                    port: 53,
                    enabled: true,
                    protocol: DnsProtocol::Udp,
                    doh_url: Some("https://cloudflare-dns.com/dns-query".to_string()),
                    group: "foreign".to_string(),
                },
                DnsServer {
                    name: "Google".to_string(),
                    ip: "8.8.8.8".to_string(),
                    port: 53,
                    enabled: false,
                    protocol: DnsProtocol::Udp,
                    doh_url: Some("https://dns.google/dns-query".to_string()),
                    group: "foreign".to_string(),
                },
            ],
            server_groups: vec![
                ServerGroup { name: "default".to_string(), description: "默认组".to_string() },
                ServerGroup { name: "domestic".to_string(), description: "国内域名".to_string() },
                ServerGroup { name: "foreign".to_string(), description: "国外域名".to_string() },
                ServerGroup { name: "proxy".to_string(), description: "代理 (ClashVerge)".to_string() },
            ],
            rules: Vec::new(),
            subscriptions: Vec::new(),
            subscription_update_interval: 120,  // 默认2小时
            latency_test_interval: 300,  // 默认5分钟
            log: LogConfig {
                level: "info".to_string(),
                file: None,
            },
            strategy: DnsStrategy::Fastest,
            tun: TunConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn config_path() -> PathBuf {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("dns-proxy");
        std::fs::create_dir_all(&config_dir).ok();
        config_dir.join("config.toml")
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            toml::from_str(&content).unwrap_or_default()
        } else {
            let config = Self::default();
            config.save().ok();
            config
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path();
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}
