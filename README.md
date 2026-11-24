<div align="center">

<img src="assets/logo.svg" alt="Chrome Stream MCP" width="80" height="80">

# Chrome Stream MCP

**View and interact with your own local Google Chrome tabs right inside Antigravity, even over a slow phone or remote connection.**

[![CI](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/actions/workflows/ci.yml)
[![Release](assets/release-badge.svg)](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-555)

English | [简体中文](README.zh-CN.md)

<img src="docs/social-preview.png" alt="Chrome Stream MCP panel preview" width="860">

</div>

## Highlights

- **Real local Chrome in Antigravity**: Click, type, scroll, navigate, and manage tabs in your existing Chrome profile (accounts and extensions intact).
- **Adaptive weak-network streaming**: Automatically scales quality across four RTT-based presets, skips unchanged frames, syncs quietly in the background, and wakes immediately when a kept-alive pane is reopened.
- **Clean tab semantics**: Closing the last tab shows a clean empty state instead of forcing an unwanted `about:blank` tab open.

## Install & Setup

### 1. Install the plugin (macOS & Linux)

```sh
curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/chrome-stream-mcp/main/scripts/install.sh | sh
```

Installs the SHA-256-verified prebuilt binary (macOS Apple Silicon/Intel, Linux x86_64) into `~/.gemini/config/plugins/chrome-stream-mcp`. Pin a version with `CHROME_STREAM_VERSION=v0.3.0`, then reload Antigravity.

### 2. Enable Chrome remote debugging (one-time)

1. Open `chrome://inspect/#remote-debugging` in your running Google Chrome and enable remote debugging (recommended on **Chrome 136+**; or launch a separate profile with `--remote-debugging-port=9222 --user-data-dir=/tmp/chrome-debug`).
2. Open **Chrome Stream** in Antigravity, click **Connect**, and click **Allow** once in Chrome.

Run `chrome-stream-mcp doctor` if Chrome doesn't connect.

## Usage

Use the panel header's **Connect**, **Pause / Resume**, and **Disconnect** controls directly, or ask your agent via the on-demand skill (zero always-on prompt overhead):

| Tool | What it does |
| :--- | :--- |
| `chrome_stream_stage` | Opens the panel and reports status without auto-connecting. |
| `chrome_stream_connect` | Connects to local Chrome and starts streaming the active tab. |
| `chrome_stream_disconnect` | Stops streaming and closes the Chrome debugging connection. |

**Optional overrides**: Auto-discovers standard macOS/Linux Chrome/Chromium profiles (falling back to localhost `9222`). Override via `CHROME_STREAM_DEBUG_PORT`, `CHROME_STREAM_PORT_FILE`, `CHROME_STREAM_USER_DATA_DIR`, or `CHROME_STREAM_LANG` (`en` / `zh-CN`).

## Architecture & Safety

- **Loopback-only (`127.0.0.1` / `::1`)**: Rejects non-loopback targets; never expose Chrome's debugging port publicly.
- **Single-approval `0600` local relay**: A user-only (`0600`) Unix socket under `~/.gemini/antigravity/plugin_data/chrome-stream-mcp/` keeps the authorized session alive across plugin reloads so you only click **Allow** once per browser session.
- **Explicit connect & 12s idle GPU stop**: Opening the panel never auto-dials Chrome; screen capture pauses automatically after 12 seconds of inactivity and wakes on your next interaction.
- **Privacy redaction & zero outbound traffic**: Strips local paths and internal WebSocket tokens before reaching the agent, with zero outbound network requests.

## Build from source

Requires Rust (`cargo`) and Node.js 20+ (panel build/tests only; zero npm dependencies):

```sh
git clone https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp.git && cd chrome-stream-mcp
node panel/build.mjs && node --test panel/net-policy.test.mjs
cargo test --locked && cargo build --release --locked
```

## Contributing & License

See [CONTRIBUTING.md](CONTRIBUTING.md), [CHANGELOG.md](CHANGELOG.md), and [SECURITY.md](SECURITY.md). Licensed under the [MIT License](LICENSE).
