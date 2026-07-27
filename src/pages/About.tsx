import { Globe, Github, Heart, Coffee, ExternalLink } from "lucide-react";

export default function About() {
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
        <p className="text-muted-foreground mb-3">版本 1.0.7</p>
        <p className="text-sm text-muted-foreground max-w-md mx-auto">
          一个类似 YogaDNS 的 Windows 全局 DNS 代理软件，支持国内外域名分流、GeoSite 路由、广告拦截。
        </p>
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
