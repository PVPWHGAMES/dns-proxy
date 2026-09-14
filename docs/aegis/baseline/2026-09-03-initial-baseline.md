# DNS Proxy 初始基线

日期：`2026-09-03`
状态：`initial dual-baseline snapshot`

## 1. Purpose
固定当前项目的产品需求边界和运行时 owner，供后续功能变更进行对齐检查。

## 2. Workspace Structure
- `src/`：React 前端
- `src-tauri/src/`：Rust/Tauri 后端
- `src-tauri/src/config.rs`：配置模型和 TOML 持久化
- `src-tauri/src/lib.rs`：Tauri 应用启动、窗口和托盘生命周期
- `src/pages/Settings.tsx`：设置页
- `src/lib/api.ts`：前端 IPC API 类型和调用封装

## 3. Current Authority Surfaces
- `CLAUDE.md`：项目运行、构建和交付约束
- `docs/architecture.md`：架构说明
- 当前没有既有 Aegis 规格或 ADR；本快照建立双基线起点。

## 4. Product / Requirement Baseline
### 4.1 Current Truth
- 目标：增加“启动后最小化”可持久化开关。
- 场景：手动启动和 Windows 开机自启动时，开启开关则不显示主窗口。
- 表现：隐藏到系统托盘，而不是普通最小化到任务栏。
- 默认：关闭；旧配置缺少字段时也按关闭处理。
- 验收：托盘图标存在，已有托盘入口可以恢复窗口；DNS/TUN 自动启动不变。

### 4.2 Non-negotiables
1. 不改变 DNS/TUN 的自动启动逻辑。
2. 不破坏托盘左键和“显示窗口”恢复行为。
3. 旧版配置必须兼容。

### 4.3 Product Non-goals
- 不新增普通任务栏最小化行为。
- 不改变开机自启动的注册方式。

## 5. Architecture / Runtime Boundary Baseline
### 5.1 Current Truth
- `AppConfig` 是 TOML 配置源。
- Rust/Tauri `setup` 是窗口初始可见性的 owner。
- `Settings.tsx` 编辑配置，`save_config` 负责持久化。
- 托盘恢复窗口逻辑已存在于 `lib.rs`。

### 5.2 Architecture Non-negotiables
1. 启动隐藏由 Rust/Tauri 生命周期负责，避免前端加载后的闪窗。
2. 新开关复用现有配置和设置页，不新增独立状态 owner。
3. 托盘恢复逻辑继续由现有托盘处理。

### 5.3 Architecture Non-goals
- 不新增窗口管理模块或新的 IPC 命令。

## 6. Ownership / Contract Snapshot
- 配置字段：`src-tauri/src/config.rs`
- 窗口启动行为：`src-tauri/src/lib.rs`
- 配置设置 UI：`src/pages/Settings.tsx`
- 前端配置类型：`src/lib/api.ts`

## 7. Current State and Risks
- 当前主窗口启动时默认显示。
- 当前关闭窗口会隐藏到托盘。
- 风险是启动隐藏若放在前端会造成闪窗，因此固定由 Tauri `setup` 执行。

## 8. Alignment Use
- 涉及配置字段时检查产品基线和 `AppConfig` owner。
- 涉及窗口/托盘时检查运行边界基线和 `lib.rs` owner。
- 两者同时变化时报告 scope: both。

## 9. Compatibility Boundary
缺少新字段的旧 TOML 配置必须成功加载，并将启动后最小化视为关闭。
