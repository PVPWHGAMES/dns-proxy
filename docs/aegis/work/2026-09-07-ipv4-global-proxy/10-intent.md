# 任务意图与基线

## TaskIntentDraft

- 目标：实现 IPv4 TCP/UDP TUN 全局接管，按 DNS 分组选择 SOCKS5 或直连出口。
- 成功证据：统一 WinTun reader；IPv4 TCP/UDP 可转发；代理失败不回退直连；路由/DNS 可回滚；最小配置 UI 可编译。
- 非目标：IPv6、ICMP、按进程分流、其它代理协议、安装包。
- 停止条件：按实施计划完成并通过 Rust/前端/生产构建及管理员手测；或遇到未解决的系统网络阻塞则标记 blocked。

## BaselineReadSetHint

- `CLAUDE.md`
- `AGENTS.md`
- `docs/architecture.md`
- `docs/aegis/BASELINE-GOVERNANCE.md`
- `docs/aegis/baseline/2026-09-03-initial-baseline.md`
- `docs/aegis/specs/2026-09-07-ipv4-global-proxy-design.md`
- `docs/aegis/plans/2026-09-07-ipv4-global-proxy.md`
- `src-tauri/src/config.rs`
- `src-tauri/src/tun/{mod,device,traffic,dns_intercept}.rs`
- `src-tauri/src/dns/{handler,server}.rs`
- `src-tauri/src/lib.rs`
- `src/lib/api.ts`
- `src/pages/NetworkSettings.tsx`

## BaselineUsageDraft

- Required baseline refs: 上述项目规则、架构文档、Aegis 基线、已批准规格和实施计划。
- Delivered context refs: TUN、DNS handler、Tauri lifecycle、配置和网络设置源码已读取。
- Acknowledged before plan refs: `CLAUDE.md`、`AGENTS.md`、架构文档、Aegis 治理/初始基线。
- Cited in plan refs: 所有 required baseline refs。
- Missing refs: 无既有 IPv4 全局代理测试/路由快照基线。
- Decision: continue。

## ImpactStatementDraft

- Owners: `TunDevice` 管系统网络，`TrafficProxy` 管唯一 TUN reader 和 IPv4 会话，`DnsHandler` 管 DNS 规则/映射，`AppConfig` 管持久化，`NetworkSettings` 管 UI。
- Invariants: 一个 WinTun session 只能一个读取循环；proxy 失败不直连；只删除本次路由；IPv6 不报告为已代理。
- Retirement: `DnsInterceptor` 主路径退休，采用 delete-first。
- Compatibility: outbound 缺省关闭；关闭时不加默认路由；DNS/规则行为不变。

## Execution Readiness View

- Intent Lock: IPv4 TCP/UDP 全局接管，按 DNS 分组分流，fail-closed。
- Scope Fence: 不做 IPv6、ICMP、其它协议和按进程分流。
- Baseline Lock: 以已批准规格和实施计划为准。
- Owner/Contract Constraints: 单 reader、snake_case 配置、TunDevice 管路由/DNS。
- Retirement Boundary: 不保留 DnsInterceptor 运行时 fallback。
- Review Gates: 每批 focused compile/test；最终全量验证和管理员手测。
- Drift/Rewind: 依赖/API或路由假设不成立则暂停并回计划，不新增隐式 fallback。
