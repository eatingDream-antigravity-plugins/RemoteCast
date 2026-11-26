---
name: chrome-stream
description: View, connect, pause, or disconnect a live local Google Chrome screencast session using Chrome Stream MCP (`chrome_stream_stage`, `chrome_stream_connect`, `chrome_stream_disconnect`), diagnose Chrome remote debugging setup, and handle zero-tab or low-bandwidth states safely.
---

# Chrome Stream MCP

Use Chrome Stream MCP when the user explicitly wants to view or interact with tabs in their local Google Chrome through the Chrome Stream MCP App stage.

## Public MCP Tools

Invoke only the public tools below (app-only tools carry `_meta.ui.visibility: ["app"]` for the panel UI):

| Tool | Purpose | Parameters |
| --- | --- | --- |
| `chrome_stream_stage` | Open the stage (`ui://chrome-stream-mcp/stage`) and report relay/Chrome readiness without auto-connecting. | `{}` |
| `chrome_stream_connect` | Connect or verify the persistent local relay session and start streaming the active tab. | `{ qualityPreset?: "auto" \| "ultra" \| "low" \| "medium" \| "high", rttMs?: number }` |
| `chrome_stream_disconnect` | Stop screencasting and close the Chrome CDP WebSocket to save CPU/GPU/battery. | `{}` |

## Workflows

- **Open viewer**: Call `chrome_stream_stage` to inspect `connected`, `portFile.valid`, `activeTargetId`, and `tabs`. Never assume Chrome is connected (staging never auto-dials). Call `chrome_stream_connect` only if the user asked to start streaming right away; otherwise let them click **Connect** in the panel.
- **Zero open tabs**: Chrome Stream never auto-creates `about:blank`. If `connected: true` and `tabs` is empty (`activeTargetId: null`), tell the user they can click **New Tab (`+`)** in the panel or open a tab in Chrome.
- **Pause vs. disconnect**: Panel **Pause / Resume** freezes capture while keeping Chrome authorization alive (and capture auto-pauses after 12s idle). Call `chrome_stream_disconnect` when the user is finished or wants to close the debugging connection.
- **Weak networks**: Default to `qualityPreset: "auto"`; on slow/cellular links (> 350 ms RTT), pass `"low"` or `"ultra"`.

## Local Chrome Debugging & Guardrails

If `portFile.valid` is `false`, guide the user to:
1. Enable remote debugging in running Chrome at `chrome://inspect/#remote-debugging` (recommended on Chrome 136+), or launch a separate profile with `--remote-debugging-port=9222 --user-data-dir=/tmp/chrome-debug`.
2. Click **Connect** (or call `chrome_stream_connect`) and click **Allow** once in Chrome. Run `chrome-stream-mcp doctor --json` locally if troubleshooting is needed.

- Connections are strictly loopback-only (`127.0.0.1` / `::1`) over a `0600` local relay; never suggest binding remote debugging to `0.0.0.0` or non-loopback interfaces.
- Tool responses automatically redact local filesystem paths, WebSocket tokens, and hostnames.

## Cross-Client Setup

Any stdio MCP client (Antigravity, Claude Code, Codex, Cursor, OpenCode) can connect via `chrome-stream-mcp serve --data-dir <dir> --panel-html <panel.html>`. Use `chrome-stream-mcp doctor --json` for machine-readable local diagnostics.
