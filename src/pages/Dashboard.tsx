import { useState, useEffect, useCallback } from "react";
import { api, DnsStats, DnsQueryLog, AppConfig, TrafficStats } from "../lib/api";
import {
  Activity,
  Server,
  Shield,
  Zap,
  Clock,
  Globe,
  Play,
  Square,
  Loader2,
  BarChart3,
} from "lucide-react";
import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  BarChart,
  Bar,
  PieChart,
  Pie,
  Cell,
} from "recharts";

export default function Dashboard() {
  const [stats, setStats] = useState<DnsStats>({
    total_queries: 0,
    blocked_queries: 0,
    cached_queries: 0,
    avg_latency: 0,
    is_running: false,
  });
  const [logs, setLogs] = useState<DnsQueryLog[]>([]);
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [trafficStats, setTrafficStats] = useState<TrafficStats | null>(null);
  const [loading, setLoading] = useState(false);
  const [initializing, setInitializing] = useState(true);

  // 刷新状态
  const refreshStatus = useCallback(async () => {
    try {
      const [newStats, newLogs, newConfig, newTrafficStats] = await Promise.all([
        api.getStats(),
        api.getLogs(),
        api.getConfig(),
        api.getTrafficStats(),
      ]);
      setStats(newStats);
      setLogs(newLogs.slice(0, 10));
      setConfig(newConfig);
      setTrafficStats(newTrafficStats);
    } catch (e) {
      console.error("获取状态失败:", e);
    } finally {
      setInitializing(false);
    }
  }, []);

  // 定时刷新
  useEffect(() => {
    refreshStatus();
    const interval = setInterval(refreshStatus, 1000);
    return () => clearInterval(interval);
  }, [refreshStatus]);

  // 启动/停止服务
  const toggleService = async () => {
    setLoading(true);
    try {
      if (stats.is_running) {
        await api.stopServer();
        // 等待端口释放
        await new Promise(resolve => setTimeout(resolve, 300));
      } else {
        await api.startServer();
      }
      // 等待一下再刷新状态
      await new Promise(resolve => setTimeout(resolve, 500));
      await refreshStatus();
    } catch (e: any) {
      const msg = String(e);
      if (msg.includes("10048") || msg.includes("address already in use")) {
        // 端口被占用，尝试重启
        try {
          await api.stopServer();
          await new Promise(resolve => setTimeout(resolve, 1000));
          await api.startServer();
          await new Promise(resolve => setTimeout(resolve, 500));
          await refreshStatus();
        } catch (e2) {
          alert("启动失败: " + e2);
        }
      } else {
        alert("操作失败: " + e);
      }
    } finally {
      setLoading(false);
    }
  };

  if (initializing) {
    return (
      <div className="flex items-center justify-center h-64">
        <Loader2 className="w-8 h-8 animate-spin text-primary" />
        <span className="ml-2">加载中...</span>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* 启动控制卡片 */}
      <div className="bg-card rounded-xl border p-6">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-lg font-semibold">DNS 代理服务</h3>
            <p className="text-sm text-muted-foreground">
              {stats.is_running ? "服务正在运行，监听端口 53" : "服务未启动"}
            </p>
          </div>
          <button
            onClick={toggleService}
            disabled={loading}
            className={`
              flex items-center gap-2 px-6 py-3 rounded-lg font-medium transition-all
              ${
                stats.is_running
                  ? "bg-destructive text-destructive-foreground hover:bg-destructive/90"
                  : "bg-primary text-primary-foreground hover:bg-primary/90"
              }
              ${loading ? "opacity-70 cursor-wait" : ""}
            `}
          >
            {loading ? (
              <>
                <Loader2 className="w-5 h-5 animate-spin" />
                处理中...
              </>
            ) : stats.is_running ? (
              <>
                <Square className="w-5 h-5" />
                停止服务
              </>
            ) : (
              <>
                <Play className="w-5 h-5" />
                启动服务
              </>
            )}
          </button>
        </div>

        {stats.is_running && (
          <div className="mt-4 flex items-center gap-2 text-sm text-green-600">
            <div className="w-2 h-2 rounded-full bg-green-500 animate-pulse" />
            <span>运行中</span>
          </div>
        )}
      </div>

      {/* 统计卡片 */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        <StatCard title="总查询数" value={stats.total_queries.toLocaleString()} icon={Activity} color="blue" />
        <StatCard title="阻止查询" value={stats.blocked_queries.toLocaleString()} icon={Shield} color="red" />
        <StatCard title="平均延迟" value={`${stats.avg_latency.toFixed(1)}ms`} icon={Zap} color="yellow" />
        <StatCard title="缓存命中" value={stats.cached_queries.toLocaleString()} icon={Server} color="green" />
      </div>

      {/* 流量图表区域 */}
      {trafficStats && (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
          {/* 请求量时间线 */}
          <div className="bg-card rounded-xl border p-4">
            <div className="flex items-center justify-between mb-4">
              <h3 className="font-semibold flex items-center gap-2">
                <BarChart3 className="w-5 h-5" />
                请求量趋势
              </h3>
              <span className="text-sm text-muted-foreground">
                QPS: {trafficStats.queries_per_second.toFixed(1)}
              </span>
            </div>
            {trafficStats.timeline.length > 0 ? (
              <ResponsiveContainer width="100%" height={200}>
                <AreaChart data={trafficStats.timeline}>
                  <CartesianGrid strokeDasharray="3 3" className="opacity-30" />
                  <XAxis
                    dataKey="time"
                    tick={{ fontSize: 12 }}
                    interval="preserveStartEnd"
                  />
                  <YAxis tick={{ fontSize: 12 }} />
                  <Tooltip
                    contentStyle={{
                      backgroundColor: 'var(--card)',
                      border: '1px solid var(--border)',
                      borderRadius: '8px',
                    }}
                  />
                  <Area
                    type="monotone"
                    dataKey="total"
                    stackId="1"
                    stroke="#3b82f6"
                    fill="#3b82f6"
                    fillOpacity={0.3}
                    name="总查询"
                  />
                  <Area
                    type="monotone"
                    dataKey="blocked"
                    stackId="2"
                    stroke="#ef4444"
                    fill="#ef4444"
                    fillOpacity={0.3}
                    name="阻止"
                  />
                  <Area
                    type="monotone"
                    dataKey="cached"
                    stackId="3"
                    stroke="#22c55e"
                    fill="#22c55e"
                    fillOpacity={0.3}
                    name="缓存"
                  />
                </AreaChart>
              </ResponsiveContainer>
            ) : (
              <div className="h-[200px] flex items-center justify-center text-muted-foreground">
                暂无数据
              </div>
            )}
          </div>

          {/* Top 10 域名 */}
          <div className="bg-card rounded-xl border p-4">
            <h3 className="font-semibold mb-4 flex items-center gap-2">
              <Globe className="w-5 h-5" />
              Top 10 热门域名
            </h3>
            {trafficStats.top_domains.length > 0 ? (
              <ResponsiveContainer width="100%" height={200}>
                <BarChart
                  data={trafficStats.top_domains}
                  layout="vertical"
                  margin={{ left: 80 }}
                >
                  <CartesianGrid strokeDasharray="3 3" className="opacity-30" />
                  <XAxis type="number" tick={{ fontSize: 12 }} />
                  <YAxis
                    type="category"
                    dataKey="domain"
                    tick={{ fontSize: 11 }}
                    width={80}
                  />
                  <Tooltip
                    contentStyle={{
                      backgroundColor: 'var(--card)',
                      border: '1px solid var(--border)',
                      borderRadius: '8px',
                    }}
                  />
                  <Bar dataKey="count" fill="#8b5cf6" radius={[0, 4, 4, 0]} name="查询次数" />
                </BarChart>
              </ResponsiveContainer>
            ) : (
              <div className="h-[200px] flex items-center justify-center text-muted-foreground">
                暂无数据
              </div>
            )}
          </div>

          {/* 延迟分布 */}
          <div className="bg-card rounded-xl border p-4">
            <h3 className="font-semibold mb-4 flex items-center gap-2">
              <Zap className="w-5 h-5" />
              响应延迟分布
            </h3>
            {trafficStats.latency_dist.some(d => d.count > 0) ? (
              <ResponsiveContainer width="100%" height={200}>
                <BarChart data={trafficStats.latency_dist}>
                  <CartesianGrid strokeDasharray="3 3" className="opacity-30" />
                  <XAxis dataKey="range" tick={{ fontSize: 11 }} />
                  <YAxis tick={{ fontSize: 12 }} />
                  <Tooltip
                    contentStyle={{
                      backgroundColor: 'var(--card)',
                      border: '1px solid var(--border)',
                      borderRadius: '8px',
                    }}
                  />
                  <Bar dataKey="count" fill="#f59e0b" radius={[4, 4, 0, 0]} name="查询次数" />
                </BarChart>
              </ResponsiveContainer>
            ) : (
              <div className="h-[200px] flex items-center justify-center text-muted-foreground">
                暂无数据
              </div>
            )}
          </div>

          {/* 查询类型分布（新增） */}
          <div className="bg-card rounded-xl border p-4">
            <h3 className="font-semibold mb-4 flex items-center gap-2">
              <Activity className="w-5 h-5" />
              查询统计
            </h3>
            <div className="space-y-4">
              <div className="grid grid-cols-2 gap-4">
                <div className="text-center p-3 bg-muted/50 rounded-lg">
                  <p className="text-2xl font-bold text-blue-600">
                    {trafficStats.total_queries.toLocaleString()}
                  </p>
                  <p className="text-sm text-muted-foreground">总查询数</p>
                </div>
                <div className="text-center p-3 bg-muted/50 rounded-lg">
                  <p className="text-2xl font-bold text-green-600">
                    {trafficStats.queries_per_second.toFixed(1)}
                  </p>
                  <p className="text-sm text-muted-foreground">QPS</p>
                </div>
              </div>
              <div className="text-sm text-muted-foreground text-center">
                统计时间: 最近 60 分钟
              </div>
            </div>
          </div>
        </div>
      )}

      {/* 最近查询 */}
      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b">
          <h3 className="font-semibold">最近查询</h3>
        </div>
        <div className="overflow-x-auto">
          <table className="w-full table-fixed min-w-[700px]">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[15%]">时间</th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[30%]">域名</th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[10%]">类型</th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[20%]">响应</th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[15%]">上游</th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[10%]">状态</th>
              </tr>
            </thead>
            <tbody>
              {logs.length === 0 ? (
                <tr>
                  <td colSpan={6} className="p-8 text-center text-muted-foreground">
                    暂无查询记录
                  </td>
                </tr>
              ) : (
                logs.map((log) => (
                  <tr key={log.id} className="border-b hover:bg-muted/50">
                    <td className="p-3 text-sm whitespace-nowrap">
                      <div className="flex items-center gap-2 truncate">
                        <Clock className="w-4 h-4 text-muted-foreground shrink-0" />
                        <span className="truncate">{log.timestamp}</span>
                      </div>
                    </td>
                    <td className="p-3 text-sm font-medium">
                      <div className="flex items-center gap-2">
                        <Globe className="w-4 h-4 text-muted-foreground shrink-0" />
                        <span className="truncate">{log.domain}</span>
                      </div>
                    </td>
                    <td className="p-3 text-sm whitespace-nowrap">
                      <span className="px-2 py-1 rounded bg-muted text-xs font-mono">{log.query_type}</span>
                    </td>
                    <td className="p-3 text-sm font-mono truncate">{log.response}</td>
                    <td className="p-3 text-sm text-muted-foreground">
                      <div className="flex items-center gap-1 truncate">
                        <span className="truncate">{log.upstream}</span>
                        {log.group && (
                          <span className={`shrink-0 px-1.5 py-0.5 rounded text-[10px] ${
                            log.group === "domestic" ? "bg-blue-100 text-blue-600" :
                            log.group === "proxy" ? "bg-orange-100 text-orange-600" :
                            "bg-gray-100 text-gray-600"
                          }`}>
                            {log.group === "domestic" ? "直连" : log.group === "proxy" ? "代理" : log.group}
                          </span>
                        )}
                      </div>
                    </td>
                    <td className="p-3 text-sm whitespace-nowrap">
                      <span className={`px-2 py-1 rounded-full text-xs font-medium ${
                        log.action === "success" ? "bg-green-100 text-green-700" :
                        log.action === "blocked" ? "bg-red-100 text-red-700" :
                        "bg-blue-100 text-blue-700"
                      }`}>
                        {log.action === "success" ? "成功" : log.action === "blocked" ? "阻止" : "缓存"}
                      </span>
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </div>

      {/* 系统信息 */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <div className="bg-card rounded-xl border p-4">
          <h4 className="font-semibold mb-3">上游 DNS 服务器</h4>
          <div className="space-y-2">
            {config ? (
              config.upstream.filter((s) => s.enabled).map((server, i) => (
                <div key={i} className="flex items-center justify-between">
                  <span className="text-muted-foreground text-sm">
                    {server.name}
                    <span className="ml-1.5 text-xs text-muted-foreground/60">({server.group || "default"})</span>
                  </span>
                  <span className="font-mono text-sm">{server.ip}:{server.port}</span>
                </div>
              ))
            ) : (
              <p className="text-sm text-muted-foreground">加载中...</p>
            )}
            {config && config.upstream.filter((s) => s.enabled).length === 0 && (
              <p className="text-sm text-muted-foreground">无启用的服务器</p>
            )}
          </div>
        </div>

        <div className="bg-card rounded-xl border p-4">
          <h4 className="font-semibold mb-3">系统信息</h4>
          <div className="space-y-3 text-sm">
            <InfoItem label="监听地址" value={config ? `${config.proxy.listen_address}:${config.proxy.listen_port}` : "-"} />
            <InfoItem label="策略" value={config ? config.strategy : "-"} />
            <InfoItem label="运行状态" value={stats.is_running ? "运行中" : "已停止"} />
            <div className="pt-2">
              <button
                onClick={async () => {
                  try {
                    await api.clearCache();
                    await refreshStatus();
                  } catch (e) {
                    console.error("清空缓存失败:", e);
                  }
                }}
                className="px-3 py-1.5 text-sm bg-muted hover:bg-muted/80 rounded-md transition-colors"
              >
                清空 DNS 缓存
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function StatCard({ title, value, icon: Icon, color }: {
  title: string; value: string; icon: any; color: string;
}) {
  const colors: Record<string, string> = {
    blue: "bg-blue-50 text-blue-600",
    red: "bg-red-50 text-red-600",
    yellow: "bg-yellow-50 text-yellow-600",
    green: "bg-green-50 text-green-600",
  };

  return (
    <div className="bg-card rounded-xl border p-4">
      <div className="flex items-center justify-between">
        <div className={`p-2 rounded-lg ${colors[color]}`}>
          <Icon className="w-5 h-5" />
        </div>
      </div>
      <div className="mt-3">
        <p className="text-2xl font-bold">{value}</p>
        <p className="text-sm text-muted-foreground">{title}</p>
      </div>
    </div>
  );
}

function InfoItem({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-mono">{value}</span>
    </div>
  );
}
