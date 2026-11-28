# Remote Cast (`chrome-stream-mcp`) — AI & Configuration Guide

Complete operational reference for AI agents and power users configuring **Remote Cast** (`chrome-stream-mcp`): MCP tool schemas, Pause vs. Disconnect semantics, Separate Chrome mode, `settings.json` hot-reload, weak-network profiles, environment variables, CLI commands, and source builds. Homepage: <https://remotecast.eatingdream.dev>.

## 1. MCP Protocol & Tools

- Supported MCP protocol versions: `2025-11-25`, `2025-06-18`.
- The first `initialize` request fixes MCP Apps capability for the transport. The client must include `"text/html;profile=mcp-app"` in `capabilities.extensions["io.modelcontextprotocol/ui"].mimeTypes`.
- With this capability, the host may call App-only tools and receive live bootstrap state in `resources/read`. Other hosts receive static panel HTML.

### Public Tools (all MCP clients)

| Tool | Parameters | Behavior |
| :--- | :--- | :--- |
| `chrome_stream_stage` | `{}` | Opens the live viewer (`ui://chrome-stream-mcp/stage`) and reports relay/Chrome readiness without auto-connecting. |
| `chrome_stream_connect` | `{ qualityPreset?: "auto" \| "ultra" \| "low" \| "medium" \| "high", rttMs?: number, mode?: "auto" \| "personal" \| "remote" }` | Connects to local Chrome (`auto` checks `DevToolsActivePort` then loopback debug port; `personal` uses port files only; `remote` uses Separate Chrome loopback port only) and starts streaming the active tab. |
| `chrome_stream_disconnect` | `{ mode?: "standby" \| "revoke" }` (default `"revoke"`) | `"standby"` (recommended) drops capture to 0 FPS while keeping the authorized Chrome debugging session warm in the relay; `"revoke"` closes the session (on My Chrome, Chrome asks to Allow again next time). |

### App-Only Tools (MCP Apps UI hosts)

| Tool | Parameters | Behavior |
| :--- | :--- | :--- |
| `chrome_stream_frame` | `{ targetId?, sinceSeq?, lastFrameHash?, knownTabsHash?, qualityPreset?, rttMs?, waitMs?, forceRefresh? }` | Long-polls for changed JPEG motion frames or crisp settled WebP stills (`crisp: true, format: "webp"`); returns `unchanged: true, imageBytes: 0` when unchanged. |
| `chrome_stream_input` | `{ targetId?, targetEpoch?, events: [...] }` | Dispatches coalesced mouse, wheel, keyboard, IME, dialog, navigation, tab, and pause/resume actions bound to `(targetId, targetEpoch)`. New tabs default to `chrome://newtab` in the background without stealing macOS window focus. |
| `chrome_stream_launch` | `{ qualityPreset?, rttMs? }` | Reuses a live endpoint verified against the isolated profile, or starts Separate Chrome (`0700` `<data_dir>/remote-profile`, `--remote-debugging-address=127.0.0.1`), connects in `"remote"` mode, and warms the active tab. |
| `chrome_stream_config` | `{ action: "get" \| "update", patch?: { ... } }` | Reads or atomically updates `${PLUGIN_DATA}/settings.json` (`0600`, `flock`-locked, preserves unknown keys) and returns `{ ok, settings, settingsPath, settingsGen, settingsError }`. |

## 2. Standby vs. Revoke & Separate Chrome

- **My Chrome** (`mode: "personal"`) discovers Chrome from its `DevToolsActivePort` files. Confirm **Allow** once in Chrome; the local relay keeps that authorized session across host reloads, **Pause**, and **Standby**.
- **Standby** (`mode: "standby"` / panel **Standby**) stops streaming at 0 FPS and returns to the Standby Ready card without dropping Chrome authorization, enabling 1-click **Resume Stream** without re-prompting.
- **Revoke** (`mode: "revoke"` / panel **Revoke** or **Revoke Grant**) closes the debugging WebSocket and clears the bootstrap page, tabs, and pixels. Chrome itself stays open.
- **Separate Chrome** starts with `--remote-debugging-address=127.0.0.1`, `--remote-debugging-port=<port>`, and `--user-data-dir=<data_dir>/remote-profile`. It reuses a running managed session matching the profile, or automatically picks a free loopback port starting at `debugPort + 1` (`9223..=9232`, falling back to an OS-assigned free loopback port).
- **Focus-safe background interaction (macOS)**: Tab switches, tab closes, and new tabs (`Target.createTarget` with `background: true`) never call `Target.activateTarget`; focus emulation (`Emulation.setFocusEmulationEnabled(true)`) keeps keyboard/mouse input working on background tabs without bringing Chrome in front of Antigravity. (Note: minimizing a Chrome window on macOS throttles Chromium's compositor; keep the Chrome window un-minimized behind Antigravity or use Separate Chrome.)

## 3. Unified Settings (`settings.json`)

Settings live at `~/.gemini/antigravity/plugin_data/chrome-stream-mcp/settings.json` (mode `0600`) and hot-reload live when edited on disk (tracked by `(mtime_ns, size, ino)`). Environment variables always override file values. Invalid JSON fails closed without overwriting the file.

| Key | Type / Values | Default | Exposed in `⚙` GUI |
| :--- | :--- | :--- | :---: |
| `lang` | `"auto" \| "en" \| "zh-CN"` | `"auto"` | Yes |
| `defaultQuality` | `"auto" \| "ultra" \| "low" \| "medium" \| "high"` | `"auto"` | Yes |
| `defaultConnectMode` | `"auto" \| "personal" \| "remote"` | `"auto"` | Yes |
| `bgPollIntervalSec` | `0` (auto) \| `5` \| `15` \| `30` \| `-1` (pause) | `0` | Yes |
| `idlePauseSec` | `0` (off) \| `12` \| `30` \| `60` | `12` | Yes |
| `debugPort` | `1024..=65535` | `9222` | File only |
| `remotePort` | `null` (auto) or `1024..=65535` | `null` | File only |
| `newTabUrl` | URL string | `"chrome://newtab"` | File only |
| `portFile` | Path to `DevToolsActivePort` | `null` | File only |
| `userDataDir` | Custom Chrome user data dir | `null` | File only |
| `wheelCoalesceMs` | `20..=500` | `75` | File only |

## 4. Weak-Network Profiles & Progressive Sharpness

Ordered weakest to sharpest (`Auto · Saver · Low · Balanced · High`):

| Preset Key | Panel Label (EN / ZH) | Motion JPEG Quality | Max Resolution | Frame Decimation |
| :--- | :--- | :---: | :---: | :---: |
| `ultra` | Saver / 省流 | 48 | 1024 × 640 | every 4th frame |
| `low` | Low / 流畅 | 58 | 1152 × 720 | every 3rd frame |
| `medium` | Balanced / 均衡 | 70 | 1280 × 800 | every 2nd frame |
| `high` | High / 高清 | 84 | 1440 × 920 | every frame |

- **EWMA + Asymmetric Hysteresis**: Smooths RTT with `alpha = 0.25`; downgrades after 2 consecutive slow samples and upgrades after 3 consecutive fast samples.
- **Two-Stage Crisp Settle**: Captures a crisp WebP still ~380 ms after motion/input stops, with 0-byte unchanged deduplication thereafter.

## 5. Environment Variables

| Variable | Purpose | Default |
| :--- | :--- | :--- |
| `CHROME_STREAM_DEBUG_PORT` | Override `debugPort` (My Chrome / fallback port) | `9222` |
| `CHROME_STREAM_REMOTE_PORT` | Override `remotePort` (pin Separate Chrome port) | Auto (`9223..=9232`, else OS free port) |
| `CHROME_STREAM_PORT_FILE` | Explicit path to Chrome `DevToolsActivePort` | Auto-discovered |
| `CHROME_STREAM_USER_DATA_DIR` | Custom Chrome user data directory for port-file scan | Standard macOS/Linux paths |
| `CHROME_STREAM_CHROME_BIN` | Explicit Chrome/Chromium executable | Platform Chrome candidates |
| `CHROME_STREAM_DATA_DIR` | Relay socket, lock file, `settings.json`, and `remote-profile` | Platform plugin data dir |
| `CHROME_STREAM_LANG` | Override `lang` (`en` or `zh-CN`) | Auto-detected |
| `CHROME_STREAM_VERSION` | Version tag pinned by `scripts/install.sh` | Latest release |

## 6. CLI Subcommands & Generic MCP Config

```sh
chrome-stream-mcp serve [--data-dir <path>] [--panel-html <path>]
chrome-stream-mcp relay [--data-dir <path>]
chrome-stream-mcp doctor [--json]
```

`scripts/install.sh` automatically refreshes the running Antigravity MCP server via `RefreshMcpServers` when invoked inside Antigravity (`--no-reload` to opt out), or reloads via `agy plugin disable/enable` when `--reload` is passed outside Antigravity.

## 7. Build & Verify from Source

Requires Rust (`cargo`) and Node.js 22.18+ (with `panel/package.json` `esbuild` + `typescript` devDependencies for deterministic builds):

```sh
git clone https://github.com/eatingDream-antigravity-plugins/remote-cast.git && cd remote-cast
npm --prefix panel ci --ignore-scripts && npm --prefix panel run typecheck && npm --prefix panel run build && npm --prefix panel test
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --locked
cargo build --release --locked
```
