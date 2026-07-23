import { useState, useEffect } from "react";
import {
  Plus,
  Trash2,
  Search,
  Download,
  Upload,
  ListFilter,
  Globe,
  RefreshCw,
  Check,
  X,
  MapPin,
} from "lucide-react";
import { api, AppConfig, Rule, RuleType, RuleAction, Subscription, SubscriptionType } from "../lib/api";

// 可展开的订阅规则列表组件
function SubscriptionRules({
  sub,
  subIndex,
  onDeleteRule,
  onBatchImport,
  onAddDomain,
}: {
  sub: Subscription;
  subIndex: number;
  onDeleteRule: (domain: string) => void;
  onBatchImport: () => void;
  onAddDomain: (domain: string) => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(0);
  const [addDomain, setAddDomain] = useState("");
  const PAGE_SIZE = 50;

  const filtered = sub.rules.filter((d) => !search || d.includes(search.toLowerCase()));
  const totalPages = Math.ceil(filtered.length / PAGE_SIZE);
  const paged = filtered.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE);

  return (
    <div className="border rounded-lg">
      {/* 标题行 - 点击展开/折叠 */}
      <button
        onClick={() => { setExpanded(!expanded); setPage(0); }}
        className="w-full px-4 py-3 flex items-center justify-between hover:bg-muted/50 transition-colors"
      >
        <div className="flex items-center gap-2">
          <span className={`text-xs transition-transform ${expanded ? "rotate-90" : ""}`}>▶</span>
          <span className="text-sm font-medium">{sub.name}</span>
          <span className="text-xs text-muted-foreground bg-muted px-2 py-0.5 rounded">
            {sub.rules.length} 条
            {search && ` · 过滤 ${filtered.length} 条`}
          </span>
          {!sub.enabled && (
            <span className="text-xs bg-yellow-100 text-yellow-700 px-2 py-0.5 rounded">已禁用</span>
          )}
        </div>
        <div className="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
          <button onClick={onBatchImport} className="p-1.5 text-muted-foreground hover:text-primary" title="批量导入">
            <Upload className="w-4 h-4" />
          </button>
          <button
            onClick={() => {
              const csv = filtered.join("\n");
              const blob = new Blob([csv], { type: "text/plain;charset=utf-8" });
              const url = URL.createObjectURL(blob);
              const a = document.createElement("a");
              a.href = url;
              a.download = `${sub.name}-rules.txt`;
              a.click();
              URL.revokeObjectURL(url);
            }}
            className="p-1.5 text-muted-foreground hover:text-primary"
            title="导出"
          >
            <Download className="w-4 h-4" />
          </button>
        </div>
      </button>

      {/* 展开内容 */}
      {expanded && (
        <div className="px-4 pb-3 space-y-2">
          {/* 搜索 + 手动添加 */}
          <div className="flex items-center gap-2">
            <div className="relative flex-1">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" />
              <input
                type="text"
                value={search}
                onChange={(e) => { setSearch(e.target.value); setPage(0); }}
                placeholder="搜索域名..."
                className="w-full pl-9 pr-3 py-1.5 border rounded bg-background text-sm"
              />
            </div>
            <input
              type="text"
              value={addDomain}
              onChange={(e) => setAddDomain(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && addDomain.trim()) {
                  onAddDomain(addDomain.trim());
                  setAddDomain("");
                }
              }}
              placeholder="手动添加域名"
              className="w-48 px-3 py-1.5 border rounded bg-background text-sm"
            />
            <button
              onClick={() => { if (addDomain.trim()) { onAddDomain(addDomain.trim()); setAddDomain(""); } }}
              disabled={!addDomain.trim()}
              className="px-3 py-1.5 text-sm bg-primary text-primary-foreground rounded hover:bg-primary/90 disabled:opacity-50"
            >
              <Plus className="w-3.5 h-3.5" />
            </button>
          </div>

          {/* 域名表格 */}
          <div className="border rounded-lg overflow-hidden">
            <div className="max-h-[300px] overflow-y-auto">
              {paged.length === 0 ? (
                <div className="p-4 text-center text-sm text-muted-foreground">
                  {search ? "没有匹配的域名" : "暂无规则"}
                </div>
              ) : (
                <table className="w-full">
                  <thead className="sticky top-0 bg-card z-10">
                    <tr className="border-b bg-muted/30">
                      <th className="text-left px-3 py-2 text-xs font-medium text-muted-foreground w-10">#</th>
                      <th className="text-left px-3 py-2 text-xs font-medium text-muted-foreground">域名</th>
                      <th className="text-right px-3 py-2 text-xs font-medium text-muted-foreground w-16">操作</th>
                    </tr>
                  </thead>
                  <tbody>
                    {paged.map((domain, i) => (
                      <tr key={i} className="border-b last:border-0 hover:bg-muted/30">
                        <td className="px-3 py-1.5 text-xs text-muted-foreground">
                          {page * PAGE_SIZE + i + 1}
                        </td>
                        <td className="px-3 py-1.5 text-sm font-mono">{domain}</td>
                        <td className="px-3 py-1.5 text-right">
                          <button
                            onClick={() => onDeleteRule(domain)}
                            className="p-1 text-muted-foreground hover:text-destructive"
                            title="删除"
                          >
                            <Trash2 className="w-3 h-3" />
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
            {totalPages > 1 && (
              <div className="flex items-center justify-between px-3 py-2 border-t bg-muted/30 text-xs text-muted-foreground">
                <span>第 {page + 1} / {totalPages} 页</span>
                <div className="flex items-center gap-1">
                  <button onClick={() => setPage(0)} disabled={page === 0} className="px-2 py-1 border rounded hover:bg-muted disabled:opacity-40">首页</button>
                  <button onClick={() => setPage((p) => Math.max(0, p - 1))} disabled={page === 0} className="px-2 py-1 border rounded hover:bg-muted disabled:opacity-40">上一页</button>
                  <button onClick={() => setPage((p) => Math.min(totalPages - 1, p + 1))} disabled={page >= totalPages - 1} className="px-2 py-1 border rounded hover:bg-muted disabled:opacity-40">下一页</button>
                  <button onClick={() => setPage(totalPages - 1)} disabled={page >= totalPages - 1} className="px-2 py-1 border rounded hover:bg-muted disabled:opacity-40">末页</button>
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

export default function Rules() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [filterType, setFilterType] = useState<string>("all");
  const [showAddForm, setShowAddForm] = useState(false);
  const [loading, setLoading] = useState(false);
  const [message, setMessage] = useState<{ type: "success" | "error"; text: string } | null>(null);

  // 新规则表单
  const [newRule, setNewRule] = useState<Rule>({
    name: "",
    pattern: "",
    rule_type: "exact",
    action: "block",
    target: "",
    enabled: true,
    priority: 100,
  });

  useEffect(() => { loadConfig(); }, []);

  const loadConfig = async () => {
    try { setConfig(await api.getConfig()); } catch (e) { console.error("加载配置失败:", e); }
  };

  const saveConfig = async (newConfig: AppConfig) => {
    try {
      await api.saveConfig(newConfig);
      setConfig(newConfig);
      setMessage({ type: "success", text: "规则已保存并生效！" });
      setTimeout(() => setMessage(null), 3000);
    } catch (e) { setMessage({ type: "error", text: "保存失败: " + e }); }
  };

  // 自定义规则操作
  const handleAddRule = async () => {
    if (!config || !newRule.name || !newRule.pattern) return;
    await saveConfig({ ...config, rules: [...config.rules, { ...newRule }] });
    setNewRule({ name: "", pattern: "", rule_type: "exact", action: "block", target: "", enabled: true, priority: 100 });
    setShowAddForm(false);
  };
  const handleDeleteRule = async (index: number) => {
    if (!config) return;
    await saveConfig({ ...config, rules: config.rules.filter((_, i) => i !== index) });
  };
  const handleToggleRule = async (index: number) => {
    if (!config) return;
    const newRules = [...config.rules];
    newRules[index] = { ...newRules[index], enabled: !newRules[index].enabled };
    await saveConfig({ ...config, rules: newRules });
  };

  // 订阅操作
  const handleUpdateSubscriptions = async () => {
    setLoading(true);
    try { await api.updateSubscriptions(); setMessage({ type: "success", text: "订阅已更新！" }); await loadConfig(); }
    catch (e) { setMessage({ type: "error", text: "更新订阅失败: " + e }); }
    finally { setLoading(false); }
  };
  const handleUpdateInterval = async (interval: number) => {
    if (!config) return;
    await saveConfig({ ...config, subscription_update_interval: interval });
  };
  const handleToggleSubscription = async (index: number) => {
    if (!config) return;
    const newSubs = [...config.subscriptions];
    newSubs[index] = { ...newSubs[index], enabled: !newSubs[index].enabled };
    await saveConfig({ ...config, subscriptions: newSubs });
  };
  const handleAddSubscription = async (type: SubscriptionType) => {
    if (!config) return;
    const name = prompt(type === "geosite" ? "域名路由名称 (如 国内域名):" : "订阅名称:");
    if (!name) return;
    const url = prompt("订阅URL:");
    if (!url) return;
    const targetGroup = type === "geosite" ? prompt("目标服务器组 (如 domestic/proxy):") || undefined : undefined;
    await saveConfig({
      ...config,
      subscriptions: [...config.subscriptions, { name, url, enabled: true, rules: [], last_updated: undefined, sub_type: type, target_group: targetGroup }],
    });
  };
  const handleDeleteSubscription = async (index: number) => {
    if (!config) return;
    await saveConfig({ ...config, subscriptions: config.subscriptions.filter((_, i) => i !== index) });
  };

  // 订阅规则操作
  const handleAddDomainToSub = async (subIndex: number, domain: string) => {
    if (!config) return;
    const d = domain.toLowerCase().replace(/^https?:\/\//, "").replace(/\/.*$/, "");
    const newSubs = [...config.subscriptions];
    const sub = { ...newSubs[subIndex] };
    if (!sub.rules.includes(d)) {
      sub.rules = [...sub.rules, d];
      newSubs[subIndex] = sub;
      await saveConfig({ ...config, subscriptions: newSubs });
    }
  };
  const handleDeleteDomainFromSub = async (subIndex: number, domain: string) => {
    if (!config) return;
    const newSubs = [...config.subscriptions];
    const sub = { ...newSubs[subIndex] };
    sub.rules = sub.rules.filter((d) => d !== domain);
    newSubs[subIndex] = sub;
    await saveConfig({ ...config, subscriptions: newSubs });
  };
  const handleBatchImport = async (subIndex: number) => {
    if (!config) return;
    const text = prompt("粘贴域名列表（每行一个）:");
    if (!text) return;
    const domains = text.split(/[\n,;]+/).map((d) => d.trim().toLowerCase().replace(/^https?:\/\//, "").replace(/\/.*$/, "")).filter((d) => d && d.includes("."));
    if (domains.length === 0) return;
    const newSubs = [...config.subscriptions];
    const sub = { ...newSubs[subIndex] };
    const existing = new Set(sub.rules);
    sub.rules = [...sub.rules, ...domains.filter((d) => !existing.has(d))];
    newSubs[subIndex] = sub;
    await saveConfig({ ...config, subscriptions: newSubs });
    setMessage({ type: "success", text: `已导入 ${domains.length} 条域名` });
    setTimeout(() => setMessage(null), 3000);
  };

  // 过滤
  const filteredRules = config?.rules.filter((rule) => {
    const matchSearch = rule.name.toLowerCase().includes(searchQuery.toLowerCase()) || rule.pattern.toLowerCase().includes(searchQuery.toLowerCase());
    const matchFilter = filterType === "all" || rule.rule_type === filterType;
    return matchSearch && matchFilter;
  }) || [];
  const blocklistSubs = config?.subscriptions.filter((s) => (s.sub_type || "blocklist") === "blocklist") || [];
  const geositeSubs = config?.subscriptions.filter((s) => s.sub_type === "geosite") || [];
  const totalBlocklistRules = blocklistSubs.reduce((acc, sub) => acc + sub.rules.length, 0);
  const totalGeositeRules = geositeSubs.reduce((acc, sub) => acc + sub.rules.length, 0);

  if (!config) {
    return <div className="flex items-center justify-center h-64"><RefreshCw className="w-8 h-8 animate-spin text-primary" /></div>;
  }

  return (
    <div className="space-y-6">
      {message && (
        <div className={`p-4 rounded-lg ${message.type === "success" ? "bg-green-100 text-green-700 border border-green-200" : "bg-red-100 text-red-700 border border-red-200"}`}>
          {message.text}
        </div>
      )}

      {/* ========== 黑名单订阅 ========== */}
      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Globe className="w-5 h-5 text-primary" />
            <h3 className="font-semibold">黑名单订阅</h3>
            <span className="text-sm text-muted-foreground">({totalBlocklistRules} 条规则)</span>
          </div>
          <div className="flex items-center gap-2">
            <button onClick={handleUpdateSubscriptions} disabled={loading} className="flex items-center gap-2 px-3 py-1.5 text-sm border rounded-lg hover:bg-muted">
              <RefreshCw className={`w-4 h-4 ${loading ? "animate-spin" : ""}`} />
              更新订阅
            </button>
            <button onClick={() => handleAddSubscription("blocklist")} className="flex items-center gap-2 px-3 py-1.5 text-sm bg-primary text-primary-foreground rounded-lg hover:bg-primary/90">
              <Plus className="w-4 h-4" />
              添加订阅
            </button>
          </div>
        </div>

        {/* 自动更新间隔 */}
        <div className="p-4 border-b bg-muted/30">
          <div className="flex items-center gap-4">
            <span className="text-sm font-medium">自动更新间隔:</span>
            <div className="flex flex-wrap gap-2">
              {[{ value: 30, label: "30分钟" }, { value: 60, label: "1小时" }, { value: 120, label: "2小时" }, { value: 360, label: "6小时" }, { value: 720, label: "12小时" }, { value: 1440, label: "24小时" }, { value: 0, label: "禁用" }].map((option) => (
                <button key={option.value} onClick={() => handleUpdateInterval(option.value)} className={`px-3 py-1.5 text-sm rounded-lg transition-colors ${config.subscription_update_interval === option.value ? "bg-primary text-primary-foreground" : "border hover:bg-muted"}`}>
                  {option.label}
                </button>
              ))}
            </div>
          </div>
        </div>

        {/* 订阅列表 */}
        <div className="p-4 space-y-3">
          {blocklistSubs.map((sub) => {
            const realIdx = config.subscriptions.indexOf(sub);
            return (
              <div key={realIdx} className="flex items-center gap-3 p-3 border rounded-lg">
                <input type="checkbox" checked={sub.enabled} onChange={() => handleToggleSubscription(realIdx)} className="w-4 h-4" />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-medium">{sub.name}</span>
                    <span className="text-xs text-muted-foreground bg-muted px-2 py-0.5 rounded">{sub.rules.length} 条</span>
                  </div>
                  <p className="text-xs text-muted-foreground truncate">{sub.url}</p>
                  {sub.last_updated && <p className="text-xs text-muted-foreground">更新于: {sub.last_updated}</p>}
                </div>
                <button onClick={() => handleDeleteSubscription(realIdx)} className="p-1.5 text-muted-foreground hover:text-destructive">
                  <Trash2 className="w-4 h-4" />
                </button>
              </div>
            );
          })}
          {blocklistSubs.length === 0 && <p className="text-sm text-muted-foreground text-center py-4">暂无黑名单订阅</p>}
        </div>

        {/* 已订阅规则 - 点击展开 */}
        {blocklistSubs.some((s) => s.rules.length > 0) && (
          <div className="p-4 border-t space-y-2">
            <p className="text-xs text-muted-foreground mb-2">点击订阅名称展开查看规则</p>
            {blocklistSubs.filter((s) => s.rules.length > 0).map((sub) => {
              const realIdx = config.subscriptions.indexOf(sub);
              return (
                <SubscriptionRules
                  key={realIdx}
                  sub={sub}
                  subIndex={realIdx}
                  onDeleteRule={(domain) => handleDeleteDomainFromSub(realIdx, domain)}
                  onBatchImport={() => handleBatchImport(realIdx)}
                  onAddDomain={(domain) => handleAddDomainToSub(realIdx, domain)}
                />
              );
            })}
          </div>
        )}
      </div>

      {/* ========== 域名路由规则 ========== */}
      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b flex items-center justify-between">
          <div className="flex items-center gap-2">
            <MapPin className="w-5 h-5 text-primary" />
            <h3 className="font-semibold">域名路由规则</h3>
            <span className="text-sm text-muted-foreground">({totalGeositeRules} 条规则)</span>
          </div>
          <div className="flex items-center gap-2">
            <button onClick={handleUpdateSubscriptions} disabled={loading} className="flex items-center gap-2 px-3 py-1.5 text-sm border rounded-lg hover:bg-muted">
              <RefreshCw className={`w-4 h-4 ${loading ? "animate-spin" : ""}`} />
              更新
            </button>
            <button onClick={() => handleAddSubscription("geosite")} className="flex items-center gap-2 px-3 py-1.5 text-sm bg-primary text-primary-foreground rounded-lg hover:bg-primary/90">
              <Plus className="w-4 h-4" />
              添加路由列表
            </button>
          </div>
        </div>

        <div className="p-4 space-y-3">
          {/* 快速添加预设 */}
          <div className="mb-4 p-3 bg-muted/30 rounded-lg">
            <p className="text-xs font-medium mb-2">快速添加预设:</p>
            <div className="space-y-2">
              <div>
                <p className="text-xs text-muted-foreground mb-1">🏠 国内域名 (→ domestic)</p>
                <div className="flex flex-wrap gap-2">
                  {[
                    { name: "CN 国内直连域名 (11万+)", url: "https://ghfast.top/https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/direct-list.txt", group: "domestic" },
                    { name: "Apple 中国域名", url: "https://ghfast.top/https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/apple-cn.txt", group: "domestic" },
                    { name: "Google 中国域名", url: "https://ghfast.top/https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/google-cn.txt", group: "domestic" },
                  ].map((preset) => (
                    <button
                      key={preset.name}
                      onClick={async () => {
                        if (!config) return;
                        const exists = config.subscriptions.some((s) => s.url === preset.url);
                        if (exists) { alert("该订阅已存在"); return; }
                        await saveConfig({
                          ...config,
                          subscriptions: [...config.subscriptions, {
                            name: preset.name, url: preset.url, enabled: true,
                            rules: [], last_updated: undefined,
                            sub_type: "geosite" as SubscriptionType, target_group: preset.group,
                          }],
                        });
                      }}
                      className="px-2 py-1 text-xs border rounded hover:bg-muted"
                    >
                      + {preset.name}
                    </button>
                  ))}
                </div>
              </div>
              <div>
                <p className="text-xs text-muted-foreground mb-1">🌐 国外域名 (→ proxy) <span className="text-muted-foreground/70">包含 Google/GitHub/YouTube/Telegram/OpenAI 等全部国外服务</span></p>
                <div className="flex flex-wrap gap-2">
                  {[
                    { name: "Proxy 需代理域名 (2.7万+)", url: "https://ghfast.top/https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/proxy-list.txt", group: "proxy" },
                  ].map((preset) => (
                    <button
                      key={preset.name}
                      onClick={async () => {
                        if (!config) return;
                        const exists = config.subscriptions.some((s) => s.url === preset.url);
                        if (exists) { alert("该订阅已存在"); return; }
                        await saveConfig({
                          ...config,
                          subscriptions: [...config.subscriptions, {
                            name: preset.name, url: preset.url, enabled: true,
                            rules: [], last_updated: undefined,
                            sub_type: "geosite" as SubscriptionType, target_group: preset.group,
                          }],
                        });
                      }}
                      className="px-2 py-1 text-xs border rounded hover:bg-muted"
                    >
                      + {preset.name}
                    </button>
                  ))}
                </div>
              </div>
              <div>
                <p className="text-xs text-muted-foreground mb-1">🚫 广告拦截 (→ blocklist)</p>
                <div className="flex flex-wrap gap-2">
                  {[
                    { name: "广告拦截域名 (16万+)", url: "https://ghfast.top/https://raw.githubusercontent.com/Loyalsoldier/v2ray-rules-dat/release/reject-list.txt" },
                  ].map((preset) => (
                    <button
                      key={preset.name}
                      onClick={async () => {
                        if (!config) return;
                        const exists = config.subscriptions.some((s) => s.url === preset.url);
                        if (exists) { alert("该订阅已存在"); return; }
                        await saveConfig({
                          ...config,
                          subscriptions: [...config.subscriptions, {
                            name: preset.name, url: preset.url, enabled: true,
                            rules: [], last_updated: undefined,
                            sub_type: "blocklist" as SubscriptionType,
                          }],
                        });
                      }}
                      className="px-2 py-1 text-xs border rounded hover:bg-muted"
                    >
                      + {preset.name}
                    </button>
                  ))}
                </div>
              </div>
            </div>
          </div>

          {geositeSubs.length === 0 ? (
            <div className="text-center py-4 text-sm text-muted-foreground">
              <p>暂无域名路由列表，请从上方预设快速添加</p>
            </div>
          ) : (
            geositeSubs.map((sub) => {
              const realIdx = config.subscriptions.indexOf(sub);
              return (
                <div key={realIdx} className="p-3 border rounded-lg space-y-2">
                  <div className="flex items-center gap-3">
                    <input type="checkbox" checked={sub.enabled} onChange={() => handleToggleSubscription(realIdx)} className="w-4 h-4" />
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="font-medium">{sub.name}</span>
                        <span className="text-xs text-muted-foreground bg-muted px-2 py-0.5 rounded">{sub.rules.length} 条</span>
                        {sub.target_group && (
                          <span className="text-xs bg-green-100 text-green-700 px-2 py-0.5 rounded">→ {sub.target_group}</span>
                        )}
                      </div>
                      <p className="text-xs text-muted-foreground truncate">{sub.url}</p>
                    </div>
                    <button onClick={() => handleDeleteSubscription(realIdx)} className="p-1.5 text-muted-foreground hover:text-destructive">
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                  {/* 可展开规则 */}
                  <SubscriptionRules
                    sub={sub}
                    subIndex={realIdx}
                    onDeleteRule={(domain) => handleDeleteDomainFromSub(realIdx, domain)}
                    onBatchImport={() => handleBatchImport(realIdx)}
                    onAddDomain={(domain) => handleAddDomainToSub(realIdx, domain)}
                  />
                </div>
              );
            })
          )}
        </div>
      </div>

      {/* ========== 自定义规则 ========== */}
      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b flex items-center justify-between">
          <div className="flex items-center gap-2">
            <ListFilter className="w-5 h-5 text-primary" />
            <h3 className="font-semibold">自定义规则</h3>
            <span className="text-sm text-muted-foreground">({filteredRules.length} 条)</span>
          </div>
          <div className="flex items-center gap-2">
            <div className="relative">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
              <input type="text" value={searchQuery} onChange={(e) => setSearchQuery(e.target.value)} placeholder="搜索规则..." className="pl-10 pr-4 py-1.5 border rounded-lg bg-background text-sm" />
            </div>
            <select value={filterType} onChange={(e) => setFilterType(e.target.value)} className="px-3 py-1.5 border rounded-lg bg-background text-sm">
              <option value="all">全部类型</option>
              <option value="exact">精确匹配</option>
              <option value="wildcard">通配符</option>
              <option value="regex">正则表达式</option>
            </select>
            <button onClick={() => setShowAddForm(!showAddForm)} className="flex items-center gap-2 px-3 py-1.5 text-sm bg-primary text-primary-foreground rounded-lg hover:bg-primary/90">
              <Plus className="w-4 h-4" />
              添加规则
            </button>
          </div>
        </div>

        {showAddForm && (
          <div className="p-4 border-b bg-muted/30">
            <h4 className="font-medium mb-3">添加新规则</h4>
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-3">
              <input type="text" value={newRule.name} onChange={(e) => setNewRule({ ...newRule, name: e.target.value })} placeholder="规则名称" className="px-3 py-2 border rounded-lg bg-background text-sm" />
              <input type="text" value={newRule.pattern} onChange={(e) => setNewRule({ ...newRule, pattern: e.target.value })} placeholder="匹配模式" className="px-3 py-2 border rounded-lg bg-background text-sm" />
              <select value={newRule.rule_type} onChange={(e) => setNewRule({ ...newRule, rule_type: e.target.value as RuleType })} className="px-3 py-2 border rounded-lg bg-background text-sm">
                <option value="exact">精确匹配</option>
                <option value="wildcard">通配符</option>
                <option value="regex">正则表达式</option>
              </select>
              <select value={newRule.action} onChange={(e) => setNewRule({ ...newRule, action: e.target.value as RuleAction })} className="px-3 py-2 border rounded-lg bg-background text-sm">
                <option value="block">阻止 (返回0.0.0.0)</option>
                <option value="block_nxdomain">阻止 (NXDOMAIN)</option>
                <option value="forward">转发到服务器组</option>
                <option value="cache">使用缓存</option>
              </select>
            </div>
            {newRule.action === "forward" && (
              <select value={newRule.target || ""} onChange={(e) => setNewRule({ ...newRule, target: e.target.value })} className="mt-2 px-3 py-2 border rounded-lg bg-background text-sm">
                <option value="">选择目标服务器组</option>
                {(config?.server_groups || []).map((g) => <option key={g.name} value={g.name}>{g.description || g.name}</option>)}
              </select>
            )}
            <div className="flex items-center gap-3 mt-3">
              <label className="flex items-center gap-2 text-sm">
                <span>优先级:</span>
                <input type="number" value={newRule.priority} onChange={(e) => setNewRule({ ...newRule, priority: parseInt(e.target.value) || 100 })} className="w-20 px-2 py-1 border rounded bg-background" />
              </label>
              <button onClick={handleAddRule} disabled={!newRule.name || !newRule.pattern} className="flex items-center gap-2 px-4 py-2 text-sm bg-primary text-primary-foreground rounded-lg hover:bg-primary/90 disabled:opacity-50">
                <Check className="w-4 h-4" /> 添加
              </button>
              <button onClick={() => setShowAddForm(false)} className="flex items-center gap-2 px-4 py-2 text-sm border rounded-lg hover:bg-muted">
                <X className="w-4 h-4" /> 取消
              </button>
            </div>
          </div>
        )}

        <div className="overflow-x-auto">
          <table className="w-full">
            <thead>
              <tr className="border-b bg-muted/50">
                <th className="text-left p-3 text-sm font-medium text-muted-foreground w-10">#</th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">名称</th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">匹配模式</th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">类型</th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">操作</th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">优先级</th>
                <th className="text-left p-3 text-sm font-medium text-muted-foreground">状态</th>
                <th className="text-right p-3 text-sm font-medium text-muted-foreground">操作</th>
              </tr>
            </thead>
            <tbody>
              {filteredRules.length === 0 ? (
                <tr><td colSpan={8} className="p-8 text-center text-muted-foreground">暂无自定义规则</td></tr>
              ) : filteredRules.map((rule, index) => (
                <tr key={index} className="border-b hover:bg-muted/50">
                  <td className="p-3 text-sm text-muted-foreground">{index + 1}</td>
                  <td className="p-3 text-sm font-medium">{rule.name}</td>
                  <td className="p-3 text-sm font-mono">{rule.pattern}</td>
                  <td className="p-3 text-sm">
                    <span className={`px-2 py-1 rounded text-xs ${rule.rule_type === "exact" ? "bg-blue-100 text-blue-700" : rule.rule_type === "wildcard" ? "bg-purple-100 text-purple-700" : "bg-orange-100 text-orange-700"}`}>
                      {rule.rule_type === "exact" ? "精确" : rule.rule_type === "wildcard" ? "通配符" : "正则"}
                    </span>
                  </td>
                  <td className="p-3 text-sm">
                    <span className={`px-2 py-1 rounded text-xs ${rule.action === "block" || rule.action === "block_null" || rule.action === "block_nxdomain" ? "bg-red-100 text-red-700" : rule.action === "forward" ? "bg-green-100 text-green-700" : "bg-yellow-100 text-yellow-700"}`}>
                      {rule.action === "block" ? "阻止" : rule.action === "block_null" ? "阻止(0.0.0.0)" : rule.action === "block_nxdomain" ? "阻止(NX)" : rule.action === "forward" ? `转发 → ${rule.target || "默认"}` : "缓存"}
                    </span>
                  </td>
                  <td className="p-3 text-sm">{rule.priority}</td>
                  <td className="p-3">
                    <button onClick={() => { const realIndex = config.rules.findIndex((r) => r === rule); if (realIndex >= 0) handleToggleRule(realIndex); }} className={`w-11 h-6 rounded-full transition-colors ${rule.enabled ? "bg-primary" : "bg-muted"}`}>
                      <div className={`w-4 h-4 rounded-full bg-white transition-transform ${rule.enabled ? "translate-x-6" : "translate-x-1"}`} />
                    </button>
                  </td>
                  <td className="p-3 text-right">
                    <button onClick={() => { const realIndex = config.rules.findIndex((r) => r === rule); if (realIndex >= 0) handleDeleteRule(realIndex); }} className="p-1.5 text-muted-foreground hover:text-destructive">
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      {/* ========== 匹配模式说明 ========== */}
      <div className="bg-card rounded-xl border p-4">
        <h4 className="font-semibold mb-3">匹配模式说明</h4>
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-3 text-sm">
          <div className="p-3 bg-blue-50 rounded-lg">
            <p className="font-medium text-blue-700">精确匹配</p>
            <p className="text-muted-foreground mt-1">完全匹配域名，如 <code>example.com</code></p>
          </div>
          <div className="p-3 bg-purple-50 rounded-lg">
            <p className="font-medium text-purple-700">通配符匹配</p>
            <p className="text-muted-foreground mt-1">使用 <code>*</code> 匹配，如 <code>*.example.com</code></p>
          </div>
          <div className="p-3 bg-orange-50 rounded-lg">
            <p className="font-medium text-orange-700">正则表达式</p>
            <p className="text-muted-foreground mt-1">使用正则匹配，如 <code>.*\.example\.com</code></p>
          </div>
          <div className="p-3 bg-red-50 rounded-lg">
            <p className="font-medium text-red-700">域名路由</p>
            <p className="text-muted-foreground mt-1">GeoSite 列表，按域名分流到不同服务器组</p>
          </div>
        </div>
      </div>
    </div>
  );
}
