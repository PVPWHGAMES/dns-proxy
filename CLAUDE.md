# DNS Proxy - Windows全局DNS代理软件

## 项目概述

开发一个类似YogaDNS的Windows全局DNS代理软件，使用Tauri + React + Rust技术栈。

## 技术栈

- **前端**: React 18 + TypeScript + Tailwind CSS + shadcn/ui
- **后端**: Rust + Tokio + trust-dns
- **框架**: Tauri v2
- **构建**: Vite + Cargo

## 项目结构

```
.
├── src-tauri/              # Rust后端
│   ├── src/
│   │   ├── main.rs         # 入口
│   │   ├── dns/            # DNS代理核心
│   │   ├── rules/          # 规则引擎
│   │   ├── config/         # 配置管理
│   │   └── logger/         # 日志系统
│   └── Cargo.toml
├── src/                    # React前端
│   ├── components/         # UI组件
│   ├── pages/              # 页面
│   ├── hooks/              # 自定义Hook
│   └── lib/                # 工具函数
├── docs/                   # 文档
│   └── architecture.md     # 架构设计
├── README.md
└── CLAUDE.md               # 本文件
```

## 开发规范

### 代码风格

**Rust**:
- 使用 `cargo fmt` 格式化
- 使用 `cargo clippy` 检查
- 遵循Rust命名规范（snake_case）

**TypeScript**:
- 使用ESLint + Prettier
- 组件使用PascalCase
- 函数使用camelCase

### 提交规范

使用约定式提交格式：
```
<type>(<scope>): <subject>

类型：
- feat: 新功能
- fix: 修复
- refactor: 重构
- docs: 文档
- style: 格式
- test: 测试
- chore: 构建/工具
```

### Git工作流

- `main`: 主分支，保持稳定
- `develop`: 开发分支
- `feature/*`: 功能分支
- `fix/*`: 修复分支

## 核心模块

### DNS代理服务器
- 监听本地53端口
- UDP/TCP双协议支持
- 异步并发处理（Tokio）
- DNS请求解析和转发

### 规则引擎
- 域名精确匹配
- 通配符匹配
- 正则表达式匹配
- 按规则转发/阻止/缓存

### 配置管理
- TOML配置文件
- 位置: `%APPDATA%\dns-proxy\config.toml`
- 支持热重载

### 日志系统
- 实时DNS请求日志
- 文件日志持久化
- 日志级别控制

## 前端页面

- **仪表盘**: 实时日志、统计图表、系统状态
- **DNS设置**: 上游服务器、监听端口、缓存配置
- **规则管理**: 规则列表、添加/编辑、导入导出
- **日志查看**: 实时日志流、历史查询、导出

## 开发命令

```bash
# 安装依赖
npm install

# 开发模式（前后端热重载）
npm run tauri dev

# 构建生产版本
npm run tauri build

# Rust测试
cd src-tauri && cargo test

# 前端测试
npm test

# 代码检查
npm run lint
```

## 工作流程约定

### 任务完成后的操作

- **自动编译**：任务完成后自动编译 `dns-proxy.exe`，包括前端资源打包
- **不生成安装包**：常规任务完成时只编译可执行文件，不生成 NSIS 安装包

### 源码更新

- **更新源码**：指将源码文件同步到 `D:\DNSProxy` 目录并提交推送到远程仓库
- 流程：复制文件 → `git add` → `git commit` → `git push`

### 版本号管理

- **自动生成安装包时**：自动将所有版本号显示位置的版本号递增（如 `1.0.6` → `1.0.7`）
- **版本号位置**：
  - 配置文件：`Cargo.toml`、`package.json`、`tauri.conf.json`
  - 前端页面：`src/components/Sidebar.tsx`（左上角）、`src/pages/About.tsx`（关于页面）
- **特殊说明优先**：如果用户明确指定了版本号，则使用用户指定的版本

## 环境要求

- Node.js >= 18
- Rust >= 1.70
- Visual Studio Build Tools (Windows)
- 管理员权限（修改系统DNS）

## 注意事项

1. **权限**: 修改系统DNS需要管理员权限
2. **端口53**: 可能被其他服务占用，需要检测
3. **防火墙**: 需要添加防火墙规则
4. **DNS泄露**: 确保所有请求都经过代理
5. **性能**: 目标 > 1000 QPS，响应 < 10ms

## 参考资源

- [Tauri文档](https://tauri.app/v2/guides/)
- [trust-dns文档](https://docs.rs/trust-dns/)
- [shadcn/ui](https://ui.shadcn.com/)
- [DNS协议RFC1035](https://tools.ietf.org/html/rfc1035)
