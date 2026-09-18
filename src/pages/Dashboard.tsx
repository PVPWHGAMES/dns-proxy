import { useState, useEffect, useCallback } from "react";
import {
  api,
  DnsStats,
  AppConfig,
  TrafficStats,
  CacheStats,
  MemoryInfo,
  DnsTakeoverStatus,
} from "../lib/api";
import {
  Activity,
  Server,
  Shield,
  Zap,
  Globe,
  Play,
  Square,
  Loader2,
  BarChart3,
  Cpu,
  Wifi,
  TrendingUp,
  PieChart as PieChartIcon,
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
  LabelList,
  PieChart,
  Pie,
  Cell,
} from "recharts";
import StatCard from "../components/ui/StatCard";

/**
 * 四个图表面板的内容区高度
 *
 * 四块面板共用同一个高度常量，卡片都是「标题 + 固定高度内容区」的两段结构，
 * 这样在 2×2 网格里行高自然一致，不需要靠 min-height 凑。
 */
const CHART_HEIGHT = 240;

/** 热门域名展示条数，与后端 top_domains 的返回上限一致 */
const TOP_DOMAIN_COUNT = 10;

/** 图表统一的 tooltip 外观 */
const TOOLTIP_STYLE = {
  backgroundColor: "var(--card)",
  border: "1px solid var(--border)",
  borderRadius: "8px",
} as const;

/** 缓存的两种去向配色 */
const CACHE_HIT_COLOR = "#22c55e";
const CACHE_MISS_COLOR = "#e5e7eb";

/** 缓存命中率环形图的半径（相对容器短边的一半） */
const CACHE_DONUT = { inner: "54%", outer: "72%" } as const;

/**
 * 图表面板外壳：统一标题区与内容区高度，保证四块等大
 *
 * 内容区用固定的数值高度，不用 flex-1：`flex: 1 1 0%` 的 flex-basis 是 0，
 * 在高度由内容决定的卡片里主轴尺寸会被算成 0，recharts 的 ResponsiveContainer
 * 拿到 0 高度就什么都不渲染（表现为「只有框、没有图」）。
 * 图表本身也一律传数值 height，不依赖父容器解析百分比高度。
 */
function ChartPanel({
  title,
  icon: Icon,
  hint,
  children,
}: {
  title: string;
  icon: React.ComponentType<{ className?: string }>;
  hint?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="bg-card rounded-xl border p-4">
      <div className="flex items-center justify-between mb-4">
        <h3 className="font-semibold flex items-center gap-2">
          <Icon className="w-5 h-5" />
          {title}
        </h3>
        {/* 提示不换行：换行会把标题区撑高，四块就不等高了 */}
        {hint && (
          <span className="text-xs text-muted-foreground whitespace-nowrap">{hint}</span>
        )}
      </div>
      <div className="relative" style={{ height: CHART_HEIGHT }}>
        {children}
      </div>
    </div>
  );
}

function EmptyChart() {
  return (
    <div className="flex items-center justify-center text-sm text-muted-foreground" style={{ height: CHART_HEIGHT }}>
      暂无数据（服务启动并接入流量后显示）
    </div>
  );
}

/** 缓存面板右侧的一行指标 */
function CacheRow({ color, label, value }: { color: string; label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="flex items-center gap-2 text-sm text-muted-foreground min-w-0">
        <span
          className="w-2.5 h-2.5 rounded-sm shrink-0"
          style={{ backgroundColor: color }}
        />
        <span className="truncate">{label}</span>
      </span>
      <span className="text-sm font-semibold tabular-nums shrink-0">{value}</span>
    </div>
  );
}

export default function Dashboard() {
  const [stats, setStats] = useState<DnsStats>({
    total_queries: 0,
    blocked_queries: 0,
    cached_queries: 0,
    avg_latency: 0,
    is_running: false,
  });
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [trafficStats, setTrafficStats] = useState<TrafficStats | null>(null);
  const [cacheStats, setCacheStats] = useState<CacheStats | null>(null);
  const [memory, setMemory] = useState<MemoryInfo | null>(null);
  const [takeover, setTakeover] = useState<DnsTakeoverStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [initializing, setInitializing] = useState(true);

  // 刷新状态（配置与接管状态不在轮询范围内，见下方）
  const refreshStatus = useCallback(async () => {
    try {
      const [newStats, newTrafficStats, newCacheStats, newMemory] = await Promise.all([
        api.getStats(),
        api.getTrafficStats(),
        api.getCacheStats(),
        api.getMemoryUsage().catch(() => null),
      ]);
      setStats(newStats);
      setTrafficStats(newTrafficStats);
      setCacheStats(newCacheStats);
      setMemory(newMemory);
    } catch (e) {
      console.error("获取状态失败:", e);
    } finally {
      setInitializing(false);
    }
  }, []);

  // 配置只在进入页面时读取一次：它含订阅规则，每次轮询都克隆开销过大
  const loadConfig = useCallback(async () => {
    try {
      setConfig(await api.getConfig());
    } catch (e) {
      console.error("获取配置失败:", e);
    }
  }, []);

  useEffect(() => {
    loadConfig();
    refreshStatus();
    const interval = setInterval(refreshStatus, 2000);
    return () => clearInterval(interval);
  }, [loadConfig, refreshStatus]);

  // 接管状态变化不频繁，单独低频刷新
  useEffect(() => {
    const load = async () => {
      try {
        setTakeover(await api.getDnsTakeoverStatus());
      } catch (e) {
        console.error("获取 DNS 接管状态失败:", e);
      }
    };
    load();
    const interval = setInterval(load, 10000);
    return () => clearInterval(interval);
  }, []);

  const toggleService = async () => {
    setLoading(true);
    setActionError(null);
    try {
      if (stats.is_running) {
        await api.stopServer();
        await new Promise((resolve) => setTimeout(resolve, 300));
      } else {
        await api.startServer();
      }
      await new Promise((resolve) => setTimeout(resolve, 500));
      await refreshStatus();
    } catch (e) {
      // 端口被占用时先停再起，其余错误直接提示
      const msg = String(e);
      if (msg.includes("10048") || msg.includes("address already in use")) {
        try {
          await api.stopServer();
          await new Promise((resolve) => setTimeout(resolve, 1000));
          await api.startServer();
          await new Promise((resolve) => setTimeout(resolve, 500));
          await refreshStatus();
        } catch (e2) {
          setActionError("启动失败: " + e2);
        }
      } else {
        setActionError("操作失败: " + e);
      }
    } finally {
      setLoading(false);
    }
  };

  const clearCache = async () => {
    try {
      await api.clearCache();
      await refreshStatus();
    } catch (e) {
      console.error("清空缓存失败:", e);
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

  const timeline = trafficStats?.timeline ?? [];
  const topDomains = trafficStats?.top_domains ?? [];
  const latencyDist = trafficStats?.latency_dist ?? [];
  const hasTimelineData = timeline.some((bucket) => bucket.total > 0);
  const hasLatencyData = latencyDist.some((item) => item.count > 0);

  // 当前 QPS 取最近一个已完成分钟的均值；后端给的是运行至今的平均值，
  // 两者含义不同，分别标注避免误读。
  const lastFullMinute = timeline.length >= 2 ? timeline[timeline.length - 2] : undefined;
  const currentQps = lastFullMinute ? lastFullMinute.total / 60 : 0;

  const cacheHits = cacheStats?.cache_hits ?? 0;
  const cacheMisses = cacheStats?.cache_misses ?? 0;
  const hasCacheData = cacheHits + cacheMisses > 0;
  const cachePie = [
    { name: "命中", value: cacheHits },
    { name: "未命中", value: cacheMisses },
  ];

  return (
    <div className="space-y-6">
      {actionError && (
        <div className="rounded-lg border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {actionError}
        </div>
      )}

      {/* 服务控制卡片 */}
      <div className="bg-card rounded-xl border p-6">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-lg font-semibold flex items-center gap-2">
              <Globe className="w-5 h-5" />
              DNS 代理服务
            </h3>
            <p className="text-sm text-muted-foreground">
              {stats.is_running
                ? `运行中 · 监听 ${config?.proxy.listen_address || "-"}:${config?.proxy.listen_port || 53}`
                : "服务未启动"}
              {stats.is_running && config && (
                <span className="ml-2 text-xs opacity-70">
                  协议 {config.proxy.protocol === "both" ? "UDP + TCP" : config.proxy.protocol.toUpperCase()}
                </span>
              )}
            </p>
          </div>
          <button
            onClick={toggleService}
            disabled={loading}
            className={`
              flex items-center gap-2 px-4 py-2 rounded-lg font-medium transition-all text-sm
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
                <Loader2 className="w-4 h-4 animate-spin" />
                处理中
              </>
            ) : stats.is_running ? (
              <>
                <Square className="w-4 h-4" />
                停止
              </>
            ) : (
              <>
                <Play className="w-4 h-4" />
                启动
              </>
            )}
          </button>
        </div>

        {stats.is_running && (
          <div className="mt-3 flex flex-wrap items-center gap-4 text-sm">
            <div className="flex items-center gap-2 text-green-600 dark:text-green-400">
              <div className="w-2 h-2 rounded-full bg-green-500 animate-pulse" />
              <span>DNS 服务正常运行</span>
            </div>

            {/* 接管状态：服务在跑但没人用它时，日志和图表都会是空的 */}
            {takeover?.active ? (
              <span className="flex items-center gap-1.5 text-green-600 dark:text-green-400">
                <Wifi className="w-4 h-4" />
                系统 DNS 已接管
              </span>
            ) : (
              <span
                className="flex items-center gap-1.5 text-yellow-600 dark:text-yellow-400"
                title="系统网卡 DNS 未指向本代理，除显式指定 127.0.0.1 的查询外不会有流量进来"
              >
                <Wifi className="w-4 h-4" />
                系统 DNS 未接管（无流量接入）
              </span>
            )}

            <button
              onClick={clearCache}
              className="px-2 py-0.5 text-xs bg-muted hover:bg-muted/80 rounded transition-colors"
            >
              清空缓存
            </button>
          </div>
        )}
      </div>

      {/* 统计卡片 */}
      <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-6 gap-4">
        <StatCard
          title="总查询数（本次运行）"
          value={stats.total_queries.toLocaleString()}
          icon={Activity}
          color="blue"
          hint="计数保存在内存里，进程重启后归零；每条查询只计一次，含缓存命中、阻止、请求合并与失败。日志页的「共 N 条」是另一回事——那是可翻查的日志缓冲区条数，与这里的累计值不相等。"
        />
        <StatCard
          title="阻止查询"
          value={stats.blocked_queries.toLocaleString()}
          icon={Shield}
          color="red"
          hint="命中自定义规则、订阅黑名单或 IPv6 屏蔽而返回黑洞/NXDOMAIN 的查询数，已包含在总查询数里。"
        />
        <StatCard
          title="平均延迟"
          value={`${stats.avg_latency.toFixed(1)}ms`}
          icon={Zap}
          color="yellow"
          hint="仅统计实际转发至上游且成功得到响应的查询；缓存命中、广告/规则拦截、请求合并与失败查询均不计入。"
        />
        <StatCard
          title="缓存命中"
          value={stats.cached_queries.toLocaleString()}
          icon={Server}
          color="green"
          hint="命中 DNS 应答缓存而直接回包的查询数；代理分组（proxy）的请求按设计不读缓存，故不计入分母。"
        />
        <StatCard
          title="当前 QPS"
          value={currentQps.toFixed(2)}
          icon={TrendingUp}
          color="orange"
          hint="取最近一个已完成的整分钟均值（该分钟查询数 ÷ 60），比「运行至今的平均 QPS」更能反映此刻的负载。"
        />
        <StatCard
          title="内存(总工作集)"
          value={memory ? `${memory.memory_mb.toFixed(0)} MB` : "-"}
          icon={Cpu}
          color="purple"
          hint="本进程驻留物理内存的总量，含与其他进程共享的代码页。任务管理器「内存」列显示的是其中的专用部分（不含共享页），所以数值会更小。"
        />
      </div>

      {/* 四个等大图表：2×2 网格 */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        {/* 1. 请求量趋势 */}
        <ChartPanel
          title="请求量趋势"
          icon={BarChart3}
          hint={`最近 60 分钟 · 平均 QPS ${(trafficStats?.queries_per_second ?? 0).toFixed(2)}`}
        >
          {hasTimelineData ? (
            <ResponsiveContainer width="100%" height={CHART_HEIGHT}>
              <AreaChart data={timeline}>
                <CartesianGrid strokeDasharray="3 3" className="opacity-30" />
                <XAxis dataKey="time" tick={{ fontSize: 11 }} interval={9} />
                <YAxis tick={{ fontSize: 11 }} allowDecimals={false} width={32} />
                <Tooltip contentStyle={TOOLTIP_STYLE} />
                <Area
                  type="monotone"
                  dataKey="total"
                  stroke="#3b82f6"
                  fill="#3b82f6"
                  fillOpacity={0.25}
                  name="总查询"
                />
                <Area
                  type="monotone"
                  dataKey="cached"
                  stroke="#22c55e"
                  fill="#22c55e"
                  fillOpacity={0.25}
                  name="缓存命中"
                />
                <Area
                  type="monotone"
                  dataKey="blocked"
                  stroke="#ef4444"
                  fill="#ef4444"
                  fillOpacity={0.25}
                  name="已阻止"
                />
              </AreaChart>
            </ResponsiveContainer>
          ) : (
            <EmptyChart />
          )}
        </ChartPanel>

        {/* 2. 响应延迟分布 */}
        <ChartPanel title="响应延迟分布" icon={Zap} hint="仅上游成功响应 · 重点 40-120ms">
          {hasLatencyData ? (
            <ResponsiveContainer width="100%" height={CHART_HEIGHT}>
              <BarChart data={latencyDist}>
                <CartesianGrid strokeDasharray="3 3" className="opacity-30" />
                <XAxis dataKey="range" tick={{ fontSize: 10 }} />
                <YAxis tick={{ fontSize: 11 }} allowDecimals={false} width={32} />
                <Tooltip contentStyle={TOOLTIP_STYLE} />
                <Bar dataKey="count" fill="#f59e0b" radius={[4, 4, 0, 0]} name="查询次数" />
              </BarChart>
            </ResponsiveContainer>
          ) : (
            <EmptyChart />
          )}
        </ChartPanel>

        {/* 3. Top 10 热门域名 */}
        <ChartPanel title={`Top ${TOP_DOMAIN_COUNT} 热门域名`} icon={Globe}>
          {topDomains.length > 0 ? (
            <ResponsiveContainer width="100%" height={CHART_HEIGHT}>
              <BarChart
                data={topDomains.slice(0, TOP_DOMAIN_COUNT)}
                layout="vertical"
                margin={{ left: 4, right: 28, top: 4, bottom: 4 }}
              >
                <CartesianGrid strokeDasharray="3 3" className="opacity-30" />
                <XAxis type="number" tick={{ fontSize: 11 }} allowDecimals={false} />
                <YAxis
                  type="category"
                  dataKey="domain"
                  tick={{ fontSize: 10 }}
                  width={168}
                  interval={0}
                />
                {/*
                  本图不挂 Tooltip：横向条形图的悬停提示定位/内容异常，
                  改为把次数直接标在条形末端，信息常显、不依赖悬停。
                */}
                <Bar dataKey="count" fill="#8b5cf6" radius={[0, 3, 3, 0]}>
                  <LabelList
                    dataKey="count"
                    position="right"
                    style={{ fontSize: 10, fill: "currentColor" }}
                  />
                </Bar>
              </BarChart>
            </ResponsiveContainer>
          ) : (
            <EmptyChart />
          )}
        </ChartPanel>

        {/* 4. 缓存命中率 */}
        <ChartPanel
          title="缓存命中率"
          icon={PieChartIcon}
          hint={cacheStats ? `${cacheStats.current_size}/${cacheStats.max_size} 条目` : undefined}
        >
          {hasCacheData ? (
            <div className="flex items-center gap-4" style={{ height: CHART_HEIGHT }}>
              {/* 环形图固定为正方形，圆心标注才能稳定居中 */}
              <div
                className="relative shrink-0"
                style={{ width: CHART_HEIGHT, height: CHART_HEIGHT }}
              >
                <ResponsiveContainer width="100%" height={CHART_HEIGHT}>
                  <PieChart>
                    <Pie
                      data={cachePie}
                      dataKey="value"
                      nameKey="name"
                      innerRadius={CACHE_DONUT.inner}
                      outerRadius={CACHE_DONUT.outer}
                      paddingAngle={2}
                      startAngle={90}
                      endAngle={-270}
                      stroke="none"
                    >
                      <Cell fill={CACHE_HIT_COLOR} />
                      <Cell fill={CACHE_MISS_COLOR} />
                    </Pie>
                    <Tooltip contentStyle={TOOLTIP_STYLE} />
                  </PieChart>
                </ResponsiveContainer>

                {/* 圆心标注命中率，比图例更直接 */}
                <div className="absolute inset-0 flex flex-col items-center justify-center pointer-events-none">
                  <span className="text-2xl font-bold text-green-600 dark:text-green-400 tabular-nums">
                    {((cacheStats?.hit_rate ?? 0) * 100).toFixed(1)}%
                  </span>
                  <span className="text-xs text-muted-foreground">命中率</span>
                </div>
              </div>

              {/* 绝对值与比例为邻而非叠加，避免互相遮挡 */}
              <div className="flex-1 min-w-0 space-y-3">
                <CacheRow
                  color={CACHE_HIT_COLOR}
                  label="命中"
                  value={cacheHits.toLocaleString()}
                />
                <CacheRow
                  color={CACHE_MISS_COLOR}
                  label="未命中"
                  value={cacheMisses.toLocaleString()}
                />
                <CacheRow
                  color="#3b82f6"
                  label="缓存条目"
                  value={
                    cacheStats ? `${cacheStats.current_size}/${cacheStats.max_size}` : "-"
                  }
                />
              </div>
            </div>
          ) : (
            <EmptyChart />
          )}
        </ChartPanel>
      </div>
    </div>
  );
}
