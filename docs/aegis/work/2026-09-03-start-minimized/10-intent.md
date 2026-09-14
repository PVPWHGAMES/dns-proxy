# 启动后最小化任务意图

父计划：`docs/aegis/plans/2026-09-03-start-minimized.md`

本工作流直接在当前非 Git 工作区执行。所有修改必须保持在父计划列出的四个源文件内，不提交、不推送、不改变 DNS/TUN、托盘恢复或开机自启动注册逻辑。

## SubagentContextPacket 摘要
- 目标：增加默认关闭、可持久化的 `start_minimized` 设置。
- 启动行为：所有启动方式开启时隐藏主窗口到托盘。
- 配置兼容：旧 TOML 缺少字段时按 false。
- owner：`config.rs` 管理配置，`lib.rs` Tauri setup 管理初始窗口，`Settings.tsx` 管理 UI，`api.ts` 管理类型。
- 验证：任务级 focused check；最终 npm build、cargo fmt/check、tauri build 和真实桌面行为。
- 禁止：新 IPC、新模块、新 fallback、静态固定隐藏、普通任务栏最小化。
