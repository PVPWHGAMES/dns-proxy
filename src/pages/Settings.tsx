import { useState, useEffect } from "react";
import { Save, Plus, Trash2, Server, Wifi, Database, Zap, RefreshCw, Timer } from "lucide-react";
import { api, AppConfig, DnsServer, DnsProtocol, DnsStrategy, DnsLatencyResult, ServerGroup } from "../lib/api";

export default function Settings() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);
  const [latencyResults, setLatencyResults] = useState<DnsLatencyResult[]>([]);
  const [testingLatency, setTestingLatency] = useState(false);
  const [lastTestTime, setLastTestTime] = useState<string | null>(null);

  // 加载配置
  useEffect(() => {
    loadConfig();
    loadLatencyResults();
  }, []);

  const loadConfig = async () => {
    setLoading(true);
    try {
      const cfg = await api.getConfig();
      setConfig(cfg);
    } catch (e) {
      console.error("加载配置失败:", e);
      setMessage({ type: "error", text: "加载配置失败: " + e });
    } finally {
      setLoading(false);
    }
  };

  const loadLatencyResults = async () => {
    try {
      const [results, lastTest] = await api.getLatencyResults();
      setLatencyResults(results);
      setLastTestTime(lastTest);
    } catch (e) {
      console.error("加载延迟结果失败:", e);
    }
  };

  // 保存配置并自动重启服务
  const handleSave = async () => {
    if (!config) return;
    setSaving(true);
    setMessage(null);
    try {
      await api.saveConfig(config);

      // 自动重启服务使配置生效
      try {
        const isRunning = await api.getServerStatus();
        if (isRunning) {
          await api.stopServer();
          await new Promise((resolve) => setTimeout(resolve, 500));
        }
        await api.startServer();
        setMessage({ type: "success", text: "配置已保存，服务已自动重启。" });
      } catch (restartErr) {
        setMessage({ type: "success", text: "配置已保存，但服务重启失败: " + restartErr });
      }
    } catch (e) {
      setMessage({ type: "error", text: "保存失败: " + e });
    } finally {
      setSaving(false);
    }
  };

  // 添加服务器
  const handleAddServer = () => {
    if (!config) return;
    const newServer: DnsServer = {
      name: "",
      ip: "",
      port: 53,
      enabled: true,
      protocol: "udp",
      doh_url: "",
      group: "default",
    };
    setConfig({
      ...config,
      upstream: [...config.upstream, newServer],
    });
  };

  // 删除服务器
  const handleRemoveServer = (index: number) => {
    if (!config) return;
    setConfig({
      ...config,
      upstream: config.upstream.filter((_, i) => i !== index),
    });
  };

  // 切换服务器启用状态
  const handleToggleServer = (index: number) => {
    if (!config) return;
    const newUpstream = [...config.upstream];
    newUpstream[index] = { ...newUpstream[index], enabled: !newUpstream[index].enabled };
    setConfig({ ...config, upstream: newUpstream });
  };

  // 更新服务器配置
  const handleServerChange = (index: number, field: keyof DnsServer, value: any) => {
    if (!config) return;
    const newUpstream = [...config.upstream];
    newUpstream[index] = { ...newUpstream[index], [field]: value };
    setConfig({ ...config, upstream: newUpstream });
  };

  // 测试DNS延迟
  const handleTestLatency = async () => {
    setTestingLatency(true);
    setLatencyResults([]);
    try {
      const results = await api.testDnsLatency();
      setLatencyResults(results);
      // 更新上次测试时间
      const now = new Date();
      setLastTestTime(`${now.getHours().toString().padStart(2, '0')}:${now.getMinutes().toString().padStart(2, '0')}:${now.getSeconds().toString().padStart(2, '0')}`);
    } catch (e) {
      setMessage({ type: "error", text: "测试延迟失败: " + e });
    } finally {
      setTestingLatency(false);
    }
  };

  // 获取延迟显示颜色
  const getLatencyColor = (latencyMs?: number) => {
    if (!latencyMs) return "text-muted-foreground";
    if (latencyMs < 50) return "text-green-500";
    if (latencyMs < 100) return "text-yellow-500";
    return "text-red-500";
  };

  if (loading || !config) {
    return (
      <div className="flex items-center justify-center h-64">
        <RefreshCw className="w-8 h-8 animate-spin text-primary" />
        <span className="ml-2">加载配置中...</span>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* 消息提示 */}
      {message && (
        <div
          className={`p-4 rounded-lg ${
            message.type === "success"
              ? "bg-green-100 text-green-700 border border-green-200"
              : "bg-red-100 text-red-700 border border-red-200"
          }`}
        >
          {message.text}
        </div>
      )}

      {/* 监听设置 */}
      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b flex items-center gap-2">
          <Wifi className="w-5 h-5 text-primary" />
          <h3 className="font-semibold">监听设置</h3>
        </div>
        <div className="p-4 space-y-4">
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div>
              <label className="block text-sm font-medium mb-2">监听地址</label>
              <input
                type="text"
                value={config.proxy.listen_address}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    proxy: { ...config.proxy, listen_address: e.target.value },
                  })
                }
                className="w-full px-3 py-2 border rounded-lg bg-background"
              />
            </div>
            <div>
              <label className="block text-sm font-medium mb-2">监听端口</label>
              <input
                type="number"
                value={config.proxy.listen_port}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    proxy: { ...config.proxy, listen_port: parseInt(e.target.value) || 53 },
                  })
                }
                className="w-full px-3 py-2 border rounded-lg bg-background"
              />
            </div>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div>
              <label className="block text-sm font-medium mb-2">缓存大小</label>
              <input
                type="number"
                value={config.proxy.cache_size}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    proxy: { ...config.proxy, cache_size: parseInt(e.target.value) || 1000 },
                  })
                }
                className="w-full px-3 py-2 border rounded-lg bg-background"
              />
            </div>
            <div>
              <label className="block text-sm font-medium mb-2">缓存TTL (秒)</label>
              <input
                type="number"
                value={config.proxy.cache_ttl}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    proxy: { ...config.proxy, cache_ttl: parseInt(e.target.value) || 300 },
                  })
                }
                className="w-full px-3 py-2 border rounded-lg bg-background"
              />
            </div>
          </div>

          <div className="space-y-3">
            <label className="flex items-center gap-3 cursor-pointer">
              <input
                type="checkbox"
                checked={config.proxy.auto_start}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    proxy: { ...config.proxy, auto_start: e.target.checked },
                  })
                }
                className="w-4 h-4"
              />
              <div>
                <p className="text-sm font-medium">开机自启动</p>
                <p className="text-xs text-muted-foreground">Windows 启动时自动运行</p>
              </div>
            </label>
            <label className="flex items-center gap-3 cursor-pointer">
              <input
                type="checkbox"
                checked={config.proxy.block_ipv6}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    proxy: { ...config.proxy, block_ipv6: e.target.checked },
                  })
                }
                className="w-4 h-4"
              />
              <div>
                <p className="text-sm font-medium">阻止 IPv6 查询</p>
                <p className="text-xs text-muted-foreground">屏蔽所有 AAAA 记录</p>
              </div>
            </label>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div>
              <label className="block text-sm font-medium mb-2">默认分组</label>
              <select
                value={config.proxy.default_group}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    proxy: { ...config.proxy, default_group: e.target.value },
                  })
                }
                className="w-full px-3 py-2 border rounded-lg bg-background"
              >
                <option value="">使用所有服务器（不筛选）</option>
                {(config.server_groups || []).map((g) => (
                  <option key={g.name} value={g.name}>{g.description || g.name}</option>
                ))}
              </select>
              <p className="text-xs text-muted-foreground mt-1">
                未匹配任何规则时，默认使用此分组的服务器解析
              </p>
            </div>
          </div>
        </div>
      </div>

      {/* DNS选择策略 */}
      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Zap className="w-5 h-5 text-primary" />
            <h3 className="font-semibold">DNS 选择策略</h3>
          </div>
          <div className="flex items-center gap-2">
            {lastTestTime && (
              <span className="text-xs text-muted-foreground">上次测试: {lastTestTime}</span>
            )}
            <button
              onClick={handleTestLatency}
              disabled={testingLatency}
              className="flex items-center gap-2 px-3 py-1.5 text-sm bg-secondary text-secondary-foreground rounded-lg hover:bg-secondary/80 disabled:opacity-50"
            >
              <Timer className={`w-4 h-4 ${testingLatency ? "animate-spin" : ""}`} />
              {testingLatency ? "测试中..." : "测试延迟"}
            </button>
          </div>
        </div>
        <div className="p-4">
          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            {[
              {
                value: "sequential" as DnsStrategy,
                title: "按顺序",
                desc: "使用第一个可用的DNS服务器",
              },
              {
                value: "fastest" as DnsStrategy,
                title: "最快响应",
                desc: "选择响应最快的DNS服务器",
              },
              {
                value: "load_balance" as DnsStrategy,
                title: "负载均衡",
                desc: "轮询分配请求到多个服务器",
              },
              {
                value: "parallel" as DnsStrategy,
                title: "并行请求",
                desc: "同时请求多个服务器，使用最快响应",
              },
            ].map((s) => (
              <label
                key={s.value}
                className={`p-4 border rounded-lg cursor-pointer transition-all ${
                  config.strategy === s.value
                    ? "border-primary bg-primary/5"
                    : "hover:bg-muted/50"
                }`}
              >
                <div className="flex items-center gap-3">
                  <input
                    type="radio"
                    name="strategy"
                    value={s.value}
                    checked={config.strategy === s.value}
                    onChange={() => setConfig({ ...config, strategy: s.value })}
                    className="text-primary"
                  />
                  <div>
                    <p className="font-medium">{s.title}</p>
                    <p className="text-xs text-muted-foreground">{s.desc}</p>
                  </div>
                </div>
              </label>
            ))}
          </div>

          {/* 自动测速间隔设置 */}
          <div className="mt-4 p-3 bg-muted/50 rounded-lg">
            <div className="flex items-center justify-between">
              <div>
                <h4 className="text-sm font-medium">自动测速间隔</h4>
                <p className="text-xs text-muted-foreground">设为 0 禁用自动测速</p>
              </div>
              <div className="flex items-center gap-2">
                <input
                  type="number"
                  value={config.latency_test_interval || 0}
                  onChange={(e) =>
                    setConfig({
                      ...config,
                      latency_test_interval: parseInt(e.target.value) || 0,
                    })
                  }
                  className="w-20 px-2 py-1 border rounded bg-background text-sm text-right"
                  min="0"
                  step="60"
                />
                <span className="text-sm text-muted-foreground">秒</span>
              </div>
            </div>
          </div>

          {/* 延迟测试结果 */}
          {latencyResults.length > 0 && (
            <div className="mt-4 p-3 bg-muted/50 rounded-lg">
              <h4 className="text-sm font-medium mb-2">延迟测试结果</h4>
              <div className="space-y-2">
                {latencyResults.map((result, index) => (
                  <div key={index} className="flex items-center justify-between text-sm">
                    <div className="flex items-center gap-2">
                      <span className="text-muted-foreground">#{index + 1}</span>
                      <span className="font-medium">{result.name}</span>
                      <span className="text-muted-foreground">({result.ip})</span>
                    </div>
                    <div className="flex items-center gap-2">
                      {result.latency_ms !== undefined ? (
                        <span className={`font-mono font-bold ${getLatencyColor(result.latency_ms)}`}>
                          {result.latency_ms} ms
                        </span>
                      ) : (
                        <span className="text-red-500 text-xs">{result.error || "失败"}</span>
                      )}
                      {index === 0 && result.latency_ms !== undefined && (
                        <span className="text-xs bg-green-100 text-green-700 px-1.5 py-0.5 rounded">
                          最快
                        </span>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>

      {/* 上游DNS服务器 */}
      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Server className="w-5 h-5 text-primary" />
            <h3 className="font-semibold">上游 DNS 服务器</h3>
          </div>
          <button
            onClick={handleAddServer}
            className="flex items-center gap-2 px-3 py-1.5 text-sm bg-primary text-primary-foreground rounded-lg hover:bg-primary/90"
          >
            <Plus className="w-4 h-4" />
            添加服务器
          </button>
        </div>
        <div className="p-4 space-y-3">
          {config.upstream.map((server, index) => (
            <div key={index} className="p-4 border rounded-lg space-y-3">
              <div className="flex items-center gap-3">
                <input
                  type="checkbox"
                  checked={server.enabled}
                  onChange={() => handleToggleServer(index)}
                  className="w-4 h-4"
                />
                <input
                  type="text"
                  value={server.name}
                  onChange={(e) => handleServerChange(index, "name", e.target.value)}
                  className="flex-1 px-3 py-1.5 border rounded bg-background text-sm"
                  placeholder="服务器名称"
                />
                <select
                  value={server.group || "default"}
                  onChange={(e) => handleServerChange(index, "group", e.target.value)}
                  className="px-3 py-1.5 border rounded bg-background text-sm"
                >
                  {(config.server_groups || []).map((g) => (
                    <option key={g.name} value={g.name}>{g.description || g.name}</option>
                  ))}
                </select>
                <select
                  value={server.protocol}
                  onChange={(e) => handleServerChange(index, "protocol", e.target.value as DnsProtocol)}
                  className="px-3 py-1.5 border rounded bg-background text-sm"
                >
                  <option value="udp">UDP</option>
                  <option value="tcp">TCP</option>
                  <option value="doh">DoH</option>
                  <option value="dot">DoT</option>
                </select>
                <button
                  onClick={() => handleRemoveServer(index)}
                  className="p-1.5 text-muted-foreground hover:text-destructive"
                >
                  <Trash2 className="w-4 h-4" />
                </button>
              </div>

              <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                <div>
                  <label className="block text-xs text-muted-foreground mb-1">IP 地址</label>
                  <input
                    type="text"
                    value={server.ip}
                    onChange={(e) => handleServerChange(index, "ip", e.target.value)}
                    className="w-full px-3 py-1.5 border rounded bg-background text-sm font-mono"
                    placeholder="1.1.1.1"
                  />
                </div>
                <div>
                  <label className="block text-xs text-muted-foreground mb-1">端口</label>
                  <input
                    type="number"
                    value={server.port}
                    onChange={(e) => handleServerChange(index, "port", parseInt(e.target.value) || 53)}
                    className="w-full px-3 py-1.5 border rounded bg-background text-sm font-mono"
                  />
                </div>
                {(server.protocol === "doh" || server.protocol === "dot") && (
                  <div>
                    <label className="block text-xs text-muted-foreground mb-1">
                      {server.protocol === "doh" ? "DoH URL" : "DoT 主机名"}
                    </label>
                    <input
                      type="text"
                      value={server.doh_url || ""}
                      onChange={(e) => handleServerChange(index, "doh_url", e.target.value)}
                      className="w-full px-3 py-1.5 border rounded bg-background text-sm"
                      placeholder={
                        server.protocol === "doh"
                          ? "https://cloudflare-dns.com/dns-query"
                          : "cloudflare-dns.com"
                      }
                    />
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* 服务器分组管理 */}
      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Database className="w-5 h-5 text-primary" />
            <h3 className="font-semibold">服务器分组</h3>
          </div>
          <button
            onClick={() => {
              if (!config) return;
              const name = prompt("分组名称 (英文，如 custom):");
              if (!name) return;
              const desc = prompt("分组描述:") || name;
              if (config.server_groups.some((g) => g.name === name)) {
                alert("该分组已存在");
                return;
              }
              setConfig({
                ...config,
                server_groups: [...(config.server_groups || []), { name, description: desc }],
              });
            }}
            className="flex items-center gap-2 px-3 py-1.5 text-sm bg-primary text-primary-foreground rounded-lg hover:bg-primary/90"
          >
            <Plus className="w-4 h-4" />
            添加分组
          </button>
        </div>
        <div className="p-4">
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-3">
            {(config.server_groups || []).map((group) => {
              const serverCount = config.upstream.filter((s) => (s.group || "default") === group.name).length;
              return (
                <div key={group.name} className="p-3 border rounded-lg">
                  <div className="flex items-center justify-between">
                    <div>
                      <p className="font-medium text-sm">{group.description || group.name}</p>
                      <p className="text-xs text-muted-foreground">{group.name} · {serverCount} 个服务器</p>
                    </div>
                    {group.name !== "default" && group.name !== "domestic" && group.name !== "proxy" && (
                      <button
                        onClick={() => {
                          if (!config) return;
                          setConfig({
                            ...config,
                            server_groups: config.server_groups.filter((g) => g.name !== group.name),
                          });
                        }}
                        className="p-1 text-muted-foreground hover:text-destructive"
                      >
                        <Trash2 className="w-3 h-3" />
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      </div>

      {/* 预设服务器 */}
      <div className="bg-card rounded-xl border p-4">
        <h4 className="font-semibold mb-3">快速添加预设服务器</h4>
        <div className="space-y-3">
          <div>
            <p className="text-xs text-muted-foreground mb-2">🇨🇳 国内 DNS（直连）</p>
            <div className="flex flex-wrap gap-2">
              {[
                { name: "阿里 DoH", ip: "223.5.5.5", protocol: "doh" as DnsProtocol, doh_url: "https://dns.alidns.com/dns-query", group: "domestic" },
                { name: "114DNS", ip: "114.114.114.114", protocol: "udp" as DnsProtocol, group: "domestic" },
                { name: "腾讯 DNS", ip: "119.29.29.29", protocol: "udp" as DnsProtocol, group: "domestic" },
              ].map((preset) => (
                <button
                  key={preset.name}
                  onClick={() => {
                    if (!config) return;
                    const exists = config.upstream.some((s) => s.ip === preset.ip && s.protocol === preset.protocol);
                    if (!exists) {
                      setConfig({
                        ...config,
                        upstream: [
                          ...config.upstream,
                          { ...preset, port: preset.protocol === "doh" ? 443 : 53, enabled: true },
                        ],
                      });
                    }
                  }}
                  className="px-3 py-1.5 text-sm border rounded-lg hover:bg-muted"
                >
                  + {preset.name}
                </button>
              ))}
            </div>
          </div>
          <div>
            <p className="text-xs text-muted-foreground mb-2">🔗 代理 DNS（代理）</p>
            <div className="flex flex-wrap gap-2">
              {[
                { name: "Clash DNS", ip: "127.0.0.1", port: 1053, protocol: "udp" as DnsProtocol, group: "proxy" },
              ].map((preset) => (
                <button
                  key={preset.name}
                  onClick={() => {
                    if (!config) return;
                    const exists = config.upstream.some((s) => s.ip === preset.ip && s.port === preset.port);
                    if (!exists) {
                      setConfig({
                        ...config,
                        upstream: [
                          ...config.upstream,
                          { ...preset, enabled: true, doh_url: "" },
                        ],
                      });
                    }
                  }}
                  className="px-3 py-1.5 text-sm border rounded-lg hover:bg-muted"
                >
                  + {preset.name}
                </button>
              ))}
            </div>
          </div>
        </div>
      </div>

      {/* 保存按钮 */}
      <div className="flex justify-end">
        <button
          onClick={handleSave}
          disabled={saving}
          className="flex items-center gap-2 px-6 py-3 bg-primary text-primary-foreground rounded-lg hover:bg-primary/90 disabled:opacity-50"
        >
          <Save className="w-5 h-5" />
          {saving ? "保存中..." : "保存设置"}
        </button>
      </div>
    </div>
  );
}
