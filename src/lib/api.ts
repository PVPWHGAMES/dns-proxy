import { invoke } from "@tauri-apps/api/core";

export type DnsProtocol = "udp" | "tcp" | "doh" | "dot";
export type DnsStrategy = "sequential" | "fastest" | "load_balance" | "parallel";
export type RuleType = "exact" | "wildcard" | "regex" | "blocklist";
export type RuleAction = "forward" | "allow" | "block" | "block_null" | "block_nxdomain" | "cache";

export interface DnsServer {
  name: string;
  ip: string;
  port: number;
  enabled: boolean;
  protocol: DnsProtocol;
  doh_url?: string;
  group: string;
}

export interface ServerGroup {
  name: string;
  description: string;
}

export interface ProxyConfig {
  listen_address: string;
  listen_port: number;
  protocol: string;
  cache_size: number;
  cache_ttl: number;
  block_ipv6: boolean;
  default_group: string;
  /** 启动服务时把系统网卡 DNS 指向本代理，停止/退出时还原 */
  takeover_system_dns: boolean;
}

export interface Rule {
  name: string;
  pattern: string;
  rule_type: RuleType;
  action: RuleAction;
  target?: string;
  enabled: boolean;
  priority: number;
}

export type SubscriptionType = "blocklist" | "geosite";

export interface Subscription {
  name: string;
  url: string;
  enabled: boolean;
  rules: string[];
  last_updated?: string;
  sub_type: SubscriptionType;
  target_group?: string;
}

export interface LogConfig {
  level: string;
  file?: string;
}

export interface EcsConfig {
  enabled: boolean;
  client_ip?: string;
  ipv4_source_mask: number;
  ipv6_source_mask: number;
}

export interface AppConfig {
  proxy: ProxyConfig;
  upstream: DnsServer[];
  rules: Rule[];
  subscriptions: Subscription[];
  subscription_update_interval: number;
  latency_test_interval: number;
  start_minimized: boolean;
  log: LogConfig;
  strategy: DnsStrategy;
  server_groups: ServerGroup[];
  ecs: EcsConfig;
}

export interface DnsQueryLog {
  id: number;
  timestamp: string;
  domain: string;
  query_type: string;
  response: string;
  upstream: string;
  latency_ms: number;
  action: string;
  group: string;
}

export interface DnsStats {
  total_queries: number;
  blocked_queries: number;
  cached_queries: number;
  avg_latency: number;
  is_running: boolean;
}

export interface DnsLatencyResult {
  name: string;
  ip: string;
  latency_ms?: number;
  error?: string;
}

export interface TimeBucket {
  time: string;
  total: number;
  blocked: number;
  cached: number;
}

export interface DomainStat {
  domain: string;
  count: number;
}

export interface LatencyDistribution {
  range: string;
  count: number;
}

export interface TrafficStats {
  timeline: TimeBucket[];
  top_domains: DomainStat[];
  latency_dist: LatencyDistribution[];
  total_queries: number;
  queries_per_second: number;
}

export interface CacheStats {
  total_queries: number;
  cache_hits: number;
  cache_misses: number;
  hit_rate: number;
  current_size: number;
  max_size: number;
}

export interface PoolStats {
  dot_idle_connections: number;
  dot_hosts: number;
  udp_channels: number;
}

export interface MemoryInfo {
  memory_mb: number;
  virtual_memory_mb: number;
}

export interface DnsTakeoverStatus {
  /** 当前是否已接管系统 DNS */
  active: boolean;
  /** 接管前各网卡的原始配置摘要 */
  detail: string;
  /** 配置项：启动服务时是否自动接管 */
  enabled: boolean;
}

export const api = {
  async getConfig(): Promise<AppConfig> {
    return await invoke("get_config");
  },

  async saveConfig(config: AppConfig): Promise<void> {
    return await invoke("save_config", { newConfig: config });
  },

  async startServer(): Promise<void> {
    return await invoke("start_server");
  },

  async stopServer(): Promise<void> {
    return await invoke("stop_server");
  },

  async getServerStatus(): Promise<boolean> {
    return await invoke("get_server_status");
  },

  async getStats(): Promise<DnsStats> {
    return await invoke("get_stats");
  },

  async getLogs(limit?: number): Promise<DnsQueryLog[]> {
    return await invoke("get_logs", { limit });
  },

  async getLogsSince(sinceId: number): Promise<DnsQueryLog[]> {
    return await invoke("get_logs_since", { sinceId });
  },

  async clearLogs(): Promise<void> {
    return await invoke("clear_logs");
  },

  async clearCache(): Promise<void> {
    return await invoke("clear_cache");
  },

  async getTrafficStats(): Promise<TrafficStats> {
    return await invoke("get_traffic_stats");
  },

  async getCacheStats(): Promise<CacheStats> {
    return await invoke("get_cache_stats");
  },

  async getPoolStats(): Promise<PoolStats> {
    return await invoke("get_pool_stats");
  },

  async getMemoryUsage(): Promise<MemoryInfo> {
    return await invoke("get_memory_usage");
  },

  async updateSubscriptions(): Promise<string> {
    return await invoke("update_subscriptions");
  },

  async testDnsLatency(): Promise<DnsLatencyResult[]> {
    return await invoke("test_dns_latency");
  },

  async getLatencyResults(): Promise<[DnsLatencyResult[], string | null]> {
    return await invoke("get_latency_results");
  },

  // 系统 DNS 接管
  async getDnsTakeoverStatus(): Promise<DnsTakeoverStatus> {
    return await invoke("get_dns_takeover_status");
  },

  async restoreSystemDns(): Promise<string> {
    return await invoke("restore_system_dns");
  },

  // 程序自启动（Windows 注册表）
  async isAutostartEnabled(): Promise<boolean> {
    return await invoke("is_autostart_enabled");
  },

  async setAutostart(enabled: boolean): Promise<void> {
    return await invoke("set_autostart", { enabled });
  },
};
