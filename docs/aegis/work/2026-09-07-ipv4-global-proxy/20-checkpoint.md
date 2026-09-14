# Checkpoint: IPv4 Global Proxy Implementation

## Status: ✅ COMPLETED

所有任务已完成，功能已实现并验证通过。

## Completed Tasks

- ✅ Specs: `docs/aegis/specs/2026-09-07-ipv4-global-proxy-design.md`
- ✅ Plan: `docs/aegis/plans/2026-09-07-ipv4-global-proxy.md`
- ✅ Aegis INDEX updated

### ✅ Task 1: 协议依赖与配置契约
  - 已添加 `socks5-impl = "0.9.6"` 和 `udp-stream = "0.0.12"` 到 `Cargo.toml`
  - `OutboundConfig` 已存在于 `tun/mod.rs`，所有必需字段完整
  - 已添加 `OutboundConfig` TypeScript 接口到 `src/lib/api.ts`
  - 已修复 `NetworkSettings.tsx` 的 TunConfig 初始化
  - 已公开导出 `pub use tun::{OutboundConfig, TunConfig, TunStatus}` 在 `lib.rs`
  - 验证通过：cargo check, npm build, 兼容性测试

### ✅ Task 2: SOCKS5 辅助函数
  - 已实现 `socks5_tcp_connect()`: TCP CONNECT 握手，支持无认证和用户名密码认证
  - 已实现 `socks5_udp_associate()`: UDP ASSOCIATE 握手，返回 UDP relay 地址和控制连接
  - 添加 3 个单元测试（标记为 #[ignore]，需要真实 SOCKS5 服务器）
  - 验证通过：cargo check, cargo test --no-run（测试编译成功）

### ✅ Task 3: 统一流量调度器
  - 已修改 `TrafficProxy::start()` 接受 `OutboundConfig` 参数
  - 在 `ip_stack.accept()` 循环中统一处理 DNS (UDP:53)、TCP、UDP 三种流
  - DNS 流通过 `DnsHandler::handle_raw_query()` 处理
  - TCP/UDP 流根据 `domain_group` 判断是否走 SOCKS5
  - 实现 fail-closed：proxy 组连接 SOCKS5 失败时直接断开，不回退直连
  - 已删除 `dns_intercept.rs` 文件
  - 已从 `lib.rs` 移除 `DnsInterceptor` 初始化和导入
  - 已添加 `DnsHandler::handle_raw_query()` 方法供 TUN 使用
  - 验证通过：cargo check, cargo test --lib（所有测试通过）

### ✅ Task 4: UI 配置面板
  - 在 `NetworkSettings.tsx` 添加完整的 Outbound 配置区块
  - 支持启用/禁用全流量代理
  - 支持配置 SOCKS5 地址、端口
  - 支持配置用户名密码（可选认证）
  - 添加配置验证：
    - 地址不能为空
    - 端口范围 1-65535
    - 用户名和密码必须同时填写或同时留空
  - 添加清晰的说明文档和注意事项
  - 验证通过：npm run build（前端编译成功）

## Implementation Summary

### 核心功能
1. **统一流量调度器**：DNS/TCP/UDP 三种流量统一在 `TrafficProxy` 中处理
2. **SOCKS5 支持**：完整实现 TCP CONNECT 握手，支持无认证和用户名密码认证
3. **Fail-closed 策略**：proxy 组连接失败时直接断开，确保不泄漏流量
4. **配置界面**：直观的 UI 配置面板，支持所有必需配置项

### 架构变更
- **删除**：`dns_intercept.rs`（旧的独立 DNS 拦截器）
- **新增**：统一调度器处理所有流量类型
- **优化**：DNS 查询现在通过 `DnsHandler::handle_raw_query()` 统一处理

### 测试验证
- ✅ Rust 编译：`cargo check` 通过
- ✅ Rust 测试：`cargo test --lib` 通过
- ✅ 前端编译：`npm run build` 通过
- ✅ 配置兼容性：旧配置文件可正常加载（无 outbound 字段时使用默认值）

## Modified Files

### Backend (Rust)
- `src-tauri/Cargo.toml`: 添加 socks5-impl 和 udp-stream 依赖
- `src-tauri/src/lib.rs`: 公开导出 OutboundConfig，修改 TUN 启动逻辑，移除 DnsInterceptor
- `src-tauri/src/tun/mod.rs`: 移除 dns_intercept 模块声明
- `src-tauri/src/tun/traffic.rs`: 实现 SOCKS5 函数和统一调度器
- `src-tauri/src/dns/handler.rs`: 添加 handle_raw_query() 方法
- `src-tauri/tests/tun_config_compat.rs`: 配置兼容性测试
- **已删除**: `src-tauri/src/tun/dns_intercept.rs`

### Frontend (TypeScript/React)
- `src/lib/api.ts`: 添加 OutboundConfig 接口定义
- `src/pages/NetworkSettings.tsx`: 添加完整的 Outbound 配置 UI 面板

## Next Steps (Optional Future Enhancements)

Phase 2 优化（可选）：
1. UDP SOCKS5 支持（当前 UDP 仅支持直连和 fail-closed）
2. 连接统计和监控
3. 代理健康检查
4. 多代理支持和负载均衡

## Notes

- 当前实现的 UDP 流量处理：DNS (UDP:53) 走统一调度器，其他 UDP 流量如果是 proxy 组则 fail-closed
- SOCKS5 UDP ASSOCIATE 函数已实现但未集成到流量处理中（UDP 数据包封装较复杂）
- 所有配置变更需要重启 TUN 设备才能生效
