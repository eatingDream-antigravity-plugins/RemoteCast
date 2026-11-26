<div align="center">

<img src="assets/logo.svg" alt="Chrome Stream MCP" width="80" height="80">

# Chrome Stream MCP

**在 Antigravity 内实时查看和操作本机 Google Chrome —— 即使手机远程或弱网环境下，依然清晰跟手。**

[English](README.md) | 简体中文 · [Releases](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/releases) · [AI 使用指南](docs/ai-guide.md) · [`llms.txt`](llms.txt)

[![CI](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/actions/workflows/ci.yml)
[![Release](assets/release-badge.svg)](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/releases)
![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-555)

<img src="docs/social-preview.png" alt="Chrome Stream MCP 面板预览" width="860">

</div>

## 它是什么 / 不是什么

- **它是什么**：基于原生 Rust 本机中继与 SEP-1865 MCP App 的实时投屏控制器，让你在 Antigravity 中直接点击、输入、滚动和管理本机 Chrome 标签页。
- **它不是什么**：不是远程桌面、云端中转或无头爬虫 —— 所有调试连接严格限定在环回地址（`127.0.0.1` / `::1`），打开面板绝不自动拨号。

## 核心特性

- **日常 Chrome 与一键独立远程 Chrome 双模式**：可复用你现有的 Chrome（保留已登录账号与扩展），也可一键启动独立回环远程 Chrome（免本机点 **Allow**）。
- **休眠保活，远程不重弹 Allow**：顶栏主按钮为 **休眠 / 唤醒**（0 FPS 停流但保持已授权连接存活）；会撤销授权的 **彻底断开 · 撤销授权** 收纳于 `⋯` 菜单并附明确警告。
- **弱网渐进高清**：滑动/打字时守住文字清晰下限，停手约 380 ms 自动补发一张高清 WebP 静帧，静止期间 0 字节去重。
- **抗抖动自适应调度**：基于 EWMA RTT 平滑与非对称迟滞（连续 2 次慢才降档、连续 3 次快才升档），弱网不抖动、滚轮与点击更跟手。
- **标签纪元防误触与干净空态**：所有页面输入绑定明确标签纪元，后台切页绝不误点错标签；关闭最后一个标签页展示干净空态，不强弹 `about:blank`。
- **Editorial-Warm 极简面板**：暗色优先零闪屏、零外部依赖单文件面板（`< 43.5 KB`），严格 `default-src 'none'` CSP，支持中英双语即时切换。

## 快速开始

### 1. 一键安装（macOS 与 Linux）

```sh
curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/chrome-stream-mcp/main/scripts/install.sh | sh
```

安装完成后重载 Antigravity 即可生效。

### 2. 连接 Chrome

- **连接日常 Chrome**：在运行中的 Chrome 打开 `chrome://inspect/#remote-debugging` 开启远程调试，在 Antigravity 打开 **Chrome Stream** 点 **连接日常 Chrome** 并在 Chrome 中点一次 **Allow**。远程离开电脑前只需点 **休眠**，切勿点彻底断开，即可长期免重弹 Allow。
- **独立远程 Chrome（免 Allow）**：在离线页直接点 **启动独立远程 Chrome（免 Allow）**，自动以 `0700` 隔离配置目录仅监听 `127.0.0.1` 启动并接入。

## AI Agent 专属指引

完整的 MCP 工具参数表、`CHROME_STREAM_*` 环境变量、CLI 诊断命令与跨客户端 stdio 配置已整理至面向 Agent 的文档中：

- **通用 Skill（`agentskills.io`）**：[`skills/chrome-stream/SKILL.md`](skills/chrome-stream/SKILL.md)（或运行 `npx skills add eatingDream-antigravity-plugins/chrome-stream-mcp`）
- **完整 AI 与配置参考**：[`docs/ai-guide.md`](docs/ai-guide.md)
- **LLM 索引入口**：[`llms.txt`](llms.txt)

> 提示词示例：*「阅读 `chrome-stream-mcp` 的 `llms.txt` 和 `docs/ai-guide.md`，帮我配置 stdio MCP、运行 `chrome-stream-mcp doctor --json`，或以休眠/独立远程模式接入 Chrome。」*

## 安全与隐私边界

- **严格环回（`127.0.0.1` / `::1`）**：拒绝一切非本机地址，绝不将 Chrome 调试端口暴露至局域网或公网。
- **私有权限隔离（`0600` Socket / `0700` 独立配置）**：中继持有文件锁与 inode 校验，并发启动或旧进程退出绝不会误删活跃 Socket。
- **显式连接与 12 秒空闲停流**：打开面板不自动连接；12 秒无交互自动暂停抓屏护显卡，下次交互立即唤醒。
- **信息脱敏与零外连**：返回给 Agent 前自动剥离本机路径、主机名与 WebSocket Token；面板严格执行 `default-src 'none'`，零外网请求。

## 相关文档

- [AI 指南与配置手册](docs/ai-guide.md) — MCP 工具完整规格、环境变量、CLI 子命令、源码构建
- [Agent Skill](skills/chrome-stream/SKILL.md) — 按需加载的 Agent 操作规范
- [LLM 索引](llms.txt) · [协作规范](AGENTS.md) · [贡献指南](CONTRIBUTING.md) · [更新日志](CHANGELOG.md) · [安全策略](SECURITY.md)

## 许可证

基于 [MIT License](LICENSE) 开源。
