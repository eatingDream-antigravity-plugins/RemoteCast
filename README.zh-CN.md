<div align="center">

<img src="assets/logo.svg" alt="Remote Cast" width="80" height="80">

# Remote Cast (`chrome-stream-mcp`)

**在 Antigravity 内直接查看和操作本机 Google Chrome —— 即使手机远程或弱网环境下，画面依然清晰跟手。**

[主页](https://remotecast.eatingdream.dev) · [English](README.md) | 简体中文 · [Releases](https://github.com/eatingDream-antigravity-plugins/remote-cast/releases) · [AI 使用指南](docs/ai-guide.md) · [`llms.txt`](llms.txt)

[![CI](https://github.com/eatingDream-antigravity-plugins/remote-cast/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream-antigravity-plugins/remote-cast/actions/workflows/ci.yml)
[![Release](assets/release-badge.svg)](https://github.com/eatingDream-antigravity-plugins/remote-cast/releases)
![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-555)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/remotecast-connected-dark.png">
  <source media="(prefers-color-scheme: light)" srcset="docs/images/remotecast-connected-light.png">
  <img src="docs/images/remotecast-connected-dark.png" alt="Remote Cast 实时浏览器面板预览" width="620">
</picture>

</div>

## 适合你吗？

- **适合使用 Remote Cast**：你需要直接在 Antigravity 里查看、点击、输入和切换自己本机 Chrome 的标签页（保留已有登录态和扩展），或在远程弱网下操控隔离的本机 Chrome。
- **不需要安装**：如果你只是想让 AI 自己去浏览网页或跑自动化操作，直接用 Antigravity 内置的 `browser` 子代理即可，无需接入个人 Chrome。

## 为什么选择 Remote Cast

- **日常 Chrome 与一键隔离 Chrome 双模式**：可直接连上你正在用的日常 Chrome（保留账号登录和插件），也可以一键启动独立隔离的本机 Chrome，无需在本机电脑上点 **Allow**。
- **休眠保活，远程不丢授权**：点击顶栏 **休眠** 可立刻降到 0 FPS 省电，同时保持已授权连接存活；会撤销授权的 **彻底断开** 收纳在 `⋯` 菜单中，避免远程误触。
- **弱网流畅、停手即高清**：滚动和打字时自动平滑降档不闪烁，停手后立刻补发高清 WebP 静帧，页面静止时零流量开销。
- **不错点、不强弹空白页**：每次点击和按键都锁定在当前所见标签页，后台切页不会误触；关闭最后一个标签页后展示干净空态，不再强行弹出 `about:blank`。
- **原生 Rust 极速轻量**：单个本地二进制加零依赖单文件面板（`< 45 KB`），暗色优先无白屏闪烁，支持中英双语即时切换。

## 快速开始

### 1. 一键安装（macOS 与 Linux）

```sh
curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/remote-cast/main/scripts/install.sh | sh
```

<sub>自动从 GitHub Releases 下载预编译二进制、校验 SHA-256 摘要，并安装到 <code>~/.gemini/config/plugins/chrome-stream-mcp</code>。随时可执行 <code>curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/remote-cast/main/scripts/install.sh | sh -s -- --uninstall</code> 卸载。</sub>

安装完成后重载 Antigravity 即可生效。

### 2. 连接 Chrome

- **连接日常 Chrome**：先在 Chrome 打开 `chrome://inspect/#remote-debugging` 开启远程调试，在 Antigravity 中打开 **Remote Cast**，点击 **连接日常 Chrome** 并在 Chrome 弹窗确认一次 **Allow**。暂停时点 **休眠** 即可保活连接。
- **独立远程 Chrome（免 Allow）**：点击 **启动独立远程 Chrome**，将在 `127.0.0.1` 启动权限为 `0700` 的隔离配置目录。若 `9222` 端口已被占用，可通过 `CHROME_STREAM_DEBUG_PORT` 指定空闲端口或改用日常 Chrome。

## AI Agent 专属指引

面向 Agent 的工具参数表、环境变量、CLI 诊断命令和 stdio 配置文档：

```sh
npx skills add eatingDream-antigravity-plugins/remote-cast
```

- **通用 Skill**：[`skills/chrome-stream/SKILL.md`](skills/chrome-stream/SKILL.md)
- **完整 AI 与配置指南**：[`docs/ai-guide.md`](docs/ai-guide.md)
- **LLM 索引入口**：[`llms.txt`](llms.txt)

> 提示词示例：*「阅读 `chrome-stream-mcp` 的 `llms.txt` 和 `docs/ai-guide.md`，帮我运行 `chrome-stream-mcp doctor --json`，或以休眠/独立远程模式连接 Chrome。」*

## 安全与隐私边界

- **严格环回（`127.0.0.1` / `::1`）**：拒绝所有非本机地址，绝不将 Chrome 调试端口暴露到局域网或公网。
- **同用户本地访问控制**：使用 `0600` 权限的 Unix 套接字并校验对端用户 ID；在使用套接字和 `0700` 隔离配置目录前检查目录所有者与权限。
- **显式连接与 12 秒空闲停流**：打开面板不会自动连接 Chrome；12 秒无操作自动暂停抓屏护显卡，下次点击或按键瞬间唤醒。
- **诊断信息默认脱敏**：`chrome-stream-mcp doctor --json` 默认隐藏本机文件路径和原始调试 WebSocket 地址。

## 相关文档

- [AI 指南与配置手册](docs/ai-guide.md) — MCP 工具完整规格、环境变量、CLI 子命令、源码构建
- [Agent Skill](skills/chrome-stream/SKILL.md) — 按需加载的 Agent 操作规范
- [LLM 索引](llms.txt) · [协作规范](AGENTS.md) · [贡献指南](CONTRIBUTING.md) · [更新日志](CHANGELOG.md) · [安全策略](SECURITY.md)

## Antigravity 插件套件

隶属于 [eatingDream Antigravity 插件套件](https://eatingdream.dev)：[Context Radar](https://github.com/eatingDream-antigravity-plugins/context-radar)（`context-radar-mcp`）· [Clam Guard](https://github.com/eatingDream-antigravity-plugins/clam-guard)（`mac-clamshell-mcp`）· **Remote Cast**（`chrome-stream-mcp`）· [Colab Deck](https://github.com/eatingDream-antigravity-plugins/colab-deck)（`colab-mcp`），由 [Panel Kit](https://github.com/eatingDream-antigravity-plugins/panel-kit)（`agy-mcp-kit`）统一驱动。

## 许可证

基于 [MIT License](LICENSE) 开源。
