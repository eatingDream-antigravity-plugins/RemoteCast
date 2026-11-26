<div align="center">

<img src="assets/logo.svg" alt="Chrome Stream MCP" width="80" height="80">

# Chrome Stream MCP

**View and interact with your local Google Chrome tabs inside Antigravity — crisp and responsive even over slow mobile or remote links.**

English | [简体中文](README.zh-CN.md) · [Releases](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/releases) · [AI Guide](docs/ai-guide.md) · [`llms.txt`](llms.txt)

[![CI](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/actions/workflows/ci.yml)
[![Release](assets/release-badge.svg)](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/releases)
![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-555)

<img src="docs/social-preview.png" alt="Chrome Stream MCP panel preview" width="860">

</div>

## What it is / What it is not

- **What it is**: A native Rust loopback relay and SEP-1865 MCP App that streams your local Google Chrome tabs into Antigravity with interactive click, type, scroll, IME, and tab controls.
- **What it is not**: A remote desktop server, cloud proxy, or headless scraping engine — traffic stays strictly on localhost (`127.0.0.1` / `::1`) and never auto-dials Chrome on load.

## Features

- **Personal Chrome or one-click Remote Chrome**: Connect to your existing Chrome profile (logins and extensions intact), or launch an isolated loopback Remote Chrome profile that never needs local **Allow** confirmation.
- **Standby keeps Allow alive remotely**: Header **Standby / Wake** freezes capture at 0 FPS while keeping the authorized session warm across reloads; **Revoke & Disconnect** lives safely in the `⋯` overflow menu with a clear warning.
- **Weak-network progressive clarity**: Maintains legible motion floors during scrolling/typing, then automatically settles into a crisp WebP still frame (~380 ms after activity) with 0-byte unchanged deduplication.
- **Zero-jitter adaptive pacing**: EWMA RTT smoothing with asymmetric hysteresis prevents quality thrashing on mobile jitter while keeping wheel and click feedback responsive.
- **Epoch-safe input & clean tab semantics**: Binds clicks/keys to the exact tab epoch so background tab switches never misroute input, and closing the last tab shows a clean empty state instead of forcing `about:blank`.
- **Editorial-Warm MCP App UI**: Dark-first, zero-flicker, zero-dependency single-file panel (`< 43.5 KB`) with strict `default-src 'none'` CSP and full EN / 中文 support.

## Quick Start

### 1. Install (macOS & Linux)

```sh
curl -fsSL https://raw.githubusercontent.com/eatingDream-antigravity-plugins/chrome-stream-mcp/main/scripts/install.sh | sh
```

Reload Antigravity after installation.

### 2. Connect Chrome

- **Personal Chrome**: Enable remote debugging once at `chrome://inspect/#remote-debugging`, open **Chrome Stream**, click **Connect Personal Chrome**, and click **Allow** once in Chrome. Use **Standby** (not Revoke) when away from your desk so you never need to re-click **Allow**.
- **Remote Chrome (No Allow)**: Click **Launch Remote Chrome (No Allow)** in the offline card to start a dedicated loopback Chrome profile (`0700` isolated directory) immediately.

## For AI Agents

Point your coding agent directly to the machine-readable guides for complete MCP tool schemas, `CHROME_STREAM_*` environment variables, CLI diagnostics, and stdio setup:

- **Skill (`agentskills.io`)**: [`skills/chrome-stream/SKILL.md`](skills/chrome-stream/SKILL.md) (or `npx skills add eatingDream-antigravity-plugins/chrome-stream-mcp`)
- **Full AI & Configuration Guide**: [`docs/ai-guide.md`](docs/ai-guide.md)
- **LLM Entry Index**: [`llms.txt`](llms.txt)

> Prompt hint: *"Read `llms.txt` and `docs/ai-guide.md` in `chrome-stream-mcp` to configure stdio MCP, run `chrome-stream-mcp doctor --json`, or connect Chrome in standby/remote mode."*

## Safety & Privacy

- **Loopback-only (`127.0.0.1` / `::1`)**: Rejects non-loopback endpoints; never exposes Chrome debugging ports to a network interface.
- **User-only `0600` local relay & `0700` remote profile**: Ownership-locked Unix socket and private profile directory protect local sessions against concurrent starts or stale cleanup.
- **Explicit connect & 12 s idle stop**: Opening the stage never auto-dials Chrome; screen capture pauses automatically after 12 seconds of inactivity and wakes on interaction.
- **Redacted outputs & zero outbound traffic**: Strips local paths, hostnames, and internal WebSocket tokens before returning tool results; the panel enforces `default-src 'none'` with zero external requests.

## Documentation

- [AI Guide & Configuration Reference](docs/ai-guide.md) — MCP tools, environment variables, CLI subcommands, build from source
- [Agent Skill](skills/chrome-stream/SKILL.md) — On-demand agent workflow rules
- [LLM Index](llms.txt) · [Contributor Guide](AGENTS.md) · [Contributing](CONTRIBUTING.md) · [Changelog](CHANGELOG.md) · [Security](SECURITY.md)

## License

Licensed under the [MIT License](LICENSE).
