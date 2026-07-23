# DNS Proxy - Windows全局DNS代理软件

一个类似YogaDNS的现代化Windows全局DNS代理软件，使用Tauri + React + Rust构建。

## ✨ 特性

- 🌐 **全局DNS接管** - 一键接管系统DNS请求
- ⚡ **高性能** - Rust异步处理，毫秒级响应
- 🎨 **美观界面** - 现代化React UI，支持深色/浅色主题
- 📝 **规则引擎** - 灵活的域名分流规则
- 📊 **实时日志** - DNS请求实时监控和统计
- 🔧 **易于配置** - 图形化配置界面
- 💾 **轻量级** - 打包体积小，资源占用低

## 📸 界面预览

*（待开发）*

## 🛠️ 技术栈

| 层级 | 技术 |
|------|------|
| 前端 | React 18 + TypeScript + Tailwind CSS + shadcn/ui |
| 后端 | Rust + Tokio + trust-dns |
| 框架 | Tauri v2 |
| 构建 | Vite + Cargo |

## 📦 安装

### 从源码构建

**前置要求**：
- Node.js >= 18
- Rust >= 1.70
- Visual Studio Build Tools (Windows)

```bash
# 克隆项目
git clone <repository-url>
cd dns-proxy

# 安装前端依赖
npm install

# 开发模式运行
npm run tauri dev

# 构建生产版本
npm run tauri build
```

### 下载安装包

*（待发布）*

## 🚀 快速开始

1. 启动程序（需要管理员权限）
2. 在设置中配置上游DNS服务器
3. 点击"启动代理"按钮
4. 系统DNS将自动切换到本地代理

## 📖 使用说明

### 配置上游DNS

支持配置多个上游DNS服务器：
- Cloudflare: `1.1.1.1`, `1.0.0.1`
- Google: `8.8.8.8`, `8.8.4.4`
- 阿里DNS: `223.5.5.5`, `223.6.6.6`
- 自定义DNS服务器

### 规则配置

支持以下规则类型：
- **精确匹配**：`example.com`
- **通配符**：`*.example.com`
- **正则表达式**：`/.*\.example\.com$/`
- **域名列表**：批量导入域名列表

### 日志查看

- 实时显示DNS请求日志
- 支持按域名、类型、时间筛选
- 支持导出日志

## 📁 项目结构

```
dns-proxy/
├── src-tauri/           # Rust后端代码
│   ├── src/
│   │   ├── main.rs      # 入口文件
│   │   ├── dns/         # DNS代理模块
│   │   ├── rules/       # 规则引擎
│   │   ├── config/      # 配置管理
│   │   └── logger/      # 日志系统
│   └── Cargo.toml
├── src/                 # React前端代码
│   ├── components/      # UI组件
│   ├── pages/           # 页面
│   ├── hooks/           # 自定义Hook
│   └── lib/             # 工具函数
├── docs/                # 文档
└── package.json
```

## 🔧 开发

### 开发模式

```bash
# 启动前端开发服务器
npm run dev

# 启动Tauri开发模式（包含前后端）
npm run tauri dev
```

### 代码规范

- Rust: 使用 `cargo fmt` 和 `cargo clippy`
- TypeScript: 使用 ESLint + Prettier
- 提交信息: 使用约定式提交格式

### 测试

```bash
# Rust测试
cd src-tauri && cargo test

# 前端测试
npm test
```

## 📋 TODO

- [ ] 基础DNS代理功能
- [ ] 系统DNS设置修改
- [ ] 上游DNS配置
- [ ] 规则引擎实现
- [ ] 日志系统
- [ ] UI界面开发
- [ ] 系统托盘
- [ ] 开机自启动
- [ ] DoH/DoT支持
- [ ] 广告过滤
- [ ] 多配置文件

## 🤝 贡献

欢迎提交Issue和Pull Request！

## 📄 许可证

MIT License

## 🙏 致谢

- [Tauri](https://tauri.app/) - 桌面应用框架
- [shadcn/ui](https://ui.shadcn.com/) - UI组件库
- [trust-dns](https://github.com/bluejekyll/trust-dns) - DNS库
