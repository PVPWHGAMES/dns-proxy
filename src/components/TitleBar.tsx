import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Minus, Square, X } from "lucide-react";

const appWindow = getCurrentWindow();

export default function TitleBar() {
  const [isMaximized, setIsMaximized] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    const watchWindowState = async () => {
      setIsMaximized(await appWindow.isMaximized());
      unlisten = await appWindow.onResized(async () => {
        setIsMaximized(await appWindow.isMaximized());
      });
    };

    watchWindowState().catch((error) => console.error("读取窗口状态失败:", error));
    return () => unlisten?.();
  }, []);

  const minimize = () => appWindow.minimize().catch((error) => console.error("最小化窗口失败:", error));
  const toggleMaximize = () => appWindow.toggleMaximize().catch((error) => console.error("切换窗口大小失败:", error));
  // 关闭请求仍交给后端 WindowEvent 处理，因此会隐藏到托盘，不会直接中止 DNS 服务。
  const close = () => appWindow.close().catch((error) => console.error("关闭窗口失败:", error));

  return (
    <div className="titlebar h-9 shrink-0 border-b border-border bg-card flex items-stretch select-none">
      {/* 拖拽与双击最大化由 Tauri 的 data-tauri-drag-region 原生处理，这里不再重复绑定 onDoubleClick，避免双击切换两次。 */}
      <div data-tauri-drag-region className="flex-1 min-w-0" />

      <div className="window-controls flex items-stretch" aria-label="窗口控制">
        <button type="button" className="window-control" onClick={minimize} title="最小化" aria-label="最小化">
          <Minus size={16} strokeWidth={2} />
        </button>
        <button type="button" className="window-control" onClick={toggleMaximize} title={isMaximized ? "还原" : "最大化"} aria-label={isMaximized ? "还原" : "最大化"}>
          {isMaximized ? <Copy size={13} strokeWidth={1.7} /> : <Square size={13} strokeWidth={1.7} />}
        </button>
        <button type="button" className="window-control window-control-close" onClick={close} title="关闭到系统托盘" aria-label="关闭到系统托盘">
          <X size={17} strokeWidth={1.8} />
        </button>
      </div>
    </div>
  );
}
