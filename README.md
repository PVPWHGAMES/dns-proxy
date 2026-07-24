# DNS Proxy - Windows 全局 DNS 代理软件

一个类似 YogaDNS / SmartDNS 的 Windows 全局 DNS 代理软件，支持国内外域名分流、GeoSite 路由、广告拦截，使用 Tauri + React + Rust 构建。

## ✨ 特性

- 🌐 **全局 DNS 接管** - 通过 TUN 虚拟网卡或系统 DNS 设置一键接管
- 🔀 **国内外域名分流** - DNS 服务器分组（直连/代理），国内域名走国内 DNS，国外域名走代理
- 🗺️ **GeoSite 域名路由** - 内置预设订阅（国内直连域名、代理域名、广告拦截）
- 🚫 **广告拦截** - 黑名单订阅系统，支持 hosts / AdGuard / 纯域名格式
- ⚡ **高性能** - Rust 异步处理（Tokio），毫秒级响应，>1000 QPS
- 📡 **多协议** - 支持 UDP / DoH（DNS over HTTPS）转发
- 🎨 **美观界面** - 现代化 React UI，支持深色/浅色主题
- 📊 **实时日志** - DNS 请求实时监控，按分组/状态/类型筛选，支持导出 CSV
- 💾 **轻量级** - Tauri 打包，体积小，资源占用低

## 📸 功能概览

| 页面 | 功能 |
|------|------|
| 仪表盘 | 服务启停、统计卡片、最近查询（含分组标记）、上游服务器状态 |
| DNS 设置 | 监听配置、缓存、DNS 策略（顺序/最快/负载均衡/并行）、上游服务器管理、延迟测试 |
| 网络设置 | TUN 虚拟网卡配置、自动路由、系统 DNS 重定向 |
| 规则管理 | 黑名单订阅、域名路由规则（GeoSite）、自定义规则（精确/通配符/正则） |
| 日志查看 | 实时日志流、搜索过滤、分组标记、导出 CSV |

## 🛠️ 技术栈

| 层级 | 技术 |
|------|------|
| 前端 | React 18 + TypeScript + Tailwind CSS |
| 后端 | Rust + Tokio + trust-dns |
| 框架 | Tauri v2 |
| 构建 | Vite + Cargo |
| TUN | WinTun |

## 📦 安装

### 下载安装包

从 [Releases](https://github.com/PVPWHGAMES/dns-proxy/releases) 页面下载：

- `DNS Proxy_1.0.3_x64_en-US.msi` - MSI 安装包
- `DNS Proxy_1.0.3_x64-setup.exe` - NSIS 安装包

> ⚠️ TUN 模式需要**管理员权限**运行

### 从源码构建

**前置要求**：
- Node.js >= 18
- Rust >= 1.70
- Visual Studio Build Tools (Windows)

```bash
git clone https://github.com/PVPWHGAMES/dns-proxy.git
cd dns-proxy
npm install
npm run tauri dev    # 开发模式
npm run tauri build  # 生产构建
```

## 🚀 快速开始

### 方案一：DNSProxy TUN + Clash 系统代理（推荐）

推荐的架构：DNSProxy 负责分流决策，Clash 负责代理转发。

```
应用发起 DNS 请求
  → DNSProxy TUN 拦截 (:53)
  → 匹配规则：
    ├─ 国内域名 → 阿里 DNS 直接解析 → 返回真实 IP → 直连
    └─ 国外域名 → 转发到 Clash (:7897) → Clash 解析并走代理
```

**步骤**：

1. **启动 Clash Verge**，确保开启**系统代理**模式（不要开 TUN）
2. **启动 DNSProxy**（管理员权限）
3. 进入 **DNS 设置** 页面：
   - 确认 `domestic`（直连）分组有国内 DNS（如阿里 DNS 223.5.5.5）
   - 确认 `proxy`（代理）分组有 Clash DNS（127.0.0.1:7897）
4. 进入 **网络设置** 页面：
   - 开启 TUN 模式
   - 开启自动路由
5. 进入 **规则管理** 页面，添加 GeoSite 预设：
   - CN 国内直连域名 → 直连
   - Proxy 需代理域名 → 代理
6. 点击「启动服务」

### 方案二：仅 DNSProxy（无代理）

不需要代理软件，仅做 DNS 分流和广告拦截。

**步骤**：

1. **启动 DNSProxy**（管理员权限）
2. 在 **DNS 设置** 配置上游 DNS 服务器
3. 在 **规则管理** 添加广告拦截订阅
4. 开启 TUN 或手动设置系统 DNS 为 `127.0.0.1`
5. 点击「启动服务」

## 📖 使用说明

### DNS 服务器分组

支持将上游 DNS 服务器分为不同组，配合规则实现域名分流：

| 分组 | 用途 | 示例 |
|------|------|------|
| `domestic`（直连） | 国内域名，直接解析 | 阿里 DNS (223.5.5.5)、114DNS (114.114.114.114)、腾讯 DNS (119.29.29.29) |
| `proxy`（代理） | 国外域名，通过代理软件解析 | Clash DNS (127.0.0.1:7897) |
| `default`（默认组） | 兜底服务器 | 未匹配规则时使用 |

### GeoSite 域名路由

内置预设一键订阅（使用 ghfast.top 加速，国内可直接访问）：

| 预设 | 目标分组 | 说明 |
|------|----------|------|
| CN 国内直连域名 | 直连 | 国内常用域名，走国内 DNS 直接解析 |
| Apple 中国域名 | 直连 | Apple 在中国的服务 |
| Google 中国域名 | 直连 | Google 在中国的服务 |
| Proxy 需代理域名 | 代理 | 需要代理访问的域名 |
| 广告拦截域名 | 阻止 | 广告/追踪域名，返回空地址 |

来源：[Loyalsoldier/v2ray-rules-dat](https://github.com/Loyalsoldier/v2ray-rules-dat)

### 规则优先级

```
① 自定义规则（最高优先级，覆盖一切订阅）
② 黑名单订阅（广告拦截）
③ 缓存
④ GeoSite 域名路由（国内外分流）
⑤ 默认策略转发
```

### DNS 选择策略

| 策略 | 说明 |
|------|------|
| 按顺序 | 使用第一个可用的 DNS 服务器 |
| 最快响应 | 并发查询，选择响应最快的 |
| 负载均衡 | 轮询分配请求到多个服务器 |
| 并行请求 | 同时请求所有服务器，使用最快响应 |

### 配置文件位置

配置文件：`%APPDATA%\dns-proxy\config.toml`

启动服务后自动生成，也可在界面中修改。

## ⚠️ 常见问题

### TUN 模式下 nslookup 返回 fake-ip

如果同时开启了 Clash TUN 和 DNSProxy TUN，两个 TUN 接口会冲突。确保只开一个 TUN：
- **推荐**：DNSProxy TUN + Clash 系统代理
- **不推荐**：两个 TUN 同时开启

### 端口 53 被占用

Windows 上可能有其他服务占用 53 端口（如 dnsmasq、DNS Client 服务）。DNSProxy 会自动检测并尝试重启。

### 修改 DNS 设置后不生效

DNS 设置修改后会自动重启服务使配置生效，无需手动操作。

### GeoSite 规则不生效

确保已点击「更新」按钮下载规则数据，并确认目标分组（直连/代理）与 DNS 服务器分组一致。

## 📁 项目结构

```
dns-proxy/
├── src-tauri/              # Rust 后端
│   ├── src/
│   │   ├── main.rs         # 入口
│   │   ├── lib.rs          # Tauri 命令、系统托盘、自动启动
│   │   ├── config.rs       # 配置管理（TOML）、配置迁移
│   │   ├── dns/
│   │   │   ├── server.rs   # DNS 服务器（UDP 监听）
│   │   │   ├── handler.rs  # 查询处理（规则、分组转发、缓存）
│   │   │   └── cache.rs    # DNS 缓存（TTL + LRU）
│   │   └── tun/
│   │       ├── device.rs   # WinTun 虚拟网卡
│   │       └── dns_intercept.rs  # TUN DNS 拦截
│   └── Cargo.toml
├── src/                    # React 前端
│   ├── pages/
│   │   ├── Dashboard.tsx   # 仪表盘
│   │   ├── Settings.tsx    # DNS 设置
│   │   ├── NetworkSettings.tsx  # TUN 配置
│   │   ├── Rules.tsx       # 规则管理
│   │   └── Logs.tsx        # 日志查看
│   ├── components/         # UI 组件
│   └── lib/api.ts          # Tauri IPC 接口
├── docs/architecture.md    # 架构设计
└── package.json
```

## 🔧 开发

```bash
npm install              # 安装依赖
npm run tauri dev        # 开发模式（前后端热重载）
npm run tauri build      # 生产构建
cd src-tauri && cargo test  # Rust 测试
```

## 📄 许可证

MIT License

## 🙏 致谢

- [Tauri](https://tauri.app/) - 桌面应用框架
- [trust-dns](https://github.com/bluejekyll/trust-dns) - DNS 库
- [Loyalsoldier/v2ray-rules-dat](https://github.com/Loyalsoldier/v2ray-rules-dat) - GeoSite 域名列表
- [WinTun](https://www.wintun.net/) - TUN 驱动
