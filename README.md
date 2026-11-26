<div align="center">

<img src="assets/logo.svg" alt="RemoteCast" width="80" height="80">

# RemoteCast (chrome-stream-mcp)

**View and interact with your local Google Chrome tabs inside Antigravity — crisp and responsive even over slow mobile or remote links.**

[Homepage](https://remotecast.eatingdream.dev) · English | [简体中文](README.zh-CN.md) · [Releases](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/releases) · [AI Guide](docs/ai-guide.md) · [`llms.txt`](llms.txt)

[![CI](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/actions/workflows/ci.yml)
[![Release](assets/release-badge.svg)](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/releases)
![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-555)

<img src="docs/social-preview.png" alt="RemoteCast panel preview" width="860">

</div>

## Overview

RemoteCast (`chrome-stream-mcp`) is a native Rust relay and SEP-1865 MCP App. It streams local Chrome tabs into Antigravity and supports interactive input and tab controls. Chrome debugging traffic stays on loopback (`127.0.0.1` and `::1`).

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

- **Personal Chrome** uses Chrome's `DevToolsActivePort` files exclusively. Enable remote debugging at `chrome://inspect/#remote-debugging`, open **RemoteCast**, select **Connect Personal Chrome**, and confirm **Allow** in Chrome. Use **Standby** to keep an authorized connection open.
- **Remote Chrome (No Allow)** launches an isolated profile with `0700` permissions on `127.0.0.1`. It reuses a running browser only when its live debugging endpoint matches the isolated profile. If the port is occupied, set `CHROME_STREAM_DEBUG_PORT` to a free port or use Personal mode.

## For AI Agents

Point your coding agent directly to the machine-readable guides for complete MCP tool schemas, `CHROME_STREAM_*` environment variables, CLI diagnostics, and stdio setup:

- **Skill (`agentskills.io`)**: [`skills/chrome-stream/SKILL.md`](skills/chrome-stream/SKILL.md) (or `npx skills add eatingDream-antigravity-plugins/chrome-stream-mcp`)
- **Full AI & Configuration Guide**: [`docs/ai-guide.md`](docs/ai-guide.md)
- **LLM Entry Index**: [`llms.txt`](llms.txt)

> Prompt hint: *"Read `llms.txt` and `docs/ai-guide.md` in `chrome-stream-mcp` to configure stdio MCP, run `chrome-stream-mcp doctor --json`, or connect Chrome in standby/remote mode."*

## Safety & Privacy

- **Loopback-only (`127.0.0.1` / `::1`)**: Rejects non-loopback endpoints; never exposes Chrome debugging ports to a network interface.
- **Local access controls** use a `0600` Unix socket, verify the peer user ID, and validate directory ownership and write permissions before using the socket or the `0700` Chrome profile.
- **Explicit connect & 12 s idle stop**: Opening the stage never auto-dials Chrome; screen capture pauses automatically after 12 seconds of inactivity and wakes on interaction.
- **Sanitized diagnostics**. `chrome-stream-mcp doctor --json` omits local filesystem paths and raw DevTools WebSocket URLs by default.
- **MCP Apps capability**. The `resources/read` response includes live bootstrap tabs and pixels only after the host negotiates the MCP App MIME type. Other hosts receive static panel HTML.

## Documentation

- [AI Guide & Configuration Reference](docs/ai-guide.md) — MCP tools, environment variables, CLI subcommands, build from source
- [Agent Skill](skills/chrome-stream/SKILL.md) — On-demand agent workflow rules
- [LLM Index](llms.txt) · [Contributor Guide](AGENTS.md) · [Contributing](CONTRIBUTING.md) · [Changelog](CHANGELOG.md) · [Security](SECURITY.md)

## Antigravity Plugin Suite

Part of the [eatingDream Antigravity Plugin Suite](https://eatingdream.dev): [ContextRadar](https://github.com/eatingDream-antigravity-plugins/context-radar-mcp) (`context-radar-mcp`) · [ClamGuard](https://github.com/eatingDream-antigravity-plugins/mac-clamshell-mcp) (`mac-clamshell-mcp`) · **RemoteCast** (`chrome-stream-mcp`) · [ColabDeck](https://github.com/eatingDream-antigravity-plugins/colab-mcp) (`colab-mcp`) — powered by [PanelKit](https://github.com/eatingDream-antigravity-plugins/agy-mcp-kit) (`agy-mcp-kit`).

## License

Licensed under the [MIT License](LICENSE).
