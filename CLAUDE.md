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

# 构建生产版本（含安装包）
npm run tauri build

# 仅编译 exe（不含安装包）← 日常开发使用这个
npx tauri build --no-bundle

# Rust 单独编译检查（不用于最终产物，缺少嵌入式前端）
cd src-tauri && cargo check

# Rust测试
cd src-tauri && cargo test

# 前端测试
npm test

# 代码检查
npm run lint
```

## 工作流程约定

### 编译可执行文件（重要）

- **正确命令**：`npx tauri build --no-bundle`
- ⚠️ **禁止使用 `cargo build --release`**：该命令编译出的 exe 含 `cfg=dev` 标记，会尝试连接 `localhost:9000` 开发服务器而非使用内嵌前端资源，导致启动后 WebView 显示「连接已中断」
- 原因：Tauri v2 需要 CLI 工具（`npx tauri`）设置正确的生产环境变量，`cargo build` 独用会默认走开发模式
- 编译前自动执行 `npm run build`（`tsc && vite build`）编译前端资源
- 产物路径：`src-tauri/target/release/dns-proxy.exe`

### 任务完成后的操作

- **自动编译**：任务完成后自动执行 `npx tauri build --no-bundle` 编译 `dns-proxy.exe`
- **不生成安装包**：常规任务完成时只编译可执行文件，不生成 NSIS/MSI 安装包

### 安装包生成

- **只生成 NSIS（exe 安装包），抛弃 MSI**：WiX 用代码页 1252 无法编码中文产品名，MSI 打包会报 `LGHT0311` 失败
- **productName 用英文 `DNS Proxy`**：安装信息（安装包名、控制面板显示名、安装目录）保持英文；界面显示名（Sidebar/About/窗口标题）仍用中文「果冻网络加速」，两者独立互不影响
- 生成命令：`npx tauri build`（`tauri.conf.json` 的 `bundle.targets` 已固定为 `["nsis"]`）
- 产物路径：`src-tauri/target/release/bundle/nsis/DNS Proxy_<版本>_x64-setup.exe`
- **安装到 Program Files**：`tauri.conf.json` 的 `bundle.windows.nsis.installMode` 设为 `"perMachine"`（安装目录默认 `C:\Program Files\DNS Proxy`，需要管理员安装）
- **双击运行弹 UAC 提权**：`build.rs` 通过 `tauri_build::WindowsAttributes::app_manifest(include_str!("app-manifest.xml"))` 设置 `requestedExecutionLevel="requireAdministrator"`。因为 TUN 模式要改系统 DNS、创建虚拟网卡，必须管理员权限，双击运行即弹 UAC 兜底，无需手动右键管理员运行
- **自启动用计划任务（不是注册表 Run 键）**：`requireAdministrator` 下，写 `HKCU\...\Run` 键的程序在登录时会被 Windows 静默跳过（无法静默提权）。所以 `lib.rs` 的 `is_autostart_enabled`/`set_autostart` 改用 `schtasks` 计划任务：`/SC ONLOGON /RL HIGHEST`，登录时以最高权限静默启动，不弹 UAC。任务名固定 `DNS Proxy`

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
