import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-shell";
import {
  Globe,
  Github,
  Heart,
  Coffee,
  ExternalLink,
  RefreshCw,
  CheckCircle2,
  AlertCircle,
  Download,
  Loader2,
} from "lucide-react";

interface UpdateInfo {
  has_update: boolean;
  current_version: string;
  latest_version: string;
  release_url: string;
  release_notes: string;
  published_at: string;
}

export default function About() {
  const [checking, setChecking] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleCheckUpdate = async () => {
    setChecking(true);
    setError(null);
    setUpdateInfo(null);

    try {
      const info = await invoke<UpdateInfo>("check_update");
      setUpdateInfo(info);
    } catch (err) {
      setError(String(err));
    } finally {
      setChecking(false);
    }
  };

  const handleOpenRelease = async () => {
    if (updateInfo?.release_url) {
      try {
        await open(updateInfo.release_url);
      } catch {
        window.open(updateInfo.release_url, "_blank");
      }
    }
  };

  const formatPublishedAt = (dateStr: string) => {
    if (!dateStr) return "";
    try {
      const date = new Date(dateStr);
      return date.toLocaleDateString("zh-CN", {
        year: "numeric",
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
      });
    } catch {
      return dateStr;
    }
  };

  return (
    <div className="space-y-6">
      {/* 项目信息 */}
      <div className="bg-card rounded-xl border p-8 text-center">
        <div className="flex justify-center mb-4">
          <div className="w-16 h-16 rounded-2xl bg-primary flex items-center justify-center">
            <Globe className="w-9 h-9 text-primary-foreground" />
          </div>
        </div>
        <h1 className="text-2xl font-bold mb-1">DNS Proxy</h1>
        <p className="text-muted-foreground mb-3">版本 1.0.9</p>
        <p className="text-sm text-muted-foreground max-w-md mx-auto">
          一个类似 YogaDNS 的 Windows 全局 DNS 代理软件，支持国内外域名分流、GeoSite 路由、广告拦截。
        </p>

        {/* 检查更新按钮 */}
        <div className="mt-6">
          <button
            onClick={handleCheckUpdate}
            disabled={checking}
            className="inline-flex items-center gap-2 px-4 py-2 bg-primary text-primary-foreground rounded-lg hover:bg-primary/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {checking ? (
              <>
                <Loader2 className="w-4 h-4 animate-spin" />
                检查中...
              </>
            ) : (
              <>
                <RefreshCw className="w-4 h-4" />
                检查更新
              </>
            )}
          </button>
        </div>

        {/* 更新状态显示 */}
        {error && (
          <div className="mt-4 p-4 bg-destructive/10 text-destructive rounded-lg text-sm max-w-md mx-auto">
            <div className="flex items-center gap-2">
              <AlertCircle className="w-4 h-4 flex-shrink-0" />
              <span>检查更新失败: {error}</span>
            </div>
          </div>
        )}

        {updateInfo && (
          <div className="mt-4 max-w-md mx-auto">
            {updateInfo.has_update ? (
              <div className="p-4 bg-green-500/10 text-green-700 dark:text-green-400 rounded-lg">
                <div className="flex items-center gap-2 mb-2">
                  <Download className="w-4 h-4" />
                  <span className="font-medium">发现新版本!</span>
                </div>
                <div className="text-sm space-y-1">
                  <p>
                    当前版本: <span className="font-mono">{updateInfo.current_version}</span>
                  </p>
                  <p>
                    最新版本:{" "}
                    <span className="font-mono font-bold">{updateInfo.latest_version}</span>
                  </p>
                  {updateInfo.published_at && (
                    <p className="text-muted-foreground">
                      发布时间: {formatPublishedAt(updateInfo.published_at)}
                    </p>
                  )}
                </div>
                {updateInfo.release_notes && (
                  <div className="mt-3 p-3 bg-background/50 rounded text-sm text-left">
                    <p className="font-medium mb-1">更新说明:</p>
                    <p className="whitespace-pre-wrap">{updateInfo.release_notes}</p>
                  </div>
                )}
                <button
                  onClick={handleOpenRelease}
                  className="mt-3 inline-flex items-center gap-2 px-4 py-2 bg-green-600 text-white rounded-lg hover:bg-green-700 transition-colors"
                >
                  <Download className="w-4 h-4" />
                  前往下载
                  <ExternalLink className="w-3 h-3" />
                </button>
              </div>
            ) : (
              <div className="p-4 bg-green-500/10 text-green-700 dark:text-green-400 rounded-lg">
                <div className="flex items-center gap-2">
                  <CheckCircle2 className="w-4 h-4" />
                  <span>已是最新版本 ({updateInfo.current_version})</span>
                </div>
              </div>
            )}
          </div>
        )}
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        {/* 作者信息 */}
        <div className="bg-card rounded-xl border">
          <div className="p-4 border-b">
            <h3 className="font-semibold">作者信息</h3>
          </div>
          <div className="p-4 space-y-3">
            <div className="flex items-center gap-3">
              <div className="w-8 h-8 rounded-full bg-muted flex items-center justify-center text-sm">
                👤
              </div>
              <div>
                <p className="text-xs text-muted-foreground">作者</p>
                <p className="text-sm font-medium">PVPWHGAMES</p>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <div className="w-8 h-8 rounded-full bg-muted flex items-center justify-center">
                <Github className="w-4 h-4" />
              </div>
              <div>
                <p className="text-xs text-muted-foreground">GitHub</p>
                <a
                  href="https://github.com/PVPWHGAMES"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-sm font-medium text-primary hover:underline flex items-center gap-1"
                >
                  PVPWHGAMES
                  <ExternalLink className="w-3 h-3" />
                </a>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <div className="w-8 h-8 rounded-full bg-muted flex items-center justify-center">
                <Globe className="w-4 h-4" />
              </div>
              <div>
                <p className="text-xs text-muted-foreground">项目地址</p>
                <a
                  href="https://github.com/PVPWHGAMES/dns-proxy"
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-sm font-medium text-primary hover:underline flex items-center gap-1"
                >
                  PVPWHGAMES/dns-proxy
                  <ExternalLink className="w-3 h-3" />
                </a>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <div className="w-8 h-8 rounded-full bg-muted flex items-center justify-center text-sm">
                📄
              </div>
              <div>
                <p className="text-xs text-muted-foreground">许可证</p>
                <p className="text-sm font-medium">MIT License</p>
              </div>
            </div>
          </div>
        </div>

        {/* 打赏支持 */}
        <div className="bg-card rounded-xl border">
          <div className="p-4 border-b flex items-center gap-2">
            <Coffee className="w-4 h-4 text-primary" />
            <h3 className="font-semibold">请作者喝杯咖啡</h3>
          </div>
          <div className="p-4">
            <p className="text-sm text-muted-foreground mb-4 text-center">
              如果这个项目对你有帮助，可以请作者喝杯咖啡 ☕
            </p>
            <div className="grid grid-cols-2 gap-4">
              <div className="text-center">
                <div className="border rounded-lg p-3 bg-white mb-2">
                  <img
                    src="/wechat-pay.png"
                    alt="微信支付"
                    className="w-full h-auto"
                    onError={(e) => {
                      const target = e.target as HTMLImageElement;
                      target.style.display = "none";
                      const parent = target.parentElement;
                      if (parent) {
                        parent.innerHTML =
                          '<div class="flex items-center justify-center h-32 text-muted-foreground text-xs">请将 wechat-pay.png<br/>放入 public 目录</div>';
                      }
                    }}
                  />
                </div>
                <p className="text-xs text-muted-foreground">微信支付</p>
              </div>
              <div className="text-center">
                <div className="border rounded-lg p-3 bg-white mb-2">
                  <img
                    src="/alipay.jpg"
                    alt="支付宝"
                    className="w-full h-auto"
                    onError={(e) => {
                      const target = e.target as HTMLImageElement;
                      target.style.display = "none";
                      const parent = target.parentElement;
                      if (parent) {
                        parent.innerHTML =
                          '<div class="flex items-center justify-center h-32 text-muted-foreground text-xs">请将 alipay.jpg<br/>放入 public 目录</div>';
                      }
                    }}
                  />
                </div>
                <p className="text-xs text-muted-foreground">支付宝</p>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* 致谢 */}
      <div className="bg-card rounded-xl border">
        <div className="p-4 border-b flex items-center gap-2">
          <Heart className="w-4 h-4 text-red-500" />
          <h3 className="font-semibold">致谢</h3>
        </div>
        <div className="p-4">
          <div className="flex flex-wrap gap-3">
            {[
              { name: "Tauri", url: "https://tauri.app/" },
              { name: "trust-dns", url: "https://github.com/bluejekyll/trust-dns" },
              { name: "Loyalsoldier", url: "https://github.com/Loyalsoldier/v2ray-rules-dat" },
              { name: "WinTun", url: "https://www.wintun.net/" },
              { name: "React", url: "https://react.dev/" },
              { name: "Tailwind CSS", url: "https://tailwindcss.com/" },
              { name: "Lucide Icons", url: "https://lucide.dev/" },
            ].map((item) => (
              <a
                key={item.name}
                href={item.url}
                target="_blank"
                rel="noopener noreferrer"
                className="px-3 py-1.5 bg-muted rounded-lg text-sm hover:bg-accent transition-colors flex items-center gap-1"
              >
                {item.name}
                <ExternalLink className="w-3 h-3 text-muted-foreground" />
              </a>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
