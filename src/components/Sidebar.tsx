import { useState, useEffect } from "react";
import { Link, useLocation } from "react-router-dom";
import {
  LayoutDashboard,
  Settings,
  ListFilter,
  FileText,
  Globe,
  Shield,
  Wifi,
  Info,
} from "lucide-react";
import { api } from "../lib/api";

const navItems = [
  {
    title: "仪表盘",
    icon: LayoutDashboard,
    path: "/",
  },
  {
    title: "DNS 设置",
    icon: Settings,
    path: "/settings",
  },
  {
    title: "网络设置",
    icon: Wifi,
    path: "/network",
  },
  {
    title: "规则管理",
    icon: ListFilter,
    path: "/rules",
  },
  {
    title: "日志查看",
    icon: FileText,
    path: "/logs",
  },
  {
    title: "关于",
    icon: Info,
    path: "/about",
  },
];

export default function Sidebar() {
  const location = useLocation();
  const [isRunning, setIsRunning] = useState(false);

  useEffect(() => {
    const refreshStatus = async () => {
      try {
        const status = await api.getServerStatus();
        setIsRunning(status);
      } catch (e) {
        console.error("获取状态失败:", e);
      }
    };

    refreshStatus();
    const interval = setInterval(refreshStatus, 2000);
    return () => clearInterval(interval);
  }, []);

  return (
    <aside className="w-64 bg-card border-r flex flex-col">
      {/* Logo */}
      <div className="p-6 border-b">
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-lg bg-primary flex items-center justify-center">
            <Globe className="w-6 h-6 text-primary-foreground" />
          </div>
          <div>
            <h1 className="font-bold text-lg">DNS Proxy</h1>
            <p className="text-xs text-muted-foreground">v1.0.8</p>
          </div>
        </div>
      </div>

      {/* 导航菜单 */}
      <nav className="flex-1 p-4 space-y-1">
        {navItems.map((item) => {
          const Icon = item.icon;
          const isActive = location.pathname === item.path;

          return (
            <Link
              key={item.path}
              to={item.path}
              className={`
                flex items-center gap-3 px-4 py-3 rounded-lg transition-all
                ${
                  isActive
                    ? "bg-primary text-primary-foreground shadow-md"
                    : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                }
              `}
            >
              <Icon className="w-5 h-5" />
              <span className="font-medium">{item.title}</span>
            </Link>
          );
        })}
      </nav>

      {/* 底部状态 */}
      <div className="p-4 border-t">
        <div
          className={`flex items-center gap-3 px-4 py-3 rounded-lg ${
            isRunning ? "bg-green-50 border border-green-200" : "bg-muted"
          }`}
        >
          <Shield
            className={`w-5 h-5 ${
              isRunning ? "text-green-600" : "text-muted-foreground"
            }`}
          />
          <div>
            <p className="text-sm font-medium">代理状态</p>
            <div className="flex items-center gap-2">
              {isRunning && (
                <div className="w-2 h-2 rounded-full bg-green-500 animate-pulse" />
              )}
              <p
                className={`text-xs ${
                  isRunning ? "text-green-600 font-medium" : "text-muted-foreground"
                }`}
              >
                {isRunning ? "运行中" : "未运行"}
              </p>
            </div>
          </div>
        </div>
      </div>
    </aside>
  );
}
