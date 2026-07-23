import { useState, useEffect, useCallback } from "react";
import { api, DnsQueryLog } from "../lib/api";
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
} from "lucide-react";

export default function Logs() {
  const [logs, setLogs] = useState<DnsQueryLog[]>([]);
  const [searchQuery, setSearchQuery] = useState("");
  const [filterAction, setFilterAction] = useState<string>("all");
  const [filterType, setFilterType] = useState<string>("all");
  const [isLive, setIsLive] = useState(true);
  const [loading, setLoading] = useState(false);

  // 刷新日志
  const refreshLogs = useCallback(async () => {
    try {
      const newLogs = await api.getLogs();
      setLogs(newLogs);
    } catch (e) {
      console.error("获取日志失败:", e);
    }
  }, []);

  // 定时刷新
  useEffect(() => {
    if (!isLive) return;
    refreshLogs();
    const interval = setInterval(refreshLogs, 2000);
    return () => clearInterval(interval);
  }, [isLive, refreshLogs]);

  // 筛选日志
  const filteredLogs = logs.filter((log) => {
    const matchesSearch = log.domain.toLowerCase().includes(searchQuery.toLowerCase());
    const matchesAction = filterAction === "all" || log.action === filterAction;
    const matchesType = filterType === "all" || log.query_type === filterType;
    return matchesSearch && matchesAction && matchesType;
  });

  // 清空日志
  const handleClearLogs = async () => {
    setLoading(true);
    try {
      await api.clearLogs();
      await refreshLogs();
    } catch (e) {
      console.error("清空日志失败:", e);
    } finally {
      setLoading(false);
    }
  };

  // 导出日志
  const handleExportLogs = () => {
    const csv = [
      ["时间", "域名", "类型", "响应", "上游", "分组", "延迟", "状态"].join(","),
      ...filteredLogs.map((log) =>
        [
          log.timestamp,
          log.domain,
          log.query_type,
          log.response,
          log.upstream,
          log.group || "-",
          `${log.latency_ms}ms`,
          log.action === "success" ? "成功" : log.action === "blocked" ? "阻止" : "缓存",
        ].join(",")
      ),
    ].join("\n");

    const blob = new Blob([csv], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `dns-logs-${new Date().toISOString().slice(0, 10)}.csv`;
    a.click();
    URL.revokeObjectURL(url);
  };

  // 计算统计
  const totalLogs = logs.length;
  const avgLatency = logs.length > 0
    ? Math.round(logs.reduce((acc, log) => acc + log.latency_ms, 0) / logs.length)
    : 0;
  const blockedRate = logs.length > 0
    ? Math.round((logs.filter((l) => l.action === "blocked").length / logs.length) * 100)
    : 0;

  return (
    <div className="space-y-6">
      {/* 工具栏 */}
      <div className="bg-card rounded-xl border p-4">
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
              className={`
                flex items-center gap-2 px-3 py-2 text-sm rounded-lg transition-colors
                ${isLive ? "bg-green-100 text-green-700" : "border hover:bg-muted"}
              `}
            >
              {isLive ? (
                <>
                  <div className="w-2 h-2 rounded-full bg-green-500 animate-pulse" />
                  实时
                </>
              ) : (
                <>
                  <RefreshCw className="w-4 h-4" />
                  刷新
                </>
              )}
            </button>
            <button
              onClick={handleExportLogs}
              className="flex items-center gap-2 px-3 py-2 text-sm border rounded-lg hover:bg-muted"
            >
              <Download className="w-4 h-4" />
              导出
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
              ({filteredLogs.length} 条记录)
            </span>
          </div>
        </div>

        <div className="overflow-x-auto max-h-[600px] overflow-y-auto">
          <table className="w-full">
            <thead className="sticky top-0 bg-card z-10">
              <tr className="border-b bg-muted/50">
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">
                  <div className="flex items-center gap-2">
                    <Clock className="w-4 h-4" />
                    时间
                  </div>
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">
                  <div className="flex items-center gap-2">
                    <Globe className="w-4 h-4" />
                    域名
                  </div>
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">
                  类型
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">
                  响应
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">
                  <div className="flex items-center gap-2">
                    <Server className="w-4 h-4" />
                    上游
                  </div>
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">
                  分组
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">
                  延迟
                </th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">
                  状态
                </th>
              </tr>
            </thead>
            <tbody>
              {filteredLogs.length === 0 ? (
                <tr>
                  <td colSpan={8} className="p-8 text-center text-muted-foreground">
                    {logs.length === 0 ? "暂无日志记录，请先启动DNS服务" : "没有匹配的日志"}
                  </td>
                </tr>
              ) : (
                filteredLogs.map((log) => (
                  <tr key={log.id} className="border-b hover:bg-muted/50 transition-colors">
                    <td className="p-3 text-sm font-mono text-muted-foreground">
                      {log.timestamp}
                    </td>
                    <td className="p-3 text-sm font-medium">{log.domain}</td>
                    <td className="p-3 text-sm">
                      <span className="px-2 py-1 rounded bg-muted text-xs font-mono">
                        {log.query_type}
                      </span>
                    </td>
                    <td className="p-3 text-sm font-mono">{log.response}</td>
                    <td className="p-3 text-sm text-muted-foreground">{log.upstream}</td>
                    <td className="p-3 text-sm">
                      {log.group ? (
                        <span className={`px-2 py-1 rounded text-xs ${
                          log.group === "domestic" ? "bg-blue-100 text-blue-700" :
                          log.group === "foreign" ? "bg-purple-100 text-purple-700" :
                          log.group === "proxy" ? "bg-orange-100 text-orange-700" :
                          "bg-gray-100 text-gray-700"
                        }`}>
                          {log.group}
                        </span>
                      ) : "-"}
                    </td>
                    <td className="p-3 text-sm">
                      <span
                        className={`font-mono ${
                          log.latency_ms < 10
                            ? "text-green-600"
                            : log.latency_ms < 30
                            ? "text-yellow-600"
                            : "text-red-600"
                        }`}
                      >
                        {log.latency_ms}ms
                      </span>
                    </td>
                    <td className="p-3 text-sm">
                      <span
                        className={`
                          px-2 py-1 rounded-full text-xs font-medium
                          ${
                            log.action === "success"
                              ? "bg-green-100 text-green-700"
                              : log.action === "blocked"
                              ? "bg-red-100 text-red-700"
                              : "bg-blue-100 text-blue-700"
                          }
                        `}
                      >
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

      {/* 统计信息 */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        <div className="bg-card rounded-xl border p-4">
          <p className="text-sm text-muted-foreground">总请求数</p>
          <p className="text-2xl font-bold">{totalLogs}</p>
        </div>
        <div className="bg-card rounded-xl border p-4">
          <p className="text-sm text-muted-foreground">平均延迟</p>
          <p className="text-2xl font-bold">{avgLatency}ms</p>
        </div>
        <div className="bg-card rounded-xl border p-4">
          <p className="text-sm text-muted-foreground">阻止率</p>
          <p className="text-2xl font-bold">{blockedRate}%</p>
        </div>
      </div>
    </div>
  );
}
