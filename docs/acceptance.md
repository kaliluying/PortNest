# PortNest 验收记录

日期：2026-10-08。验收仅使用本项目创建的测试服务，没有关闭用户已有服务。

## 环境

- macOS 27.0.1，Apple Silicon arm64；配置最低 macOS 14，未在 macOS 14 上执行。
- Node.js 24.21.0，pnpm 12.9.1，Rust 1.98.1。
- Tauri 2.12.1、React 19.3.0、TypeScript 7.0.2、Vite 8.3.4。

## 自动化证据

| 检查 | 结果 | 证据边界 |
| --- | --- | --- |
| Rust 核心测试 | 16 项通过 | 包含真实本地 IPv4/IPv6 多端口、项目归属、正常退出、僵尸进程、忽略 SIGTERM 后强制退出、监听范围变化拒绝；其他身份和令牌策略为模拟检查 |
| 核心 Clippy | 通过，`-D warnings` | Rust 静态检查 |
| 桌面适配测试 | 1 项通过 | 数字绑定地址、通配地址、IPv6 URL 及非法地址拒绝 |
| 桌面 Clippy / 格式 | 通过 | 不等于界面验收 |
| 前端测试 | 17 项通过 | 模拟 IPC 验证空态、错误与旧结果、确认/强制确认、设置错误、原生活动状态与隐藏停止、搜索计数 |
| TypeScript / Vite | 通过 | `pnpm check` 和构建 |
| release `.app` 构建 | 通过 | `pnpm package --ci`，包含最新原生刷新修正 |
| 签名完整性 | 通过 | `codesign --verify --deep --strict`，ad-hoc 签名 |

共 34 项自动化检查通过。下面单独记录原生 release 应用与真实服务的验收，避免把模拟 IPC 检查当作实机证据。

## 原生界面验收

交付目录中的 `outputs/PortNest.app` 已实际启动。测试服务在 `fixtures/portnest-acceptance` 中运行，使用随机端口，操作只针对下表中本次创建的 PID。

| 用户链路 | 实机结果 |
| --- | --- |
| 发现 → 搜索 → 展开详情 | PID 82355 归属正确；显示 64642 / 127.0.0.1、64643 / ::1，搜索计数为 1 个进程、2 个端口 |
| 正常关闭 | 确认框列出双端口和 PID；点击后成功。lsof 无监听、ps 无该 PID，fixture 退出码 143（SIGTERM） |
| 拒绝 SIGTERM | PID 83389，端口 49658 / 49659；等待 5 秒后显示仍在监听，lsof 证明双端口仍存在 |
| 过期强制确认 | 30 秒确认过期后显示明确错误，进程继续监听，没有发送强制信号 |
| 再次确认强制关闭 | 重新核验并正常退出超时，第二个确认框再次列出 PID 与两个端口；确认后成功。lsof 无监听、ps 无该 PID，fixture 退出码 137（SIGKILL） |
| 五秒刷新 | 新测试服务通过聚焦面板自动出现，无需手动刷新；观察到更新时间按 5 秒递增 |
| 搜索无结果 | 正常/强制关闭后显示 0 个进程、0 个端口及无匹配提示，与真实进程消失一致 |
| 独立设置窗口 | 面板隐藏，设置在带原生标题栏的独立窗口中显示，深色渲染清晰 |
| 登录启动注册 | 默认 off；打开后创建 `~/Library/LaunchAgents/PortNest.plist`，再关闭后移除；已恢复 off |

原生首次展示刷新使用 Tauri 焦点与可见状态；实际从菜单栏打开后已自动扫描。隐藏/失焦停止轮询、空列表、扫描失败保留旧结果、设置失败等细节有前端模拟 IPC 回归；尚未在原生系统中制造 lsof 故障，也没有通过系统跟踪工具测量隐藏期间的扫描调用次数。

证据截图位于 `outputs/evidence/`：`native-process.png`、`native-graceful-closed.png`、`native-still-listening.png`、`native-expired-confirmation.png`、`native-force-confirmation.png`、`native-force-closed.png`、`native-settings.png`。截图仅保留测试项目或本应用设置。

## 可重复的原生验收

先运行自己创建的 fixture，记录输出 PID 和两个随机端口：

```sh
cd fixtures/portnest-acceptance
node service.mjs
```

1. 打开 PortNest，搜索 `portnest-acceptance` 或输出 PID。
2. 展开详情，核对项目目录、UID、PID 和 IPv4/IPv6 端口。
3. 点击关闭，确认弹窗列出两个端口；正常关闭。
4. 使用 `/usr/sbin/lsof -nP -a -p <输出PID> -iTCP -sTCP:LISTEN` 检查无监听，fixture 进程退出。
5. 运行 `node service.mjs --ignore-term`，重复发现和正常关闭；等待最多 5 秒后应仍在监听。
6. 点击“确认强制关闭”，核对第二次确认，再执行；检查两个端口释放。
7. 使用搜索无结果、受保护进程详情、独立设置窗口、隐藏/重新打开、主题和刷新时间完成界面检查。

只操作 fixture 输出的 PID；不根据名称随意关闭其他 node/Python 服务。

## 分发限制

arm64 `.app` 构建和 `codesign --verify --deep --strict` 已通过，使用 ad-hoc 签名，没有 Apple 公证。未验证 Intel、Windows、Linux、其他 macOS 版本或实际下次登录启动。

交付：`outputs/PortNest.app` 和 `outputs/PortNest-0.1.0-macOS-arm64.zip`。

ZIP SHA-256：`89e19c91e7a0a1fe9f93469e1024eadab036f1feace5e650387b63a7ded0411c`。
