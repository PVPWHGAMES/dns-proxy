import { useCallback, useEffect, useRef, useState } from "react";
import { api, FlowEntry, FlowMonitorStatus } from "../lib/api";
import {
  Activity,
  AlertTriangle,
  Network,
  Play,
  RefreshCw,
  Square,
} from "lucide-react";

/** 观察运行时的刷新间隔 */
const REFRESH_INTERVAL_MS = 1000;
/** 与后端快照上限一致，超出部分用总数交代 */
const MAX_ROWS = 300;

export default function FlowMonitor() {
  const [status, setStatus] = useState<FlowMonitorStatus | null>(null);
  const [flows, setFlows] = useState<FlowEntry[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** 防止慢请求下轮询重入 */
  const fetchingRef = useRef(false);

  const refresh = useCallback(async () => {
    if (fetchingRef.current) return;
    fetchingRef.current = true;
    try {
      const next = await api.getFlowMonitorStatus();
      setStatus(next);
      // 未运行时不必再拉列表：后端此时也不会产生新事件
      setFlows(next.running ? await api.getActiveFlows() : []);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      fetchingRef.current = false;
    }
  }, []);

  // 首次进入先取一次状态，用于显示「是否具备运行条件」
  useEffect(() => {
    refresh();
  }, [refresh]);

  // 观察中才轮询：只依赖运行标志，避免每次状态对象变化都重建定时器
  const running = status?.running ?? false;
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(refresh, REFRESH_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [running, refresh]);

  const handleStart = async () => {
    setBusy(true);
    setError(null);
    try {
      setStatus(await api.startFlowMonitor());
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleStop = async () => {
    setBusy(true);
    try {
      setStatus(await api.stopFlowMonitor());
      setFlows([]);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="p-6 space-y-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2">
            <Activity className="w-6 h-6 text-primary" />
            流量观察
            <span className="text-xs font-medium px-2 py-0.5 rounded bg-amber-500/15 text-amber-600">
              测试功能
            </span>
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            在 WinDivert 的 FLOW 层只读观察本机连接，可区分进程与流。
            句柄以「嗅探 + 只收」模式打开，不修改、不丢弃、不注入任何数据包。
          </p>
          <p className="text-xs text-amber-600 mt-1">
            测试功能：依赖 WinDivert 驱动，尚未在真实网络下长期验证，结果仅供参考。
          </p>
        </div>

        <div className="flex items-center gap-2 shrink-0">
          <button
            onClick={refresh}
            className="flex items-center gap-2 px-3 py-2 rounded-lg border text-sm hover:bg-accent transition-colors"
            title="立即刷新"
          >
            <RefreshCw className="w-4 h-4" />
            刷新
          </button>
          {running ? (
            <button
              onClick={handleStop}
              disabled={busy}
              className="flex items-center gap-2 px-4 py-2 rounded-lg bg-destructive text-destructive-foreground text-sm font-medium hover:opacity-90 disabled:opacity-50 transition-opacity"
            >
              <Square className="w-4 h-4" />
              停止观察
            </button>
          ) : (
            <button
              onClick={handleStart}
              disabled={busy}
              className="flex items-center gap-2 px-4 py-2 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:opacity-90 disabled:opacity-50 transition-opacity"
            >
              <Play className="w-4 h-4" />
              开始观察
            </button>
          )}
        </div>
      </div>

      {error && (
        <div className="flex items-start gap-2 p-3 rounded-lg border border-destructive/40 bg-destructive/10 text-sm">
          <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0 text-destructive" />
          <span>{error}</span>
        </div>
      )}

      {!error && status?.message && (
        <div className="flex items-start gap-2 p-3 rounded-lg border border-amber-500/40 bg-amber-500/10 text-sm">
          <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0 text-amber-500" />
          <span>{status.message}</span>
        </div>
      )}

      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
        <StatBox
          label="运行状态"
          value={running ? "观察中" : "已停止"}
          tone={running ? "text-green-500" : "text-muted-foreground"}
        />
        <StatBox label="活动流" value={String(status?.active_count ?? 0)} />
        <StatBox label="涉及进程" value={String(status?.process_count ?? 0)} />
        <StatBox
          label="累计建立 / 删除"
          value={`${status?.total_established ?? 0} / ${status?.total_deleted ?? 0}`}
        />
      </div>

      <div className="border rounded-lg overflow-hidden">
        <div className="px-4 py-3 border-b bg-muted/40 flex items-center justify-between">
          <span className="font-medium flex items-center gap-2">
            <Network className="w-4 h-4" />
            活动连接
          </span>
          <span className="text-xs text-muted-foreground">
            {flows.length > 0 && `显示 ${flows.length} 条`}
            {status && status.active_count > flows.length
              ? `（共 ${status.active_count} 条，仅显示最近 ${MAX_ROWS} 条）`
              : ""}
          </span>
        </div>

        {flows.length === 0 ? (
          <div className="px-4 py-12 text-center text-sm text-muted-foreground">
            {running
              ? "暂未观察到流量"
              : status?.available
                ? "点击「开始观察」后即可看到本机连接与对应进程"
                : "缺少 WinDivert 运行库（WinDivert.dll 与 WinDivert64.sys）"}
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead className="bg-muted/30 text-muted-foreground">
                <tr>
                  <th className="text-left font-medium px-4 py-2 whitespace-nowrap">时间</th>
                  <th className="text-left font-medium px-4 py-2 whitespace-nowrap">进程</th>
                  <th className="text-right font-medium px-4 py-2 whitespace-nowrap">PID</th>
                  <th className="text-left font-medium px-4 py-2 whitespace-nowrap">协议</th>
                  <th className="text-left font-medium px-4 py-2 whitespace-nowrap">本地</th>
                  <th className="text-left font-medium px-4 py-2 whitespace-nowrap">远程</th>
                </tr>
              </thead>
              <tbody>
                {flows.map((flow) => (
                  <tr
                    key={flow.endpoint_id}
                    className="border-t hover:bg-accent/40 transition-colors"
                  >
                    <td className="px-4 py-2 text-muted-foreground whitespace-nowrap">
                      {flow.established_at}
                    </td>
                    <td
                      className="px-4 py-2 max-w-[220px] truncate"
                      title={flow.process_name}
                    >
                      {flow.process_name}
                      {flow.loopback && (
                        <span className="ml-2 text-xs px-1.5 py-0.5 rounded bg-muted text-muted-foreground">
                          回环
                        </span>
                      )}
                    </td>
                    <td className="px-4 py-2 text-right text-muted-foreground">
                      {flow.process_id}
                    </td>
                    <td className="px-4 py-2">{flow.protocol}</td>
                    <td className="px-4 py-2 whitespace-nowrap">
                      {flow.local_addr}:{flow.local_port}
                    </td>
                    <td className="px-4 py-2 whitespace-nowrap">
                      <span className="text-muted-foreground mr-1">
                        {flow.outbound ? "→" : "←"}
                      </span>
                      {flow.remote_addr}:{flow.remote_port}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}

function StatBox({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: string;
}) {
  return (
    <div className="border rounded-lg px-4 py-3 bg-card">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className={`text-lg font-semibold mt-1 ${tone ?? ""}`}>{value}</p>
    </div>
  );
}
