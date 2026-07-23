import { ReactNode } from "react";
import Sidebar from "./Sidebar";
import Header from "./Header";

interface LayoutProps {
  children: ReactNode;
}

export default function Layout({ children }: LayoutProps) {
  return (
    <div className="flex h-screen bg-background">
      {/* 侧边栏 */}
      <Sidebar />

      {/* 主内容区 */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {/* 顶部栏 */}
        <Header />

        {/* 页面内容 */}
        <main className="flex-1 overflow-auto p-6 animate-fade-in">
          {children}
        </main>
      </div>
    </div>
  );
}
