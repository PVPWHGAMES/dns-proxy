use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 是否启用 GeoSite 域名路由
///
/// 当前阶段所有查询都走直连（默认分组的上游），域名路由规则不参与决策，因此停用。
/// 停用带来三件事，都由本开关统一控制：
/// - 不加载十几万条规则到内存，也不再把它们下载/写回配置
/// - 命中 proxy 列表的域名不再被判为「代理请求」而跳过缓存与请求合并
/// - 加载配置时顺带清理已停用的 geosite 订阅，避免配置文件被无用数据撑大
///
/// 前端 `src/pages/Rules.tsx` 的 `ROUTING_RULES_ENABLED` 必须与之保持一致。
/// 恢复按域名分流时，把这两处一起改成 true。
pub const GEOSITE_ROUTING_ENABLED: bool = false;

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
    pub subscription_update_interval: u64, // 订阅更新间隔（分钟）
    pub latency_test_interval: u64,        // 延迟测试间隔（秒），0表示禁用
    pub log: LogConfig,
    pub strategy: DnsStrategy,
    #[serde(default)]
    pub start_minimized: bool,
    /// 应用界面字体；旧配置未设置时默认使用微软雅黑。
    #[serde(default = "default_app_font")]
    pub app_font: String,
    /// 登录后延迟启动 DNS 服务的秒数；0 表示立即启动。
    #[serde(default)]
    pub startup_delay_seconds: u64,
    /// DNS 服务定时重启间隔（小时）；0 表示关闭定时重启。
    #[serde(default)]
    pub dns_restart_interval_hours: u64,
    #[serde(default = "default_server_groups")]
    pub server_groups: Vec<ServerGroup>,
    #[serde(default)]
    pub ecs: EcsConfig,
}

/// EDNS Client Subnet (ECS) 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EcsConfig {
    /// 是否启用 ECS
    #[serde(default)]
    pub enabled: bool,
    /// 客户端 IP 地址（如果为 None，则使用系统 IP）
    #[serde(default)]
    pub client_ip: Option<String>,
    /// IPv4 源掩码长度（默认 24，即 /24 子网）
    #[serde(default = "default_ipv4_mask")]
    pub ipv4_source_mask: u8,
    /// IPv6 源掩码长度（默认 56，即 /56 子网）
    #[serde(default = "default_ipv6_mask")]
    pub ipv6_source_mask: u8,
}

impl Default for EcsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            client_ip: None,
            ipv4_source_mask: 24,
            ipv6_source_mask: 56,
        }
    }
}

fn default_app_font() -> String {
    "Microsoft YaHei".to_string()
}

fn default_ipv4_mask() -> u8 {
    24
}

fn default_ipv6_mask() -> u8 {
    56
}

fn default_server_groups() -> Vec<ServerGroup> {
    vec![
        ServerGroup {
            name: "default".to_string(),
            description: "默认组".to_string(),
        },
        ServerGroup {
            name: "domestic".to_string(),
            description: "直连".to_string(),
        },
        ServerGroup {
            name: "proxy".to_string(),
            description: "代理".to_string(),
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub listen_address: String,
    pub listen_port: u16,
    pub protocol: String,
    pub cache_size: usize,
    /// DNS 缓存 TTL（秒）；0 表示无上限、完全信任权威服务器返回的原始 TTL（使用果冻解析时建议保持为 0）
    pub cache_ttl: u64,
    pub block_ipv6: bool,
    /// 默认分组：未匹配任何规则时使用，空字符串表示使用所有服务器
    #[serde(default = "default_default_group")]
    pub default_group: String,
    /// 启动服务时把系统网卡的 DNS 指向本地代理，停止/退出时还原
    #[serde(default = "default_takeover")]
    pub takeover_system_dns: bool,
    /// DoH 主机名的引导解析服务器
    ///
    /// DoH 上游只给域名时，必须先解析出它的地址才能建立 DoH 连接；而此时系统 DNS
    /// 往往已指向本程序，走系统解析器就会递归回自身。这里直连这几台服务器解析。
    ///
    /// 缺省值是两台国内公共 DNS：它们只用来解析 DoH 主机名，不参与正常查询。
    #[serde(default = "default_bootstrap_dns")]
    pub bootstrap_dns: Vec<String>,
}

/// 引导解析服务器的默认值：阿里 DNS 与 DNSPod
fn default_bootstrap_dns() -> Vec<String> {
    vec!["223.5.5.5".to_string(), "119.29.29.29".to_string()]
}

fn default_takeover() -> bool {
    true
}

fn default_default_group() -> String {
    "domestic".to_string()
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
    Blocklist, // 黑名单模式
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Forward,
    Allow, // 白名单，跳过黑名单检查
    Block,
    BlockNull,     // 返回0.0.0.0
    BlockNxdomain, // 返回NXDOMAIN
    Cache,
}

// 订阅类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionType {
    Blocklist, // 黑名单（广告拦截）
    Geosite,   // 域名路由（国内外分流）
}

// 订阅
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub rules: Vec<String>, // 缓存的规则列表
    pub last_updated: Option<String>,
    #[serde(default = "default_sub_type")]
    pub sub_type: SubscriptionType,
    #[serde(default)]
    pub target_group: Option<String>, // geosite 类型的目标服务器组
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
                listen_address: "127.0.0.1".to_string(),
                listen_port: 53,
                protocol: "both".to_string(),
                cache_size: 1000,
                cache_ttl: 0,
                block_ipv6: false,
                default_group: "domestic".to_string(),
                takeover_system_dns: true,
                bootstrap_dns: default_bootstrap_dns(),
            },
            upstream: vec![
                DnsServer {
                    name: "阿里 DNS".to_string(),
                    ip: "223.5.5.5".to_string(),
                    port: 53,
                    enabled: true,
                    protocol: DnsProtocol::Udp,
                    doh_url: None,
                    group: "domestic".to_string(),
                },
                DnsServer {
                    name: "DNSPod".to_string(),
                    ip: "119.29.29.29".to_string(),
                    port: 53,
                    enabled: true,
                    protocol: DnsProtocol::Udp,
                    doh_url: None,
                    group: "domestic".to_string(),
                },
                DnsServer {
                    name: "Clash DNS".to_string(),
                    ip: "127.0.0.1".to_string(),
                    port: 1053,
                    enabled: true,
                    protocol: DnsProtocol::Udp,
                    doh_url: None,
                    group: "proxy".to_string(),
                },
            ],
            server_groups: vec![
                ServerGroup {
                    name: "default".to_string(),
                    description: "默认组".to_string(),
                },
                ServerGroup {
                    name: "domestic".to_string(),
                    description: "直连".to_string(),
                },
                ServerGroup {
                    name: "proxy".to_string(),
                    description: "代理".to_string(),
                },
            ],
            rules: Vec::new(),
            subscriptions: Vec::new(),
            subscription_update_interval: 120, // 默认2小时
            latency_test_interval: 300,        // 默认5分钟
            log: LogConfig {
                level: "info".to_string(),
                file: None,
            },
            strategy: DnsStrategy::Fastest,
            start_minimized: false,
            app_font: default_app_font(),
            startup_delay_seconds: 0,
            dns_restart_interval_hours: 0,
            ecs: EcsConfig::default(),
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
        let mut config = if path.exists() {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            toml::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        };
        config.migrate();
        config
    }

    /// 迁移旧配置：规范化后仅在确有改动时落盘
    fn migrate(&mut self) {
        if self.normalize() {
            self.save().ok();
        }
    }

    /// 规范化历史配置，返回是否发生了改动
    ///
    /// 与落盘分离，便于在测试中直接验证规则而不触碰用户真实配置文件。
    fn normalize(&mut self) -> bool {
        let mut changed = false;

        // 0.0.0.0 会在所有网卡上监听，等于把本机变成局域网开放解析器。
        // 这个程序只服务本机，历史配置一律收回回环地址。
        if self.proxy.listen_address == "0.0.0.0" || self.proxy.listen_address == "::" {
            self.proxy.listen_address = "127.0.0.1".to_string();
            changed = true;
        }

        // 监听的传输协议写错时回退到 udp，避免配置拼写错误导致完全不监听
        if !matches!(self.proxy.protocol.as_str(), "udp" | "tcp" | "both") {
            self.proxy.protocol = "udp".to_string();
            changed = true;
        }

        // 引导服务器写错会让 DoH 主机名解析不出去：统一修剪、去重并丢掉无效项。
        // 全被丢空时补回默认值——空列表等于放弃引导解析，不该由一次笔误造成。
        {
            let before = self.proxy.bootstrap_dns.clone();
            let mut cleaned: Vec<String> = Vec::new();
            for entry in &before {
                let trimmed = entry.trim();
                if trimmed.is_empty() || crate::dns::bootstrap::parse_server(trimmed).is_none() {
                    continue;
                }
                if !cleaned.iter().any(|existing| existing == trimmed) {
                    cleaned.push(trimmed.to_string());
                }
            }
            if cleaned.is_empty() {
                cleaned = default_bootstrap_dns();
            }
            if cleaned != before {
                self.proxy.bootstrap_dns = cleaned;
                changed = true;
            }
        }

        // 域名路由已停用时清掉对应的订阅及其缓存的规则
        //
        // 这些规则不参与任何计算，却会让配置文件膨胀到三十多万行，
        // 每次保存都要整份重写。恢复分流时在规则页重新添加预设即可。
        if !GEOSITE_ROUTING_ENABLED {
            let before = self.subscriptions.len();
            let removed_rules: usize = self
                .subscriptions
                .iter()
                .filter(|sub| sub.sub_type == SubscriptionType::Geosite)
                .map(|sub| sub.rules.len())
                .sum();
            self.subscriptions
                .retain(|sub| sub.sub_type != SubscriptionType::Geosite);

            if self.subscriptions.len() != before {
                tracing::info!(
                    removed_subscriptions = before - self.subscriptions.len(),
                    removed_rules,
                    "域名路由已停用，清理 geosite 订阅及其规则"
                );
                changed = true;
            }
        }

        // 将 upstream 中 group="foreign" 改为 "proxy"
        for server in &mut self.upstream {
            if server.group == "foreign" {
                server.group = "proxy".to_string();
                changed = true;
            }
        }

        // 将订阅中 target_group="foreign" 改为 "proxy"
        for sub in &mut self.subscriptions {
            if sub.target_group.as_deref() == Some("foreign") {
                sub.target_group = Some("proxy".to_string());
                changed = true;
            }
        }

        // 移除 foreign 分组
        let before = self.server_groups.len();
        self.server_groups.retain(|g| g.name != "foreign");
        if self.server_groups.len() != before {
            changed = true;
        }

        // 更新分组描述
        for group in &mut self.server_groups {
            match group.name.as_str() {
                "domestic" => {
                    if group.description != "直连" {
                        group.description = "直连".to_string();
                        changed = true;
                    }
                }
                "proxy" => {
                    if group.description != "代理" {
                        group.description = "代理".to_string();
                        changed = true;
                    }
                }
                _ => {}
            }
        }

        // 确保 domestic 和 proxy 分组存在
        if !self.server_groups.iter().any(|g| g.name == "domestic") {
            self.server_groups.push(ServerGroup {
                name: "domestic".to_string(),
                description: "直连".to_string(),
            });
            changed = true;
        }
        if !self.server_groups.iter().any(|g| g.name == "proxy") {
            self.server_groups.push(ServerGroup {
                name: "proxy".to_string(),
                description: "代理".to_string(),
            });
            changed = true;
        }

        changed
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::config_path();
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_pins_wildcard_listen_address_back_to_loopback() {
        for wildcard in ["0.0.0.0", "::"] {
            let mut config = AppConfig::default();
            config.proxy.listen_address = wildcard.to_string();

            assert!(config.normalize());
            assert_eq!(
                config.proxy.listen_address, "127.0.0.1",
                "全网卡监听必须收回回环，否则本机变成局域网开放解析器"
            );
        }
    }

    #[test]
    fn normalize_keeps_explicitly_configured_listen_address() {
        let mut config = AppConfig::default();
        config.proxy.listen_address = "192.168.1.10".to_string();

        // 用户显式指定的地址不属于规范化范围，仅在 wildcard 时改写
        let before = config.proxy.listen_address.clone();
        config.normalize();
        assert_eq!(config.proxy.listen_address, before);
    }

    #[test]
    fn bootstrap_dns_defaults_to_two_public_resolvers() {
        assert_eq!(
            AppConfig::default().proxy.bootstrap_dns,
            vec!["223.5.5.5".to_string(), "119.29.29.29".to_string()]
        );
    }

    /// 老配置里没有这个字段时必须补上默认值，而不是留空导致 DoH 解析不出去
    #[test]
    fn bootstrap_dns_is_filled_in_for_configs_that_lack_the_field() {
        // 用默认配置序列化出真实 TOML，再删掉这一行，模拟升级前写下的配置文件。
        // 手写一段精简 TOML 是不行的：AppConfig 还有若干没有默认值的字段。
        let text = toml::to_string(&AppConfig::default()).expect("默认配置应可序列化");
        let without_field = text
            .lines()
            .filter(|line| !line.trim_start().starts_with("bootstrap_dns"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !without_field.contains("bootstrap_dns"),
            "前置条件失败：样例里仍有该字段"
        );

        let config: AppConfig = toml::from_str(&without_field).expect("老配置应当能解析");
        assert_eq!(
            config.proxy.bootstrap_dns,
            default_bootstrap_dns(),
            "缺少 bootstrap_dns 字段时应使用默认值"
        );
    }

    #[test]
    fn normalize_cleans_bootstrap_dns_entries() {
        let mut config = AppConfig::default();
        config.proxy.bootstrap_dns = vec![
            " 223.5.5.5 ".to_string(),
            "223.5.5.5".to_string(),
            "".to_string(),
            "dns.example.com".to_string(),
            "119.29.29.29:5353".to_string(),
        ];

        assert!(config.normalize(), "修剪与去重应被视为配置改动");
        assert_eq!(
            config.proxy.bootstrap_dns,
            vec!["223.5.5.5".to_string(), "119.29.29.29:5353".to_string()],
            "应修剪空白、去掉重复与非法项，保留合法写法"
        );

        // 再规范化一次不该再有改动
        assert!(!config.normalize(), "规范化应当是幂等的");
    }

    #[test]
    fn normalize_restores_defaults_when_bootstrap_dns_is_empty() {
        let mut config = AppConfig::default();
        config.proxy.bootstrap_dns = vec!["不是地址".to_string()];

        assert!(config.normalize());
        assert_eq!(
            config.proxy.bootstrap_dns,
            vec!["223.5.5.5".to_string(), "119.29.29.29".to_string()],
            "全部无效时应补回默认值，而不是留下空列表放弃引导解析"
        );
    }

    fn subscription(name: &str, sub_type: SubscriptionType) -> Subscription {
        Subscription {
            name: name.to_string(),
            url: format!("https://example.invalid/{}", name),
            enabled: true,
            rules: vec![format!("{}.example.com", name)],
            last_updated: None,
            sub_type,
            target_group: None,
        }
    }

    /// 停用域名路由后，geosite 订阅连规则一起清掉，广告过滤订阅必须保留
    #[test]
    fn normalize_drops_geosite_subscriptions_when_routing_is_disabled() {
        let mut config = AppConfig::default();
        config.subscriptions = vec![
            subscription("广告拦截域名", SubscriptionType::Blocklist),
            subscription("CN 国内直连域名", SubscriptionType::Geosite),
            subscription("Proxy 需代理域名", SubscriptionType::Geosite),
        ];

        assert!(config.normalize(), "清理订阅应被视为配置改动");

        assert_eq!(
            config.subscriptions.len(),
            1,
            "geosite 订阅应被清掉，黑名单订阅必须留下"
        );
        assert_eq!(config.subscriptions[0].name, "广告拦截域名");
        assert_eq!(
            config.subscriptions[0].sub_type,
            SubscriptionType::Blocklist,
            "广告过滤订阅的类型与规则都不应被改动"
        );
        assert_eq!(config.subscriptions[0].rules.len(), 1);
    }

    /// 没有 geosite 订阅时不应反复判定为「有改动」，否则每次启动都白写一次配置
    #[test]
    fn normalize_is_idempotent_once_geosite_is_gone() {
        let mut config = AppConfig::default();
        config.subscriptions = vec![subscription("广告拦截域名", SubscriptionType::Blocklist)];

        // 第一次规范化可能因分组描述等历史项而改动，第二次必须稳定
        config.normalize();
        assert!(
            !config.normalize(),
            "已无 geosite 订阅时不应再判定为改动，避免每次启动都重写配置文件"
        );
    }

    #[test]
    fn app_behavior_defaults_are_safe_for_existing_configurations() {
        let text = toml::to_string(&AppConfig::default()).expect("默认配置应可序列化");
        let legacy_text = text
            .lines()
            .filter(|line| {
                !line.trim_start().starts_with("startup_delay_seconds")
                    && !line.trim_start().starts_with("dns_restart_interval_hours")
            })
            .collect::<Vec<_>>()
            .join("\n");

        let config: AppConfig = toml::from_str(&legacy_text).expect("旧配置应能解析");
        assert_eq!(config.startup_delay_seconds, 0);
        assert_eq!(config.dns_restart_interval_hours, 0);
    }

    #[test]
    fn normalize_falls_back_to_udp_for_unknown_transport() {
        let mut config = AppConfig::default();
        config.proxy.protocol = "bothh".to_string();

        assert!(config.normalize());
        assert_eq!(config.proxy.protocol, "udp");
    }

    #[test]
    fn normalize_accepts_supported_transports() {
        for protocol in ["udp", "tcp", "both"] {
            let mut config = AppConfig::default();
            config.proxy.protocol = protocol.to_string();
            config.normalize();
            assert_eq!(config.proxy.protocol, protocol);
        }
    }
}
