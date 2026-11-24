<div align="center">

<img src="assets/logo.svg" alt="Chrome Stream MCP" width="80" height="80">

# Chrome Stream MCP

**在 Antigravity 里直接查看和操作你本机的 Google Chrome，即使手机远程或弱网环境下也能顺畅使用。**

[![CI](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/actions/workflows/ci.yml)
[![Release](assets/release-badge.svg)](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-555)

[English](README.md) | 简体中文

<img src="docs/social-preview.png" alt="Chrome Stream MCP 面板预览" width="860">

</div>

## 核心特性

- **在 Antigravity 内直接操作本机 Chrome**：复用已登录账号与扩展，支持点击、输入、滚轮、地址栏跳转及标签页管理。
- **弱网自适应与后台轻刷**：按 RTT 在四档画质间自动切换，画面未变时零字节跳过，后台低频同步且再次展开面板时秒级刷新。
- **贴合正常浏览器习惯**：关闭最后一个标签页后展示干净空态，不会强行弹回 `about:blank`。

## 安装与准备

### 1. 安装插件（macOS 与 Linux）

```sh
curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/chrome-stream-mcp/main/scripts/install.sh | sh
```

自动下载并校验 SHA-256 预编译程序（macOS Apple Silicon/Intel、Linux x86_64）至 `~/.gemini/config/plugins/chrome-stream-mcp`；如需指定版本可前置 `CHROME_STREAM_VERSION=v0.3.0`，装完后重载 Antigravity。

### 2. 开启 Chrome 远程调试（仅需一次）

1. 在运行中的 Google Chrome 打开 `chrome://inspect/#remote-debugging` 并开启远程调试（**Chrome 136+** 推荐；或终端单独启动 `--remote-debugging-port=9222 --user-data-dir=/tmp/chrome-debug`）。
2. 在 Antigravity 打开 **Chrome Stream**，点击 **连接 Chrome**，并在 Chrome 弹窗中点一次 **允许**。

如无法连接，可运行 `chrome-stream-mcp doctor` 排查。

## 使用方式

可直接使用面板顶栏的 **连接**、**暂停 / 继续**、**断开** 按钮，或在对话中通过按需加载的 Skill（平时零上下文开销）让 Agent 调用：

| 工具 | 作用 |
| :--- | :--- |
| `chrome_stream_stage` | 打开面板并查看状态，不会自动连接 Chrome。 |
| `chrome_stream_connect` | 连接到本机 Chrome 并开始投屏当前标签页。 |
| `chrome_stream_disconnect` | 停止投屏并断开 Chrome 调试连接。 |

**可选环境变量**：默认自动扫描 macOS/Linux 常见 Chrome/Chromium 配置目录并回退至本机 `9222` 端口；支持 `CHROME_STREAM_DEBUG_PORT`、`CHROME_STREAM_PORT_FILE`、`CHROME_STREAM_USER_DATA_DIR` 与 `CHROME_STREAM_LANG`（`en` / `zh-CN`）。

## 架构与安全边界

- **仅限本机环回（`127.0.0.1` / `::1`）**：拒绝任何非环回地址；请勿将 Chrome 调试端口暴露至公网。
- **单次授权 `0600` 本地 Socket 中继**：通过 `~/.gemini/antigravity/plugin_data/chrome-stream-mcp/` 下仅当前用户可读写的 `0600` Unix Socket 保活会话，插件重载不反复弹窗。
- **显式触发与 12 秒无操作停流护显卡**：打开面板绝不自动拨号；12 秒无交互自动暂停抓屏让 GPU 休息，下次交互立即唤醒。
- **信息脱敏与零外连**：向 Agent 返回前自动剔除本地路径与内部 WebSocket Token，插件本身不发起任何对外网络请求。

## 从源码构建

需要 Rust（`cargo`）和 Node.js 20+（仅用于面板打包和测试，零 npm 依赖）：

```sh
git clone https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp.git && cd chrome-stream-mcp
node panel/build.mjs && node --test panel/net-policy.test.mjs
cargo test --locked && cargo build --release --locked
```

## 参与贡献与许可证

详见 [CONTRIBUTING.md](CONTRIBUTING.md)、[CHANGELOG.md](CHANGELOG.md) 和 [SECURITY.md](SECURITY.md)。基于 [MIT License](LICENSE) 开源。
