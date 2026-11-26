# RemoteCast (chrome-stream-mcp) — AI & Configuration Guide

Complete operational reference for AI agents and power users configuring **RemoteCast** (`chrome-stream-mcp`): MCP tool schemas, Standby vs. Revoke semantics, isolated Remote Chrome mode, weak-network profiles, environment variables, CLI commands, and source builds. Homepage: <https://remotecast.eatingdream.dev>.

## 1. MCP Protocol & Tools

- Supported MCP protocol versions: `2025-11-25`, `2025-06-18`.
- The first `initialize` request fixes MCP Apps capability for the transport. The client must include the exact string `"text/html;profile=mcp-app"` in `capabilities.extensions["io.modelcontextprotocol/ui"].mimeTypes`.
- With this capability, the host may call App-only tools and receive live bootstrap state in `resources/read`. Other hosts receive static panel HTML. The public stage tool returns an allowlisted readiness and connection summary.

### Public Tools (all MCP clients)

| Tool | Parameters | Behavior |
| :--- | :--- | :--- |
| `chrome_stream_stage` | `{}` | Opens the live viewer (`ui://chrome-stream-mcp/stage`) and reports relay/Chrome readiness. The tool leaves the Chrome connection unchanged. |
| `chrome_stream_connect` | `{ qualityPreset?: "auto" \| "ultra" \| "low" \| "medium" \| "high", rttMs?: number, mode?: "auto" \| "personal" \| "remote" }` | Connects to local Chrome (`auto` checks `DevToolsActivePort` then loopback debug port; `personal` uses port files only; `remote` uses loopback debug port only) and starts streaming the active tab. |
| `chrome_stream_disconnect` | `{ mode?: "standby" \| "revoke" }` (default `"revoke"`) | `"standby"` pauses capture (0 FPS) while keeping the authorized CDP WebSocket warmly alive in the relay; `"revoke"` closes the CDP WebSocket (requiring local Chrome **Allow** again on personal Chrome). |

### App-Only Tools (MCP Apps UI hosts)

| Tool | Parameters | Behavior |
| :--- | :--- | :--- |
| `chrome_stream_frame` | `{ targetId?, sinceSeq?, lastFrameHash?, knownTabsHash?, qualityPreset?, rttMs?, waitMs?, forceRefresh? }` | Long-polls for changed JPEG motion frames or crisp settled WebP stills (`crisp: true, format: "webp"`); returns `unchanged: true, imageBytes: 0` for unchanged frames. The bootstrap cache shares its stored image across unchanged updates. |
| `chrome_stream_input` | `{ targetId?, targetEpoch?, events: [...] }` | Dispatches coalesced mouse, wheel, touch, keyboard, IME, dialog, navigation, tab, and pause/resume actions bound to `(targetId, targetEpoch)`. Closing a tab requires an explicit `targetId`. A stale `targetEpoch` or missing target yields `staleTarget: true`. |
| `chrome_stream_launch_remote` | `{ qualityPreset?, rttMs? }` | Reuses a live endpoint verified against the isolated profile, or starts an isolated Chrome (`0700` `<data_dir>/remote-profile`, `--remote-debugging-address=127.0.0.1`), connects in `"remote"` mode, and warms the active tab. |

## 2. Standby vs. Revoke & Dedicated Remote Chrome

- **Personal Chrome** (`mode: "personal"`) discovers Chrome exclusively from its `DevToolsActivePort` files. Confirm **Allow** in Chrome. The local relay can keep that authorized WebSocket across host reloads and standby.
- **Standby** (`mode: "standby"`) uses the relay's `pause` RPC and reports retained authorization only after the relay confirms both the connection and the pause.
- **Revoke** (`mode: "revoke"`) closes the Chrome DevTools Protocol (CDP) WebSocket and clears the bootstrap page, tabs, and pixels. A relay failure returns `isError: true` with `unconfirmed: true`. Query live status or retry the revoke.
- **Dedicated Remote Chrome** starts with `--remote-debugging-address=127.0.0.1`, `--remote-debugging-port=<port>`, and `--user-data-dir=<data_dir>/remote-profile`. The launcher checks ongoing sessions against the profile's endpoint record and the live browser endpoint. On a conflict, set `CHROME_STREAM_DEBUG_PORT` to a free port or use Personal mode. The explicit `connect` option `mode: "remote"` attaches to the configured loopback port.
- The relay checks same-user Unix socket credentials and validates the directory chain before using socket or profile files. Requests to the relay and MCP server have a 1 MiB line limit. Relay replies have a 16 MiB limit for calls and a 64 KiB limit for probes.

## 3. Weak-Network Profiles & Progressive Sharpness

| Preset | Motion JPEG Quality | Max Resolution | Frame Decimation |
| :--- | :---: | :---: | :---: |
| `ultra` | 48 | 1024 × 640 | every 4th frame |
| `low` | 58 | 1152 × 720 | every 3rd frame |
| `medium` | 70 | 1280 × 800 | every 2nd frame |
| `high` | 84 | 1440 × 920 | every frame |

- **EWMA + Asymmetric Hysteresis**: Smooths RTT with `alpha = 0.25`; downgrades only after 2 consecutive slow samples and upgrades only after 3 consecutive fast samples.
- **Two-Stage Crisp Settle**: The relay requests a WebP still about 380 ms after the last pixel, viewport, or input change. It retries transient capture failures with backoff up to three total attempts. Later unchanged polls report `imageBytes: 0`.

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

`doctor --json` returns a sanitized readiness summary by default. It omits local paths, raw DevTools WebSocket URLs, and live page data. The text form displays local socket and panel paths for interactive troubleshooting.

Generic stdio MCP client configuration

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
git clone https://github.com/eatingDream-antigravity-plugins/RemoteCast.git && cd RemoteCast
node panel/build.mjs && node --test panel/net-policy.test.mjs
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --locked
cargo build --release --locked
```
