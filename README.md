# PortNest

一个 macOS 菜单栏端口管理工具。用项目目录辨认开发服务，查看同一进程的全部 TCP 监听端口，并在确认影响范围后关闭进程。

技术栈：Tauri 2、React、TypeScript、Rust。独立实现，产品灵感来自 [LeftOpen](https://github.com/SonghaiFan/leftopen)。首版没有 CLI。

## 使用

打开 `PortNest.app`，点击菜单栏的 PortNest 图标展开面板。输入端口号、进程名、PID 或项目名搜索；展开一行可以查看目录、启动时间、可执行文件和 IPv4/IPv6 绑定地址。

关闭前会展示该 PID 的全部监听端口，重新核验身份和监听范围。先发送 SIGTERM，最多等待 5 秒；仍在监听时，只有再次确认才发送 SIGKILL。只操作选定进程，不递归关闭子进程。

系统、其他用户、App 包内及 launchd 托管服务受保护。身份读取失败时禁止关闭；未知项目的当前用户进程需要先展开详情。扫描失败保留旧结果及更新时间。

面板打开时刷新，聚焦期间每 5 秒刷新，隐藏后停止。设置在独立窗口中，中文界面，外观跟随系统，登录启动默认关闭。登录启动开关表示注册状态，实际下次登录行为需单独验收。

只读取系统与本地文件信息，不主动探测 HTTP 服务、不收集遥测、不联网获取图标或检查更新。点击端口的“打开”才会调用默认浏览器；TCP 端口不一定提供 HTTP。

## 开发

要求 macOS 14 或更高、Xcode Command Line Tools、Rust、Node.js 和 pnpm。已验证工具链版本记录在 [验收记录](docs/acceptance.md)。

```sh
pnpm install --frozen-lockfile
pnpm desktop
```

```sh
pnpm check
pnpm test
pnpm test:core
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
pnpm package --ci
```

构建产物在 `src-tauri/target/release/bundle/macos/PortNest.app`。本地产物使用 ad-hoc 签名，没有 Apple 公证，不是通用发行安装包。

## 目录

- `src/`：端口面板、设置、错误反馈及 Tauri IPC。
- `src-tauri/`：菜单栏、窗口、浏览器打开和登录启动。
- `crates/portnest-core/`：TCP 扫描、项目归属、安全关闭和回归检查。
- `fixtures/portnest-acceptance/`：可重复的真实双端口测试服务。
- `docs/`、`GLOSSARY.md`：范围、领域名词、设计决策和验收记录。

## 边界

首版仅支持 macOS TCP；UDP、Windows、Linux、批量关闭、递归关闭、项目启动、域名和 HTTPS 代理未实现。项目归属由最近项目标记推断，不保证识别所有运行方式。“可能局域网”只表示绑定范围，未验证网络可达性。

macOS 的按 PID 发信号接口存在核验与发送之间极短的竞态窗口；PortNest 在发送前立即复核微秒启动时间、UID、执行路径、工作目录和全部监听范围，变化时拒绝操作。
