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
  /** DoH 主机名的引导解析服务器（系统 DNS 已指向本程序时不能再走系统解析器） */
  bootstrap_dns: string[];
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

/** 一条活动流（FLOW 层只读观察） */
export interface FlowEntry {
  endpoint_id: number;
  process_id: number;
  process_name: string;
  /** TCP / UDP / ICMP / ICMPv6 / 协议 n */
  protocol: string;
  local_addr: string;
  local_port: number;
  remote_addr: string;
  remote_port: number;
  outbound: boolean;
  loopback: boolean;
  established_at: string;
}

export interface FlowMonitorStatus {
  running: boolean;
  /** WinDivert 运行库是否就位 */
  available: boolean;
  /** 失败原因，例如需要管理员权限 */
  message?: string | null;
  active_count: number;
  total_established: number;
  total_deleted: number;
  process_count: number;
}

/** 重定向规则状态（四个分支的计数器） */
export interface RedirectStatus {
  running: boolean;
  available: boolean;
  message?: string | null;
  rule?: {
    target: string;
    relay_port: number;
    sentinel_port: number;
  } | null;
  /** 分支 1：客户端 → 中继 */
  to_relay: number;
  /** 分支 2：中继 → 客户端 */
  to_client: number;
  /** 分支 3：中继拨号被映射到真实目标 */
  dial_mapped: number;
  /** 分支 4：目标回包被映射回哨兵端口 */
  reply_mapped: number;
  passed_through: number;
  skipped: number;
  send_failed: number;
}

export interface RelayStatus {
  running: boolean;
  listen_addr?: string | null;
  destination?: string | null;
  accepted: number;
  active: number;
  bytes_up: number;
  bytes_down: number;
  last_error?: string | null;
}

export interface RedirectOverview {
  redirect: RedirectStatus;
  relay: RelayStatus;
}

export interface RedirectRequest {
  target_addr: string;
  target_port: number;
  /** 中继绑定的本机地址，必须是面向客户端的网卡地址 */
  relay_bind: string;
  relay_port: number;
  sentinel_port: number;
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

  // FLOW 层只读观察（不修改、不丢弃、不注入任何数据包）
  async startFlowMonitor(): Promise<FlowMonitorStatus> {
    return await invoke("start_flow_monitor");
  },

  async stopFlowMonitor(): Promise<FlowMonitorStatus> {
    return await invoke("stop_flow_monitor");
  },

  async getFlowMonitorStatus(): Promise<FlowMonitorStatus> {
    return await invoke("get_flow_monitor_status");
  },

  async getActiveFlows(): Promise<FlowEntry[]> {
    return await invoke("get_active_flows");
  },

  // 最小重定向（实验性，单目标，默认不启动）
  async startRedirect(request: RedirectRequest): Promise<RedirectOverview> {
    return await invoke("start_redirect", { request });
  },

  async stopRedirect(): Promise<RedirectOverview> {
    return await invoke("stop_redirect");
  },

  async getRedirectStatus(): Promise<RedirectOverview> {
    return await invoke("get_redirect_status");
  },
};
