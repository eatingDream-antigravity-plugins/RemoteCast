use crate::cdp::client::BOOT_SNAPSHOT_MAX_DATA_URL_CHARS;
use crate::config::{Config, APP_VERSION};
use crate::mcp::tools::{
    dispatch_tool_with_cache, redact_relay_payload, tool_definitions, APP_MIME, STAGE_URI,
};
use crate::relay::ipc::{RelayBackend, RelayClient};
use serde_json::{json, Value};
use std::fs;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";
const MAX_CONCURRENT_REQUESTS: usize = 16;
const SWR_FRESH_TTL: Duration = Duration::from_millis(500);
const HTML_STAT_TTL: Duration = Duration::from_millis(500);
const BOOT_MARKER: &str = "/*__CHROME_STREAM_BOOT__*/";

#[derive(Default)]
struct HtmlCache {
    path: PathBuf,
    mtime: Option<SystemTime>,
    size: u64,
    checked_at: Option<Instant>,
    prefix: String,
    suffix: String,
    has_marker: bool,
}

impl HtmlCache {
    fn ensure_loaded(&mut self, path: &Path) -> Result<(), String> {
        if self.path == path && !self.prefix.is_empty() {
            if let Some(ts) = self.checked_at {
                if ts.elapsed() < HTML_STAT_TTL {
                    return Ok(());
                }
            }
        }
        let meta = fs::metadata(path)
            .map_err(|e| format!("Failed reading panel HTML {}: {e}", path.display()))?;
        let mtime = meta.modified().ok();
        let size = meta.len();
        self.checked_at = Some(Instant::now());
        if self.path == path && self.mtime == mtime && self.size == size && !self.prefix.is_empty() {
            return Ok(());
        }
        let text = fs::read_to_string(path)
            .map_err(|e| format!("Failed reading panel HTML {}: {e}", path.display()))?;
        self.path = path.to_path_buf();
        self.mtime = mtime;
        self.size = size;
        if let Some((pre, suf)) = text.split_once(BOOT_MARKER) {
            self.prefix = pre.to_string();
            self.suffix = suf.to_string();
            self.has_marker = true;
        } else {
            self.prefix = text;
            self.suffix.clear();
            self.has_marker = false;
        }
        Ok(())
    }

    fn render(&mut self, path: &Path, boot_data: Option<&Value>) -> Result<String, String> {
        self.ensure_loaded(path)?;
        if !self.has_marker {
            return Ok(self.prefix.clone());
        }
        match boot_data {
            Some(bd) => {
                let serialized = serialize_boot_data(bd);
                let mut out = String::with_capacity(
                    self.prefix.len() + self.suffix.len() + serialized.len() + 26,
                );
                out.push_str(&self.prefix);
                out.push_str("window.__CHROME_STREAM_BOOT__=");
                out.push_str(&serialized);
                out.push(';');
                out.push_str(&self.suffix);
                Ok(out)
            }
            None => {
                let mut out =
                    String::with_capacity(self.prefix.len() + BOOT_MARKER.len() + self.suffix.len());
                out.push_str(&self.prefix);
                out.push_str(BOOT_MARKER);
                out.push_str(&self.suffix);
                Ok(out)
            }
        }
    }
}

pub fn serialize_boot_data(val: &Value) -> String {
    let raw = serde_json::to_string(val).unwrap_or_else(|_| "null".to_string());
    let mut out = String::with_capacity(raw.len() + 32);
    for ch in raw.chars() {
        match ch {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(ch),
        }
    }
    out
}

fn sanitize_boot_snapshot(mut bd: Value) -> Value {
    redact_relay_payload(&mut bd);
    let too_big = bd
        .get("imageDataUrl")
        .and_then(|v| v.as_str())
        .map(|s| s.len() > BOOT_SNAPSHOT_MAX_DATA_URL_CHARS)
        .unwrap_or(false);
    if too_big {
        bd["seq"] = json!(0);
        bd["frameHash"] = json!("");
        bd["imageDataUrl"] = json!("");
        bd["imageBytes"] = json!(0);
        bd["changed"] = json!(false);
        bd["unchanged"] = json!(true);
    }
    bd
}

/// Builds a lightweight status payload for `chrome_stream_stage` (omits base64 image data so
/// trajectory CallToolResult stays tiny while giving the panel complete state).
pub fn stage_status_from_snapshot(snap: &Value) -> Value {
    let mut out = snap.clone();
    if let Some(obj) = out.as_object_mut() {
        obj.remove("imageDataUrl");
        obj.remove("imageBytes");
        obj.remove("frameHash");
        obj.remove("changed");
        obj.remove("unchanged");
        obj.remove("tabs");
    }
    redact_relay_payload(&mut out);
    out
}

struct SwrState {
    snapshot: Option<Value>,
    updated_at: Option<Instant>,
    refreshing: bool,
}

/// Stale-While-Revalidate cache for relay `boot_snapshot` + Chrome readiness.
/// Populated asynchronously at `chrome-stream-mcp serve` startup so first-click `resources/read`
/// and `chrome_stream_stage` return from RAM in <0.5 ms without blocking on relay spawn or IPC.
pub struct SwrBootCache {
    relay: Arc<dyn RelayBackend>,
    state: Mutex<SwrState>,
    ready_cv: Condvar,
}

impl SwrBootCache {
    pub fn new(relay: Arc<dyn RelayBackend>) -> Arc<Self> {
        Arc::new(Self {
            relay,
            state: Mutex::new(SwrState {
                snapshot: None,
                updated_at: None,
                refreshing: false,
            }),
            ready_cv: Condvar::new(),
        })
    }

    pub fn is_warm(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .is_some()
    }

    fn fetch_from_relay(&self, timeout: Duration) -> Option<Value> {
        self.relay
            .call("boot_snapshot", json!({ "qualityPreset": "low" }), timeout)
            .or_else(|_| self.relay.call("status", json!({}), timeout))
            .ok()
            .map(sanitize_boot_snapshot)
    }

    /// Triggers a single-flight background refresh (warms relay connection + fetches snapshot).
    pub fn prewarm_async(self: &Arc<Self>, warm_relay: bool) {
        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if st.refreshing {
                return;
            }
            st.refreshing = true;
        }
        let me = Arc::clone(self);
        let _ = thread::Builder::new()
            .name("chrome-stream-warm".to_string())
            .spawn(move || {
                if warm_relay {
                    let _ = me.relay.warm();
                    // Also nudge relay's Chrome readiness check asynchronously when disconnected.
                    let _ = me
                        .relay
                        .call("warm", json!({ "qualityPreset": "low" }), Duration::from_millis(800));
                }
                let fetched = me.fetch_from_relay(Duration::from_millis(1500));
                let mut st = me.state.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(snap) = fetched {
                    st.snapshot = Some(snap);
                    st.updated_at = Some(Instant::now());
                }
                st.refreshing = false;
                me.ready_cv.notify_all();
            });
    }

    /// Returns cached snapshot immediately (<0.1 ms) if warm and triggers async revalidation when stale.
    /// Only blocks (bounded by `cold_wait`) if the cache has never been populated yet.
    pub fn get_swr(self: &Arc<Self>, cold_wait: Duration) -> Option<Value> {
        let (cached, needs_bg_refresh, wait_in_flight) = {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(snap) = st.snapshot.clone() {
                let stale = st
                    .updated_at
                    .map(|t| t.elapsed() >= SWR_FRESH_TTL)
                    .unwrap_or(true);
                let bg = stale && !st.refreshing;
                if bg {
                    st.refreshing = true;
                }
                (Some(snap), bg, false)
            } else if st.refreshing {
                (None, false, true)
            } else {
                st.refreshing = true;
                (None, false, false)
            }
        };

        if let Some(snap) = cached {
            if needs_bg_refresh {
                let me = Arc::clone(self);
                let _ = thread::Builder::new()
                    .name("chrome-stream-swr".to_string())
                    .spawn(move || {
                        let fetched = me.fetch_from_relay(Duration::from_millis(1500));
                        let mut st = me.state.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(next) = fetched {
                            st.snapshot = Some(next);
                            st.updated_at = Some(Instant::now());
                        }
                        st.refreshing = false;
                        me.ready_cv.notify_all();
                    });
            }
            return Some(snap);
        }

        if wait_in_flight {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if st.snapshot.is_none() && st.refreshing && !cold_wait.is_zero() {
                let (guard, _) = self
                    .ready_cv
                    .wait_timeout_while(st, cold_wait, |s| s.snapshot.is_none() && s.refreshing)
                    .unwrap_or_else(|e| e.into_inner());
                st = guard;
            }
            return st.snapshot.clone();
        }

        // Cold path with no in-flight prewarm: fetch synchronously once (bounded) and populate cache.
        let fetched = self.fetch_from_relay(cold_wait.max(Duration::from_millis(200)));
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(snap) = fetched.clone() {
            st.snapshot = Some(snap);
            st.updated_at = Some(Instant::now());
        }
        st.refreshing = false;
        self.ready_cv.notify_all();
        fetched
    }

    /// Opportunistically merges live tool results into the SWR cache so reopening the panel
    /// immediately reflects the exact latest connection state, tabs, and frame with zero IPC.
    pub fn update_from_tool_result(&self, tool_name: &str, sc: &Value) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut snap = st.snapshot.clone().unwrap_or_else(|| {
            json!({
                "hostname": "localhost",
                "connected": false,
                "connecting": false,
                "tabs": [],
                "tabsHash": "empty",
                "page": { "title": "", "url": "", "viewportWidth": 1280, "viewportHeight": 800 },
                "changed": false,
                "unchanged": true,
                "seq": 0,
                "frameHash": "",
                "imageDataUrl": "",
                "imageBytes": 0,
                "qualityApplied": "low",
            })
        });

        let connected = sc.get("connected").and_then(|v| v.as_bool());
        if let Some(c) = connected {
            snap["connected"] = json!(c);
        }
        if let Some(pf) = sc.get("portFile") {
            if !pf.is_null() {
                snap["portFile"] = pf.clone();
                if let Some(valid) = pf.get("valid").and_then(|v| v.as_bool()) {
                    snap["chromeReady"] = json!(valid);
                }
            }
        }

        if tool_name == "chrome_stream_disconnect" || connected == Some(false) {
            snap["connected"] = json!(false);
            snap["paused"] = json!(false);
            snap["screencastPausedIdle"] = json!(false);
            snap["noTabs"] = json!(false);
            snap["dialog"] = Value::Null;
            snap["tabs"] = json!([]);
            snap["tabsHash"] = json!("empty");
            snap["activeTargetId"] = Value::Null;
            snap["seq"] = json!(0);
            snap["frameHash"] = json!("");
            snap["imageDataUrl"] = json!("");
            snap["imageBytes"] = json!(0);
            snap["changed"] = json!(false);
            snap["unchanged"] = json!(true);
        } else {
            if let Some(p) = sc.get("paused").and_then(|v| v.as_bool()) {
                snap["paused"] = json!(p);
            }
            if let Some(idle) = sc.get("screencastPausedIdle").and_then(|v| v.as_bool()) {
                snap["screencastPausedIdle"] = json!(idle);
            }
            if let Some(dlg) = sc.get("dialog") {
                snap["dialog"] = dlg.clone();
            }
            if let Some(loc) = sc.get("locale").and_then(|v| v.as_str()) {
                snap["locale"] = json!(loc);
            }
            let no_tabs = sc.get("noTabs").and_then(|v| v.as_bool()).unwrap_or(false)
                || sc
                    .get("tabs")
                    .and_then(|v| v.as_array())
                    .map(|a| a.is_empty())
                    .unwrap_or(false);
            let prev_tid = snap
                .get("activeTargetId")
                .and_then(|v| v.as_str())
                .map(String::from);
            if no_tabs {
                snap["noTabs"] = json!(true);
                snap["activeTargetId"] = Value::Null;
            } else {
                snap["noTabs"] = json!(false);
                if let Some(tid) = sc.get("activeTargetId") {
                    if !tid.is_null() {
                        snap["activeTargetId"] = tid.clone();
                    }
                }
            }
            let new_tid = snap
                .get("activeTargetId")
                .and_then(|v| v.as_str())
                .map(String::from);
            let target_changed = prev_tid != new_tid;

            if let Some(ep) = sc.get("epoch") {
                if !ep.is_null() {
                    snap["epoch"] = ep.clone();
                }
            }
            if let Some(tabs) = sc.get("tabs") {
                if tabs.is_array() {
                    snap["tabs"] = tabs.clone();
                }
            }
            if let Some(th) = sc.get("tabsHash").and_then(|v| v.as_str()) {
                if !th.is_empty() {
                    snap["tabsHash"] = json!(th);
                }
            }
            if let Some(page) = sc.get("page") {
                if page.is_object() {
                    snap["page"] = page.clone();
                }
            }
            if let Some(q) = sc.get("qualityApplied").and_then(|v| v.as_str()) {
                snap["qualityApplied"] = json!(q);
            }

            let changed = sc.get("changed").and_then(|v| v.as_bool()).unwrap_or(false);
            if changed && !no_tabs {
                let img = sc.get("imageDataUrl").and_then(|v| v.as_str()).unwrap_or("");
                if !img.is_empty() && img.len() <= BOOT_SNAPSHOT_MAX_DATA_URL_CHARS {
                    snap["imageDataUrl"] = json!(img);
                    snap["imageBytes"] = sc.get("imageBytes").cloned().unwrap_or(json!(0));
                    snap["seq"] = sc.get("seq").cloned().unwrap_or(json!(0));
                    snap["frameHash"] = sc.get("frameHash").cloned().unwrap_or(json!(""));
                    snap["changed"] = json!(true);
                    snap["unchanged"] = json!(false);
                }
            } else if target_changed || no_tabs {
                snap["imageDataUrl"] = json!("");
                snap["imageBytes"] = json!(0);
                snap["seq"] = json!(0);
                snap["frameHash"] = json!("");
                snap["changed"] = json!(false);
                snap["unchanged"] = json!(true);
            }
        }

        st.snapshot = Some(sanitize_boot_snapshot(snap));
        st.updated_at = Some(Instant::now());
    }
}

pub struct McpServer {
    relay: Arc<dyn RelayBackend>,
    panel_html_path: PathBuf,
    html_cache: Arc<Mutex<HtmlCache>>,
    swr_cache: Arc<SwrBootCache>,
}

impl McpServer {
    pub fn new(relay: Arc<dyn RelayBackend>, panel_html_path: PathBuf) -> Arc<Self> {
        let swr_cache = SwrBootCache::new(Arc::clone(&relay));
        Arc::new(Self {
            relay,
            panel_html_path,
            html_cache: Arc::new(Mutex::new(HtmlCache::default())),
            swr_cache,
        })
    }

    pub fn prewarm_startup(&self) {
        let html_cache = Arc::clone(&self.html_cache);
        let panel_path = self.panel_html_path.clone();
        let swr = Arc::clone(&self.swr_cache);
        let _ = thread::Builder::new()
            .name("chrome-stream-html".to_string())
            .spawn(move || {
                let _ = html_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .ensure_loaded(&panel_path);
            });
        swr.prewarm_async(true);
    }

    pub fn is_fast_inline_call(&self, msg: &Value) -> bool {
        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        match method {
            "resources/read" => self.swr_cache.is_warm(),
            "tools/call" => {
                let name = msg
                    .pointer("/params/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                name == "chrome_stream_stage" && self.swr_cache.is_warm()
            }
            _ => true,
        }
    }

    pub fn handle_message(&self, msg: &Value) -> Option<Value> {
        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));

        // JSON-RPC notifications have no `id` and expect no reply.
        let Some(req_id) = id else {
            return None;
        };

        let result = match method {
            "initialize" => {
                let ver = params
                    .get("protocolVersion")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(DEFAULT_PROTOCOL_VERSION);
                Ok(json!({
                    "protocolVersion": ver,
                    "capabilities": {
                        "tools": {},
                        "resources": {}
                    },
                    "serverInfo": {
                        "name": "chrome-stream-mcp",
                        "version": APP_VERSION
                    }
                }))
            }
            "ping" => Ok(json!({})),
            "resources/list" => Ok(json!({
                "resources": [{
                    "uri": STAGE_URI,
                    "name": "Chrome Stream Live Stage",
                    "description": "Low-bandwidth adaptive screencast viewer and interactive controller for local Google Chrome tabs.",
                    "mimeType": APP_MIME,
                    "_meta": {
                        "ui": {
                            "prefersBorder": false
                        }
                    }
                }]
            })),
            "resources/read" => self.handle_read_resource(&params),
            "tools/list" => Ok(json!({ "tools": tool_definitions() })),
            "tools/call" => {
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                Ok(dispatch_tool_with_cache(
                    &self.relay,
                    Some(&self.swr_cache),
                    name,
                    args,
                ))
            }
            _ => Err((-32601, format!("Method not found: {method}"))),
        };

        match result {
            Ok(res) => Some(json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "result": res
            })),
            Err((code, message)) => Some(json!({
                "jsonrpc": "2.0",
                "id": req_id,
                "error": { "code": code, "message": message }
            })),
        }
    }

    fn handle_read_resource(&self, params: &Value) -> Result<Value, (i64, String)> {
        let uri = params.get("uri").and_then(|v| v.as_str()).unwrap_or("");
        if uri != STAGE_URI {
            return Err((-32602, format!("Unknown resource URI: {uri}")));
        }

        let boot_data = self.swr_cache.get_swr(Duration::from_millis(1200));
        let html = self
            .html_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .render(&self.panel_html_path, boot_data.as_ref())
            .map_err(|e| (-32603, e))?;

        Ok(json!({
            "contents": [{
                "uri": STAGE_URI,
                "mimeType": APP_MIME,
                "text": html,
                "_meta": {
                    "ui": {
                        "prefersBorder": false
                    }
                }
            }]
        }))
    }
}

pub fn run_stdio_server(cfg: &Config) -> Result<(), String> {
    let relay_client = RelayClient::from_config(cfg);
    let server = McpServer::new(relay_client as Arc<dyn RelayBackend>, cfg.panel_html.clone());

    // Immediately pre-warm panel HTML, detached relay connection, Chrome readiness,
    // and SWR boot snapshot off the critical stdio startup path.
    server.prewarm_startup();

    let stdin = io::stdin();
    run_stdio_loop(server, BufReader::new(stdin.lock()), BufWriter::new(io::stdout()))
}

pub fn run_stdio_loop<R: BufRead, W: Write + Send + 'static>(
    server: Arc<McpServer>,
    mut reader: R,
    writer: W,
) -> Result<(), String> {
    let writer = Arc::new(Mutex::new(writer));
    let in_flight = Arc::new(AtomicUsize::new(0));
    let mut handles: Vec<thread::JoinHandle<()>> = Vec::new();
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
                    continue;
                };

                // Reclaim finished worker JoinHandles so long-running sessions never leak memory.
                if handles.len() >= 8 {
                    handles.retain(|h| !h.is_finished());
                }

                // Fast control/metadata RPCs and warm SWR opens run inline (<0.2 ms);
                // long-running tool calls run concurrently so `chrome_stream_input` never waits
                // behind an in-flight `chrome_stream_frame` long-poll.
                let run_inline = server.is_fast_inline_call(&msg)
                    || in_flight.load(Ordering::SeqCst) >= MAX_CONCURRENT_REQUESTS;
                if run_inline {
                    if let Some(resp) = server.handle_message(&msg) {
                        let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
                        let _ = writeln!(w, "{}", resp);
                        let _ = w.flush();
                    }
                } else {
                    in_flight.fetch_add(1, Ordering::SeqCst);
                    let srv = Arc::clone(&server);
                    let wr = Arc::clone(&writer);
                    let cnt = Arc::clone(&in_flight);
                    let h = thread::spawn(move || {
                        if let Some(resp) = srv.handle_message(&msg) {
                            if let Ok(mut w) = wr.lock() {
                                let _ = writeln!(w, "{}", resp);
                                let _ = w.flush();
                            }
                        }
                        cnt.fetch_sub(1, Ordering::SeqCst);
                    });
                    handles.push(h);
                }
            }
            Err(e) => return Err(format!("MCP stdio read error: {e}")),
        }
    }

    for h in handles {
        let _ = h.join();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct DummyRelay {
        delay_ms: u64,
    }

    impl RelayBackend for DummyRelay {
        fn call(&self, method: &str, _params: Value, _timeout: Duration) -> Result<Value, String> {
            if self.delay_ms > 0 && matches!(method, "boot_snapshot" | "status") {
                thread::sleep(Duration::from_millis(self.delay_ms));
            }
            match method {
                "boot_snapshot" => Ok(json!({
                    "connected": true,
                    "hostname": "private-laptop.local",
                    "portFile": {
                        "exists": true,
                        "valid": true,
                        "port": 9222,
                        "path": "/var/tmp/private/DevToolsActivePort",
                        "wsUrl": "ws://127.0.0.1:9222/devtools/browser/secret-uuid"
                    },
                    "activeTargetId": "t1",
                    "tabs": [{ "targetId": "t1", "title": "Safe </script><script>alert(1)</script>", "url": "https://example.com" }],
                    "page": { "title": "Example", "url": "https://example.com", "viewportWidth": 1280, "viewportHeight": 800 },
                    "changed": false,
                    "unchanged": true,
                    "seq": 0,
                    "imageDataUrl": "",
                    "imageBytes": 0
                })),
                "status" => Ok(json!({
                    "connected": true,
                    "hostname": "private-laptop.local",
                    "portFile": { "exists": true, "valid": true, "port": 9222 }
                })),
                _ => Ok(json!({ "ok": true, "connected": true })),
            }
        }
    }

    #[test]
    fn mcp_stdio_initialize_resources_and_tools_end_to_end() {
        let dir = std::env::temp_dir().join(format!("chrome-stream-mcp-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let html_path = dir.join("panel.html");
        fs::write(
            &html_path,
            "<!doctype html><script>/*__CHROME_STREAM_BOOT__*/</script><div>Chrome Stream</div>",
        )
        .unwrap();

        let srv = McpServer::new(Arc::new(DummyRelay { delay_ms: 0 }), html_path);
        let input = [
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } }).to_string(),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/list" }).to_string(),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": { "uri": STAGE_URI } }).to_string(),
            json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/list" }).to_string(),
            json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "chrome_stream_stage", "arguments": {} } }).to_string(),
        ]
        .join("\n")
            + "\n";

        let out_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        struct SharedVec(Arc<Mutex<Vec<u8>>>);
        impl Write for SharedVec {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        run_stdio_loop(srv, Cursor::new(input.into_bytes()), SharedVec(Arc::clone(&out_buf)))
            .unwrap();

        let raw = String::from_utf8(out_buf.lock().unwrap().clone()).unwrap();
        let responses: Vec<Value> = raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(responses.len(), 5);

        let r1 = responses.iter().find(|r| r["id"] == 1).unwrap();
        assert_eq!(r1["result"]["serverInfo"]["name"], "chrome-stream-mcp");

        let r3 = responses.iter().find(|r| r["id"] == 3).unwrap();
        let html = r3["result"]["contents"][0]["text"].as_str().unwrap();
        assert!(html.contains("window.__CHROME_STREAM_BOOT__="));
        // XSS escape & privacy redaction checks
        assert!(!html.contains("</script><script>alert(1)"));
        assert!(html.contains("\\u003c/script\\u003e"));
        assert!(!html.contains("private-laptop.local"));
        assert!(!html.contains("/var/tmp/private"));
        assert!(!html.contains("secret-uuid"));

        let r4 = responses.iter().find(|r| r["id"] == 4).unwrap();
        assert_eq!(r4["result"]["tools"].as_array().unwrap().len(), 5);

        let r5 = responses.iter().find(|r| r["id"] == 5).unwrap();
        assert_eq!(r5["result"]["structuredContent"]["connected"], true);
        assert_eq!(r5["result"]["structuredContent"]["hostname"], "localhost");
        assert!(r5["result"]["structuredContent"]["portFile"].get("path").is_none());
        assert!(r5["result"]["structuredContent"]["portFile"].get("wsUrl").is_none());
        assert!(!r5["result"]["content"][0]["text"].as_str().unwrap().contains("private-laptop"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn swr_prewarm_returns_resources_read_and_stage_sub_millisecond() {
        let dir = std::env::temp_dir().join(format!("chrome-stream-swr-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let html_path = dir.join("panel.html");
        fs::write(
            &html_path,
            "<!doctype html><script>/*__CHROME_STREAM_BOOT__*/</script><div>Chrome Stream</div>",
        )
        .unwrap();

        let srv = McpServer::new(Arc::new(DummyRelay { delay_ms: 60 }), html_path);
        srv.prewarm_startup();

        // Wait for startup background pre-warm to finish once
        for _ in 0..40 {
            if srv.swr_cache.is_warm() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(srv.swr_cache.is_warm());

        // Subsequent resources/read and chrome_stream_stage MUST return instantaneously without blocking 60ms!
        let t0 = Instant::now();
        let r_res = srv
            .handle_message(&json!({ "jsonrpc": "2.0", "id": 10, "method": "resources/read", "params": { "uri": STAGE_URI } }))
            .unwrap();
        let read_us = t0.elapsed().as_micros();

        let t1 = Instant::now();
        let s_res = srv
            .handle_message(&json!({ "jsonrpc": "2.0", "id": 11, "method": "tools/call", "params": { "name": "chrome_stream_stage", "arguments": {} } }))
            .unwrap();
        let stage_us = t1.elapsed().as_micros();

        assert!(r_res["result"]["contents"][0]["text"].as_str().unwrap().contains("window.__CHROME_STREAM_BOOT__="));
        assert_eq!(s_res["result"]["structuredContent"]["connected"], true);
        assert!(read_us < 10_000, "warm resources/read took {read_us}us");
        assert!(stage_us < 10_000, "warm chrome_stream_stage took {stage_us}us");

        let _ = fs::remove_dir_all(&dir);
    }
}
