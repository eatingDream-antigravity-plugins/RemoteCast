# Changelog

All notable changes to this project are documented here.
The project uses [Semantic Versioning](https://semver.org/) and updates this file automatically via [Release Please](https://github.com/googleapis/release-please) from Conventional Commits.

## [0.2.0](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/compare/v0.1.0...v0.2.0) (2026-11-24)


### Features

* complete interactive controls, idle/dialog/disconnect fixes, discovery, bilingual UI, and safety docs ([#6](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/issues/6)) ([e49290a](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/commit/e49290ab5a94a4ffabe9f709c458cc0edf4bfb70))


### Documentation

* **readme:** rewrite bilingual READMEs for human readability and usage focus ([#3](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/issues/3)) ([7448dfc](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/commit/7448dfc72e35c5555b4dac94863026aa2ed0e69c))

## [0.1.0](https://github.com/eatingDream-antigravity-plugins/chrome-stream-mcp/releases/tag/v0.1.0) - 2026-11-24

Initial open-source release.

### Features
- **Native Rust Single Binary (`chrome-stream-mcp`)**: Pure-Rust loopback RFC 6455 WebSocket CDP client with SSRF guards, FIPS 180-4 SHA-1 frame hashing, detached `0600` Unix-socket relay daemon (`relay`), concurrent MCP stdio server (`serve`), and diagnostics CLI (`doctor --json`).
- **Persistent Single-Approval CDP Relay**: Keeps Chrome's authorized remote-debugging WebSocket alive across MCP server restarts and auto-stops GPU screencasting after 12 seconds of inactivity.
- **Resilient Zero-Tab & Explicit Connection Lifecycle**: Respects zero open Chrome tabs without forcing `about:blank`, avoids auto-dialing Chrome on stage open, and provides always-visible **Connect**, **Pause/Resume**, and **Disconnect** controls.
- **Weak-Network Streaming Engine**: 4-tier adaptive screencast quality (`ultra`, `low`, `medium`, `high`) driven by RTT telemetry, SHA-1 frame deduplication (`unchanged: true`, zero redundant bytes), `tabsHash` elision, and coalesced non-blocking input dispatch.
- **Zero-Flash Dark-First SEP-1865 Panel (`panel/dist/panel.html`, < 42 KiB)**: Self-contained `default-src 'none'` UI with transparent-background host CSS variable guards, double-buffered `createImageBitmap` rendering, and zero external URLs.
- **On-Demand Agent Skill (`skills/chrome-stream/SKILL.md`)**: Minimal-intrusion Antigravity plugin packaging with zero `rules/AGENTS.md` and automated cross-platform GitHub Release binaries + `scripts/install.sh`.
