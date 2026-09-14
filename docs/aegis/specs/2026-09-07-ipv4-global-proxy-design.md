# IPv4 全局 TCP/UDP 代理设计规格

日期：`2026-09-07`
状态：已确认，待实施

## 1. 目标

在 Windows TUN 模式下接管 IPv4 TCP/UDP 流量，并按现有 DNS 查询产生的 IP/域名分组映射选择出口：

- `proxy` 分组通过 SOCKS5 代理；
- `domestic`、默认分组和未命中映射的目标直连；
- 代理建立失败时阻断代理流量，不回退直连。

## 2. 范围

### 2.1 本次包含

- IPv4 TCP 会话接收、直连转发和 SOCKS5 CONNECT 转发；
- IPv4 UDP 会话接收、直连转发和 SOCKS5 UDP ASSOCIATE 转发；
- IPv4 DNS UDP 查询在统一 TUN 数据面中交给现有 `DnsHandler`；
- 单一 WinTun session 读取循环，统一分发所有 IPv4 会话；
- IPv4 默认路由接管、代理/DNS 出口绕行和停止时路由回滚；
- `tun.outbound` 的 Rust/TypeScript 配置契约；
- 网络设置页的最小 SOCKS5 配置界面；
- IPv6 未代理状态的明确标记。

### 2.2 本次不包含

- IPv6 转发或 IPv6 默认路由；
- ICMP、IGMP 或其它非 TCP/UDP 协议；
- 按进程分流；
- HTTP、SOCKS4 或其它代理协议；
- 安装包生成。

## 3. 启用语义

- `tun.enabled` 控制 TUN/DNS 功能；
- `tun.outbound.enabled` 控制 IPv4 TCP/UDP 全局数据面；
- 只有 `tun.enabled && tun.outbound.enabled && tun.auto_route` 同时成立时，才接管 IPv4 默认路由；
- `outbound.enabled=false` 时不添加默认路由，避免非 DNS 流量进入无人处理的 TUN；
- 旧配置缺少 `outbound` 字段时按默认关闭处理。

## 4. 运行时 owner

- `TunDevice`：WinTun adapter/session、IPv4 地址、系统 DNS、路由状态和回滚；
- `TrafficProxy`：唯一的 WinTun session 读取循环、`ipstack` 会话分发、TCP/UDP 出口；
- `DnsHandler`：DNS 查询规则、上游转发、缓存和 IP 到域名/分组映射；
- `AppConfig`：TOML 配置源；
- `NetworkSettings`：配置编辑 UI。

现有 `DnsInterceptor` 与 `TrafficProxy` 同时读取同一个 WinTun session，属于内部重复 owner。统一 dispatcher 后退休 `DnsInterceptor` 主路径，不保留并行读取 fallback。

## 5. 数据流

```text
Windows IPv4 流量
        |
        v
WinTun session（唯一读取循环）
        |
        v
ipstack IPv4 TCP/UDP
   |          |             |
 DNS UDP   TCP 会话       UDP 会话
   |          |             |
DnsHandler  分组选择      分组选择
              |             |
       proxy -> SOCKS5   proxy -> UDP ASSOCIATE
       其它 -> 直连       其它 -> 直连
```

DNS 处理不再由独立读取器完成；DNS 响应必须通过对应 `IpStackUdpStream` 写回原始客户端。

## 6. TCP 行为

1. 根据目标 IPv4 查询 `DnsHandler::lookup_ip`；
2. 有效映射且分组为 `proxy` 时，连接配置的 SOCKS5 地址并发送 `CONNECT`；
3. 代理目标优先使用已映射域名，未映射时使用 IPv4 地址；
4. 其它目标创建绕过 TUN 的直连 socket；
5. 两端双向复制直到任一端关闭；
6. SOCKS5 连接、认证或 CONNECT 失败时结束当前会话，不尝试直连。

## 7. UDP 行为

1. DNS 目标端口 `53` 交给 `DnsHandler`，每个查询返回到对应 TUN UDP stream；
2. `proxy` 分组建立 SOCKS5 UDP ASSOCIATE，并添加/解析 RFC 1928 UDP header；
3. 其它分组使用物理网卡出口的 IPv4 UDP socket；
4. 使用 `ipstack` UDP stream 的会话超时释放闲置会话；
5. SOCKS5 UDP 建立、认证或包解析失败时结束当前会话，不尝试直连；
6. 只接受 IPv4 目标，IPv6 不进入本数据面。

## 8. 路由与回滚

### 启动

- 启动前识别可用物理 IPv4 默认路由及接口；
- 为 SOCKS5 服务器和必要 DNS 上游添加更具体的物理出口路由，防止代理回环；
- 仅在全局数据面启用时添加 TUN IPv4 默认路由；
- 任一步失败时撤销已添加项，并恢复已修改的 DNS；
- 记录本次实际添加的路由，不能凭模式猜测删除目标。

### 停止

1. 停止 dispatcher 和代理会话；
2. 删除本次添加的路由；
3. 恢复原系统 DNS 配置；
4. 释放 WinTun session 和 adapter。

不得删除启动前已经存在的系统路由。

## 9. 配置契约

`OutboundConfig` 字段：

- `enabled: bool`，默认 `false`；
- `protocol: String`，当前只允许 `socks5`；
- `address: String`，默认 `127.0.0.1`；
- `port: u16`，默认 `7890`；
- `username: Option<String>`；
- `password: Option<String>`。

启用时校验协议和端口；认证字段只在配置存在时发送。前端与 Rust 字段保持 snake_case 对齐。

## 10. 错误和安全边界

- `proxy` 流量不允许在代理失败时回退直连；
- 代理密码不得写入日志；
- 路由和 DNS 修改失败必须可观察并触发回滚；
- IPv6 状态必须显示为“未代理”，不能报告为全局代理已覆盖；
- 旧 TOML 缺少新增字段时必须成功加载。

## 11. 验收标准

### 自动验证

- `cargo fmt --check`；
- `cargo test`；
- `cargo clippy`；
- `npm run build`；
- `npx tauri build --no-bundle`。

### Rust 单元验证

- 分组选择与默认组行为；
- 代理失败不回退直连；
- SOCKS5 IPv4 地址和认证编码；
- SOCKS5 UDP header 编码/解析；
- 旧配置反序列化；
- 路由记录只包含本次添加项。

### Windows 管理员手动验证

- outbound 关闭时默认路由不变；
- outbound 开启时 IPv4 默认路由指向 TUN；
- TCP/UDP 直连和 SOCKS5 代理均可收发；
- SOCKS5 不可用时 proxy 流量不发生直连；
- 停止 TUN 后路由和 DNS 恢复；
- IPv6 明确显示为未代理。

## 12. 架构决策信号

本规格触及运行时 owner、TUN 路由、配置契约和兼容边界。实施完成后应补充 ADR 或基线同步记录，确认统一 dispatcher、路由回滚和 IPv4-only 边界成为项目当前架构事实。
