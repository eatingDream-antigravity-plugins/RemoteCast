# AGENTS.md

Guidance for AI coding agents working in `chrome-stream-mcp`. For runtime MCP tool usage and Chrome debugging workflows, see [skills/chrome-stream/SKILL.md](skills/chrome-stream/SKILL.md). Humans: see [README.md](README.md).

## Architecture

- `src/cdp/` — Loopback-only Chrome DevTools discovery (`discovery.rs`), WebSocket protocol (`protocol.rs`), and screencast/input client (`client.rs`).
- `src/relay/` — Persistent single-approval `0600` Unix-socket IPC relay daemon (`daemon.rs`, `ipc.rs`) keeping Chrome debugging authorization alive across reloads.
- `src/mcp/` — MCP stdio JSON-RPC server (`server.rs`), 3 public tools + App-only panel tools with automatic path/token redaction.
- `panel/` — Zero-dependency dark-first SEP-1865 canvas viewer (`build.ts` -> `panel/dist/panel.html`, kept `<45 KiB`).
- `plugin.json`, `mcp_config.json`, `skills/chrome-stream/SKILL.md` — Official Antigravity plugin & portable Agent Skill bundle (no always-on `rules/AGENTS.md` to keep idle prompt overhead at zero).

## Build, Test & Security Gates (run before every PR)

```sh
npm --prefix panel run typecheck && npm --prefix panel run build && npm --prefix panel test
cargo test --locked
./scripts/check_sanitized.sh
```

Release binary build (remap local paths):
```sh
RUSTFLAGS="--remap-path-prefix=$HOME=/build" cargo build --release --locked
```

## Invariants

- Enforce loopback-only (`127.0.0.1` / `::1`) targets and `0600` local relay socket permissions; never make outbound network requests.
- Keep `panel/dist/panel.html` strictly `<45 KiB`; keep tool outputs redacted of local paths and raw DevTools WebSocket tokens.
- Never auto-dial Chrome on `chrome_stream_stage` and never auto-create `about:blank` when all tabs are closed.
- Run `./scripts/check_sanitized.sh` before every push (must exit 0).
