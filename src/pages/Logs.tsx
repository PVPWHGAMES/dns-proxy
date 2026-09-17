import { useState, useEffect, useCallback, useRef, useLayoutEffect } from "react";
import { api, DnsQueryLog } from "../lib/api";
import { dnsActionLabel, dnsActionVariant } from "../lib/dnsAction";
import Badge from "../components/ui/Badge";
import {
  Search,
  Filter,
  Download,
  Trash2,
  RefreshCw,
  Clock,
  Globe,
  Server,
  FileText,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
} from "lucide-react";

/**
 * 每页条数的自适应区间
 *
 * 下限保证窄窗口下也还能看到成规模的列表，不至于一页只有两三行；
 * 上限防止超高分辨率窗口一次渲染上千行，白白拖慢滚动与筛选。
 */
const MIN_PAGE_SIZE = 10;
const MAX_PAGE_SIZE = 200;

/**
 * 表头与数据行的兜底高度（px）
 *
 * 仅在首帧还没渲染出表格时用来估算，量到真实高度后立刻改用实测值。
 * 行高来自 td 的 p-3（上下各 12px 内边距）加单行 text-sm（20px 行高）再加 1px 边框。
 */
const ROW_HEIGHT_FALLBACK = 45;
const HEADER_HEIGHT_FALLBACK = 45;

/** 分页栏高度（px），用于反推表格可用高度 */
const PAGINATION_BAR_HEIGHT = 52;

/**
 * 页脚统计卡片区与间距预留（px）
 *
 * 这块内容在表格下方，表格若把窗口剩余高度全吃掉，它就会被挤出可视区。
 * 前两项与布局里写死的 Tailwind 类一一对应，改布局时要同步：
 * - `space-y-6` = 24px，表格卡片与卡片区之间的间隔
 * - `main` 的 `p-6` = 24px，页面底部内边距
 * 卡片自身高度不用常量，用 Ref 实测。
 */
const FOOTER_GAP = 24;
const PAGE_BOTTOM_PADDING = 24;
/** 卡片块还没渲染出来时的兜底高度 */
const FOOTER_HEIGHT_FALLBACK = 100;

/** 实时刷新间隔（毫秒） */
const LIVE_REFRESH_INTERVAL = 2000;

/**
 * 搜索输入防抖（毫秒）
 *
 * 筛选改由后端做之后，每敲一个字符都会触发一次后端查询，而每次查询都要锁住日志缓冲区
 * 并扫最多一万条记录。防抖到停止输入后再发，输入框本身仍然是即时响应的。
 */
const SEARCH_DEBOUNCE = 300;

export default function Logs() {
  const [logs, setLogs] = useState<DnsQueryLog[]>([]);
  /** 输入框的实时值 */
  const [searchQuery, setSearchQuery] = useState("");
  /** 防抖后真正发往后端的关键字 */
  const [activeKeyword, setActiveKeyword] = useState("");
  const [filterAction, setFilterAction] = useState<string>("all");
  const [filterType, setFilterType] = useState<string>("all");
  const [isLive, setIsLive] = useState(true);
  const [loading, setLoading] = useState(false);
  /** 当前页码，从 1 开始 */
  const [page, setPage] = useState(1);
  /** 每页条数，按可视高度算出来 */
  const [pageSize, setPageSize] = useState(MIN_PAGE_SIZE);
  /** 命中筛选条件的总条数（后端按整段缓冲区统计） */
  const [total, setTotal] = useState(0);
  /** 后端缓冲区保留情况，用于说明「为什么只能翻到这么久以前」 */
  const [buffer, setBuffer] = useState({ retained: 0, capacity: 0 });
  /** 后端累计统计，日志页三张卡与仪表盘同一口径 */
  const [stats, setStats] = useState({ total: 0, blocked: 0, avgLatency: 0 });

  /** 表格可视容器，用来反推一页放得下几行 */
  const tableBoxRef = useRef<HTMLDivElement>(null);
  /** 表头与首行，量真实行高用 */
  const theadRef = useRef<HTMLTableSectionElement>(null);
  const firstRowRef = useRef<HTMLTableRowElement>(null);
  /** 表格下方的分页栏与统计卡片，扣掉它们才是表格能占的高度 */
  const paginationRef = useRef<HTMLDivElement>(null);
  const footerRef = useRef<HTMLDivElement>(null);

  /** 防止慢请求下轮询重入 */
  const fetchingRef = useRef(false);

  const totalPages = Math.max(1, Math.ceil(total / pageSize));
  /**
   * 筛选条件的指纹
   *
   * 用的是防抖后的关键字，而不是输入框实时值：否则每敲一个字符都会把页码打回第一页。
   */
  const filterKey = `${activeKeyword}\u0000${filterAction}\u0000${filterType}`;

  // 输入停止 300ms 后才把关键字交给查询，避免逐字符敲后端
  useEffect(() => {
    const timer = setTimeout(() => setActiveKeyword(searchQuery), SEARCH_DEBOUNCE);
    return () => clearTimeout(timer);
  }, [searchQuery]);

  /**
   * 拉取当前页
   *
   * 过滤全部交给后端：只有后端知道整段缓冲区里命中了多少条，
   * 前端拿这个总数才能算出正确页数，搜索也不会被当前页限制住。
   */
  const loadPage = useCallback(async () => {
    if (fetchingRef.current) return;
    fetchingRef.current = true;
    try {
      const [pageData, newStats] = await Promise.all([
        api.getLogsPage({
          offset: (page - 1) * pageSize,
          limit: pageSize,
          keyword: activeKeyword,
          action: filterAction,
          queryType: filterType,
        }),
        api.getStats().catch(() => null),
      ]);

      setLogs(pageData.logs);
      setTotal(pageData.total);
      setBuffer({ retained: pageData.retained, capacity: pageData.capacity });
      if (newStats) {
        setStats({
          total: newStats.total_queries,
          blocked: newStats.blocked_queries,
          avgLatency: newStats.avg_latency,
        });
      }
    } catch (e) {
      console.error("获取日志失败:", e);
    } finally {
      fetchingRef.current = false;
    }
  }, [page, pageSize, activeKeyword, filterAction, filterType]);

  // 筛选条件或每页条数变化 → 回到第一页
  //
  // 每页条数变了，同一个页码指向的记录区间也变了，留在原页码会看起来像「跳了一大段」。
  useEffect(() => {
    setPage(1);
  }, [filterKey, pageSize]);

  // 页码越界兜底：缓冲区被新记录挤掉旧记录后，总页数会缩小，
  // 停在已不存在的页码上会看到空列表
  useEffect(() => {
    if (page > totalPages) setPage(totalPages);
  }, [page, totalPages]);

  // 页码或每页条数变化后拉取
  useEffect(() => {
    loadPage();
  }, [loadPage]);

  /**
   * 实时刷新
   *
   * 只在第一页自动刷新：翻到历史页时，新记录会不断把旧记录往后顶，
   * 页面内容在用户眼皮底下漂移，还不如停住。
   */
  useEffect(() => {
    if (!isLive || page !== 1) return;
    const interval = setInterval(loadPage, LIVE_REFRESH_INTERVAL);
    return () => clearInterval(interval);
  }, [isLive, page, loadPage]);

  /**
   * 按可视高度智能计算每页条数
   *
   * 逻辑：表格上沿到窗口底部的剩余高度，扣掉分页栏与下方统计卡片，
   * 再扣表头，除以实测行高向下取整 —— 一页正好铺满可视区，
   * 既不出现半截行，也不用在表格内部再套一层滚动条。
   * 表头、首行、分页栏、卡片块都用 Ref 量真实高度，不写死魔法数字。
   */
  useLayoutEffect(() => {
    const measure = () => {
      const box = tableBoxRef.current;
      if (!box) return;

      const headerHeight = theadRef.current?.getBoundingClientRect().height || HEADER_HEIGHT_FALLBACK;
      const rowHeight = firstRowRef.current?.getBoundingClientRect().height || ROW_HEIGHT_FALLBACK;
      const paginationHeight =
        paginationRef.current?.getBoundingClientRect().height || PAGINATION_BAR_HEIGHT;
      const footerHeight =
        footerRef.current?.getBoundingClientRect().height || FOOTER_HEIGHT_FALLBACK;

      // 表格上沿到窗口底部，逐项扣掉下方的分页栏、卡片区间距、卡片块与页面底部内边距；
      // 表格上沿每次都要重新取：上方工具栏换行时它会变
      const available =
        window.innerHeight -
        box.getBoundingClientRect().top -
        paginationHeight -
        FOOTER_GAP -
        footerHeight -
        PAGE_BOTTOM_PADDING;

      const rows = Math.floor((available - headerHeight) / rowHeight);
      const next = Math.min(MAX_PAGE_SIZE, Math.max(MIN_PAGE_SIZE, rows || MIN_PAGE_SIZE));
      setPageSize((prev) => (prev === next ? prev : next));
    };

    measure();
    window.addEventListener("resize", measure);
    // 窗口大小没变但上方内容高度变了（例如统计卡片换行）时也要重算
    const observer = new ResizeObserver(measure);
    if (footerRef.current) observer.observe(footerRef.current);

    return () => {
      window.removeEventListener("resize", measure);
      observer.disconnect();
    };
  }, [logs.length, page]);

  /** 翻页后把表格拉回视口顶部，避免停在上一页的滚动位置 */
  const gotoPage = (next: number) => {
    const clamped = Math.min(totalPages, Math.max(1, next));
    if (clamped !== page) {
      setPage(clamped);
    }
  };

  // 清空日志：缓冲区清空但统计计数不清零，清完立刻重拉当前页
  const handleClearLogs = async () => {
    setLoading(true);
    try {
      await api.clearLogs();
      setPage(1);
      setLogs([]);
      setTotal(0);
      await loadPage();
    } catch (e) {
      console.error("清空日志失败:", e);
    } finally {
      setLoading(false);
    }
  };

  // 导出当前页（导出内容与屏幕上看到的一致）
  const handleExport = () => {
    const header = ["时间", "域名", "类型", "响应", "上游", "分组", "延迟", "状态"];
    const rows = logs.map((log) => [
      log.timestamp,
      log.domain,
      log.query_type,
      log.response,
      log.upstream,
      log.group || "-",
      `${log.latency_ms}ms`,
      dnsActionLabel(log.action),
    ]);

    const csv = [header.join(","), ...rows.map((row) => row.join(","))].join("\n");
    const blob = new Blob([csv], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `dns-logs-${new Date().toISOString().slice(0, 10)}.csv`;
    a.click();
    URL.revokeObjectURL(url);
  };

  // 阻止率按后端累计口径算，与「总请求数」同一分母
  const blockedRate = stats.total > 0 ? Math.round((stats.blocked / stats.total) * 100) : 0;
  /** 窗口够宽时页码按钮多显示几个，窄了自动收窄 */
  const pageWindow = pageSize > 40 ? 5 : 3;
  const pageNumbers: number[] = [];
  for (let offset = -Math.floor(pageWindow / 2); offset <= Math.floor(pageWindow / 2); offset++) {
    const candidate = page + offset;
    if (candidate >= 1 && candidate <= totalPages) pageNumbers.push(candidate);
  }

  return (
    <div className="space-y-6">
      {/* 工具栏 */}
      <div className="bg-card rounded-xl border p-4 space-y-4">
        <div className="flex flex-col md:flex-row gap-4 items-center justify-between">
          <div className="relative flex-1 w-full">
            <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="搜索域名..."
              className="w-full pl-10 pr-4 py-2 border rounded-lg bg-background focus:outline-none focus:ring-2 focus:ring-primary"
            />
          </div>

          <div className="flex items-center gap-2">
            <Filter className="w-4 h-4 text-muted-foreground" />
            <select
              value={filterAction}
              onChange={(e) => setFilterAction(e.target.value)}
              className="px-3 py-2 border rounded-lg bg-background text-sm"
            >
              <option value="all">全部状态</option>
              <option value="success">成功</option>
              <option value="blocked">阻止</option>
              <option value="cached">缓存</option>
              <option value="coalesced">合并</option>
              <option value="failed">失败</option>
            </select>
            <select
              value={filterType}
              onChange={(e) => setFilterType(e.target.value)}
              className="px-3 py-2 border rounded-lg bg-background text-sm"
            >
              <option value="all">全部类型</option>
              <option value="A">A</option>
              <option value="AAAA">AAAA</option>
              <option value="CNAME">CNAME</option>
              <option value="MX">MX</option>
            </select>
          </div>

          <div className="flex items-center gap-2">
            <button
              onClick={() => setIsLive(!isLive)}
              title={
                isLive
                  ? "实时刷新每 2 秒重拉第 1 页；翻到历史页时自动暂停，避免新记录把内容顶走"
                  : "点击恢复实时刷新"
              }
              className={`
                flex items-center gap-2 px-3 py-2 text-sm rounded-lg transition-colors
                ${isLive ? "bg-green-100 text-green-700 dark:bg-green-500/15 dark:text-green-400" : "border hover:bg-muted"}
              `}
            >
              {isLive ? (
                <>
                  <div className={`w-2 h-2 rounded-full bg-green-500 ${page === 1 ? "animate-pulse" : ""}`} />
                  {page === 1 ? "实时" : "实时已暂停"}
                </>
              ) : (
                <>
                  <RefreshCw className="w-4 h-4" />
                  刷新
                </>
              )}
            </button>
            <button
              onClick={handleExport}
              className="flex items-center gap-2 px-3 py-2 text-sm border rounded-lg hover:bg-muted"
            >
              <Download className="w-4 h-4" />
              导出本页
            </button>
            <button
              onClick={handleClearLogs}
              disabled={loading}
              className="flex items-center gap-2 px-3 py-2 text-sm border rounded-lg hover:bg-destructive hover:text-destructive-foreground"
            >
              <Trash2 className="w-4 h-4" />
              清空
            </button>
          </div>
        </div>
      </div>

      {/* 日志表格 */}
      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b flex items-center justify-between">
          <div className="flex items-center gap-2">
            <FileText className="w-5 h-5 text-primary" />
            <h3 className="font-semibold">DNS 查询日志</h3>
            <span className="text-sm text-muted-foreground">
              （共 {total} 条记录）
            </span>
          </div>
          <span
            className="text-xs text-muted-foreground"
            title="日志只保留在内存里，超出容量后自动丢弃最旧的记录；进程重启即清空"
          >
            缓冲区 {buffer.retained}/{buffer.capacity} 条
          </span>
        </div>

        {/* 高度由「每页条数 × 实测行高」决定，一页正好铺满，不在表格内部再滚动 */}
        <div ref={tableBoxRef} className="overflow-x-auto">
          <table className="w-full table-fixed min-w-[1000px]">
            <thead ref={theadRef} className="bg-card">
              <tr className="border-b bg-muted/50">
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[11%]">
                  <div className="flex items-center gap-2">
                    <Clock className="w-4 h-4" />
                    时间
                  </div>
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[26%]">
                  <div className="flex items-center gap-2">
                    <Globe className="w-4 h-4" />
                    域名
                  </div>
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[8%]">
                  类型
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[18%]">
                  响应
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[14%]">
                  <div className="flex items-center gap-2">
                    <Server className="w-4 h-4" />
                    上游
                  </div>
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[8%]">
                  分组
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[7%]">
                  延迟
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-[8%]">
                  状态
                </th>
              </tr>
            </thead>
            <tbody>
              {logs.length === 0 ? (
                <tr>
                  <td colSpan={8} className="p-8 text-center text-muted-foreground">
                    {total === 0 && buffer.retained === 0
                      ? "暂无日志记录，请先启动DNS服务"
                      : "没有匹配的日志"}
                  </td>
                </tr>
              ) : (
                logs.map((log, index) => (
                  <tr
                    key={log.id}
                    ref={index === 0 ? firstRowRef : undefined}
                    className="border-b hover:bg-muted/50 transition-colors"
                  >
                    <td className="p-3 text-sm font-mono text-muted-foreground whitespace-nowrap truncate">
                      {log.timestamp}
                    </td>
                    <td className="p-3 text-sm font-medium">
                      <div className="truncate">{log.domain}</div>
                    </td>
                    <td className="p-3 text-sm whitespace-nowrap">
                      <span className="px-2 py-1 rounded bg-muted text-xs font-mono">
                        {log.query_type}
                      </span>
                    </td>
                    <td className="p-3 text-sm font-mono truncate">{log.response}</td>
                    <td className="p-3 text-sm text-muted-foreground truncate">{log.upstream}</td>
                    <td className="p-3 text-sm whitespace-nowrap">
                      {log.group ? (
                        <Badge variant={log.group === "domestic" ? "info" : log.group === "proxy" ? "orange" : "neutral"}>
                          {log.group === "domestic" ? "直连" : log.group === "proxy" ? "代理" : log.group}
                        </Badge>
                      ) : "-"}
                    </td>
                    <td className="p-3 text-sm whitespace-nowrap">
                      <span
                        className={`font-mono tabular-nums ${
                          log.latency_ms < 10
                            ? "text-green-600 dark:text-green-400"
                            : log.latency_ms < 30
                            ? "text-yellow-600 dark:text-yellow-400"
                            : "text-red-600 dark:text-red-400"
                        }`}
                      >
                        {log.latency_ms}ms
                      </span>
                    </td>
                    <td className="p-3 text-sm whitespace-nowrap">
                      <Badge variant={dnsActionVariant(log.action)}>
                        {dnsActionLabel(log.action)}
                      </Badge>
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>

        {/* 分页栏 */}
        <div
          ref={paginationRef}
          className="flex flex-wrap items-center justify-between gap-3 border-t px-4 py-3"
        >
          <span className="text-sm text-muted-foreground">
            第 {page} / {totalPages} 页 · 每页 {pageSize} 条
            {isLive && page !== 1 && (
              <span className="ml-2 text-xs">（实时刷新只在第 1 页生效）</span>
            )}
          </span>

          <div className="flex items-center gap-1">
            <button
              onClick={() => gotoPage(1)}
              disabled={page === 1}
              title="第一页"
              className="p-2 border rounded-lg hover:bg-muted disabled:opacity-40 disabled:hover:bg-transparent"
            >
              <ChevronsLeft className="w-4 h-4" />
            </button>
            <button
              onClick={() => gotoPage(page - 1)}
              disabled={page === 1}
              title="上一页"
              className="p-2 border rounded-lg hover:bg-muted disabled:opacity-40 disabled:hover:bg-transparent"
            >
              <ChevronLeft className="w-4 h-4" />
            </button>

            {pageNumbers.map((number) => (
              <button
                key={number}
                onClick={() => gotoPage(number)}
                className={`min-w-[36px] px-2 py-1.5 text-sm border rounded-lg ${
                  number === page ? "bg-primary text-primary-foreground" : "hover:bg-muted"
                }`}
              >
                {number}
              </button>
            ))}

            <button
              onClick={() => gotoPage(page + 1)}
              disabled={page >= totalPages}
              title="下一页"
              className="p-2 border rounded-lg hover:bg-muted disabled:opacity-40 disabled:hover:bg-transparent"
            >
              <ChevronRight className="w-4 h-4" />
            </button>
            <button
              onClick={() => gotoPage(totalPages)}
              disabled={page >= totalPages}
              title="最后一页"
              className="p-2 border rounded-lg hover:bg-muted disabled:opacity-40 disabled:hover:bg-transparent"
            >
              <ChevronsRight className="w-4 h-4" />
            </button>
          </div>
        </div>
      </div>

      {/* 统计信息：与仪表盘同一口径（本次运行的累计值） */}
      <div ref={footerRef} className="grid grid-cols-1 md:grid-cols-3 gap-4">
        <div
          className="bg-card rounded-xl border p-4 cursor-help"
          title="本次运行的累计查询数，含缓存命中、阻止、请求合并与失败；进程重启后归零"
        >
          <p className="text-sm text-muted-foreground">总请求数（本次运行）</p>
          <p className="text-2xl font-bold tabular-nums">{stats.total}</p>
        </div>
        <div
          className="bg-card rounded-xl border p-4 cursor-help"
          title="只统计成功转向上游的查询，缓存命中、阻止与失败不计入；口径与仪表盘「平均延迟」一致"
        >
          <p className="text-sm text-muted-foreground">平均延迟（上游成功查询）</p>
          <p className="text-2xl font-bold tabular-nums">{stats.avgLatency.toFixed(1)}ms</p>
        </div>
        <div
          className="bg-card rounded-xl border p-4 cursor-help"
          title="阻止查询数 / 本次运行总查询数，与仪表盘口径一致"
        >
          <p className="text-sm text-muted-foreground">阻止率（本次运行）</p>
          <p className="text-2xl font-bold tabular-nums">{blockedRate}%</p>
        </div>
      </div>
    </div>
  );
}
