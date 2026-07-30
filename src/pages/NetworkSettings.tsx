import { useState, useEffect } from "react";
import {
  Shield,
  Settings,
  Loader2,
} from "lucide-react";
import { api, TunConfig } from "../lib/api";

export default function NetworkSettings() {
  const [loading, setLoading] = useState(false);
  const [message, setMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);
  const [tunConfig, setTunConfig] = useState<TunConfig>({
    enabled: false,
    interface_name: "DNS-Proxy-TUN",
    subnet: "10.10.0.0/24",
    gateway: "10.10.0.1",
    dns_servers: ["10.10.0.1"],
    auto_route: true,
  });
  const [initializing, setInitializing] = useState(true);

  useEffect(() => {
    loadConfig();
  }, []);

  const loadConfig = async () => {
    try {
      const config = await api.getTunConfig();
      setTunConfig(config);
    } catch (e) {
      console.error("加载配置失败:", e);
    } finally {
      setInitializing(false);
    }
  };

  const handleSaveConfig = async () => {
    setLoading(true);
    setMessage(null);
    try {
      await api.saveTunConfig(tunConfig);
      setMessage({ type: "success", text: "配置已保存" });
    } catch (e) {
      setMessage({ type: "error", text: "保存失败: " + e });
    } finally {
      setLoading(false);
    }
  };

  const updateConfig = (updates: Partial<TunConfig>) => {
    setTunConfig({ ...tunConfig, ...updates });
  };

  if (initializing) {
    return (
      <div className="flex items-center justify-center h-64">
        <Loader2 className="w-8 h-8 animate-spin text-primary" />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* 消息提示 */}
      {message && (
        <div className={`p-4 rounded-lg ${
          message.type === "success"
            ? "bg-green-100 text-green-700 border border-green-200"
            : "bg-red-100 text-red-700 border border-red-200"
        }`}>
          {message.text}
        </div>
      )}

      {/* TUN配置 */}
      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Shield className="w-5 h-5 text-primary" />
            <h3 className="font-semibold">TUN 配置</h3>
          </div>
        </div>

        <div className="p-4 space-y-4">
          <label className="flex items-center gap-3 cursor-pointer">
            <input
              type="checkbox"
              checked={tunConfig.enabled}
              onChange={(e) => updateConfig({ enabled: e.target.checked })}
              className="w-4 h-4"
            />
            <div>
              <p className="text-sm font-medium">启用TUN虚拟网卡</p>
              <p className="text-xs text-muted-foreground">需要管理员权限</p>
            </div>
          </label>

          <div>
            <label className="block text-sm font-medium mb-2">网卡名称</label>
            <input
              type="text"
              value={tunConfig.interface_name}
              onChange={(e) => updateConfig({ interface_name: e.target.value })}
              className="w-full px-3 py-2 border rounded-lg bg-background"
            />
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div>
              <label className="block text-sm font-medium mb-2">子网</label>
              <input
                type="text"
                value={tunConfig.subnet}
                onChange={(e) => updateConfig({ subnet: e.target.value })}
                className="w-full px-3 py-2 border rounded-lg bg-background"
              />
            </div>
            <div>
              <label className="block text-sm font-medium mb-2">网关</label>
              <input
                type="text"
                value={tunConfig.gateway}
                onChange={(e) => updateConfig({ gateway: e.target.value })}
                className="w-full px-3 py-2 border rounded-lg bg-background"
              />
            </div>
          </div>

          <label className="flex items-center gap-3 cursor-pointer">
            <input
              type="checkbox"
              checked={tunConfig.auto_route}
              onChange={(e) => updateConfig({ auto_route: e.target.checked })}
              className="w-4 h-4"
            />
            <div>
              <p className="text-sm font-medium">自动配置路由</p>
              <p className="text-xs text-muted-foreground">自动将DNS流量路由到TUN</p>
            </div>
          </label>

          <button
            onClick={handleSaveConfig}
            disabled={loading}
            className="flex items-center gap-2 px-4 py-2 text-sm border rounded-lg hover:bg-muted disabled:opacity-50"
          >
            {loading ? <Loader2 className="w-4 h-4 animate-spin" /> : <Settings className="w-4 h-4" />}
            保存配置
          </button>
        </div>
      </div>

      {/* 说明 */}
      <div className="bg-card rounded-xl border p-4">
        <h4 className="font-semibold mb-3">TUN模式说明</h4>
        <div className="space-y-2 text-sm text-muted-foreground">
          <p>• TUN模式会创建虚拟网卡，拦截所有DNS请求</p>
          <p>• 需要管理员权限运行</p>
          <p>• 首次使用需要安装WinTun驱动</p>
          <p>• 启动后系统DNS会自动设置为127.0.0.1</p>
          <p>• TUN 状态和启动按钮在首页「DNS 代理服务」卡片中</p>
        </div>
      </div>
    </div>
  );
}
