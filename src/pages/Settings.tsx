import { useState, useEffect } from "react";
import { Save, Plus, Trash2, Server, Wifi, Database, Zap, RefreshCw, Timer, Monitor } from "lucide-react";
import { api, AppConfig, DnsServer, DnsProtocol, DnsStrategy, DnsLatencyResult, ServerGroup, DnsTakeoverStatus } from "../lib/api";
import MessageBanner from "../components/ui/MessageBanner";
import Badge from "../components/ui/Badge";

export default function Settings() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [initializing, setInitializing] = useState(true);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);
  const [latencyResults, setLatencyResults] = useState<DnsLatencyResult[]>([]);
  const [testingLatency, setTestingLatency] = useState(false);
  const [lastTestTime, setLastTestTime] = useState<string | null>(null);
  const [autostartEnabled, setAutostartEnabled] = useState(false);
  const [takeover, setTakeover] = useState<DnsTakeoverStatus | null>(null);
  const [restoring, setRestoring] = useState(false);

  // 加载配置
  useEffect(() => {
    loadConfig();
    loadLatencyResults();
    loadAutostartStatus();
    loadTakeoverStatus();
  }, []);

  const loadTakeoverStatus = async () => {
    try {
      setTakeover(await api.getDnsTakeoverStatus());
    } catch (e) {
      console.error("加载 DNS 接管状态失败:", e);
    }
  };

  // 手动还原：接管状态下系统解析全靠本进程，需要一键退路
  const handleRestoreSystemDns = async () => {
    setRestoring(true);
    try {
      const result = await api.restoreSystemDns();
      setMessage({ type: "success", text: result });
      await loadTakeoverStatus();
    } catch (e) {
      setMessage({ type: "error", text: "还原系统 DNS 失败: " + e });
    } finally {
      setRestoring(false);
    }
  };

  const loadAutostartStatus = async () => {
    try {
      const enabled = await api.isAutostartEnabled();
      setAutostartEnabled(enabled);
    } catch (e) {
      console.error("加载自启动状态失败:", e);
    }
  };

  const handleToggleAutostart = async () => {
    const newState = !autostartEnabled;
    try {
      await api.setAutostart(newState);
      setAutostartEnabled(newState);
      setMessage({ type: "success", text: newState ? "已启用开机自启动" : "已取消开机自启动" });
    } catch (e) {
      setMessage({ type: "error", text: "设置自启动失败: " + e });
    }
  };

  const loadConfig = async () => {
    setInitializing(true);
    try {
      const cfg = await api.getConfig();
      setConfig(cfg);
    } catch (e) {
      console.error("加载配置失败:", e);
      setMessage({ type: "error", text: "加载配置失败: " + e });
    } finally {
      setInitializing(false);
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

  // 切换协议时自动调整默认值
  const handleProtocolChange = (index: number, protocol: DnsProtocol) => {
    if (!config) return;
    const newUpstream = [...config.upstream];
    const server = { ...newUpstream[index] };
    server.protocol = protocol;

    // 根据协议设置合适的默认端口
    if (protocol === "dot") {
      server.port = 853;
    } else if (protocol === "doh") {
      server.port = 443;
    } else {
      server.port = 53;
    }

    newUpstream[index] = server;
    setConfig({ ...config, upstream: newUpstream });
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

  if (initializing) {
    return (
      <div className="flex items-center justify-center h-64">
        <RefreshCw className="w-8 h-8 animate-spin text-primary" />
        <span className="ml-2">加载配置中...</span>
      </div>
    );
  }

  if (!config) {
    return (
      <div className="flex flex-col items-center justify-center h-64 gap-3">
        <p className="text-sm text-destructive">{message?.text || "配置暂时不可用"}</p>
        <button
          onClick={loadConfig}
          className="flex items-center gap-2 px-3 py-2 text-sm border rounded-lg hover:bg-muted"
        >
          <RefreshCw className="w-4 h-4" />
          重试
        </button>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {/* 消息提示 */}
      {message && <MessageBanner type={message.type} text={message.text} />}

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
              <p className="text-xs text-muted-foreground mt-1">
                默认 127.0.0.1 仅本机可用；填 0.0.0.0 会把本机变成局域网开放解析器
              </p>
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

          <div>
            <label className="block text-sm font-medium mb-2">监听协议</label>
            <div className="flex gap-2">
              {[
                { value: "both", label: "UDP + TCP", hint: "推荐：大响应与 DNSSEC 依赖 TCP" },
                { value: "udp", label: "仅 UDP", hint: "响应被截断时客户端无法重试" },
                { value: "tcp", label: "仅 TCP", hint: "排查 UDP 相关问题时使用" },
              ].map((option) => (
                <button
                  key={option.value}
                  type="button"
                  title={option.hint}
                  onClick={() =>
                    setConfig({
                      ...config,
                      proxy: { ...config.proxy, protocol: option.value },
                    })
                  }
                  className={`px-3 py-2 text-sm rounded-lg transition-colors ${
                    config.proxy.protocol === option.value
                      ? "bg-primary text-primary-foreground"
                      : "border hover:bg-muted"
                  }`}
                >
                  {option.label}
                </button>
              ))}
            </div>
            <p className="text-xs text-muted-foreground mt-1">
              只监听 UDP 时，超过客户端声明尺寸的响应会被置 TC 位，而客户端改走 TCP 重试会直接失败
            </p>
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
              <label className="block text-sm font-medium mb-2">缓存TTL上限 (秒)</label>
              <input
                type="number"
                value={config.proxy.cache_ttl}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    proxy: { ...config.proxy, cache_ttl: Math.max(0, parseInt(e.target.value) || 0) },
                  })
                }
                className="w-full px-3 py-2 border rounded-lg bg-background"
              />
              <p className="text-xs text-muted-foreground mt-1">
                使用果冻解析时建议填 0，直接使用权威服务器返回的原始 TTL；填写其他数值时，缓存 TTL 将受此上限限制。命中缓存后 TTL 会按驻留时间递减
              </p>
            </div>
          </div>

          {/* DoH 引导解析服务器 */}
          <div>
            <label className="block text-sm font-medium mb-2">DoH 引导解析服务器</label>
            <BootstrapDnsInput
              value={config.proxy.bootstrap_dns ?? []}
              onChange={(bootstrap_dns) =>
                setConfig({ ...config, proxy: { ...config.proxy, bootstrap_dns } })
              }
            />
            <p className="text-xs text-muted-foreground mt-1">
              只用来解析 DoH 上游的主机名。系统 DNS 已指向本程序时不能再走系统解析器，
              否则会递归回自身、DoH 直接不可用。逗号或空格分隔，可带端口如
              <code className="mx-1">223.5.5.5:53</code>。
            </p>
          </div>

          {/* 系统 DNS 接管 */}
          <div className="border rounded-lg p-3 space-y-3">
            <label className="flex items-start gap-3 cursor-pointer">
              <input
                type="checkbox"
                checked={config.proxy.takeover_system_dns}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    proxy: { ...config.proxy, takeover_system_dns: e.target.checked },
                  })
                }
                className="w-4 h-4 mt-0.5"
              />
              <div>
                <p className="text-sm font-medium">接管系统 DNS</p>
                <p className="text-xs text-muted-foreground">
                  启动服务时把在用网卡的 DNS 指向本机代理，停止服务或退出程序时按原配置精确还原
                </p>
              </div>
            </label>

            <div className="flex items-center justify-between gap-3 pl-7">
              <div className="text-xs">
                {takeover?.active ? (
                  <span className="text-green-600 dark:text-green-400">
                    ● 已接管
                    {takeover.detail && (
                      <span className="text-muted-foreground"> — 原配置：{takeover.detail}</span>
                    )}
                  </span>
                ) : (
                  <span className="text-muted-foreground">○ 未接管（系统仍用原有 DNS）</span>
                )}
              </div>
              <button
                onClick={handleRestoreSystemDns}
                disabled={restoring || !takeover?.active}
                className="shrink-0 px-3 py-1.5 text-xs border rounded-lg hover:bg-muted disabled:opacity-40 disabled:cursor-not-allowed"
              >
                {restoring ? "还原中..." : "立即还原系统 DNS"}
              </button>
            </div>
          </div>

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

          {/* ECS 配置 */}
          <div className="space-y-3 pt-3 border-t">
            <label className="flex items-center gap-3 cursor-pointer">
              <input
                type="checkbox"
                checked={config.ecs?.enabled || false}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    ecs: { ...config.ecs, enabled: e.target.checked },
                  })
                }
                className="w-4 h-4"
              />
              <div>
                <p className="text-sm font-medium">启用 EDNS Client Subnet (ECS)</p>
                <p className="text-xs text-muted-foreground">传递客户端位置信息，让 CDN 返回更近的节点</p>
              </div>
            </label>

            {config.ecs?.enabled && (
              <div className="ml-7 space-y-3">
                <div>
                  <label className="block text-sm font-medium mb-2">客户端 IP</label>
                  <input
                    type="text"
                    value={config.ecs?.client_ip || ""}
                    onChange={(e) =>
                      setConfig({
                        ...config,
                        ecs: { ...config.ecs, client_ip: e.target.value || undefined },
                      })
                    }
                    placeholder="留空自动获取公网 IP"
                    className="w-full px-3 py-2 border rounded-lg bg-background"
                  />
                  <p className="text-xs text-muted-foreground mt-1">
                    留空将自动获取公网 IP（每5分钟更新），也可手动填写固定 IP
                  </p>
                </div>

                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                  <div>
                    <label className="block text-sm font-medium mb-2">IPv4 掩码长度</label>
                    <select
                      value={config.ecs?.ipv4_source_mask || 24}
                      onChange={(e) =>
                        setConfig({
                          ...config,
                          ecs: { ...config.ecs, ipv4_source_mask: parseInt(e.target.value) },
                        })
                      }
                      className="w-full px-3 py-2 border rounded-lg bg-background"
                    >
                      <option value={32}>/32 - 精确到主机</option>
                      <option value={24}>/24 - 精确到子网（推荐）</option>
                      <option value={16}>/16 - 精确到城市</option>
                      <option value={8}>/8 - 精确到国家</option>
                    </select>
                  </div>
                  <div>
                    <label className="block text-sm font-medium mb-2">IPv6 掩码长度</label>
                    <select
                      value={config.ecs?.ipv6_source_mask || 56}
                      onChange={(e) =>
                        setConfig({
                          ...config,
                          ecs: { ...config.ecs, ipv6_source_mask: parseInt(e.target.value) },
                        })
                      }
                      className="w-full px-3 py-2 border rounded-lg bg-background"
                    >
                      <option value={128}>/128 - 精确到主机</option>
                      <option value={56}>/56 - 精确到子网（推荐）</option>
                      <option value={48}>/48 - 精确到站点</option>
                      <option value={32}>/32 - 精确到 ISP</option>
                    </select>
                  </div>
                </div>
              </div>
            )}
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
                        <Badge variant="success" className="text-[10px]">最快</Badge>
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
                  onChange={(e) => handleProtocolChange(index, e.target.value as DnsProtocol)}
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
                {server.protocol === "doh" ? (
                  /* DoH: 只需要 URL */
                  <div className="md:col-span-3">
                    <label className="block text-xs text-muted-foreground mb-1">DoH URL</label>
                    <input
                      type="text"
                      value={server.doh_url || ""}
                      onChange={(e) => handleServerChange(index, "doh_url", e.target.value)}
                      className="w-full px-3 py-1.5 border rounded bg-background text-sm"
                      placeholder="https://cloudflare-dns.com/dns-query"
                    />
                    <p className="text-xs text-muted-foreground mt-1">DoH 协议使用 URL 访问，无需填写 IP 地址</p>
                  </div>
                ) : server.protocol === "dot" ? (
                  /* DoT: 主机名 + 端口 */
                  <>
                    <div className="md:col-span-2">
                      <label className="block text-xs text-muted-foreground mb-1">主机名（域名或 IP）</label>
                      <input
                        type="text"
                        value={server.ip}
                        onChange={(e) => handleServerChange(index, "ip", e.target.value)}
                        className="w-full px-3 py-1.5 border rounded bg-background text-sm font-mono"
                        placeholder="cloudflare-dns.com 或 1.1.1.1"
                      />
                    </div>
                    <div>
                      <label className="block text-xs text-muted-foreground mb-1">端口</label>
                      <input
                        type="number"
                        value={server.port || 853}
                        onChange={(e) => handleServerChange(index, "port", parseInt(e.target.value) || 853)}
                        className="w-full px-3 py-1.5 border rounded bg-background text-sm font-mono"
                        placeholder="853"
                      />
                    </div>
                  </>
                ) : (
                  /* UDP/TCP: IP + 端口 */
                  <>
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
                  </>
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

/**
 * 引导服务器输入框
 *
 * 内部持有文本草稿、失焦时才提交：直接把它绑定成 `join(", ")` 的受控输入会有个
 * 讨厌的副作用——刚敲下的逗号或空格立刻被规范化掉，导致第二个地址根本敲不进去。
 */
function BootstrapDnsInput({
  value,
  onChange,
}: {
  value: string[];
  onChange: (next: string[]) => void;
}) {
  const [draft, setDraft] = useState(value.join(", "));
  const serialized = value.join(",");

  // 外部值变化（配置加载完成、保存后回读）时同步草稿；打字过程中父组件的值不变，
  // 所以这里的同步不会打断输入
  useEffect(() => {
    setDraft(value.join(", "));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serialized]);

  const commit = () => {
    const next = draft
      .split(/[,，;\s]+/)
      .map((entry) => entry.trim())
      .filter((entry) => entry.length > 0);
    onChange(next);
    setDraft(next.join(", "));
  };

  return (
    <input
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.currentTarget.blur();
        }
      }}
      placeholder="223.5.5.5, 119.29.29.29"
      className="w-full px-3 py-2 border rounded-lg bg-background"
    />
  );
}
