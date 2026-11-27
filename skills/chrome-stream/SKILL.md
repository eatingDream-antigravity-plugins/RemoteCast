---
name: chrome-stream
description: >-
  ONLY use when the user explicitly asks to stream, view, pause, resume, or debug tabs in their local Google Chrome via Remote Cast (`chrome_stream_stage`, `chrome_stream_connect`, `chrome_stream_disconnect`). NEVER use for general web browsing, logins, or web app tasks (use Antigravity's built-in `browser` subagent instead).
---

# Remote Cast (`chrome-stream-mcp`)

Use Remote Cast (`chrome-stream-mcp`) when the user explicitly wants to view or interact with tabs in their local Google Chrome through the Remote Cast MCP App panel. For full configuration tables, environment variables, and app-only tool specs, read `docs/ai-guide.md`.

## Public MCP Tools

Invoke only the public tools below (app-only tools carry `_meta.ui.visibility: ["app"]` for the panel UI and are rejected for non-Apps sessions):

| Tool | Purpose | Parameters |
| --- | --- | --- |
| `chrome_stream_stage` | Open the panel (`ui://chrome-stream-mcp/stage`) and report relay/Chrome readiness without auto-connecting. | `{}` |
| `chrome_stream_connect` | Connect or verify the persistent local relay session and start streaming the active tab. | `{ qualityPreset?: "auto" \| "ultra" \| "low" \| "medium" \| "high", rttMs?: number, mode?: "auto" \| "personal" \| "remote" }` |
| `chrome_stream_disconnect` | Pause capture (keeps Chrome Allow alive) or disconnect and close the Chrome debugging session. | `{ mode?: "standby" \| "revoke" }` (default `"revoke"`) |

## Workflows

- **Open viewer**: Call `chrome_stream_stage` to inspect `connected`, `connectionMode`, `portFile.valid`, and `noTabs`. Never assume Chrome is connected (staging never auto-dials). Call `chrome_stream_connect` only if the user asked to connect right away; otherwise let them choose **Connect my Chrome** or **Open separate Chrome** in the panel.
- **Pause vs. Disconnect**: Prefer `chrome_stream_disconnect({ mode: "standby" })` (or panel header **Pause / Resume**) when the user wants to pause capture without losing Chrome's **Allow** permission. Use `mode: "revoke"` (or header **Disconnect**) when the user wants to end the session.
- **Zero open tabs / closed window**: Remote Cast never auto-creates `about:blank`. If `connected: true` and `noTabs: true`, tell the user they can click **New tab** (opens `chrome://newtab` in the background) or **Disconnect** in the panel.
- **Weak networks**: Default to `qualityPreset: "auto"`. The 5 tiers ordered weakest to sharpest are `auto`, `ultra` (**Saver**), `low` (**Low**), `medium` (**Balanced**), and `high` (**High**).

## Configuration (`settings.json`)

User settings live at `~/.gemini/antigravity/plugin_data/chrome-stream-mcp/settings.json` (mode `0600`, atomic temp+rename, `flock`-locked) and hot-reload live without restarting Antigravity:

- **GUI Settings (`⚙` drawer & app-only `chrome_stream_config`)**:
  - `lang`: `"auto" | "en" | "zh-CN"`
  - `defaultQuality`: `"auto" | "ultra" | "low" | "medium" | "high"`
  - `defaultConnectMode`: `"auto" | "personal" | "remote"`
  - `bgPollIntervalSec`: `0` (auto) | `5` | `15` | `30` | `-1` (pause in background)
  - `idlePauseSec`: `12` | `30` | `60` | `0` (off)
- **Advanced file-only keys** (edit `settings.json` directly; env vars override file values):
  - `debugPort` (default `9222`), `remotePort` (default auto `9223..=9232`), `newTabUrl` (default `"chrome://newtab"`), `userDataDir`, `portFile`, `wheelCoalesceMs` (default `75`).
- Invalid JSON in `settings.json` fails closed (never overwritten; last good settings remain active and the panel surfaces an inline notice).

## Local Chrome Debugging & Guardrails

If `portFile.valid` is `false` and the user wants their everyday Chrome:
1. Enable remote debugging in Chrome at `chrome://inspect/#remote-debugging`, or click **Open separate Chrome** in the panel for an isolated loopback profile (`0700` `<data_dir>/remote-profile`, no permission prompt).
2. Click **Connect my Chrome** (or call `chrome_stream_connect`) and click **Allow** once in Chrome. Run `chrome-stream-mcp doctor --json` locally if troubleshooting is needed.

- Connections are strictly loopback-only (`127.0.0.1` / `::1`) over an ownership-locked `0600` local relay; never suggest binding remote debugging to `0.0.0.0` or non-loopback interfaces.
- Tool responses automatically redact local filesystem paths, WebSocket tokens, and hostnames.
