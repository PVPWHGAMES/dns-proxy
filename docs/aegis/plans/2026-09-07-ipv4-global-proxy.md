# IPv4 全局 TCP/UDP 代理实施计划

## Goal

实现已批准的 IPv4 TCP/UDP TUN 全局接管：所有 IPv4 TCP/UDP 进入统一 WinTun dispatcher；DNS 交给现有 `DnsHandler`；`proxy` 分组经 SOCKS5，其他分组物理出口直连；代理失败不回退直连；TUN 停止时恢复 DNS、路由和会话资源。

## Architecture

- `TunDevice` 继续拥有 WinTun adapter/session、IPv4 地址、系统 DNS、路由状态和回滚。
- `TrafficProxy` 成为 WinTun session 的唯一读取 owner，使用 `ipstack` 统一拆分 IPv4 TCP/UDP/DNS 会话。
- `DnsHandler` 继续拥有 DNS 规则、缓存和 IP→域名/分组映射。
- `AppConfig` 是 TOML 源，`NetworkSettings` 是配置编辑 owner。
- 旧 `DnsInterceptor` 从主路径退休，不保留第二个 session reader。

## Tech Stack

Rust 2021、Tokio、ipstack 1.0.1、WinTun、Windows IpHelper/WinSock API、socks5-impl 0.9.6、udp-stream 0.0.12、Tauri v2、React 18 + TypeScript。

## Baseline/Authority Refs

- `CLAUDE.md`
- `AGENTS.md`
- `docs/architecture.md`
- `docs/aegis/BASELINE-GOVERNANCE.md`
- `docs/aegis/baseline/2026-09-03-initial-baseline.md`
- `docs/aegis/specs/2026-09-07-ipv4-global-proxy-design.md`
- `src-tauri/src/config.rs`
- `src-tauri/src/tun/device.rs`
- `src-tauri/src/tun/traffic.rs`
- `src-tauri/src/tun/dns_intercept.rs`
- `src-tauri/src/dns/handler.rs`
- `src-tauri/src/lib.rs`
- `src/lib/api.ts`
- `src/pages/NetworkSettings.tsx`

## Compatibility Boundary

- 旧 TOML 缺少 `tun.outbound` 时按 `enabled=false` 加载。
- `tun.enabled` 仍控制 TUN/DNS；`tun.outbound.enabled` 才开启 IPv4 TCP/UDP 数据面。
- outbound 关闭时不添加 IPv4 默认路由。
- 只删除本次启动添加的路由，不删除启动前已有路由。
- IPv6、ICMP、HTTP/SOCKS4 和按进程分流保持非目标。
- 不修改 DNS 规则、缓存和上游协议行为。
- 不修改安装包配置或版本号。

## TDD Route

- Mode: off
- Decision: skipped
- Strict authority: not applicable
- Test posture: post-change regression plus focused protocol/unit tests
- Reason: 用户未要求严格测试先行，项目当前也未建立完整 TDD 流程；按风险配置针对性回归测试。
- Verification: `cargo test`、`cargo clippy`、`npm run build`、管理员 Windows 手测。

## Verification

- 每个 Rust 变更切片后运行 `cargo check` 或对应 focused test。
- 协议 helper 通过本地 mock SOCKS5 server 测试 CONNECT、认证、UDP header 和 UDP ASSOCIATE。
- 数据面通过单一 reader 静态引用检查和 `cargo test` 验证。
- 前端运行 `npm run build`。
- 最终运行 `cargo fmt --check`、`cargo test`、`cargo clippy`、`npm run build`、`npx tauri build --no-bundle`。
- Windows 管理员环境手测路由、TCP/UDP 直连、TCP/UDP SOCKS5、代理故障阻断和停止回滚。

## BaselineUsageDraft

- Required baseline refs: `CLAUDE.md`、`AGENTS.md`、`docs/architecture.md`、`docs/aegis/BASELINE-GOVERNANCE.md`、初始基线、已批准设计规格。
- Delivered context refs: TUN device/traffic/DNS handler/Tauri lifecycle/config/API/network settings。
- Cited in plan refs: all listed refs above。
- Missing refs: 没有既有路由快照或全局代理测试基线；通过新增 focused tests 和 Windows 手测补足。
- Decision: continue。

## Requirement Ready Check

- Requirement source refs: 已批准设计规格及本轮确认的产品决策。
- Goals and scope refs: IPv4 TCP/UDP 全局接管、按 DNS 分组分流。
- User/scenario refs: Windows 全局应用流量、SOCKS5 代理和直连分流。
- Requirement item refs: 双开关、fail-closed、IPv6 未代理、路由回滚、最小 UI。
- Acceptance/verification refs: 设计规格第 11 节及本计划 Verification。
- Open blocker questions: 无。
- Decision: ready。

## Change Necessity

- User-visible need: 当前 `TrafficProxy` 丢弃 UDP，TCP proxy 分组只记录后断开，且双 reader 会丢包；无法实现已确认功能。
- No-change/non-code option: 仅修改文档或配置不能改变 TUN 数据面行为。
- Why code change is necessary: 必须新增 TCP/UDP 出口、统一 session reader、路由回滚和配置 UI。
- Minimum change boundary: `tun/traffic.rs`、`tun/device.rs`、`tun/mod.rs`、`lib.rs`、`config.rs`、`api.ts`、`NetworkSettings.tsx`，以及依赖锁文件。
- Decision: code-change。

## Existence Check

- Proposed new surface: `TunDevice` 内部路由状态/helper 和 `TrafficProxy` 内部 SOCKS5/UDP helper。
- Existing owner/reuse candidate: `TunDevice`、`TrafficProxy`、`DnsHandler`。
- Why insufficient: 现有 owner 没有保存路由变更、没有 SOCKS5 出口、且 TrafficProxy 未处理 UDP。
- Creation proof: 没有这些最小 helper 无法防止回环、无法实现代理协议或安全回滚。
- Entropy/retirement impact: helper 限定在现有 owner 内；统一 dispatcher 后删除 `DnsInterceptor` 主路径。
- Decision: add-with-proof。

## Architecture Integrity Lens

- Invariant: 一个 WinTun session 只能有一个读取循环，所有进入 TUN 的 IPv4 TCP/UDP 必须有确定出口。
- Canonical owner/contract: `TrafficProxy` 统一读取和会话处理，`DnsHandler` 处理 DNS，`TunDevice` 管理系统网络。
- Responsibility overlap: `DnsInterceptor` 与 `TrafficProxy` 当前重复读取；将迁移 DNS 行为后退休旧 owner。
- Higher-level simplification: 在 `ipstack` 层统一处理 DNS/TCP/UDP，移除裸包解析的主路径。
- Retirement/falsifier: `lib.rs` 不再构造或启动 `DnsInterceptor`；任何残留引用只能存在于已删除文件之外的文档，不得参与运行时。
- Verdict: proceed with owner consolidation。

## Plan Pressure Test

- Owner/contract/retirement: 已明确，配置和系统网络边界由既有 owner 承担。
- Architecture integrity/higher-level path: 统一 `ipstack` dispatcher 是较高层且更简单的路径。
- Verification scope: 协议、分组、路由回滚、UI 编译和 Windows 手测均覆盖。
- Task executability: 每个任务限定文件、接口和命令，可逐片验证。
- Pressure result: proceed。

## Plan-Time Complexity Check

- Artifact class: 跨模块运行时功能和系统网络适配。
- Target files: 现有 `tun/traffic.rs`、`tun/device.rs`、`lib.rs` 规模较大，`config.rs`、API 和页面为契约传播文件。
- Current pressure: session reader 重复、路由无状态、TrafficProxy 未完成出口。
- Projected pressure: protocol helper 和 route state 增加，但不新建通用代理框架。
- Better file boundary: 网络状态留在 `TunDevice`；会话/协议留在 `TrafficProxy`；生命周期仅传递句柄。
- Recommendation: edit existing owners plus small private helpers; retire duplicate reader。

## Execution Readiness View

- Intent Lock: IPv4 TCP/UDP 全局接管，按 DNS 分组分流，代理 fail-closed。
- Scope Fence: 不做 IPv6、ICMP、其它代理协议、按进程分流和安装包。
- Baseline Lock: 遵守已批准设计规格和双开关兼容语义。
- Approved Behavior: `proxy`→SOCKS5，其他→直连，DNS→DnsHandler，IPv6→明确未代理。
- Owner/Contract Constraints: 单一 WinTun reader；AppConfig/TunConfig snake_case；TunDevice 负责路由和 DNS。
- Compatibility Boundary: 旧配置默认 outbound 关闭；停止只清理本次路由。
- Retirement Boundary: `DnsInterceptor` 不再进入主路径。
- Task Batches: 依赖/配置 → 协议 helper → dispatcher → 路由 → lifecycle/UI → full verification。
- Test Obligations: focused Rust tests、cargo tests/clippy、前端 build、Tauri production build、管理员 Windows 手测。
- Review Gates: 每批 focused check；统一 reader 和路由回滚完成后做架构复核。
- Drift/Rewind Rules: 若发现 ipstack API 或 Windows 路由假设不成立，暂停当前批次，回到 owner/contract 设计，不添加旁路 fallback。
- Evidence Required Before Completion: 命令输出、构建产物路径、Windows 路由/DNS 回滚证据和 IPv4 TCP/UDP 流量证据。
- Advisory Boundary: 本视图是执行指导，不替代完成判定。

## Tasks

### 1. 加入协议依赖并扩展配置契约

**Files:** `src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`、`src-tauri/src/tun/mod.rs`、`src/lib/api.ts`。

**Why:** SOCKS5 TCP/UDP 需要稳定协议实现；前后端必须能保存和读取 outbound 配置。

**Change Necessity:** 现有依赖和 TypeScript `TunConfig` 无法表达 SOCKS5 出口，必须修改最小契约边界。

**Impact/Compatibility:** 使用 `socks5-impl = { version = "0.9.6", default-features = false, features = ["client", "tokio"] }`；使用 `udp-stream = "0.0.12"`；保留 `OutboundConfig` 默认值和 serde defaults。

**Steps:**

1. 更新 Cargo 依赖并执行 `cd src-tauri && cargo check`，确认锁文件解析成功。
2. 为 `TunConfig`/`OutboundConfig` 补齐 `Debug/Clone/Serialize/Deserialize/Default` 所需契约，验证缺少 outbound 的旧 TOML 仍反序列化成功。
3. 在 `src/lib/api.ts` 增加 `OutboundConfig`，并将其嵌入 `TunConfig`；更新默认前端状态。
4. 执行 `npm run build`，确认 API 类型传播没有遗漏。

### 2. 实现可测试的 SOCKS5 TCP/UDP 出口 helper

**Files:** `src-tauri/src/tun/traffic.rs`，必要时仅在该文件内增加私有结构和测试。

**Why:** 当前 TCP proxy 分组未转发，UDP 分支直接丢弃；需要实现 CONNECT、认证、UDP ASSOCIATE 和 RFC 1928 header。

**Change Necessity:** 这是用户确认的核心功能，不能由配置或现有直连代码替代。

**Impact/Compatibility:** 仅接受 IPv4 目标；认证字段可选；代理建立失败返回错误，由 caller 阻断当前会话；密码和凭据不写日志。

**Steps:**

1. 定义从 `OutboundConfig` 构造 SOCKS5 server address、可选 `UserKey` 和目标 `Address` 的私有 helper。
2. 实现 TCP `connect_socks5`：通过物理出口连接 SOCKS5 server，调用 `socks5_impl::client::connect`，成功后返回已建立的 `BufStream<TcpStream>`。
3. 实现 UDP ASSOCIATE：保持控制 TCP stream 存活，创建 IPv4 UDP socket，调用 `SocksDatagram::udp_associate`，用 `send_to`/`recv_from` 将单个 TUN UDP session 映射到 SOCKS5 UDP 数据报。
4. 为直连 TCP/UDP 提供物理出口创建 helper；不得让出口 socket 重新进入 TUN。
5. 添加 focused tests：代理目标分组选择、代理错误返回、IPv4 地址编码、UDP header round-trip；运行 `cargo test tun::traffic`。

### 3. 将 DNS、TCP、UDP 合并到唯一 WinTun dispatcher

**Files:** `src-tauri/src/tun/traffic.rs`、`src-tauri/src/tun/dns_intercept.rs`、`src-tauri/src/lib.rs`。

**Why:** 两个 reader 竞争同一个 WinTun session 会丢包；统一 dispatcher 才能可靠处理 DNS、TCP 和 UDP。

**Repair Track:** 将 DNS 查询提取为 `IpStackUdpStream` 上的处理函数，复用 `DnsHandler::handle_query` 和对应 stream 地址；TCP/UDP 会话都从同一个 `IpStack::accept()` 来。

**Retirement Track:** 退休 `DnsInterceptor` 运行时 owner，删除 `lib.rs` 的构造/启动引用；确认没有重复 reader。

**Steps:**

1. 修改 `WintunAsyncDevice` 保持单一后台接收线程，把 packet stream 交给一个 `IpStack`。
2. 在 `TrafficProxy::start` 中配置 `ipstack` IPv4 TCP/UDP 超时，并按 `IpStackStream` 分支 spawn DNS/TCP/UDP handler。
3. DNS UDP 目标端口 53 调用 `DnsHandler`，使用 stream 的源地址，不再手工构造裸 IPv4 response。
4. TCP handler 按 `lookup_ip` 选择 SOCKS5 或物理直连；`proxy` 失败直接结束会话。
5. UDP handler 按同样规则选择 SOCKS5 UDP 或物理直连，正确把响应写回 `IpStackUdpStream`。
6. 对 IPv6、UnknownTransport 和 UnknownNetwork 明确记录并丢弃，不改变 IPv4 主路径。
7. 移除 `lib.rs` 对 `DnsInterceptor` 的启动；让 `TrafficProxy` 在 outbound 启用时负责完整数据面，在 outbound 关闭时仍能以 DNS-only dispatcher 工作，或由生命周期明确只启动 DNS dispatcher，不能启动第二个 reader。
8. 删除不再使用的 `dns_intercept.rs`，然后执行 `rg "DnsInterceptor|dns_intercept" src-tauri/src`，结果不得有运行时引用。
9. 执行 `cd src-tauri && cargo test tun` 和 `cargo check`。

### 4. 在 TunDevice 内实现 IPv4 路由状态、出口绕行与回滚

**Files:** `src-tauri/src/tun/device.rs`，必要时 `src-tauri/src/tun/mod.rs`。

**Why:** 当前添加默认路由后不删除；代理出口会回环；停止后可能残留系统网络状态。

**Change Necessity:** 全局 IPv4 接管无法安全工作，除非系统路由和出口绕行有可回滚 owner。

**Impact/Compatibility:** 仅 Windows IPv4；只清理当前实例添加的路由；所有系统命令错误都必须可观察。

**Steps:**

1. 增加私有 `RouteEntry`/`RouteState`，记录目标前缀、掩码、网关、接口索引、metric 和是否由本次实例创建。
2. 启动 TUN 前获取物理 IPv4 默认接口/网关，并保存到 `TunDevice`；避免把代理服务器和 DNS 上游连接送进 TUN。
3. 仅当 `outbound.enabled && auto_route` 时添加 TUN 默认路由；outbound 关闭时跳过默认路由。
4. 为 SOCKS5 endpoint 和直连所需的物理出口添加更具体路由，使用 Windows IpHelper API 或现有 `route` 命令封装；不要删除预先存在的同等路由。
5. 为物理出口 socket 增加接口选择/绑定所需的 Windows WinSock 设置，验证 `socket2` 当前版本能提供的 API；若不能，使用已记录的具体 host route 作为唯一绕行机制，不添加未经验证的 fallback。
6. 在 `TunDevice::stop` 和启动失败回滚中逆序删除 `RouteState` 记录的新增路由，再恢复 DNS 和 session。
7. 增加纯函数测试：路由记录去重、只删除 owned entries、outbound 关闭不生成默认路由。
8. 执行 `cd src-tauri && cargo test tun::device` 和 `cargo check`。

### 5. 接通 Tauri 生命周期与状态

**Files:** `src-tauri/src/lib.rs`、`src-tauri/src/tun/traffic.rs`。

**Why:** 新 dispatcher、路由状态和代理配置必须在手动启动、自动启动、重启、停止路径中保持一致。

**Impact/Compatibility:** 保留现有 Tauri commands；不新增重复状态 owner；`traffic_proxy` 句柄停止前先释放。

**Steps:**

1. 修改 `start_tun_internal`：创建/启动 TunDevice 后取得 session，启动唯一 TrafficProxy；根据 outbound 状态选择 full dispatcher；错误时停止 TUN 并回滚。
2. 修改 `stop_tun`：先停止并 take TrafficProxy，再停止 TunDevice；确保 dispatcher 不再读取已释放 session。
3. 修改自动启动和托盘重启路径，全部调用同一内部启动/停止流程，删除仅启动 `DnsInterceptor` 的旁路代码。
4. 扩展 `TunStatus` 增加 IPv4 数据面状态和 IPv6 未代理可见信息（保持已有字段兼容，新增字段使用 serde default/前端默认值）。
5. 检查并修复配置保存时 `AppConfig.tun` 与独立 `tun_config` 的同步。
6. 执行 `cd src-tauri && cargo check`，并搜索所有 TUN 启停入口确认使用统一流程。

### 6. 增加网络设置 UI 和前端状态显示

**Files:** `src/pages/NetworkSettings.tsx`、`src/lib/api.ts`、必要时 `src/pages/Dashboard.tsx`。

**Why:** 用户需要启用/配置 SOCKS5，而不是手动编辑 TOML；IPv6-only 边界必须可见。

**Change Necessity:** 已确认最小 UI 是本次需求范围，后端字段没有 UI 会导致功能不可用。

**Steps:**

1. 在网络设置页增加“IPv4 全局 TCP/UDP 代理”开关。
2. 增加 SOCKS5 地址、端口、用户名、密码输入；密码输入使用 password 类型；仅在 outbound enabled 时显示或启用相关字段。
3. 添加提示：IPv4 TCP/UDP 按 DNS 分组分流，IPv6 当前未代理；代理不可用时 proxy 分组流量阻断。
4. 保持保存按钮复用 `saveTunConfig`，不新建配置状态 owner。
5. 在 Dashboard 的 TUN 状态区域显示数据面是否开启和 IPv6 未代理提示；不伪造实际运行状态。
6. 执行 `npm run build`。

### 7. 文档、架构复核与全量验证

**Files:** `docs/architecture.md`、`docs/aegis/INDEX.md`，必要时新增 `docs/aegis/adr/` 记录。

**Why:** 统一 reader、IPv4-only 边界和路由回滚已经成为运行时架构事实，需要同步文档，避免下次实现重新引入重复 owner。

**Steps:**

1. 更新架构文档的数据流、TUN owner、IPv4 TCP/UDP 出口和 IPv6 非目标。
2. 将本实施计划登记到 Aegis 索引；规格已登记。
3. 执行 `cd src-tauri && cargo fmt --check`、`cargo test`、`cargo clippy`。
4. 执行 `npm run build`。
5. 执行 `npx tauri build --no-bundle`，确认产物为 `src-tauri/target/release/dns-proxy.exe`。
6. 管理员 Windows 环境手测：outbound 关闭路由不变；开启后 IPv4 TCP/UDP 直连和 SOCKS5 均可用；代理失败不直连；停止后 DNS/路由恢复；IPv6 显示未代理。

## Risks

- WinTun session 的读取线程和 `ipstack` 异步设备关闭语义必须在停止时验证；不能用第二个 reader 兜底。
- Windows 路由在多网卡、VPN 或已有默认路由时可能存在差异；失败必须回滚并保留诊断日志。
- SOCKS5 UDP ASSOCIATE 要求控制 TCP stream 保活；每个 UDP session 必须持有控制 stream 直到结束。
- 目标 IP 没有 DNS 映射时只能使用默认分组，不应猜测域名。
- IPv6 流量不能被误报为已代理；不能用 IPv4 fallback 掩盖范围限制。

## Retirement

- 旧 owner：`DnsInterceptor`，当前 active reader。
- 新 owner：`TrafficProxy`/`ipstack` 统一 dispatcher。
- Retirement decision：`delete-first`，因为这是内部重复 owner，不是外部兼容边界。
- Retirement trigger：`rg "DnsInterceptor|dns_intercept" src-tauri/src` 无运行时引用，且 DNS/TCP/UDP 全部由统一 reader 测试和手测覆盖。
- Residual debt：IPv6、ICMP 和更复杂的系统路由场景留在明确非目标，不得通过隐式 fallback 扩大本次范围。

## ADR/基线同步信号

实施完成后应记录统一 TUN dispatcher、项目内路由回滚、IPv4-only 和 outbound 双开关语义；这些决策触及 owner、运行时边界、配置契约和兼容性，不能只留在代码注释中。
