# Chrome Stream MCP — AI & Configuration Guide

Complete operational reference for AI agents and power users: MCP tool schemas, Standby vs. Revoke semantics, isolated Remote Chrome mode, weak-network profiles, environment variables, CLI commands, and source builds.

## 1. MCP Protocol & Tools

- Supported MCP protocol versions: `2025-11-25`, `2025-06-18`.
- App-only tools (`_meta.ui.visibility: ["app"]`) are advertised in `tools/list` and permitted in `tools/call` **only** when the client negotiates MCP Apps UI capability (`extensions["io.modelcontextprotocol/ui"].mimeTypes` containing `"text/html;profile=mcp-app"`, or explicit legacy UI host capability).

### Public Tools (all MCP clients)

| Tool | Parameters | Behavior |
| :--- | :--- | :--- |
| `chrome_stream_stage` | `{}` | Opens the live viewer (`ui://chrome-stream-mcp/stage`) and reports relay/Chrome readiness without auto-connecting. |
| `chrome_stream_connect` | `{ qualityPreset?: "auto" \| "ultra" \| "low" \| "medium" \| "high", rttMs?: number, mode?: "auto" \| "personal" \| "remote" }` | Connects to local Chrome (`auto` checks `DevToolsActivePort` then loopback debug port; `personal` uses port files only; `remote` uses loopback debug port only) and starts streaming the active tab. |
| `chrome_stream_disconnect` | `{ mode?: "standby" \| "revoke" }` (default `"revoke"`) | `"standby"` pauses capture (0 FPS) while keeping the authorized CDP WebSocket warmly alive in the relay; `"revoke"` closes the CDP WebSocket (requiring local Chrome **Allow** again on personal Chrome). |

### App-Only Tools (MCP Apps UI hosts)

| Tool | Parameters | Behavior |
| :--- | :--- | :--- |
| `chrome_stream_frame` | `{ targetId?, sinceSeq?, lastFrameHash?, knownTabsHash?, qualityPreset?, rttMs?, waitMs?, forceRefresh? }` | Long-polls for changed JPEG motion frames or crisp settled WebP stills (`crisp: true, format: "webp"`); returns `unchanged: true, imageBytes: 0` with zero image cloning when unchanged. |
| `chrome_stream_input` | `{ targetId?, targetEpoch?, events: [...] }` | Dispatches coalesced mouse, wheel, touch, keyboard, IME, dialog, navigation, tab, and pause/resume actions bound to `(targetId, targetEpoch)`. Rejects stale targets with `staleTarget: true` instead of misrouting input. |
| `chrome_stream_launch_remote` | `{ qualityPreset?, rttMs? }` | Reuses an existing `127.0.0.1:<port>` Remote Chrome or spawns a detached isolated Chrome (`0700` `<data_dir>/remote-profile`, `--remote-debugging-address=127.0.0.1`), connects in `"remote"` mode, and warms the active tab. |

## 2. Standby vs. Revoke & Dedicated Remote Chrome

- **Personal Chrome (`connectionMode: "personal"`)**: Connects via Chrome's `DevToolsActivePort`. Click **Allow** once locally; the user-only `0600` local relay holds the authorized session alive across plugin reloads and standby pauses (never exits on idle while connected).
- **Standby (`mode: "standby"` or panel header Standby/Wake)**: Stops screencasting (`0 FPS`) without revoking Chrome consent. Recommended whenever stepping away from the computer.
- **Revoke (`mode: "revoke"` or `⋯` menu -> Revoke & Disconnect)**: Tears down the CDP WebSocket and releases the Chrome debugging session.
- **Remote Chrome (`connectionMode: "remote"`)**: Launches Chrome directly via `exec` argv (never `sh -c`) with `--remote-debugging-address=127.0.0.1`, `--remote-debugging-port=<port>`, and `--user-data-dir=<data_dir>/remote-profile` (`0700`), completely isolated from personal Chrome and requiring no local **Allow** click.

## 3. Weak-Network Profiles & Progressive Sharpness

| Preset | Motion JPEG Quality | Max Resolution | Frame Decimation |
| :--- | :---: | :---: | :---: |
| `ultra` | 48 | 1024 × 640 | every 4th frame |
| `low` | 58 | 1152 × 720 | every 3rd frame |
| `medium` | 70 | 1280 × 800 | every 2nd frame |
| `high` | 84 | 1440 × 920 | every frame |

- **EWMA + Asymmetric Hysteresis**: Smooths RTT with `alpha = 0.25`; downgrades only after 2 consecutive slow samples and upgrades only after 3 consecutive fast samples.
- **Two-Stage Crisp Settle**: ~380 ms after the last motion frame or input burst on an active unpaused tab, captures one crisp WebP still (`quality: 86`, `crisp: true`, `format: "webp"`), then deduplicates subsequent polls at 0 bytes until motion/input resumes.

## 4. Environment Variables

| Variable | Purpose | Default |
| :--- | :--- | :--- |
| `CHROME_STREAM_DEBUG_PORT` | Loopback debugging port for Remote Chrome / fallback probe | `9222` |
| `CHROME_STREAM_PORT_FILE` | Explicit path to Chrome `DevToolsActivePort` | Auto-discovered |
| `CHROME_STREAM_USER_DATA_DIR` | Custom Chrome user data directory for port-file scan | Standard macOS/Linux paths |
| `CHROME_STREAM_CHROME_BIN` | Explicit Chrome/Chromium executable for `launch_remote` | Platform Chrome candidates |
| `CHROME_STREAM_DATA_DIR` | Relay socket, lock file, and isolated `remote-profile` directory | Platform plugin data dir |
| `CHROME_STREAM_LANG` | Panel/error locale override (`en` or `zh-CN`) | Auto-detected |
| `CHROME_STREAM_VERSION` | Version tag pinned by `scripts/install.sh` | Latest release |

## 5. CLI Subcommands & Generic MCP Config

```sh
chrome-stream-mcp serve [--data-dir <path>] [--panel-html <path>]
chrome-stream-mcp relay [--data-dir <path>]
chrome-stream-mcp doctor [--json]
```

Generic stdio MCP client config (Claude Code, Codex, Cursor, OpenCode, etc.):

```json
{
  "mcpServers": {
    "chrome-stream-mcp": {
      "command": "/path/to/chrome-stream-mcp",
      "args": [
        "serve",
        "--data-dir", "/path/to/chrome-stream-data",
        "--panel-html", "/path/to/panel.html"
      ]
    }
  }
}
```

## 6. Build & Verify from Source

Requires Rust (`cargo`) and Node.js 20+ (panel bundler & tests; zero npm dependencies):

```sh
git clone https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp.git && cd chrome-stream-mcp
node panel/build.mjs && node --test panel/net-policy.test.mjs
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --locked
cargo build --release --locked
```
