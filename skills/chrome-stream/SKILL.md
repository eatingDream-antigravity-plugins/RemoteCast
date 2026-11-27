---
name: chrome-stream
description: >-
  ONLY use when the user explicitly asks to stream, view, standby, wake, or debug tabs in their external local Google Chrome via Chrome Stream MCP (`chrome_stream_stage`, `chrome_stream_connect`, `chrome_stream_disconnect`). NEVER use for general web browsing, logins, or web app tasks (use Antigravity's built-in `browser` subagent instead).
---

# RemoteCast (chrome-stream-mcp)

Use RemoteCast (`chrome-stream-mcp`) when the user explicitly wants to view or interact with tabs in their local Google Chrome through the RemoteCast MCP App stage. For full configuration tables, environment variables, and app-only tool specs, read `docs/ai-guide.md`.

## Public MCP Tools

Invoke only the public tools below (app-only tools carry `_meta.ui.visibility: ["app"]` for the panel UI and are rejected for non-Apps sessions):

| Tool | Purpose | Parameters |
| --- | --- | --- |
| `chrome_stream_stage` | Open the stage (`ui://chrome-stream-mcp/stage`) and report relay/Chrome readiness without auto-connecting. | `{}` |
| `chrome_stream_connect` | Connect or verify the persistent local relay session and start streaming the active tab. | `{ qualityPreset?: "auto" \| "ultra" \| "low" \| "medium" \| "high", rttMs?: number, mode?: "auto" \| "personal" \| "remote" }` |
| `chrome_stream_disconnect` | Pause capture in standby (keeps Chrome Allow alive) or revoke and close the Chrome CDP WebSocket. | `{ mode?: "standby" \| "revoke" }` (default `"revoke"`) |

## Workflows

- **Open viewer**: Call `chrome_stream_stage` to inspect `connected`, `connectionMode`, `portFile.valid`, and `noTabs`. Never assume Chrome is connected (staging never auto-dials). Call `chrome_stream_connect` only if the user asked to connect right away; otherwise let them choose **Connect Personal Chrome** or **Launch Remote Chrome (No Allow)** in the panel.
- **Standby vs. Revoke (remote-safe)**: Prefer `chrome_stream_disconnect({ mode: "standby" })` (or panel header **Standby / Wake**) when the user wants to pause capture without losing Chrome's local **Allow** consent. Only use `mode: "revoke"` when the user explicitly wants to tear down the debugging connection.
- **Zero open tabs**: Chrome Stream never auto-creates `about:blank`. If `connected: true` and `noTabs: true`, tell the user they can click **New Tab (`+`)** in the panel or open a tab in Chrome.
- **Weak networks**: Default to `qualityPreset: "auto"`. The relay uses EWMA RTT smoothing with asymmetric hysteresis and automatically settles to a crisp WebP still frame ~380 ms after motion/input stops, with 0-byte unchanged deduplication.

## Local Chrome Debugging & Guardrails

If `portFile.valid` is `false` and the user wants their personal Chrome profile:
1. Enable remote debugging in running Chrome at `chrome://inspect/#remote-debugging` (recommended on Chrome 136+), or use the panel's **Launch Remote Chrome (No Allow)** button for an isolated loopback profile (`0700` `<data_dir>/remote-profile`).
2. Click **Connect Personal Chrome** (or call `chrome_stream_connect`) and click **Allow** once in Chrome. Run `chrome-stream-mcp doctor --json` locally if troubleshooting is needed.

- Connections are strictly loopback-only (`127.0.0.1` / `::1`) over an ownership-locked `0600` local relay; never suggest binding remote debugging to `0.0.0.0` or non-loopback interfaces.
- Tool responses automatically redact local filesystem paths, WebSocket tokens, and hostnames.
