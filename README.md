<div align="center">

<img src="assets/logo.svg" alt="Remote Cast" width="80" height="80">

# Remote Cast (`chrome-stream-mcp`)

**Your Chrome, live inside Antigravity.**

Remote Cast streams a Chrome tab into the side panel so you can watch and steer what an agent is doing — click, type, switch tabs. Use your everyday Chrome or a separate clean profile; quality adapts to your network.

[Homepage](https://remotecast.eatingdream.dev) · **English** | [简体中文](README.zh-CN.md) · [Releases](https://github.com/eatingDream-antigravity-plugins/remote-cast/releases) · [AI Guide](docs/ai-guide.md) · [`llms.txt`](llms.txt)

[![CI](https://github.com/eatingDream-antigravity-plugins/remote-cast/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream-antigravity-plugins/remote-cast/actions/workflows/ci.yml)
[![Release](assets/release-badge.svg)](https://github.com/eatingDream-antigravity-plugins/remote-cast/releases)
![Built with Rust](https://img.shields.io/badge/built%20with-Rust-12151E?logo=rust&logoColor=white)
[![License: MIT](https://img.shields.io/badge/license-MIT-2952CC)](LICENSE)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-181C28)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/remotecast-connected-dark.png">
  <source media="(prefers-color-scheme: light)" srcset="docs/images/remotecast-connected-light.png">
  <img src="docs/images/remotecast-connected-dark.png" alt="Remote Cast live Chrome panel preview" width="620">
</picture>

</div>

## Is It For You?

- **Use Remote Cast** if you want to view, scroll, type, and switch tabs in your own Google Chrome (keeping existing logins and extensions) or a separate clean Chrome profile right inside Antigravity, even over a slow link.
- **Skip it** if you only want an AI agent to browse or scrape autonomously — Antigravity's built-in `browser` subagent already handles general browsing without touching your Chrome.

## Why Remote Cast

- **My Chrome or Separate Chrome**: Attach to your everyday Chrome with logins intact, or open a separate Chrome with a clean profile and no permission prompt.
- **Standby-first Disconnect in the header**: Click **Pause** or **Standby** (`mode: "standby"`) to stop streaming at 0 FPS while keeping Chrome's Allow permission warm for instant 1-click **Resume Stream**; click **Revoke** (`mode: "revoke"`) when you explicitly want to sever the debugging session.
- **Stays in the background on macOS**: Clicking, switching tabs, and opening new tabs (`chrome://newtab`) never steal window focus away from Antigravity.
- **Adapts to slow networks, sharpens when still**: Pick **Auto**, **Saver**, **Low**, **Balanced**, or **High**. While you scroll or type it stays responsive, then settles into a crisp still frame with zero wasted bytes when idle.
- **Settings right in the panel**: Click **Settings** (`⚙`) to change language, default quality, default connect mode, background refresh, and idle pause — backed by a hot-reloading `settings.json`.
- **Fast, lightweight native panel**: Single Rust binary with a zero-dependency panel (`<= 64 KiB`), dark-first zero-flicker loading, and instant EN / 中文 switching.

## Quick Start

### 1. Install (macOS & Linux)

```sh
curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/remote-cast/main/scripts/install.sh | sh
```

<sub>Verifies SHA-256 checksums, installs into <code>~/.gemini/config/plugins/chrome-stream-mcp</code>, and hot-reloads the MCP server automatically inside Antigravity (use <code>--reload</code> outside Antigravity or <code>--no-reload</code> to skip). Uninstall anytime with <code>--uninstall</code>.</sub>

### 2. Connect Chrome

- **My Chrome**: Open `chrome://inspect/#remote-debugging` in Chrome and turn on remote debugging, then click **Connect my Chrome** in Remote Cast and click **Allow** once in Chrome. Use **Standby** or **Pause** when stepping away.
- **Separate Chrome**: Click **Open separate Chrome** for a fresh, isolated local profile (`0700` permissions on `127.0.0.1`) with no permission prompt. It picks a free local port automatically so it runs cleanly alongside your everyday Chrome.

## For AI Agents

Machine-readable tool schemas, `settings.json` keys, CLI diagnostics, and stdio setup:

```sh
npx skills add eatingDream-antigravity-plugins/remote-cast
```

- **Agent Skill**: [`skills/chrome-stream/SKILL.md`](skills/chrome-stream/SKILL.md)
- **Full AI & Configuration Guide**: [`docs/ai-guide.md`](docs/ai-guide.md)
- **LLM Entry Index**: [`llms.txt`](llms.txt)

## Safety & Privacy

- **Loopback-only (`127.0.0.1` / `::1`)**: Rejects non-loopback endpoints and never exposes Chrome debugging ports to your LAN or the internet.
- **Same-user local socket**: Uses a `0600` Unix socket, checks peer user ID, and verifies ownership and permissions on sockets, `settings.json` (`0600`), and the `0700` separate profile.
- **Explicit connect & idle pause**: Opening the panel never auto-connects to Chrome; screen capture pauses automatically while idle (default 12 s) and resumes on your next click or key.
- **Sanitized diagnostics**: `chrome-stream-mcp doctor --json` omits local paths and raw debugging WebSocket URLs by default.

## Documentation

- [AI Guide & Configuration Reference](docs/ai-guide.md) — MCP tools, `settings.json`, environment variables, CLI subcommands, build from source
- [Agent Skill](skills/chrome-stream/SKILL.md) — On-demand agent workflow rules
- [LLM Index](llms.txt) · [Contributor Guide](AGENTS.md) · [Contributing](CONTRIBUTING.md) · [Changelog](CHANGELOG.md) · [Security](SECURITY.md)

## Antigravity Plugin Suite

Part of the [Antigravity Plugins](https://github.com/eatingDream-antigravity-plugins) suite: [Context Radar](https://github.com/eatingDream-antigravity-plugins/context-radar) · [Clam Guard](https://github.com/eatingDream-antigravity-plugins/clam-guard) · **Remote Cast** · [Colab Deck](https://github.com/eatingDream-antigravity-plugins/colab-deck) · [Panel Kit](https://github.com/eatingDream-antigravity-plugins/panel-kit)

## License

Licensed under the [MIT License](LICENSE).
