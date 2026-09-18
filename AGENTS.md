# Repository Guidelines

## 项目结构与模块组织

- `src/` 是 React 18 + TypeScript 前端：页面放在 `pages/`，可复用界面放在 `components/`，IPC 与通用函数放在 `lib/`，全局样式放在 `styles/`。
- `src-tauri/src/` 是 Rust/Tauri 后端；`dns/` 负责 DNS 服务、转发、缓存与上游连接池，`config.rs` 负责 TOML 配置，`lib.rs` 注册 Tauri 命令和应用状态。
- `public/` 保存前端静态资源，`src-tauri/icons/` 保存桌面应用图标，`docs/` 保存架构和开发记录。不要手动编辑 `dist/` 或 `src-tauri/target/` 生成物。

## 构建、测试与本地开发

先安装 Node.js 18+、Rust 1.70+ 和 Windows Visual Studio Build Tools，然后运行：

```bash
npm install                 # 安装前端依赖
npm run dev                 # 仅启动 Vite 前端（默认 9000 端口）
npm run build               # TypeScript 类型检查并构建前端
npm run tauri dev           # 启动 Tauri 桌面开发模式
npx tauri build --no-bundle # 构建可执行文件，不生成安装包
cd src-tauri && cargo check # 检查 Rust 编译
cd src-tauri && cargo test  # 运行 Rust 测试
```

## 编码风格与命名约定

TypeScript 使用 2 个空格、双引号和函数式 React 组件；组件文件与组件名使用 `PascalCase`（如 `StatCard.tsx`），函数和变量使用 `camelCase`。Rust 提交前运行 `cargo fmt`，并以 `snake_case` 命名模块、函数和变量；可用 `cargo clippy` 进行额外检查。沿用现有 Tailwind 类和导入别名 `@/*`。

## 测试指南

当前未配置前端测试运行器或 `npm test` 脚本；前端改动至少执行 `npm run build`。Rust 单元测试放在对应模块的 `#[cfg(test)]` 中，测试函数使用描述行为的 `snake_case` 名称，并通过 `cd src-tauri && cargo test` 运行。涉及 DNS 或系统设置的改动应在 Windows 管理员环境中手动验证。

## 提交与 Pull Request

提交信息遵循 Conventional Commits：`<type>(<scope>): <subject>`，例如 `fix(dns): 修复缓存过期处理`；常用类型为 `feat`、`fix`、`refactor`、`docs`、`test`、`chore`。PR 应说明问题与实现、列出验证命令、关联 issue；UI 改动附前后截图，系统权限或配置行为改动注明 Windows 版本和所需权限。

## 安全与配置

监听本地 53 端口通常需要管理员权限，并可能与其他 DNS 服务或防火墙规则冲突。用户配置位于 `%APPDATA%\dns-proxy\config.toml`；不要提交其中的敏感地址、令牌或本机网络信息，新增外部服务时使用环境变量或本地配置注入。
