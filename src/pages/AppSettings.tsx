import { useEffect, useState } from "react";
import { Clock3, MonitorDown, Palette, Power, RefreshCw, Save, TimerReset } from "lucide-react";
import { api, AppConfig } from "../lib/api";
import { applyAppFont, FONT_OPTIONS, normalizeAppFont } from "../lib/font";
import MessageBanner from "../components/ui/MessageBanner";

const MAX_STARTUP_DELAY_SECONDS = 3600;
const MAX_RESTART_INTERVAL_HOURS = 168;

export default function AppSettings() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);

  useEffect(() => {
    const load = async () => {
      try {
        const [loadedConfig, enabled] = await Promise.all([
          api.getConfig(),
          api.isAutostartEnabled(),
        ]);
        const normalizedConfig = { ...loadedConfig, app_font: normalizeAppFont(loadedConfig.app_font) };
        setConfig(normalizedConfig);
        applyAppFont(normalizedConfig.app_font);
        setAutostartEnabled(enabled);
      } catch (error) {
        setMessage({ type: "error", text: "加载应用设置失败: " + error });
      } finally {
        setLoading(false);
      }
    };
    load();
  }, []);

  const setNumber = (
    field: "startup_delay_seconds" | "dns_restart_interval_hours",
    rawValue: string,
    max: number,
  ) => {
    if (!config) return;
    const parsed = Number.parseInt(rawValue, 10);
    const value = Number.isFinite(parsed) ? Math.min(Math.max(parsed, 0), max) : 0;
    setConfig({ ...config, [field]: value });
  };

  const handleFontChange = (font: string) => {
    if (!config) return;
    applyAppFont(font);
    setConfig({ ...config, app_font: font });
  };

  const handleToggleAutostart = async () => {
    const next = !autostartEnabled;
    try {
      await api.setAutostart(next);
      setAutostartEnabled(next);
      setMessage({ type: "success", text: next ? "已启用开机自启动" : "已取消开机自启动" });
    } catch (error) {
      setMessage({ type: "error", text: "设置开机自启动失败: " + error });
    }
  };

  const handleSave = async () => {
    if (!config) return;
    setSaving(true);
    setMessage(null);
    try {
      await api.saveAppSettings(config);
      setMessage({
        type: "success",
        text: "应用设置已保存。延迟启动仅在下次启动应用时生效；定时重启会从现在开始按新间隔计时。",
      });
    } catch (error) {
      setMessage({ type: "error", text: "保存应用设置失败: " + error });
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <RefreshCw className="w-8 h-8 animate-spin text-primary" />
        <span className="ml-2">加载应用设置中...</span>
      </div>
    );
  }

  if (!config) {
    return <p className="text-sm text-destructive">{message?.text || "应用设置暂时不可用"}</p>;
  }

  return (
    <div className="space-y-6">
      {message && <MessageBanner type={message.type} text={message.text} />}

      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b flex items-center gap-2">
          <Power className="w-5 h-5 text-primary" />
          <h3 className="font-semibold">启动行为</h3>
        </div>
        <div className="p-4 space-y-5">
          <label className="flex items-start gap-3 cursor-pointer">
            <input
              type="checkbox"
              checked={autostartEnabled}
              onChange={handleToggleAutostart}
              className="w-4 h-4 mt-0.5"
            />
            <div>
              <p className="text-sm font-medium">开机自启动</p>
              <p className="text-xs text-muted-foreground">通过 Windows 计划任务以最高权限启动，满足本地 DNS 监听需要。</p>
            </div>
          </label>

          <label className="flex items-start gap-3 cursor-pointer">
            <input
              type="checkbox"
              checked={config.start_minimized}
              onChange={(event) => setConfig({ ...config, start_minimized: event.target.checked })}
              className="w-4 h-4 mt-0.5"
            />
            <div>
              <p className="text-sm font-medium flex items-center gap-1.5"><MonitorDown className="w-4 h-4" />启动后最小化</p>
              <p className="text-xs text-muted-foreground">窗口启动后隐藏到系统托盘，可从托盘菜单恢复。</p>
            </div>
          </label>

          <div className="max-w-sm">
            <label className="block text-sm font-medium mb-2 flex items-center gap-1.5"><Clock3 className="w-4 h-4" />延迟启动 DNS 服务（秒）</label>
            <input
              type="number"
              min={0}
              max={MAX_STARTUP_DELAY_SECONDS}
              value={config.startup_delay_seconds}
              onChange={(event) => setNumber("startup_delay_seconds", event.target.value, MAX_STARTUP_DELAY_SECONDS)}
              className="w-full px-3 py-2 border rounded-lg bg-background"
            />
            <p className="text-xs text-muted-foreground mt-1">0 表示立即启动，最长 3600 秒。用于等待网卡、VPN 或其他代理组件先就绪；仅下次启动应用生效。</p>
          </div>
        </div>
      </div>

      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b flex items-center gap-2">
          <Palette className="w-5 h-5 text-primary" />
          <h3 className="font-semibold">界面显示</h3>
        </div>
        <div className="p-4 max-w-xl">
          <label htmlFor="app-font" className="block text-sm font-medium mb-2">界面字体</label>
          <select
            id="app-font"
            value={config.app_font}
            onChange={(event) => handleFontChange(event.target.value)}
            className="w-full max-w-sm px-3 py-2 border rounded-lg bg-background"
          >
            {FONT_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>{option.label}</option>
            ))}
          </select>
        </div>
      </div>

      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b flex items-center gap-2">
          <TimerReset className="w-5 h-5 text-primary" />
          <h3 className="font-semibold">稳定性维护</h3>
        </div>
        <div className="p-4 space-y-3 max-w-xl">
          <div className="max-w-sm">
            <label className="block text-sm font-medium mb-2">定时重启 DNS 服务（小时）</label>
            <input
              type="number"
              min={0}
              max={MAX_RESTART_INTERVAL_HOURS}
              value={config.dns_restart_interval_hours}
              onChange={(event) => setNumber("dns_restart_interval_hours", event.target.value, MAX_RESTART_INTERVAL_HOURS)}
              className="w-full px-3 py-2 border rounded-lg bg-background"
            />
          </div>
          <p className="text-xs text-muted-foreground leading-5">
            0 表示关闭，建议先设为 12 小时观察。执行时不会退出桌面应用：它会先停止 DNS 服务、还原系统 DNS、清理缓存和上游连接，再启动服务并重新接管 DNS，避免长期运行的卡滞状态累积。
          </p>
        </div>
      </div>

      <div className="flex justify-end">
        <button
          onClick={handleSave}
          disabled={saving}
          className="flex items-center gap-2 px-5 py-2.5 bg-primary text-primary-foreground rounded-lg hover:bg-primary/90 disabled:opacity-60"
        >
          {saving ? <RefreshCw className="w-4 h-4 animate-spin" /> : <Save className="w-4 h-4" />}
          {saving ? "保存中..." : "保存应用设置"}
        </button>
      </div>
    </div>
  );
}
