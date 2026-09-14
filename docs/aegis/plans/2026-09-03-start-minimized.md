# 启动后最小化实施计划

## Goal
增加一个可持久化的“启动后最小化”设置。启用后，程序无论由用户手动启动还是 Windows 开机自启动任务启动，均在完成 Tauri 初始化后隐藏主窗口并驻留系统托盘；关闭设置时保持当前启动即显示主窗口的行为。DNS 服务和 TUN 的自动启动逻辑保持不变。

## Architecture
- `AppConfig` 继续作为 TOML 配置源，新增字段使用 Serde 默认值兼容旧配置。
- Rust/Tauri `setup` 继续作为主窗口启动生命周期的 owner，在托盘创建完成后执行初始隐藏。
- `Settings.tsx` 只编辑配置并复用现有 `save_config` 持久化流程。
- 现有托盘菜单“显示窗口”和左键恢复逻辑不变。

## Tech Stack
- Rust 2021、Tauri v2、Serde、TOML、Tokio
- React 18、TypeScript、Vite、Tailwind CSS
- Windows Tauri 桌面运行时和现有系统托盘

## Baseline/Authority Refs
- `CLAUDE.md`：Tauri 构建命令、Windows 运行约束和配置位置。
- `docs/architecture.md`：配置管理、系统托盘和前后端边界说明。
- `docs/aegis/baseline/2026-09-03-initial-baseline.md`：本次工作的产品/需求与运行边界基线。
- `src-tauri/src/config.rs`：当前 `AppConfig`、默认值和 TOML 读写实现。
- `src-tauri/src/lib.rs`：当前 Tauri `setup`、窗口事件和托盘恢复逻辑。
- `src/pages/Settings.tsx`：当前设置页和配置保存流程。
- `src/lib/api.ts`：前端 `AppConfig` 类型定义和 IPC 封装。

## Compatibility Boundary
- 旧版 `%APPDATA%\\dns-proxy\\config.toml` 缺少新字段时必须正常加载，并按关闭处理。
- 新字段的 TOML 名称固定为 `start_minimized`，前后端类型含义一致。
- 不改变开机自启动计划任务、DNS 自动启动、TUN 自动启动、关闭窗口隐藏、托盘恢复窗口等现有行为。
- 不引入新的 IPC 命令、窗口管理模块或任务栏普通最小化行为。

## TDD Route
- Mode: off
- Decision: skipped
- Strict authority: not applicable；用户确认了功能设计，但没有要求严格测试先行。
- Test posture: post-change regression
- Reason: 变更范围是一个兼容配置字段、一个已有生命周期分支和一个设置控件，比例合适的做法是进行配置/类型/构建回归及真实桌面行为验证。
- Verification: `npm run build`、Rust `cargo fmt --check`、`cargo check`，以及 Windows Tauri 应用手动验证。

## Aegis Visibility
本计划用于固定配置 owner、Tauri 窗口 owner、旧配置兼容边界和真实启动验证，避免把启动隐藏错误放到前端导致闪窗或改变后台服务生命周期。

## Plan Basis
已批准的设计决定：
- 适用于所有启动方式。
- “最小化”定义为隐藏到系统托盘。
- 作为设置页可持久化开关。
- 默认关闭。
- 复用现有托盘恢复入口。

## BaselineUsageDraft
- Required baseline refs: `CLAUDE.md`、`docs/architecture.md`、初始双基线、配置模型、Tauri 启动入口、设置页和 API 类型。
- Delivered context refs: 已在计划前读取上述代码和项目文档。
- Acknowledged before plan refs: `CLAUDE.md`、`docs/architecture.md`、`src-tauri/src/config.rs`、`src-tauri/src/lib.rs`。
- Cited in plan refs: 全部列于 Baseline/Authority Refs。
- Missing refs: 无阻塞性缺失；项目没有现有测试脚手架需要补充。
- Decision: continue

## Requirement Ready Check
- Requirement source refs: 本轮用户确认和已批准设计。
- Goals and scope refs: 新增启动后最小化开关，不改变服务启动。
- User / scenario refs: 手动启动、Windows 开机自启动、托盘恢复。
- Requirement item refs: 持久化、默认关闭、隐藏到托盘、所有启动方式生效。
- Acceptance / verification criteria refs: 旧配置兼容、设置保存、重启行为、托盘恢复、服务不变、构建通过。
- Open blocker questions: 无。
- Decision: ready

## Change Necessity
- User-visible need: 用户希望程序启动后可直接后台运行。
- No-change / non-code option: 只修改 Tauri 静态配置无法提供默认关闭的用户开关；只修改设置文案不会改变窗口行为。
- Why code change is necessary: 需要持久化配置、设置控件以及 Tauri 启动阶段读取配置并隐藏窗口。
- Minimum change boundary: `src-tauri/src/config.rs`、`src-tauri/src/lib.rs`、`src/lib/api.ts`、`src/pages/Settings.tsx`。
- Decision: code-change

## Existence Check
- Proposed new surface: 一个 `AppConfig` 字段和一个现有设置页控件；启动行为新增一个已有 `setup` 分支。
- Existing owner / reuse candidate: `AppConfig`、`Settings.tsx`、`lib.rs` 的 `setup` 和托盘处理。
- Why existing surface is insufficient: 现有模型没有该设置，现有 setup 没有按配置隐藏逻辑。
- Creation proof: 用户已确认需要可持久化开关；Tauri `setup` 是避免闪窗的现有生命周期位置。
- Entropy / retirement impact: 不新增 owner、IPC 命令、fallback 或兼容路径；无需退休旧逻辑。
- Decision: reuse-existing

## Architecture Integrity Lens
- Invariant: 窗口可见性不影响 DNS/TUN 服务生命周期。
- Canonical owner / contract: 配置由 `AppConfig` 持有；初始窗口可见性由 Rust/Tauri `setup` 决定；恢复由现有托盘处理。
- Responsibility overlap: 前端不参与启动隐藏，避免形成第二个窗口 owner。
- Higher-level simplification: 直接使用已有配置和 setup，不抽取窗口管理模块。
- Retirement / falsifier: 若真实运行证明 setup 隐藏发生在托盘创建前并导致恢复失败，才重新评估生命周期位置；当前没有该证据。
- Verdict: aligned，按现有 owner 原地修改。

## Plan Pressure Test
- Owner / contract / retirement: 只增加配置字段和已有 setup 分支，没有新 owner 或旧路径退休问题。
- Architecture integrity / higher-level path: 配置、Tauri 生命周期、托盘恢复的责任边界清晰。
- Verification scope: 静态类型、Rust 编译、旧配置反序列化、实际启动隐藏和托盘恢复。
- Task executability: 每个任务有明确文件、代码形状和命令。
- Pressure result: proceed

## Plan-Time Complexity Check
- Artifact class: 维护中的配置模型、Tauri 启动入口、设置页和 API 类型。
- Target files / artifacts: `src-tauri/src/config.rs`、`src-tauri/src/lib.rs`、`src/lib/api.ts`、`src/pages/Settings.tsx`。
- Current pressure: `lib.rs` 较长，但启动与托盘逻辑已集中在 `setup`；设置页已有开机自启动控件和统一保存按钮。
- Projected post-change pressure: 增加一个字段、一个 setup 条件和一个设置控件，低增量。
- Budget result: within-budget
- Planned governance: 只在现有 owner 内编辑，不抽象、不拆分、不新增持久化载体。
- Better file boundary: 继续编辑现有文件。
- Recommendation: edit-in-place

## Files

### 修改文件
- `src-tauri/src/config.rs`：在 `AppConfig` 中新增 `start_minimized`，使用 `#[serde(default)]`，并在 `Default` 中设为 `false`。
- `src-tauri/src/lib.rs`：在现有 `setup` 中读取已加载配置；托盘创建完成后，若 `start_minimized` 为 `true`，隐藏名为 `main` 的 Webview 窗口。隐藏失败只记录错误，不阻止 DNS/TUN 后台任务或应用启动。
- `src/lib/api.ts`：在 `AppConfig` 接口增加 `start_minimized: boolean`，供设置页类型安全地编辑。
- `src/pages/Settings.tsx`：在“监听设置”中增加“启动后最小化”复选框，绑定 `config.start_minimized`；保存仍由现有“保存设置”按钮触发。

### 不修改文件
- `src-tauri/tauri.conf.json`：不能用静态配置替代用户可控且默认关闭的运行时设置。
- `src-tauri/src/main.rs`：入口无需变化。
- `src-tauri` 托盘恢复和关闭事件逻辑：已有行为满足需求。

## Tasks

### Task 1：增加兼容的配置字段
**Files:** `src-tauri/src/config.rs`

**Why:** 让设置具备持久化来源，并保证旧 TOML 配置可继续读取。

**Change Necessity:** 静态窗口配置无法表达用户选择；最小源代码边界是 `AppConfig` 字段和默认值。

**Impact/Compatibility:** 新配置写出 `start_minimized = false/true`；旧配置缺少字段时由 Serde 默认值补为 `false`。其他配置字段和迁移逻辑不变。

**Steps:**
1. 在 `AppConfig` 字段中加入 `#[serde(default)] pub start_minimized: bool`，放在现有顶层配置字段附近。
2. 在 `impl Default for AppConfig` 的返回值中加入 `start_minimized: false`。
3. 运行 `cd src-tauri && cargo fmt --check && cargo check`。
4. 预期：格式检查和编译检查通过，且没有字段初始化遗漏。

**Verification:** Rust 格式检查和编译检查；后续通过 Tauri 构建验证 TOML 序列化链路。

**Retirement Track:** 无旧 owner、fallback 或兼容分支需要删除；`serde(default)` 是旧配置兼容边界的一部分。

### Task 2：在 Tauri setup 阶段按配置隐藏窗口
**Files:** `src-tauri/src/lib.rs`

**Why:** 让所有启动方式在 WebView 首次可见前进入托盘状态，避免前端加载后闪窗，并保持服务生命周期独立。

**Change Necessity:** 只有 Tauri 后端启动生命周期能稳定覆盖手动启动和计划任务启动；前端隐藏不满足无闪窗边界。

**Impact/Compatibility:** 仅当配置为 `true` 时隐藏 `main` 窗口；托盘创建、菜单恢复、左键恢复、关闭隐藏和 DNS/TUN 异步启动保持原样。

**Steps:**
1. 在现有 `setup` 创建托盘成功之后、启动 DNS/TUN 后台任务之前，获取 `app.get_webview_window("main")`。
2. 当已加载的 `AppConfig.start_minimized` 为 `true` 时调用 `window.hide()`。
3. 对隐藏错误记录一条现有 tracing 日志；不要让窗口隐藏失败阻断 `setup` 返回或后台服务启动。
4. 确保配置值来自 `run()` 中加载并放入 `AppState` 的同一份配置，不新增全局状态或 IPC 命令。
5. 运行 `cd src-tauri && cargo fmt --check && cargo check`。
6. 预期：Rust 编译通过，启动隐藏分支位于 Tauri setup owner 内。

**Verification:** Rust 编译；Windows Tauri 应用启动时分别使用开关关闭/开启验证窗口可见性、托盘存在和后台服务仍启动。

**Retirement Track:** 不退休现有托盘恢复或关闭事件逻辑；新分支只增加初始状态控制。

### Task 3：接入前端配置类型和设置控件
**Files:** `src/lib/api.ts`, `src/pages/Settings.tsx`

**Why:** 让用户能够读取、修改并保存该持久化设置。

**Change Necessity:** 没有前端字段和控件，用户无法改变配置；复用现有 `AppConfig` 加载/保存流程即可，不需要新 IPC。

**Impact/Compatibility:** 设置页加载旧配置时 Rust 已补出 `false`，因此复选框稳定为未选中；点击“保存设置”才写入配置，其他设置行为不变。

**Steps:**
1. 在 `src/lib/api.ts` 的 `AppConfig` 接口增加 `start_minimized: boolean`。
2. 在 `Settings.tsx` 现有“监听设置”开机自启动控件附近增加复选框。
3. 复选框的 `checked` 绑定 `config.start_minimized`，`onChange` 使用 `setConfig({ ...config, start_minimized: e.target.checked })`。
4. 文案使用“启动后最小化”，说明使用“启动后隐藏到系统托盘，可通过托盘恢复窗口”。
5. 不复用开机自启动的点击处理函数；该设置只修改本地 `config`，由现有保存按钮统一提交。
6. 运行 `npm run build`。
7. 预期：TypeScript 类型检查和 Vite 构建通过，设置页成功编译。

**Verification:** `npm run build`；实际应用中打开设置页，切换开关、保存、重启并验证状态持久化。

**Retirement Track:** 无旧前端控件或 API 需要删除；现有开机自启动控件继续独立工作。

### Task 4：执行端到端验证并构建可执行文件
**Files:** 无新增源文件；验证前述四个修改文件。

**Why:** 该功能的核心结果是桌面窗口真实启动状态，不能只依赖类型检查。

**Change Necessity:** Tauri 窗口显示/隐藏行为只有实际桌面运行才能确认；项目约束要求任务完成后构建 exe。

**Impact/Compatibility:** 验证不修改配置意外状态以外的项目内容；不生成安装包。

**Steps:**
1. 运行 `npm run build`，确认前端生产构建成功。
2. 运行 `cd src-tauri && cargo fmt --check && cargo check`，确认 Rust 代码格式和编译通过。
3. 运行 `npx tauri build --no-bundle`，确认生成 `src-tauri/target/release/dns-proxy.exe`。
4. 启动 exe，确认默认/关闭状态下主窗口正常显示，DNS 服务仍自动启动。
5. 在设置页开启“启动后最小化”并点击“保存设置”，退出后重新启动 exe；确认主窗口不显示、托盘图标存在。
6. 通过托盘左键和“显示窗口”菜单分别恢复窗口，确认窗口显示并获得焦点。
7. 关闭设置并保存，重新启动确认主窗口恢复正常显示。
8. 检查旧配置兼容：将配置复制为不含 `start_minimized` 的旧格式后启动，确认程序正常运行且窗口显示。
9. 验证现有开机自启动开关仍能单独启用/关闭；不改变计划任务实现。

**Verification:** 以上构建命令成功；手动/托盘/持久化/旧配置四类行为均符合验收标准。

**Retirement Track:** 无需清理旧路径；未引入 fallback 或重复 owner。

## Execution Readiness View
- Intent Lock: 交付可持久化的“启动后最小化”开关，默认关闭，所有启动方式隐藏到托盘。
- Scope Fence: 只改配置模型、Tauri setup、前端配置类型和设置控件；不改 DNS/TUN、托盘恢复、开机自启动注册方式。
- Baseline Lock: 遵循 `CLAUDE.md`、`docs/architecture.md` 和 `2026-09-03` 双基线；`AppConfig`、`lib.rs setup`、`Settings.tsx` 是既有 owner。
- Approved Behavior: 开启后手动启动和 Windows 计划任务启动均隐藏主窗口；关闭时正常显示；托盘可恢复。
- Owner / Contract Constraints: `start_minimized` 是唯一配置字段；不新增 IPC 命令；Rust setup 负责初始隐藏；托盘现有处理负责恢复。
- Compatibility Boundary: 缺失字段等于 false；已有服务启动、托盘和自启动行为不变。
- Retirement Boundary: 无需退休旧 owner、fallback 或兼容载体。
- Task Batches: 先配置字段，再 setup 行为，再前端控件，最后构建和真实桌面验证。
- Test Obligations: `npm run build`、`cargo fmt --check`、`cargo check`、`npx tauri build --no-bundle`，以及开关状态、托盘恢复、旧配置行为验证。
- Review Gates: 修改后检查 diff；构建成功后再报告完成。
- Drift / Rewind Rules: 若实现需要新增 IPC、窗口管理模块或改变服务启动时序，停止并回到设计确认；若仅是现有 owner 内实现差异，保持在本计划边界内。
- Evidence Required Before Completion: 构建输出、Rust 检查输出和真实启动/托盘操作结果。
- Advisory Boundary: 本视图是执行指导，不是 GateDecision、PolicySnapshot 或完成授权。

## Risks
- **隐藏时机风险**：若隐藏放在前端，会出现短暂闪窗；计划固定在 Tauri `setup`。
- **配置兼容风险**：旧 TOML 缺字段可能反序列化失败；计划使用 `#[serde(default)]` 并以旧配置启动验证。
- **托盘恢复风险**：隐藏发生在托盘创建前可能造成无入口状态；计划在托盘创建成功后隐藏，并验证菜单与左键恢复。
- **后台服务风险**：窗口隐藏不应取消 setup 后的异步任务；验证 DNS/TUN 状态仍按原逻辑启动。

## Rollback Surface
如验证失败，只需移除 `start_minimized` 字段、setup 中的隐藏分支、前端接口字段和设置控件；现有配置中该字段可被忽略或由旧版本正常读取，不涉及迁移数据删除、系统任务删除或外部共享状态。

## ADR / Baseline-Sync Signal
本次变更复用现有 owner，不形成新的持久架构决策；不需要新增 ADR。实施完成后应检查实现是否仍满足初始双基线，特别是配置兼容和窗口/服务生命周期分离。

## Self-Review
- Spec coverage: 已覆盖默认关闭、持久化、所有启动方式、隐藏到托盘、恢复窗口、服务不变和构建验证。
- Placeholder scan: 无 TBD、TODO 或未定义任务。
- Type consistency: Rust `bool`、Serde TOML 字段和 TypeScript `boolean` 均为 `start_minimized`。
- Compatibility: 明确旧配置缺字段按 false，现有行为不变。
- Minimality: 未新增 owner、IPC、模块、fallback 或依赖。
- Verification: 每个源文件修改任务都有对应编译/构建/运行验证，最终任务覆盖真实桌面行为。
- Decision: 计划可执行。
