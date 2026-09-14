# 启动后最小化工作检查点

日期：`2026-09-03`
状态：`in_progress`
执行方式：当前工作区直接执行（用户已明确允许；项目不是 Git 仓库，未创建 worktree）
父计划：`docs/aegis/plans/2026-09-03-start-minimized.md`

## TaskIntentDraft
- 目标：新增可持久化“启动后最小化”开关。
- 成功证据：开启后所有启动方式隐藏到托盘；关闭时正常显示；托盘可恢复；DNS/TUN 自动启动不变；构建通过。
- 停止条件：完成、阻塞、需要验证或超出范围时停止并报告，不绕过检查。
- 非目标：不改变开机自启动注册、普通任务栏最小化、DNS/TUN 启动和托盘恢复逻辑。

## BaselineReadSetHint
- `CLAUDE.md`
- `docs/architecture.md`
- `docs/aegis/baseline/2026-09-03-initial-baseline.md`
- `src-tauri/src/config.rs`
- `src-tauri/src/lib.rs`
- `src/lib/api.ts`
- `src/pages/Settings.tsx`

## BaselineUsageDraft
- Required refs: 上述项目约束、架构文档、配置模型、Tauri 启动入口、前端 API 和设置页。
- Acknowledged refs: 已在规划和执行前读取。
- Cited refs: 已列入 `docs/aegis/plans/2026-09-03-start-minimized.md`。
- Missing refs: 无阻塞性缺失。
- Decision: continue

## ImpactStatementDraft
- 配置 owner：`AppConfig` / `config.rs`。
- 窗口启动 owner：Tauri `setup` / `lib.rs`。
- UI owner：`Settings.tsx`。
- 前端类型 owner：`api.ts`。
- 兼容边界：旧配置缺少 `start_minimized` 时按 false。

## Execution Readiness View
- Intent Lock：持久化、默认关闭、所有启动方式、隐藏到托盘。
- Scope Fence：只修改计划列出的四个源文件；不新增 IPC、模块或 fallback。
- Baseline Lock：配置由 `AppConfig` 持有，启动隐藏由 Tauri setup 负责，托盘恢复复用现有实现。
- Approved Behavior：开启隐藏、关闭显示、托盘恢复、服务启动不变。
- Review Gates：每个实现任务后进行规格符合性和代码质量检查；最终进行构建和真实桌面验证。
- Evidence Required：源文件实际 diff、`npm run build`、`cargo fmt --check`、`cargo check`、`npx tauri build --no-bundle`、手动启动与托盘操作结果。
- Advisory Boundary：本视图是执行指导，不是完成授权。

## TodoCheckpointDraft
### 当前 todo
1. [completed] 增加兼容的配置字段
2. [completed] 在 Tauri setup 阶段按配置隐藏窗口
3. [completed] 接入前端配置类型和设置控件
4. [in_progress] 执行端到端验证并构建可执行文件

### 已完成 todo
- 需求和设计已确认。
- 实施计划已写入。
- Aegis 工作区和初始双基线已建立。
- Task 1 已完成；规格符合性审查和代码质量审查均通过。
- Task 1 `cargo check` 通过；`cargo fmt --check` 仅报告未修改的既有文件差异。
- Task 2 已完成；规格符合性审查和代码质量审查均通过。
- Task 2 `cargo check` 通过。
- Task 3 已完成；规格符合性审查通过。
- Task 3 质量审查中的 `tun` 类型缺口和保存后重复服务重启，均确认属于既有范围外问题，未扩展本次改动。
- Task 3 `npm run build` 通过，仅有既有 chunk 体积警告。

### 当前切片
Task 4：执行前端构建、Rust 检查、Tauri 无安装包构建，并验证启动隐藏、托盘恢复、配置持久化和旧配置兼容。

### 下一步
派发最终验证子代理；完成后进行最终代码审查和完成前验证。

## DriftCheckDraft
- 当前工作仍服务原始目标：是。
- 是否超出范围：否；质量审查提出的既有问题未被纳入。
- 是否新增 owner/fallback/adapter：否。
- 旧路径退休：无；配置字段由 Serde 默认值兼容旧配置。
- Decision: continue

## EvidenceBundleDraft
- 当前证据：配置字段、Tauri setup 隐藏逻辑、前端类型和设置控件已完成；Task 1-3 规格审查通过；Task 1-2 Rust cargo check 和 Task 3 npm build 通过。
- 范围外观察：`AppConfig` 前端既有 `tun` 类型缺口；保存配置既有重复服务重启；本次未修改。
- 状态：partial

## Risk / Unknown
- 项目无 Git 仓库，无法使用 worktree 和 diff/commit 作为隔离与版本证据；已获用户明确许可直接执行。
- 真实窗口隐藏仍需在 Windows Tauri 应用中验证。
