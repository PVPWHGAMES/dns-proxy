# DNS代理软件 - 架构设计文档

## 项目概述

开发一个类似YogaDNS的Windows全局DNS代理软件，具备以下核心功能：
- 接管系统全局DNS请求
- 支持自定义DNS服务器配置
- DNS请求日志和统计
- 规则引擎（按域名/IP分流）
- 美观的现代化GUI界面

## 技术架构

```
┌─────────────────────────────────────────────────────────┐
│                    前端层 (WebView)                       │
│  React + TypeScript + Tailwind CSS + shadcn/ui          │
├─────────────────────────────────────────────────────────┤
│                    Tauri Bridge                          │
│              (IPC 通信桥接层)                            │
├─────────────────────────────────────────────────────────┤
│                    后端层 (Rust)                          │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐       │
│  │ DNS Proxy   │ │ Rule Engine │ │ System Tray │       │
│  │ Server      │ │             │ │ Manager     │       │
│  └─────────────┘ └─────────────┘ └─────────────┘       │
│  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐       │
│  │ Config      │ │ Logger      │ │ Network     │       │
│  │ Manager     │ │             │ │ Adapter     │       │
│  └─────────────┘ └─────────────┘ └─────────────┘       │
├─────────────────────────────────────────────────────────┤
│                    系统层                                 │
│  Windows DNS Service / 网络适配器 / 系统防火墙           │
└─────────────────────────────────────────────────────────┘
```

## 核心模块设计

### 1. DNS代理服务器 (dns-proxy)

**职责**：监听本地DNS请求，转发到上游服务器

```rust
// 核心结构
struct DnsProxy {
    listen_addr: SocketAddr,
    upstream_servers: Vec<DnsServer>,
    rule_engine: RuleEngine,
    cache: DnsCache,
}
```

**功能**：
- UDP/TCP DNS监听（端口53）
- DNS请求解析和转发
- 响应缓存
- 并发处理（tokio异步运行时）

### 2. 规则引擎 (rule-engine)

**职责**：根据规则决定DNS请求的处理方式

```rust
enum RuleAction {
    Forward(DnsServer),    // 转发到指定DNS
    Block,                 // 阻止解析
    Cache,                 // 使用缓存
    Direct,                // 直连
}

struct Rule {
    pattern: DomainPattern,  // 域名匹配模式
    action: RuleAction,
    priority: u32,
}
```

**规则类型**：
- 域名精确匹配
- 域名通配符匹配
- 正则表达式匹配
- GeoIP分流（可选）

### 3. 配置管理 (config)

**职责**：管理所有配置信息

```rust
struct AppConfig {
    proxy: ProxyConfig,
    upstream_servers: Vec<DnsServer>,
    rules: Vec<Rule>,
    log_level: LogLevel,
    auto_start: bool,
}
```

**配置文件位置**：`%APPDATA%\dns-proxy\config.toml`

### 4. 日志系统 (logger)

**职责**：记录DNS请求日志和系统事件

```rust
struct DnsLogEntry {
    timestamp: DateTime<Utc>,
    domain: String,
    query_type: RecordType,
    response: Option<IpAddr>,
    upstream: DnsServer,
    latency_ms: u64,
    action: RuleAction,
}
```

### 5. 系统集成 (system)

**职责**：与Windows系统交互

- 修改系统DNS设置
- 系统托盘管理
- 开机自启动
- 防火墙规则

## 前端界面设计

### 页面结构

```
├── 仪表盘 (Dashboard)
│   ├── 实时请求日志
│   ├── 统计图表
│   └── 系统状态
│
├── DNS设置 (Settings)
│   ├── 上游DNS服务器配置
│   ├── 监听端口设置
│   └── 缓存配置
│
├── 规则管理 (Rules)
│   ├── 规则列表
│   ├── 添加/编辑规则
│   └── 规则导入导出
│
├── 日志查看 (Logs)
│   ├── 实时日志流
│   ├── 历史日志查询
│   └── 日志导出
│
└── 关于 (About)
    ├── 版本信息
    ├── 更新检查
    └── 帮助文档
```

### UI设计原则

- 使用shadcn/ui组件库，保证一致性
- 深色/浅色主题切换
- 响应式布局
- 流畅的动画效果

## 数据流

```
用户请求 → 系统DNS → [本地代理:53] → 规则引擎匹配
                                        ↓
                              ┌─────────┴─────────┐
                              ↓                   ↓
                         使用缓存              转发上游
                              ↓                   ↓
                         返回响应              等待响应
                              ↓                   ↓
                              └─────────┬─────────┘
                                        ↓
                                   记录日志
                                        ↓
                                   返回客户端
```

## 依赖库

### Rust后端
- `tokio` - 异步运行时
- `trust-dns` - DNS协议处理
- `serde` / `toml` - 配置序列化
- `tracing` - 日志
- `windows` - Windows API绑定

### React前端
- `react` / `react-dom` - UI框架
- `tailwindcss` - 样式
- `shadcn/ui` - 组件库
- `recharts` - 图表
- `@tauri-apps/api` - Tauri API

## 安全考虑

1. **权限管理**：需要管理员权限修改系统DNS
2. **输入验证**：严格验证域名和IP格式
3. **防DNS泄露**：确保所有请求都经过代理
4. **配置加密**：敏感配置可选择加密存储

## 性能目标

- DNS响应延迟：< 10ms（缓存命中）
- 并发处理：> 1000 QPS
- 内存占用：< 50MB
- 启动时间：< 2秒

## 后续扩展

- [ ] DoH/DoT支持
- [ ] DNS-over-HTTPS
- [ ] 广告过滤
- [ ] 多配置文件切换
- [ ] 远程管理API
- [ ] 插件系统
