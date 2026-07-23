import { useState, useEffect } from "react";
import { useLocation } from "react-router-dom";
import { Moon, Sun } from "lucide-react";

const pageTitles: Record<string, string> = {
  "/": "仪表盘",
  "/settings": "DNS 设置",
  "/rules": "规则管理",
  "/logs": "日志查看",
};

export default function Header() {
  const location = useLocation();
  const [isDark, setIsDark] = useState(false);
  const [currentTime, setCurrentTime] = useState(new Date());

  useEffect(() => {
    const timer = setInterval(() => {
      setCurrentTime(new Date());
    }, 1000);

    return () => clearInterval(timer);
  }, []);

  const toggleTheme = () => {
    setIsDark(!isDark);
    document.documentElement.classList.toggle("dark");
  };

  const pageTitle = pageTitles[location.pathname] || "DNS Proxy";

  return (
    <header className="h-16 border-b bg-card flex items-center justify-between px-6">
      {/* 左侧标题 */}
      <div>
        <h2 className="text-xl font-semibold">{pageTitle}</h2>
        <p className="text-xs text-muted-foreground">
          {currentTime.toLocaleDateString("zh-CN", {
            year: "numeric",
            month: "long",
            day: "numeric",
            weekday: "long",
          })}{" "}
          {currentTime.toLocaleTimeString("zh-CN")}
        </p>
      </div>

      {/* 右侧控制 */}
      <div className="flex items-center gap-2">
        {/* 主题切换 */}
        <button
          onClick={toggleTheme}
          className="p-2 rounded-lg hover:bg-accent transition-colors"
          title={isDark ? "切换到浅色模式" : "切换到深色模式"}
        >
          {isDark ? (
            <Sun className="w-5 h-5" />
          ) : (
            <Moon className="w-5 h-5" />
          )}
        </button>
      </div>
    </header>
  );
}
