import { useCallback, useEffect, useRef, useState } from "react";
import { api, RedirectOverview, RedirectRequest } from "../lib/api";
import {
  AlertTriangle,
  ArrowLeftRight,
  Play,
  RefreshCw,
  ShieldAlert,
  Square,
} from "lucide-react";

/** 运行时的刷新间隔 */
const REFRESH_INTERVAL_MS = 1000;

const EMPTY_FORM: RedirectRequest = {
  target_addr: "",
  target_port: 80,
  relay_bind: "",
  relay_port: 34567,
  sentinel_port: 34568,
};

export default function Redirect() {
  const [form, setForm] = useState<RedirectRequest>(EMPTY_FORM);
  const [confirmed, setConfirmed] = useState(false);
  const [overview, setOverview] = useState<RedirectOverview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fetchingRef = useRef(false);

  const refresh = useCallback(async () => {
    if (fetchingRef.current) return;
    fetchingRef.current = true;
    try {
      setOverview(await api.getRedirectStatus());
    } catch (e) {
      setError(String(e));
    } finally {
      fetchingRef.current = false;
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const running = overview?.redirect.running ?? false;
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(refresh, REFRESH_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [running, refresh]);

  const update = (patch: Partial<RedirectRequest>) => {
    setForm((previous) => ({ ...previous, ...patch }));
  };

  const handleStart = async () => {
    setBusy(true);
    setError(null);
    try {
      setOverview(await api.startRedirect(form));
    } catch (e) {
      setError(String(e));
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  const handleStop = async () => {
    setBusy(true);
    try {
      setOverview(await api.stopRedirect());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const redirect = overview?.redirect;
  const relay = overview?.relay;

  return (
    <div className="p-6 space-y-6">
      <div>
        <h1 className="text-2xl font-bold flex items-center gap-2">
          <ArrowLeftRight className="w-6 h-6 text-primary" />
          流量接管
          <span className="text-xs font-medium px-2 py-0.5 rounded bg-amber-500/15 text-amber-600">
            测试功能
          </span>
        </h1>
        <p className="text-sm text-muted-foreground mt-1">
          把指向某一个目标的 TCP 连接改投到本机中继，由中继连回原目标。
          用于验证全局接管链路，目前只支持单个 IPv4 目标。
        </p>
        <p className="text-xs text-amber-600 mt-1">
          测试功能：会真实改写并重新注入报文，尚未在真实互联网目标上验证，请仅在受控环境中使用。
        </p>
      </div>

      <div className="p-4 rounded-lg border border-amber-500/40 bg-amber-500/10 space-y-2 text-sm">
        <div className="flex items-center gap-2 font-medium text-amber-600">
          <ShieldAlert className="w-4 h-4" />
          开始之前请读完这段
        </div>
        <ul className="list-disc pl-5 space-y-1 text-muted-foreground">
          <li>
            它**不再是只读**：匹配到的报文会被真的取走再重新注入，只有那一个目标端口受影响，
            但该目标的连接在接管期间可能失败。
          </li>
          <li>
            点「立即停止」即回滚；直接退出程序同样是回滚——驱动在句柄关闭时会把未超时的
            报文重新注入协议栈，不会留下断网状态。
          </li>
          <li>
            中继绑定地址要填<strong>面向客户端的网卡地址</strong>：注入包的目的地址是客户端
            本机地址，绑回环地址收不到。填 <code>0.0.0.0</code> 等于把中继开放给整个网段，
            仅在受控网络里使用。
          </li>
          <li>短时间内的连接可能被丢弃（队列时限最长 2 秒），TCP 会靠重传恢复。</li>
          <li>该能力尚未在真实互联网目标上验证过，只在回环与 WSL 虚拟对端上验证过。</li>
        </ul>
      </div>

      {error && (
        <div className="flex items-start gap-2 p-3 rounded-lg border border-destructive/40 bg-destructive/10 text-sm">
          <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0 text-destructive" />
          <span>{error}</span>
        </div>
      )}

      {redirect?.message && !error && (
        <div className="flex items-start gap-2 p-3 rounded-lg border border-amber-500/40 bg-amber-500/10 text-sm">
          <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0 text-amber-500" />
          <span>{redirect.message}</span>
        </div>
      )}

      <div className="border rounded-lg p-4 space-y-4">
        <div className="grid grid-cols-2 lg:grid-cols-5 gap-4">
          <Field label="目标地址">
            <input
              value={form.target_addr}
              onChange={(e) => update({ target_addr: e.target.value })}
              placeholder="例如 93.184.216.34"
              disabled={running}
              className="w-full px-3 py-2 rounded-lg border bg-background text-sm disabled:opacity-60"
            />
          </Field>
          <Field label="目标端口">
            <input
              type="number"
              value={form.target_port}
              onChange={(e) => update({ target_port: Number(e.target.value) })}
              disabled={running}
              className="w-full px-3 py-2 rounded-lg border bg-background text-sm disabled:opacity-60"
            />
          </Field>
          <Field label="中继绑定地址">
            <input
              value={form.relay_bind}
              onChange={(e) => update({ relay_bind: e.target.value })}
              placeholder="面向客户端的网卡地址"
              disabled={running}
              className="w-full px-3 py-2 rounded-lg border bg-background text-sm disabled:opacity-60"
            />
          </Field>
          <Field label="中继端口">
            <input
              type="number"
              value={form.relay_port}
              onChange={(e) => update({ relay_port: Number(e.target.value) })}
              disabled={running}
              className="w-full px-3 py-2 rounded-lg border bg-background text-sm disabled:opacity-60"
            />
          </Field>
          <Field label="哨兵端口">
            <input
              type="number"
              value={form.sentinel_port}
              onChange={(e) => update({ sentinel_port: Number(e.target.value) })}
              disabled={running}
              className="w-full px-3 py-2 rounded-lg border bg-background text-sm disabled:opacity-60"
            />
          </Field>
        </div>

        <p className="text-xs text-muted-foreground">
          中继端口与哨兵端口都要避开 Windows 动态端口范围（49152 起），否则可能与客户端
          临时源端口冲突后被误判；两者也不能相同。
        </p>

        {!running && (
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={confirmed}
              onChange={(e) => setConfirmed(e.target.checked)}
            />
            我明白它会在接管期间取走并重新注入上述目标的报文
          </label>
        )}

        <div className="flex items-center gap-2">
          {running ? (
            <button
              onClick={handleStop}
              disabled={busy}
              className="flex items-center gap-2 px-4 py-2 rounded-lg bg-destructive text-destructive-foreground text-sm font-medium hover:opacity-90 disabled:opacity-50"
            >
              <Square className="w-4 h-4" />
              立即停止（回滚）
            </button>
          ) : (
            <button
              onClick={handleStart}
              disabled={busy || !confirmed}
              className="flex items-center gap-2 px-4 py-2 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:opacity-90 disabled:opacity-50"
            >
              <Play className="w-4 h-4" />
              开始接管
            </button>
          )}
          <button
            onClick={refresh}
            className="flex items-center gap-2 px-3 py-2 rounded-lg border text-sm hover:bg-accent"
          >
            <RefreshCw className="w-4 h-4" />
            刷新
          </button>
          <span className={`text-sm ${running ? "text-green-500" : "text-muted-foreground"}`}>
            {running
              ? `接管中：${redirect?.rule?.target ?? ""}`
              : redirect?.available
                ? "未接管"
                : "缺少 WinDivert 运行库"}
          </span>
        </div>
      </div>

      <div className="grid lg:grid-cols-2 gap-4">
        <div className="border rounded-lg p-4 space-y-3">
          <h2 className="font-medium">报文改写（四个分支）</h2>
          <Counter label="客户端 → 中继" value={redirect?.to_relay ?? 0} />
          <Counter label="中继 → 客户端" value={redirect?.to_client ?? 0} />
          <Counter label="拨号映射到真实目标" value={redirect?.dial_mapped ?? 0} />
          <Counter label="目标回包映射到哨兵端口" value={redirect?.reply_mapped ?? 0} />
          <div className="pt-2 border-t space-y-1">
            <Counter label="原样放行（未归类）" value={redirect?.passed_through ?? 0} />
            <Counter label="跳过（分片等）" value={redirect?.skipped ?? 0} />
            <Counter
              label="注入失败"
              value={redirect?.send_failed ?? 0}
              tone={(redirect?.send_failed ?? 0) > 0 ? "text-destructive" : undefined}
            />
          </div>
        </div>

        <div className="border rounded-lg p-4 space-y-3">
          <h2 className="font-medium">本地中继</h2>
          <Counter label="已接受连接" value={relay?.accepted ?? 0} />
          <Counter label="当前活动" value={relay?.active ?? 0} />
          <Counter label="上行字节" value={relay?.bytes_up ?? 0} />
          <Counter label="下行字节" value={relay?.bytes_down ?? 0} />
          <div className="pt-2 border-t text-xs text-muted-foreground space-y-1">
            <p>监听：{relay?.listen_addr ?? "—"}</p>
            <p>拨向：{relay?.destination ?? "—"}</p>
            {relay?.last_error && <p className="text-destructive">错误：{relay.last_error}</p>}
          </div>
        </div>
      </div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="space-y-1 block">
      <span className="text-xs text-muted-foreground">{label}</span>
      {children}
    </label>
  );
}

function Counter({
  label,
  value,
  tone,
}: {
  label: string;
  value: number;
  tone?: string;
}) {
  return (
    <div className="flex items-center justify-between text-sm">
      <span className="text-muted-foreground">{label}</span>
      <span className={`font-mono ${tone ?? ""}`}>{value}</span>
    </div>
  );
}
