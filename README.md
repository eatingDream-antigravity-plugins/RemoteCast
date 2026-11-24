<div align="center">

<img src="assets/logo.svg" alt="Chrome Stream MCP" width="80" height="80">

# Chrome Stream MCP

**View and interact with your own local Google Chrome tabs right inside Antigravity, even over a slow phone or remote connection.**

[![CI](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/actions/workflows/ci.yml)
[![Release](https://img.shields.io/static/v1?label=release&message=v0.2.0&color=D97757)](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/releases)<!-- x-release-please-version -->
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-555)

English | [简体中文](README.zh-CN.md)

<img src="docs/social-preview.png" alt="Chrome Stream MCP panel preview" width="860">

</div>

## Why this exists

Sometimes you just want to check a page, click a button, or finish a login inside your real local Google Chrome (where your accounts and extensions already live) without leaving Antigravity. That's especially handy when you're controlling your workstation remotely from a phone over mobile data.

Chrome Stream MCP streams your local Chrome tabs into an interactive Antigravity panel and adjusts image quality automatically so it stays usable even on slow connections.

## What it does

- **Interactive local Chrome in Antigravity**: Click, type, scroll, navigate, and switch, open, or close tabs in your real local Chrome window directly from the panel.
- **Click "Allow" once per browser session**: Keeps a background local relay connected so Chrome doesn't keep popping up the debugging approval prompt whenever the plugin reloads.
- **You control when it connects**: Opening the panel never connects to Chrome behind your back. Use **Connect**, **Pause / Resume**, and **Disconnect** right in the header, and screen capture automatically rests when idle to save battery.
- **Built for slow connections**: Automatically adapts image quality across four presets based on your connection latency, and skips sending frames when nothing on the page changed.
- **Normal tab behavior**: Closing the last tab shows a clean empty state instead of forcing an unwanted `about:blank` tab back open.

## Install & Setup

### 1. Install the plugin (macOS & Linux)

```sh
curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/chrome-stream-mcp/main/scripts/install.sh | sh
```

This downloads the prebuilt binary for your platform (macOS Apple Silicon, macOS Intel, or Linux x86_64), verifies its SHA-256 hash, and installs it into `~/.gemini/config/plugins/chrome-stream-mcp`. Restart or reload Antigravity afterward.

To pin a specific release:

```sh
CHROME_STREAM_VERSION=v0.1.0 curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/chrome-stream-mcp/main/scripts/install.sh | sh
```

### 2. Turn on Chrome remote debugging (one-time)

1. Keep Google Chrome open on your computer.
2. Open `chrome://inspect/#remote-debugging` in Chrome and enable remote debugging (recommended on **Chrome 136+**, which blocks `--remote-debugging-port` on the default profile directory). If you prefer launching a separate debug instance from the terminal, pass `--remote-debugging-port=9222 --user-data-dir=/tmp/chrome-debug`.
3. Open **Chrome Stream** in Antigravity, click **Connect**, and click **Allow** once when Chrome asks.

If Chrome doesn't connect, run `chrome-stream-mcp doctor` in a terminal to see what's missing.

## How to use it

### In the panel

Click **Connect** to start viewing your active tab. Use the tab bar, address bar, and mouse/keyboard just like a normal browser. Click **Pause** whenever you want to freeze the stream without losing authorization, or **Disconnect** when you're done to close the Chrome connection.

### Ask your agent

Chrome Stream ships an on-demand skill (with zero always-on prompt overhead), so you can also control it from chat:

- *"Open Chrome Stream."*
- *"Connect Chrome Stream to my local Chrome."*
- *"Disconnect Chrome Stream when we're done."*

| Tool | What it does |
| :--- | :--- |
| `chrome_stream_stage` | Opens the Chrome Stream panel and shows connection status without auto-connecting. |
| `chrome_stream_connect` | Connects to your local Chrome session and starts streaming the active tab. |
| `chrome_stream_disconnect` | Stops streaming and closes the Chrome debugging connection. |

### Custom Chrome port, profile, or language (optional)

By default, Chrome Stream scans standard macOS and Linux Chrome/Chromium profile directories (checking that `DevToolsActivePort` is actually listening, and falling back to localhost port `9222`). If you run a custom setup, you can set `CHROME_STREAM_DEBUG_PORT`, `CHROME_STREAM_PORT_FILE`, `CHROME_STREAM_USER_DATA_DIR`, or `CHROME_STREAM_LANG` (`en` / `zh-CN`).

## How It Works & Safety Principles

Chrome Stream is designed to behave like a quiet, strictly local companion to your browser without surprising system changes or background resource drain:

- **Loopback-only (`127.0.0.1` / `::1`)**: Chrome Stream only connects to Chrome on your local loopback interface and refuses any remote or non-loopback address. Never expose Chrome's remote debugging port to the public network.
- **Private local `0600` relay socket**: A small background relay keeps your authorized session over a user-only (`0600`) Unix domain socket in `~/.gemini/antigravity/plugin_data/chrome-stream-mcp/`, so Antigravity doesn't re-trigger Chrome's permission prompt on every restart.
- **Single-approval flow & explicit connect**: Opening the panel never dials Chrome behind your back. You click **Connect** once, approve **Allow** once in Chrome, and can **Pause** or **Disconnect** from the header at any time.
- **12-second idle GPU stop**: If you leave the panel open without interacting for 12 seconds, screen capture pauses automatically so Chrome's GPU rests, then wakes immediately on your next click, scroll, keystroke, or wake tap.
- **Privacy redaction & zero outbound traffic**: Local filesystem paths and internal WebSocket tokens are stripped before anything reaches the agent, and the plugin makes zero outbound network requests.

## Build from source

Requires Rust (`cargo`) and Node.js 20+ (only to build and test the panel; no npm dependencies).

```sh
git clone https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp.git
cd chrome-stream-mcp
node panel/build.mjs && node --test panel/net-policy.test.mjs
cargo test --locked
cargo build --release --locked
```

## Contributing & License

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md), [CHANGELOG.md](CHANGELOG.md), and [SECURITY.md](SECURITY.md).

Licensed under the [MIT License](LICENSE).
