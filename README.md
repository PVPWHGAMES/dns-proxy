# DNS Proxy - Windows 全局 DNS 代理软件

一个类似 YogaDNS / SmartDNS 的 Windows 全局 DNS 代理软件，支持国内外域名分流、GeoSite 路由、广告拦截，使用 Tauri + React + Rust 构建。

## ✨ 特性

- 🌐 **全局 DNS 接管** - 通过 TUN 虚拟网卡或系统 DNS 设置一键接管
- 🔀 **国内外域名分流** - DNS 服务器分组，国内域名走国内 DNS，国外域名走代理
- 🗺️ **GeoSite 域名路由** - 内置预设订阅（11 万+ 国内域名、2.7 万+ 代理域名）
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

- `DNS-Proxy-1.0.2-x64.msi` - MSI 安装包
- `DNS-Proxy-1.0.2-Setup.exe` - NSIS 安装包
- `dns-proxy.exe` - 独立可执行文件

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

1. 启动程序（TUN 模式需管理员权限）
2. 在**设置页面**配置上游 DNS 服务器（可一键添加国内/代理预设）
3. 在**规则页面**添加 GeoSite 域名路由（国内域名 → domestic，国外域名 → proxy）
4. 点击「启动服务」，系统 DNS 自动切换到本地代理

## 📖 使用说明

### DNS 服务器分组

支持将上游 DNS 服务器分为不同组，配合规则实现域名分流：

| 分组 | 用途 | 示例 |
|------|------|------|
| `domestic` | 国内域名 | 阿里 DNS (223.5.5.5)、114DNS (114.114.114.114) |
| `proxy` | 国外域名（走代理） | Clash DNS (127.0.0.1:1053) |
| `default` | 默认组 | 兜底服务器 |

### GeoSite 域名路由

内置预设一键订阅（使用 ghfast.top 加速，国内可直接访问）：

| 预设 | 规模 | 目标分组 |
|------|------|----------|
| CN 国内直连域名 | 11.2 万条 | domestic |
| Proxy 需代理域名 | 2.7 万条 | proxy |
| 广告拦截域名 | 16.8 万条 | blocklist |

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

### 与 ClashVerge 配合

推荐配置方式：

1. ClashVerge 开启 TUN 模式，DNS 监听在 `127.0.0.1:1053`
2. dns-proxy 的 `proxy` 分组配置 Clash DNS (`127.0.0.1:1053`)
3. 国外域名解析返回 Clash fake-ip，由 Clash TUN 拦截并走代理

## 📁 项目结构

```
dns-proxy/
├── src-tauri/              # Rust 后端
│   ├── src/
│   │   ├── main.rs         # 入口
│   │   ├── lib.rs          # Tauri 命令、系统托盘、自动启动
│   │   ├── config.rs       # 配置管理（TOML）
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
