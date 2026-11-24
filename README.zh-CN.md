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

## 为什么做这个插件

干活的时候经常需要看一眼、点一下自己本机已经登录好账号和插件的 Chrome 页面，尤其是出门用手机远程连回家里或公司电脑时，来回切窗口非常麻烦。

Chrome Stream MCP 把你本机的 Chrome 标签页实时投到 Antigravity 面板里，点击、滚动、输入和切标签都能直接操作，还会按网络延迟自动调画质，弱网下也能用得住。

## 它能帮你做什么

- **在 Antigravity 里直接操作本机 Chrome**：支持点击、输入文字、滚轮、地址栏跳转，以及新建、切换和关闭标签页。
- **每次开浏览器只需点一次"允许"**：后台会保活本地连接，插件重载时不用反复弹 Chrome 调试授权框。
- **连接、暂停、断开全由你控制**：打开面板不会偷偷自动连 Chrome；顶栏常驻 **连接**、**暂停 / 继续** 和 **断开** 按钮，长时间不操作也会自动停止抓屏省电。
- **弱网也能流畅看**：根据当前延迟在四档画质间自动切换，页面没变化时直接跳过传图，省流量也不卡顿。
- **后台轻刷与开窗秒更**：隐藏在后台时以低频延迟自适应节奏同步状态与标签变化（内容未变零传输，且绝不打扰手动 **暂停**）；再次展开常驻侧边或全局面板时立即唤醒并刷新最新画面。
- **贴合正常浏览器习惯**：关掉最后一个标签页后只显示干净的空页面，不会强行给你弹回一个 `about:blank`。

## 安装与准备

### 1. 安装插件（macOS 与 Linux）

```sh
curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/chrome-stream-mcp/main/scripts/install.sh | sh
```

脚本会自动下载对应平台的预编译程序（支持 macOS Apple Silicon、macOS Intel 和 Linux x86_64），校验 SHA-256 后安装到 `~/.gemini/config/plugins/chrome-stream-mcp`。装完后重启或重载 Antigravity 即可。

如需指定版本：

```sh
CHROME_STREAM_VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/chrome-stream-mcp/main/scripts/install.sh | sh
```

### 2. 开启 Chrome 远程调试（只需设置一次）

1. 保持电脑上的 Google Chrome 处于打开状态。
2. 在 Chrome 地址栏打开 `chrome://inspect/#remote-debugging` 并开启远程调试（**Chrome 136+** 默认禁止在主配置目录直接使用 `--remote-debugging-port`，推荐用此方式）。如果你更习惯在终端单独启动调试实例，可加上 `--remote-debugging-port=9222 --user-data-dir=/tmp/chrome-debug`。
3. 在 Antigravity 里打开 **Chrome Stream**，点击 **连接 Chrome**，在 Chrome 弹出的提示框里点一次 **允许** 即可。

如果连不上，在终端里跑一下 `chrome-stream-mcp doctor` 就能看到具体缺哪一步。

## 怎么用

### 在面板里直接操作

点 **连接** 后即可看到当前标签页画面，像普通浏览器一样用标签栏、地址栏和键鼠操作。临时不想刷新画面可以点 **暂停**（不断开授权），用完了点 **断开** 关闭调试连接即可。

### 在对话里让 Agent 帮忙

插件自带按需加载的 Skill（平时不占上下文），也可以直接发指令：

- *"打开 Chrome Stream 面板。"*
- *"帮我连上本机 Chrome。"*
- *"用完了，断开 Chrome Stream 吧。"*

| 工具 | 作用 |
| :--- | :--- |
| `chrome_stream_stage` | 打开 Chrome Stream 面板并查看当前状态，不会自动发起连接。 |
| `chrome_stream_connect` | 连接到本机 Chrome 并开始投屏当前标签页。 |
| `chrome_stream_disconnect` | 停止投屏并断开 Chrome 调试连接。 |

### 自定义端口、配置目录或语言（可选）

默认会自动扫描 macOS 与 Linux 常见 Chrome / Chromium 配置目录并验证 `DevToolsActivePort` 端口是否真实存活，失效时自动回退到本地 `9222` 端口。如有特殊路径或语言偏好，可设置 `CHROME_STREAM_DEBUG_PORT`、`CHROME_STREAM_PORT_FILE`、`CHROME_STREAM_USER_DATA_DIR` 或 `CHROME_STREAM_LANG`（`en` / `zh-CN`）。

## 实现原理与安全边界

Chrome Stream 只做本地轻量中继，不修改系统配置，也不会在后台偷偷连网或占用显卡资源：

- **仅限本机环回（`127.0.0.1` / `::1`）**：只允许连接本机环回地址上的 Chrome，直接拒绝任何远程或非本机地址；也请不要把 Chrome 调试端口暴露到公网。
- **用户专属 `0600` 本地 Socket 中继**：通过 `~/.gemini/antigravity/plugin_data/chrome-stream-mcp/` 下权限为 `0600`（仅当前系统用户可读写）的本地 Unix Socket 保持已授权会话，避免每次重启服务都反复弹窗。
- **单次授权、显式触发**：打开面板本身绝不会自动拨号连接 Chrome；只有你手动点击 **连接 Chrome**（或调用连接工具）并在 Chrome 中点一次 **允许** 才会建立连接，随时可点 **暂停** 或 **断开**。
- **12 秒无操作自动停流护显卡**：即使面板一直开着，只要 12 秒内没有键鼠或触控交互，后台会自动暂停抓屏让 Chrome GPU 休息；下次点击、滚动、输入或点唤醒条时立即恢复。
- **信息脱敏与零外连**：返回给 Agent 的结果会自动剔除本地文件路径和内部调试 Token，插件自身不发起任何对外网络请求。

## 从源码构建

需要 Rust（`cargo`）和 Node.js 20+（仅用于打包和测试前端面板，无需安装 npm 依赖）。

```sh
git clone https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp.git
cd chrome-stream-mcp
node panel/build.mjs && node --test panel/net-policy.test.mjs
cargo test --locked
cargo build --release --locked
```

## 参与贡献与许可证

欢迎提 PR 或反馈问题，详情见 [CONTRIBUTING.md](CONTRIBUTING.md)、[CHANGELOG.md](CHANGELOG.md) 和 [SECURITY.md](SECURITY.md)。

本项目基于 [MIT License](LICENSE) 开源。
