import { invoke } from "@tauri-apps/api/core";

export type DnsProtocol = "udp" | "tcp" | "doh" | "dot";
export type DnsStrategy = "sequential" | "fastest" | "load_balance" | "parallel";
export type RuleType = "exact" | "wildcard" | "regex" | "blocklist";
export type RuleAction = "forward" | "block" | "block_null" | "block_nxdomain" | "cache";

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
  auto_start: boolean;
  block_ipv6: boolean;
  default_group: string;
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
  log: LogConfig;
  strategy: DnsStrategy;
  server_groups: ServerGroup[];
  ecs: EcsConfig;
}

export interface TunConfig {
  enabled: boolean;
  interface_name: string;
  subnet: string;
  gateway: string;
  dns_servers: string[];
  auto_route: boolean;
}

export interface TunStatus {
  active: boolean;
  starting: boolean;
  interface_name: string;
  ip_address: string;
  dns_redirected: boolean;
  packets_processed: number;
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

  async getLogs(): Promise<DnsQueryLog[]> {
    return await invoke("get_logs");
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

  async updateSubscriptions(): Promise<string> {
    return await invoke("update_subscriptions");
  },

  // TUN 相关
  async getTunConfig(): Promise<TunConfig> {
    return await invoke("get_tun_config");
  },

  async saveTunConfig(config: TunConfig): Promise<void> {
    return await invoke("save_tun_config", { newConfig: config });
  },

  async startTun(): Promise<string> {
    return await invoke("start_tun");
  },

  async stopTun(): Promise<string> {
    return await invoke("stop_tun");
  },

  async getTunStatus(): Promise<TunStatus> {
    return await invoke("get_tun_status");
  },

  async testDnsLatency(): Promise<DnsLatencyResult[]> {
    return await invoke("test_dns_latency");
  },

  async getLatencyResults(): Promise<[DnsLatencyResult[], string | null]> {
    return await invoke("get_latency_results");
  },
};
