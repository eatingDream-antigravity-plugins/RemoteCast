---
name: chrome-stream
description: View, connect, pause, or disconnect a live local Google Chrome screencast session using Chrome Stream MCP (`chrome_stream_stage`, `chrome_stream_connect`, `chrome_stream_disconnect`), diagnose Chrome remote debugging setup, and handle zero-tab or low-bandwidth states safely.
---

# Chrome Stream MCP

Use Chrome Stream MCP when the user explicitly wants to view or interactively control tabs in their local Google Chrome browser through the Chrome Stream MCP App stage.

## Public MCP Tools

Only invoke the public tools listed below. App-only tools (`chrome_stream_tabs`, `chrome_stream_frame`, `chrome_stream_input`, `chrome_stream_navigate`) carry `_meta.ui.visibility: ["app"]` and are reserved exclusively for the embedded panel UI.

| Tool | Purpose | Parameters |
| --- | --- | --- |
| `chrome_stream_stage` | Open the Chrome Stream MCP App stage (`ui://chrome-stream-mcp/stage`) and report current relay/Chrome readiness without forcing a dial. | `{}` |
| `chrome_stream_connect` | Establish or verify the persistent local CDP relay session and warm the active tab screencast. | `{ qualityPreset?: "auto" \| "ultra" \| "low" \| "medium" \| "high", rttMs?: number }` |
| `chrome_stream_disconnect` | Stop active screencasting and close the Chrome CDP WebSocket to release GPU/CPU and save battery. | `{}` |

## Standard Workflows

### 1. Opening the live viewer
1. Call `chrome_stream_stage` to open the stage UI and read the current status snapshot (`connected`, `portFile.valid`, `activeTargetId`, `tabs`).
2. Do **not** assume Chrome is already connected: opening the stage intentionally never auto-dials Chrome when disconnected, avoiding surprise Chrome permission prompts or white flashes.
3. If the user asks to start streaming immediately and Chrome debugging is ready, call `chrome_stream_connect` (default `qualityPreset: "auto"`, or `"ultra"` / `"low"` on constrained remote links). Otherwise, let the user click **Connect** directly inside the stage panel.

### 2. Handling zero open tabs
- Chrome Stream respects zero open tabs and **never** forcibly creates an `about:blank` tab when the last tab is closed or when Chrome has no open pages.
- When `connected: true` and `tabs` is empty (`activeTargetId: null`), inform the user that Chrome is connected with zero open tabs and they can click **New Tab (`+`)** in the stage toolbar or open a page in Chrome.

### 3. Pausing vs. disconnecting
- **Pause / Resume** (inside the stage toolbar): pauses frame polling and stops GPU screencasting while keeping the authorized CDP relay WebSocket alive, so resuming requires no new Chrome permission prompt. The relay also stops GPU screencasting automatically after 12 seconds of inactivity.
- **Disconnect** (`chrome_stream_disconnect` or the stage **Disconnect** button): terminates the CDP WebSocket completely to minimize power usage. Use `chrome_stream_disconnect` whenever the user finishes their session or asks to free battery/resources.

### 4. Weak-network quality tuning
- Default to `qualityPreset: "auto"`, which adapts across four profiles (`ultra`, `low`, `medium`, `high`) based on measured RTT with SHA-1 frame deduplication (`unchanged: true`, zero redundant image bytes).
- On high-latency or cellular links (> 350 ms RTT), recommend or pass `qualityPreset: "low"` or `"ultra"`.

## Safe Chrome Remote Debugging Setup

If `chrome_stream_stage` reports that Chrome debugging is not reachable (`portFile.valid: false`), guide the user through these local-only steps:

1. Ensure Google Chrome is running locally.
2. Open `chrome://inspect/#remote-debugging` in Chrome and enable remote debugging (or launch Chrome with `--remote-debugging-port=9222` bound to localhost).
3. Trigger **Connect** (or `chrome_stream_connect`) and click **Allow** once in Chrome's permission prompt. The persistent `0600` Unix-socket relay (`chrome-stream-mcp relay`) retains the authorized WebSocket across MCP server restarts.
4. For troubleshooting, the user can run `chrome-stream-mcp doctor --json` locally to inspect `DevToolsActivePort` validity, port `9222` fallback, relay socket mode (`0600`), and panel bundle health.

## Security & Privacy Guardrails

- Chrome Stream connects exclusively to loopback addresses (`127.0.0.1` / `::1`) with SSRF checks and stores relay IPC files under the plugin data directory with `0600` permissions.
- Tool responses automatically redact local filesystem paths, WebSocket tokens, and hostnames. Never ask the user to expose Chrome remote debugging on `0.0.0.0` or a non-loopback interface.
