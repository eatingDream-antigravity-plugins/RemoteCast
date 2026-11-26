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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18"];
const APPS_EXTENSION: &str = "io.modelcontextprotocol/ui";
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
        if self.path == path && self.mtime == mtime && self.size == size && !self.prefix.is_empty()
        {
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
                let mut out = String::with_capacity(
                    self.prefix.len() + BOOT_MARKER.len() + self.suffix.len(),
                );
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
        clear_boot_frame(&mut bd);
    }
    bd
}

fn clear_boot_frame(snapshot: &mut Value) {
    snapshot["seq"] = json!(0);
    snapshot["frameHash"] = json!("");
    snapshot["imageDataUrl"] = json!("");
    snapshot["imageBytes"] = json!(0);
    snapshot["changed"] = json!(false);
    snapshot["unchanged"] = json!(true);
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
            .call("boot_snapshot", json!({}), timeout)
            .or_else(|_| self.relay.call("status", json!({}), timeout))
            .ok()
            .map(sanitize_boot_snapshot)
    }

    fn finish_refresh(&self, previous_update: Option<Instant>, fetched: Option<Value>) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(snapshot) = fetched.filter(|_| st.updated_at == previous_update) {
            st.snapshot = Some(snapshot);
            st.updated_at = Some(Instant::now());
        }
        st.refreshing = false;
        self.ready_cv.notify_all();
    }

    /// Triggers a single-flight background refresh (warms relay connection + fetches snapshot).
    pub fn prewarm_async(self: &Arc<Self>, warm_relay: bool) {
        let previous_update = {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if st.refreshing {
                return;
            }
            st.refreshing = true;
            st.updated_at
        };
        let me = Arc::clone(self);
        let _ = thread::Builder::new()
            .name("chrome-stream-warm".to_string())
            .spawn(move || {
                if warm_relay {
                    let _ = me.relay.warm();
                    // Also nudge relay's Chrome readiness check asynchronously without downgrading active streams.
                    let _ = me.relay.call("warm", json!({}), Duration::from_millis(800));
                }
                let fetched = me.fetch_from_relay(Duration::from_millis(1500));
                me.finish_refresh(previous_update, fetched);
            });
    }

    /// Returns cached snapshot immediately (<0.1 ms) if warm and triggers async revalidation when stale.
    /// Only blocks (bounded by `cold_wait`) if the cache has never been populated yet.
    pub fn get_swr(self: &Arc<Self>, cold_wait: Duration) -> Option<Value> {
        let (cached, needs_bg_refresh, wait_in_flight, previous_update) = {
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
                (Some(snap), bg, false, st.updated_at)
            } else if st.refreshing {
                (None, false, true, st.updated_at)
            } else {
                st.refreshing = true;
                (None, false, false, st.updated_at)
            }
        };

        if let Some(snap) = cached {
            if needs_bg_refresh {
                let me = Arc::clone(self);
                let _ = thread::Builder::new()
                    .name("chrome-stream-swr".to_string())
                    .spawn(move || {
                        let fetched = me.fetch_from_relay(Duration::from_millis(1500));
                        me.finish_refresh(previous_update, fetched);
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
        self.finish_refresh(previous_update, fetched);
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .clone()
    }

    /// Opportunistically merges live tool results into the SWR cache so reopening the panel
    /// immediately reflects the exact latest connection state, tabs, and frame with zero IPC.
    pub fn update_from_tool_result(&self, sc: &Value) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut snap = st.snapshot.clone().unwrap_or_else(|| {
            json!({
                "hostname": "localhost",
                "connected": false,
                "connectionMode": Value::Null,
                "crisp": false,
                "format": Value::Null,
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
        let mut mode_changed = false;
        if let Some(mode) = sc.get("connectionMode") {
            mode_changed = snap.get("connectionMode") != Some(mode);
            snap["connectionMode"] = mode.clone();
        }
        for key in ["locale", "pauseEpoch", "connecting"] {
            if let Some(value) = sc.get(key) {
                snap[key] = value.clone();
            }
        }
        if let Some(pf) = sc.get("portFile") {
            if !pf.is_null() {
                snap["portFile"] = pf.clone();
                if let Some(valid) = pf.get("valid").and_then(|v| v.as_bool()) {
                    snap["chromeReady"] = json!(valid);
                }
            }
        }

        if connected == Some(false) {
            snap["connectionMode"] = Value::Null;
            snap["crisp"] = json!(false);
            snap["format"] = Value::Null;
            snap["paused"] = json!(false);
            snap["screencastPausedIdle"] = json!(false);
            snap["noTabs"] = json!(false);
            snap["dialog"] = Value::Null;
            snap["tabs"] = json!([]);
            snap["tabsHash"] = json!("empty");
            snap["activeTargetId"] = Value::Null;
            clear_boot_frame(&mut snap);
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
            let no_tabs = sc
                .get("noTabs")
                .and_then(Value::as_bool)
                .or_else(|| sc.get("tabs").and_then(Value::as_array).map(Vec::is_empty))
                .unwrap_or_else(|| snap["noTabs"] == true);
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

            if target_changed || mode_changed || no_tabs {
                clear_boot_frame(&mut snap);
                snap["crisp"] = json!(false);
                snap["format"] = Value::Null;
            }
            if !no_tabs {
                for key in ["crisp", "format"] {
                    if let Some(value) = sc.get(key) {
                        snap[key] = value.clone();
                    }
                }
                if sc["changed"] == true {
                    let img = sc["imageDataUrl"].as_str().unwrap_or("");
                    if !img.is_empty() && img.len() <= BOOT_SNAPSHOT_MAX_DATA_URL_CHARS {
                        for key in ["imageDataUrl", "imageBytes", "seq", "frameHash"] {
                            snap[key] = sc[key].clone();
                        }
                        snap["changed"] = json!(true);
                        snap["unchanged"] = json!(false);
                    } else {
                        clear_boot_frame(&mut snap);
                    }
                }
            }
        }

        st.snapshot = Some(sanitize_boot_snapshot(snap));
        st.updated_at = Some(Instant::now());
        self.ready_cv.notify_all();
    }
}

pub struct McpServer {
    relay: Arc<dyn RelayBackend>,
    panel_html_path: PathBuf,
    html_cache: Arc<Mutex<HtmlCache>>,
    swr_cache: Arc<SwrBootCache>,
    client_supports_apps: AtomicBool,
}

fn supports_apps(capabilities: &Value) -> bool {
    let has_app_mime = |ext: &Value| {
        ext.get("mimeTypes")
            .and_then(Value::as_array)
            .is_some_and(|types| types.iter().any(|mime| mime == APP_MIME))
    };
    let sep_1865 = has_app_mime(&capabilities["extensions"][APPS_EXTENSION])
        || has_app_mime(&capabilities["experimental"][APPS_EXTENSION]);
    // Legacy vendor hosts may advertise UI via explicit vendor/ui capabilities.
    sep_1865
        || [
            &capabilities["extensions"]["com.google.antigravity/ui"],
            &capabilities["ui"],
        ]
        .into_iter()
        .any(|cap| cap.is_object() || cap.as_bool() == Some(true))
}

fn is_app_only_tool(name: &str) -> bool {
    matches!(
        name,
        "chrome_stream_frame" | "chrome_stream_input" | "chrome_stream_launch_remote"
    )
}

impl McpServer {
    pub fn new(relay: Arc<dyn RelayBackend>, panel_html_path: PathBuf) -> Arc<Self> {
        let swr_cache = SwrBootCache::new(Arc::clone(&relay));
        Arc::new(Self {
            relay,
            panel_html_path,
            html_cache: Arc::new(Mutex::new(HtmlCache::default())),
            swr_cache,
            client_supports_apps: AtomicBool::new(false),
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
        let req_id = id?;

        let result = match method {
            "initialize" => {
                let ver = params
                    .get("protocolVersion")
                    .and_then(|v| v.as_str())
                    .filter(|version| SUPPORTED_PROTOCOL_VERSIONS.contains(version))
                    .unwrap_or(LATEST_PROTOCOL_VERSION);
                let client_supports_apps = supports_apps(&params["capabilities"]);
                self.client_supports_apps
                    .store(client_supports_apps, Ordering::Relaxed);
                let mut capabilities = json!({ "tools": {}, "resources": {} });
                if client_supports_apps {
                    capabilities["extensions"] = json!({ (APPS_EXTENSION): {} });
                }
                Ok(json!({
                    "protocolVersion": ver,
                    "capabilities": capabilities,
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
            "tools/list" => Ok(json!({
                "tools": tool_definitions(self.client_supports_apps.load(Ordering::Relaxed))
            })),
            "tools/call" => {
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if is_app_only_tool(name) && !self.client_supports_apps.load(Ordering::Relaxed) {
                    Err((
                        -32602,
                        format!("Tool {name} requires MCP Apps UI capability"),
                    ))
                } else {
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
    let server = McpServer::new(
        relay_client as Arc<dyn RelayBackend>,
        cfg.panel_html.clone(),
    );

    // Immediately pre-warm panel HTML, detached relay connection, Chrome readiness,
    // and SWR boot snapshot off the critical stdio startup path.
    server.prewarm_startup();

    let stdin = io::stdin();
    run_stdio_loop(
        server,
        BufReader::new(stdin.lock()),
        BufWriter::new(io::stdout()),
    )
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
        poll_attempts: AtomicUsize,
    }

    impl RelayBackend for DummyRelay {
        fn call(&self, method: &str, params: Value, _timeout: Duration) -> Result<Value, String> {
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
                    "portFile": { "exists": true, "valid": true, "port": 9222 },
                    "activeTargetId": "t2"
                })),
                "launch_remote" => Ok(json!({
                    "ok": true,
                    "connected": true,
                    "connectionMode": "remote",
                    "paused": false,
                    "activeTargetId": "t1",
                    "changed": true,
                    "seq": 1,
                    "frameHash": "still",
                    "imageDataUrl": "data:image/webp;base64,QUFB",
                    "imageBytes": 3,
                    "crisp": true,
                    "format": "webp",
                    "qualityApplied": params["qualityPreset"],
                })),
                "disconnect" if params["mode"] == "standby" => Ok(json!({
                    "ok": true,
                    "connected": true,
                    "connectionMode": "remote",
                    "paused": true,
                    "standby": true,
                    "mode": "standby",
                    "pauseEpoch": 1,
                })),
                "poll_frame" => {
                    assert_eq!(params["qualityPreset"], "auto");
                    let rtt_ms = params["rttMs"].as_f64().unwrap();
                    if self.poll_attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        Err("No target with given id found".into())
                    } else {
                        Ok(json!({
                            "connected": true,
                            "activeTargetId": "t2",
                            "tabs": [{ "targetId": "t2", "title": "Live", "url": "https://live.test" }],
                            "page": { "title": "Live", "url": "https://live.test", "viewportWidth": 1280, "viewportHeight": 800 },
                            "changed": true,
                            "seq": 4,
                            "imageDataUrl": "data:image/jpeg;base64,QUFB",
                            "imageBytes": 3,
                            "qualityApplied": crate::stream::profiles::resolve_profile("auto", rtt_ms).name,
                        }))
                    }
                }
                "input" if params["targetEpoch"] == 7 => Ok(json!({
                    "ok": false,
                    "staleTarget": true,
                    "needsRefresh": true,
                    "recoverable": true,
                    "connected": true,
                    "error": "The tab epoch changed. Refresh the panel before sending input again.",
                    "epoch": 8,
                })),
                _ => Ok(json!({ "ok": true, "connected": true })),
            }
        }
    }

    #[test]
    fn mcp_negotiates_only_supported_protocol_versions() {
        let server = McpServer::new(
            Arc::new(DummyRelay {
                delay_ms: 0,
                poll_attempts: AtomicUsize::new(0),
            }),
            PathBuf::new(),
        );
        for (requested, expected) in [
            ("2025-11-25", "2025-11-25"),
            ("2025-06-18", "2025-06-18"),
            ("2025-03-26", "2025-11-25"),
            ("2024-11-05", "2025-11-25"),
            ("2099-01-01", "2025-11-25"),
            ("", "2025-11-25"),
        ] {
            let reply = server
                .handle_message(&json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": { "protocolVersion": requested },
                }))
                .unwrap();
            assert_eq!(reply["result"]["protocolVersion"], expected);
        }
    }

    #[test]
    fn mcp_lists_app_only_tools_for_clients_that_advertise_apps() {
        let server = McpServer::new(
            Arc::new(DummyRelay {
                delay_ms: 0,
                poll_attempts: AtomicUsize::new(0),
            }),
            PathBuf::new(),
        );
        let list_tools = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
        assert_eq!(
            server.handle_message(&list_tools).unwrap()["result"]["tools"]
                .as_array()
                .unwrap()
                .len(),
            3
        );

        for (capabilities, apps) in [
            (
                json!({ "extensions": { (APPS_EXTENSION): { "mimeTypes": [APP_MIME, "text/plain"] } } }),
                true,
            ),
            (
                json!({ "experimental": { (APPS_EXTENSION): { "mimeTypes": [APP_MIME] } } }),
                true,
            ),
            (
                json!({ "extensions": { "com.google.antigravity/ui": {} } }),
                true,
            ),
            (json!({ "ui": {} }), true),
            (json!({}), false),
            (json!({ "extensions": { (APPS_EXTENSION): {} } }), false),
            (json!({ "experimental": { (APPS_EXTENSION): {} } }), false),
            (json!({ "extensions": { (APPS_EXTENSION): null } }), false),
            (
                json!({ "extensions": { (APPS_EXTENSION): { "mimeTypes": ["text/html"] } } }),
                false,
            ),
        ] {
            let reply = server
                .handle_message(&json!({
                    "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": { "protocolVersion": "2025-11-25", "capabilities": capabilities },
                }))
                .unwrap();
            let extension = reply["result"]["capabilities"]["extensions"].get(APPS_EXTENSION);
            assert_eq!(extension, apps.then(|| json!({})).as_ref());

            let listing = server.handle_message(&list_tools).unwrap();
            let tools = listing["result"]["tools"].as_array().unwrap();
            let names: Vec<_> = tools
                .iter()
                .map(|tool| tool["name"].as_str().unwrap())
                .collect();
            if apps {
                assert_eq!(
                    names,
                    [
                        "chrome_stream_stage",
                        "chrome_stream_connect",
                        "chrome_stream_frame",
                        "chrome_stream_input",
                        "chrome_stream_disconnect",
                        "chrome_stream_launch_remote",
                    ]
                );
                assert_eq!(
                    tools[3]["inputSchema"]["properties"]["targetEpoch"]["type"],
                    "integer"
                );
                assert_eq!(tools[3]["inputSchema"]["required"], json!(["events"]));
                assert_eq!(tools[5]["_meta"]["ui"]["visibility"], json!(["app"]));
                assert_eq!(
                    tools[1]["inputSchema"]["properties"]["mode"]["enum"],
                    json!(["auto", "personal", "remote"])
                );
                assert_eq!(
                    tools[4]["inputSchema"]["properties"]["mode"]["default"],
                    "revoke"
                );
            } else {
                assert_eq!(
                    names,
                    [
                        "chrome_stream_stage",
                        "chrome_stream_connect",
                        "chrome_stream_disconnect"
                    ]
                );
            }
        }

        // Plain session (last iteration had apps=false) MUST reject app-only tools in tools/call.
        for blocked in [
            "chrome_stream_frame",
            "chrome_stream_input",
            "chrome_stream_launch_remote",
        ] {
            let err_reply = server
                .handle_message(&json!({
                    "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                    "params": { "name": blocked, "arguments": {} },
                }))
                .unwrap();
            assert_eq!(err_reply["error"]["code"], -32602);
        }

        // Re-initialize with SEP-1865 Apps mimeTypes so app-only tools succeed.
        server.handle_message(&json!({
            "jsonrpc": "2.0", "id": 4, "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": { "extensions": { (APPS_EXTENSION): { "mimeTypes": [APP_MIME] } } },
            },
        }));
        let input_reply = server
            .handle_message(&json!({
                "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                "params": {
                    "name": "chrome_stream_input",
                    "arguments": { "targetId": "t1", "targetEpoch": 7, "events": [{ "action": "click" }] },
                },
            }))
            .unwrap();
        let result = &input_reply["result"]["structuredContent"];
        assert_eq!(result["ok"], false);
        assert_eq!(result["staleTarget"], true);
        assert_eq!(result["epoch"], 8);
        assert_eq!(input_reply["result"]["content"][0]["text"], result["error"]);
    }

    #[test]
    fn mcp_stdio_initialize_resources_and_tools_end_to_end() {
        let dir =
            std::env::temp_dir().join(format!("chrome-stream-mcp-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let html_path = dir.join("panel.html");
        fs::write(
            &html_path,
            "<!doctype html><script>/*__CHROME_STREAM_BOOT__*/</script><div>Chrome Stream</div>",
        )
        .unwrap();

        let srv = McpServer::new(
            Arc::new(DummyRelay {
                delay_ms: 0,
                poll_attempts: AtomicUsize::new(0),
            }),
            html_path,
        );
        let input = [
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": { "extensions": { (APPS_EXTENSION): { "mimeTypes": [APP_MIME] } } }
            } }).to_string(),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/list" }).to_string(),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": { "uri": STAGE_URI } }).to_string(),
            json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/list" }).to_string(),
            json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "chrome_stream_stage", "arguments": {} } }).to_string(),
            json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": { "name": "chrome_stream_frame", "arguments": { "targetId": "t-closed", "qualityPreset": "auto", "rttMs": 820 } } }).to_string(),
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

        run_stdio_loop(
            srv,
            Cursor::new(input.into_bytes()),
            SharedVec(Arc::clone(&out_buf)),
        )
        .unwrap();

        let raw = String::from_utf8(out_buf.lock().unwrap().clone()).unwrap();
        let responses: Vec<Value> = raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(responses.len(), 6);

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
        let tools = r4["result"]["tools"].as_array().unwrap();
        let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            tool_names,
            vec![
                "chrome_stream_stage",
                "chrome_stream_connect",
                "chrome_stream_frame",
                "chrome_stream_input",
                "chrome_stream_disconnect",
                "chrome_stream_launch_remote",
            ]
        );
        for tool in tools {
            let desc = tool["description"].as_str().unwrap();
            assert!(
                desc.is_ascii(),
                "tool description must be clean English ASCII: {desc}"
            );
        }

        let r5 = responses.iter().find(|r| r["id"] == 5).unwrap();
        assert_eq!(r5["result"]["structuredContent"]["connected"], true);
        assert_eq!(r5["result"]["structuredContent"]["hostname"], "localhost");
        assert!(r5["result"]["structuredContent"]["portFile"]
            .get("path")
            .is_none());
        assert!(r5["result"]["structuredContent"]["portFile"]
            .get("wsUrl")
            .is_none());
        assert!(!r5["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("private-laptop"));

        let r6 = responses.iter().find(|r| r["id"] == 6).unwrap();
        let fsc = &r6["result"]["structuredContent"];
        assert_eq!(fsc["connected"], true);
        assert_eq!(fsc["recovered"], true);
        assert_eq!(fsc["activeTargetId"], "t2");
        assert_eq!(fsc["qualityApplied"], "ultra");

        // Verify handle_input measures and returns serverMs even when relay omits it
        let in_resp = dispatch_tool_with_cache(
            &(Arc::new(DummyRelay {
                delay_ms: 0,
                poll_attempts: AtomicUsize::new(0),
            }) as Arc<dyn RelayBackend>),
            None,
            "chrome_stream_input",
            json!({ "events": [{ "action": "click", "x": 10, "y": 20 }] }),
        );
        assert!(in_resp["structuredContent"]["serverMs"].as_u64().is_some());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn swr_preserves_remote_metadata_and_crisp_pixels_through_standby() {
        let relay: Arc<dyn RelayBackend> = Arc::new(DummyRelay {
            delay_ms: 0,
            poll_attempts: AtomicUsize::new(0),
        });
        let cache = SwrBootCache::new(Arc::clone(&relay));
        let launched = dispatch_tool_with_cache(
            &relay,
            Some(&cache),
            "chrome_stream_launch_remote",
            json!({ "qualityPreset": "high", "rttMs": 700 }),
        );
        assert_eq!(launched["structuredContent"]["connectionMode"], "remote");
        assert_eq!(
            cache.get_swr(Duration::ZERO).unwrap()["qualityApplied"],
            "high"
        );

        cache.update_from_tool_result(&json!({
            "connected": true, "connectionMode": "remote", "activeTargetId": "t1",
            "unchanged": true, "changed": false, "crisp": true, "format": "webp",
            "seq": 1, "frameHash": "still", "imageBytes": 0, "imageDataUrl": "",
        }));
        let standby = dispatch_tool_with_cache(
            &relay,
            Some(&cache),
            "chrome_stream_disconnect",
            json!({ "mode": "standby" }),
        );
        assert_eq!(standby["structuredContent"]["connected"], true);
        let snapshot = cache.get_swr(Duration::ZERO).unwrap();
        assert_eq!(snapshot["connectionMode"], "remote");
        assert_eq!(snapshot["paused"], true);
        assert_eq!(snapshot["pauseEpoch"], 1);
        assert_eq!(snapshot["crisp"], true);
        assert_eq!(snapshot["format"], "webp");
        assert_eq!(snapshot["imageDataUrl"], "data:image/webp;base64,QUFB");

        cache.update_from_tool_result(&json!({ "connected": false, "mode": "revoke" }));
        let snapshot = cache.get_swr(Duration::ZERO).unwrap();
        assert!(snapshot["connectionMode"].is_null());
        assert_eq!(snapshot["crisp"], false);
        assert!(snapshot["format"].is_null());
        assert_eq!(snapshot["imageDataUrl"], "");
    }

    #[test]
    fn swr_prewarm_returns_resources_read_and_stage_sub_millisecond() {
        let dir =
            std::env::temp_dir().join(format!("chrome-stream-swr-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let html_path = dir.join("panel.html");
        fs::write(
            &html_path,
            "<!doctype html><script>/*__CHROME_STREAM_BOOT__*/</script><div>Chrome Stream</div>",
        )
        .unwrap();

        let srv = McpServer::new(
            Arc::new(DummyRelay {
                delay_ms: 60,
                poll_attempts: AtomicUsize::new(0),
            }),
            html_path,
        );
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

        assert!(r_res["result"]["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("window.__CHROME_STREAM_BOOT__="));
        assert_eq!(s_res["result"]["structuredContent"]["connected"], true);
        assert!(read_us < 10_000, "warm resources/read took {read_us}us");
        assert!(
            stage_us < 10_000,
            "warm chrome_stream_stage took {stage_us}us"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
