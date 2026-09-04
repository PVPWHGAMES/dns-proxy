import { Link, useLocation } from "react-router-dom";
import {
  LayoutDashboard,
  Settings,
  ListFilter,
  FileText,
  Wifi,
  Info,
} from "lucide-react";

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

  return (
    <aside className="w-64 bg-card border-r flex flex-col">
      {/* Logo */}
      <div className="h-16 px-4 border-b flex items-center">
        <div className="flex items-center gap-3">
          <div className="w-10 h-10 rounded-lg bg-primary flex items-center justify-center overflow-hidden">
            <img src="/logo.png" alt="果冻网络加速" className="w-full h-full object-cover" />
          </div>
          <div>
            <h1 className="font-bold text-lg">果冻网络加速</h1>
            <p className="text-xs text-muted-foreground">v1.1.3</p>
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
                flex items-center gap-3 px-4 py-3 rounded-lg transition-all active:scale-95
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
    </aside>
  );
}
