use crate::cdp::capture::{self, FramePipeline};
use crate::cdp::discovery::{
    discover_port_file, probe_debug_port_ws_url, resolve_ws_url_candidates, PortFileStatus,
};
use crate::cdp::input::InputDispatcher;
use crate::cdp::protocol::{
    default_ws_factory, CdpTransport, TransportEvent, TransportFactory,
};
use crate::cdp::targets::{self, TabStateStore};
use crate::i18n::{chrome_not_found_error, current_lang};
use crate::stream::dedup::{hash_tabs, is_target_gone_error};
use crate::stream::profiles::{QualityProfile, TabInfo};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::cdp::capture::SCREENCAST_TIER_DWELL;
#[cfg(test)]
use crate::cdp::input::edit_command_def_for_os;

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(65);
pub(crate) const CMD_TIMEOUT: Duration = Duration::from_secs(12);
pub const BOOT_SNAPSHOT_MAX_DATA_URL_CHARS: usize = 120_000;

#[derive(Debug, Clone)]
pub struct CdpError {
    pub message: String,
    pub cdp_code: Option<i64>,
    pub method: Option<String>,
}

impl CdpError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            cdp_code: None,
            method: None,
        }
    }

    pub fn is_target_gone(&self) -> bool {
        is_target_gone_error(&self.message, self.cdp_code)
    }
}

impl std::fmt::Display for CdpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(m) = &self.method {
            write!(f, "{m}: {}", self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameRecord {
    pub seq: u64,
    pub epoch: u64,
    pub target_id: String,
    pub hash: String,
    pub image_data_url: String,
    pub image_bytes: usize,
    pub viewport_width: u32,
    pub viewport_height: u32,
    pub captured_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JsDialogInfo {
    pub target_id: String,
    #[serde(rename = "type")]
    pub dialog_type: String,
    pub message: String,
    pub default_prompt: String,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PollFrameParams {
    pub target_id: Option<String>,
    #[serde(default)]
    pub since_seq: u64,
    #[serde(default)]
    pub last_frame_hash: String,
    #[serde(default)]
    pub known_tabs_hash: String,
    #[serde(default = "default_quality_preset")]
    pub quality_preset: String,
    #[serde(default)]
    pub rtt_ms: f64,
    #[serde(default = "default_wait_ms")]
    pub wait_ms: u64,
    #[serde(default)]
    pub force_refresh: bool,
}

fn default_quality_preset() -> String {
    "auto".to_string()
}

fn default_wait_ms() -> u64 {
    350
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InputBatchParams {
    pub target_id: Option<String>,
    #[serde(default)]
    pub events: Vec<InputEvent>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InputEvent {
    pub action: String,
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub button: Option<String>,
    #[serde(default)]
    pub buttons: Option<u8>,
    #[serde(default)]
    pub click_count: Option<u8>,
    #[serde(default)]
    pub modifiers: Option<i32>,
    #[serde(default)]
    pub delta_x: Option<f64>,
    #[serde(default)]
    pub delta_y: Option<f64>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub press_enter: Option<bool>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub repeat: Option<u8>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub accept: Option<bool>,
    #[serde(default)]
    pub prompt_text: Option<String>,
}

struct PendingCall {
    tx: mpsc::Sender<Result<Value, CdpError>>,
    method: String,
}

struct TransportState {
    transport: Option<Arc<dyn CdpTransport>>,
    connecting: bool,
    last_keepalive_at: Instant,
}

impl TransportState {
    fn new() -> Self {
        Self {
            transport: None,
            connecting: false,
            last_keepalive_at: Instant::now(),
        }
    }

    fn is_open(&self) -> bool {
        self.transport.as_ref().map(|t| t.is_open()).unwrap_or(false)
    }
}

/// Facade coordinating orthogonal CDP subsystems (`TransportState`, `TabStateStore`,
/// `FramePipeline`, and `InputDispatcher`).
pub struct CdpClient {
    port_files: Vec<PathBuf>,
    debug_port: u16,
    ws_url_override: Option<String>,
    ws_factory: TransportFactory,
    connect_lock: Mutex<()>,
    transport_state: Mutex<TransportState>,
    pub(crate) tabs: Mutex<TabStateStore>,
    pub(crate) capture: Mutex<FramePipeline>,
    pending: Mutex<HashMap<u64, PendingCall>>,
    next_id: AtomicU64,
    frame_notify: (Mutex<u64>, Condvar),
    fallback_port_ready: AtomicBool,
    disposed: AtomicBool,
}

impl CdpClient {
    pub fn new_with_candidates(port_files: Vec<PathBuf>, debug_port: u16) -> Arc<Self> {
        Self::with_candidates_and_factory(port_files, debug_port, None, default_ws_factory(), true)
    }

    #[allow(dead_code)]
    pub fn new(port_file: PathBuf, debug_port: u16) -> Arc<Self> {
        Self::new_with_candidates(vec![port_file], debug_port)
    }

    pub fn with_factory(
        port_file: PathBuf,
        debug_port: u16,
        ws_url_override: Option<String>,
        ws_factory: TransportFactory,
        spawn_keepalive: bool,
    ) -> Arc<Self> {
        Self::with_candidates_and_factory(
            vec![port_file],
            debug_port,
            ws_url_override,
            ws_factory,
            spawn_keepalive,
        )
    }

    pub fn with_candidates_and_factory(
        port_files: Vec<PathBuf>,
        debug_port: u16,
        ws_url_override: Option<String>,
        ws_factory: TransportFactory,
        spawn_keepalive: bool,
    ) -> Arc<Self> {
        let client = Arc::new(Self {
            port_files,
            debug_port,
            ws_url_override,
            ws_factory,
            connect_lock: Mutex::new(()),
            transport_state: Mutex::new(TransportState::new()),
            tabs: Mutex::new(TabStateStore::new()),
            capture: Mutex::new(FramePipeline::new()),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            frame_notify: (Mutex::new(0), Condvar::new()),
            fallback_port_ready: AtomicBool::new(false),
            disposed: AtomicBool::new(false),
        });

        if spawn_keepalive {
            let weak = Arc::downgrade(&client);
            let _ = thread::Builder::new()
                .name("chrome-stream-keepalive".to_string())
                .spawn(move || {
                    loop {
                        thread::sleep(Duration::from_secs(2));
                        let Some(c) = weak.upgrade() else { break };
                        if c.disposed.load(Ordering::SeqCst) {
                            break;
                        }
                        c.keepalive_tick();
                    }
                });
        }

        client
    }

    pub fn dispose(&self) {
        self.disposed.store(true, Ordering::SeqCst);
        self.disconnect();
    }

    fn keepalive_tick(self: &Arc<Self>) {
        self.tabs.lock().unwrap().prune_closed();
        let need_ping = {
            let mut tr = self.transport_state.lock().unwrap();
            let now = Instant::now();
            let ping = tr.is_open() && now.duration_since(tr.last_keepalive_at) >= KEEPALIVE_INTERVAL;
            if ping {
                tr.last_keepalive_at = now;
            }
            ping
        };
        if need_ping {
            let _ = self.send_cdp("Browser.getVersion", json!({}), None, Duration::from_secs(5));
        }
        capture::check_idle_screencast_stop(self);
    }

    pub fn port_file_status(&self) -> PortFileStatus {
        discover_port_file(&self.port_files)
    }

    /// Background non-blocking Chrome readiness probe (DevToolsActivePort + optional loopback debug port).
    /// Never opens a Chrome WebSocket so it never triggers Chrome's Allow dialog prematurely.
    pub fn prewarm_readiness(&self) {
        let pf = self.port_file_status();
        if pf.valid {
            self.fallback_port_ready.store(false, Ordering::Relaxed);
            return;
        }
        let ok = probe_debug_port_ws_url(self.debug_port).is_some();
        self.fallback_port_ready.store(ok, Ordering::Relaxed);
    }

    /// Privacy-safe port summary for MCP/UI payloads (never leaks local home path or browser WS UUID).
    pub fn public_port_file(&self) -> Value {
        let pf = self.port_file_status();
        let mut summary = pf.public_summary();
        if !pf.valid {
            let fb_ready = self.fallback_port_ready.load(Ordering::Relaxed)
                || probe_debug_port_ws_url(self.debug_port).is_some();
            self.fallback_port_ready.store(fb_ready, Ordering::Relaxed);
            if fb_ready {
                summary["valid"] = json!(true);
                summary["stale"] = json!(false);
                summary["port"] = json!(self.debug_port);
                summary["fallback"] = json!(true);
            }
        }
        summary
    }

    pub fn is_connected(&self) -> bool {
        self.transport_state.lock().unwrap().is_open()
    }

    pub fn is_connecting(&self) -> bool {
        self.transport_state.lock().unwrap().connecting
    }

    pub fn is_paused(&self) -> bool {
        self.tabs.lock().unwrap().paused
    }

    pub fn pause_epoch(&self) -> u64 {
        self.tabs.lock().unwrap().pause_epoch
    }

    #[cfg(test)]
    pub fn is_screencast_idle_paused(&self) -> bool {
        self.capture.lock().unwrap().screencast_idle_paused
    }

    #[cfg(test)]
    pub fn rewind_capture_idle(&self, age: Duration) {
        let mut cap = self.capture.lock().unwrap();
        cap.capture_idle_at = Instant::now().checked_sub(age).unwrap_or_else(Instant::now);
    }

    #[cfg(test)]
    pub fn rewind_screencast_started_at(&self, age: Duration) {
        let mut cap = self.capture.lock().unwrap();
        cap.screencast_started_at = Some(Instant::now().checked_sub(age).unwrap_or_else(Instant::now));
    }

    pub fn pause(self: &Arc<Self>) {
        self.tabs.lock().unwrap().set_paused(true);
        self.capture.lock().unwrap().screencast_idle_paused = false;
        self.stop_screencast();
        self.notify_frame_waiters();
    }

    pub fn resume(&self) {
        self.tabs.lock().unwrap().set_paused(false);
        self.capture.lock().unwrap().notify_activity();
        self.notify_frame_waiters();
    }

    pub fn active_target_id(&self) -> Option<String> {
        self.tabs.lock().unwrap().active_target_id.clone()
    }

    pub fn active_dialog(&self) -> Value {
        self.tabs.lock().unwrap().active_dialog_value()
    }

    pub fn tab_epoch(&self) -> u64 {
        self.tabs.lock().unwrap().tab_epoch
    }

    #[cfg(test)]
    pub fn deduped_frames(&self) -> u64 {
        self.capture.lock().unwrap().deduped_frames
    }

    #[cfg(test)]
    pub fn screencast_target_id(&self) -> Option<String> {
        self.capture.lock().unwrap().screencast_target_id.clone()
    }

    #[cfg(test)]
    pub fn screencast_profile(&self) -> Option<String> {
        self.capture.lock().unwrap().screencast_profile.clone()
    }

    pub(crate) fn handle_target_gone(&self, target_id: &str, mark_closed: bool) -> bool {
        let active_changed = self
            .tabs
            .lock()
            .unwrap()
            .mark_target_gone(target_id, mark_closed);
        {
            let mut cap = self.capture.lock().unwrap();
            cap.clear_screencast_if_target(target_id);
            cap.clear_frame_if_target(target_id);
        }
        active_changed
    }

    pub fn status_snapshot(&self, relay_pid: u32, relay_version: &str) -> Value {
        let port_file = self.public_port_file();
        let chrome_ready = port_file
            .get("valid")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let (connected, connecting) = {
            let tr = self.transport_state.lock().unwrap();
            (tr.is_open(), tr.connecting)
        };
        let (paused, pause_epoch, no_tabs, active_tid, dialog, epoch) = {
            let ts = self.tabs.lock().unwrap();
            (
                connected && ts.paused,
                ts.pause_epoch,
                connected && ts.cached_tabs.is_empty() && ts.active_target_id.is_none(),
                ts.active_target_id.clone(),
                if connected {
                    ts.active_dialog_value()
                } else {
                    Value::Null
                },
                ts.tab_epoch,
            )
        };
        let (idle_paused, has_cached_frame, profile, deduped_frames) = {
            let cap = self.capture.lock().unwrap();
            (
                connected && cap.screencast_idle_paused,
                cap.latest_frame.is_some(),
                cap.screencast_profile.clone(),
                cap.deduped_frames,
            )
        };

        json!({
            "hostname": "localhost",
            "locale": current_lang().as_str(),
            "connected": connected,
            "connecting": connecting,
            "paused": paused,
            "pauseEpoch": pause_epoch,
            "screencastPausedIdle": idle_paused,
            "noTabs": no_tabs,
            "chromeReady": chrome_ready,
            "portFile": port_file,
            "activeTargetId": active_tid,
            "dialog": dialog,
            "epoch": epoch,
            "hasCachedFrame": has_cached_frame,
            "qualityProfile": profile,
            "dedupedFrames": deduped_frames,
            "relayVersion": relay_version,
            "relayPid": relay_pid,
            "checkedAt": now_iso(),
        })
    }

    /// Builds a zero-side-effect boot snapshot for SWR HTML injection.
    /// Never downgrades or restarts an already-active high/medium screencast.
    pub fn boot_snapshot(
        self: &Arc<Self>,
        relay_pid: u32,
        relay_version: &str,
        quality_preset: Option<&str>,
        rtt_ms: f64,
    ) -> Value {
        let connected = self.is_connected();
        let (paused, pause_epoch, tabs, active_tid, epoch, dialog) = {
            let ts = self.tabs.lock().unwrap();
            (
                connected && ts.paused,
                ts.pause_epoch,
                if connected {
                    ts.cached_tabs.clone()
                } else {
                    Vec::new()
                },
                ts.active_target_id.clone(),
                ts.tab_epoch,
                if connected {
                    ts.active_dialog_value()
                } else {
                    Value::Null
                },
            )
        };
        let (idle_paused, frame, profile, has_active_sc) = {
            let cap = self.capture.lock().unwrap();
            let f = if connected {
                cap.latest_frame.as_ref().and_then(|f| {
                    if active_tid.as_deref() == Some(f.target_id.as_str())
                        && f.image_data_url.len() < BOOT_SNAPSHOT_MAX_DATA_URL_CHARS
                    {
                        Some(f.clone())
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            (
                connected && cap.screencast_idle_paused,
                f,
                cap.screencast_profile
                    .clone()
                    .unwrap_or_else(|| "low".into()),
                cap.screencast_target_id.is_some(),
            )
        };

        // Only warm on cold start when NO screencast is active; never downgrade an active stream!
        if connected && !paused && !idle_paused && !tabs.is_empty() && !has_active_sc {
            let c = Arc::clone(self);
            let qp = quality_preset.map(String::from);
            let _ = thread::spawn(move || {
                c.warm_active_tab(qp.as_deref(), rtt_ms);
            });
        }

        let tab_meta = active_tid
            .as_deref()
            .and_then(|id| tabs.iter().find(|t| t.target_id == id));
        let title = tab_meta.map(|t| t.title.as_str()).unwrap_or("");
        let url = tab_meta.map(|t| t.url.as_str()).unwrap_or("");
        let t_hash = hash_tabs(&tabs);
        let port_file = self.public_port_file();
        let chrome_ready = port_file
            .get("valid")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        json!({
            "hostname": "localhost",
            "locale": current_lang().as_str(),
            "connected": connected,
            "connecting": self.is_connecting(),
            "paused": paused,
            "pauseEpoch": pause_epoch,
            "screencastPausedIdle": idle_paused,
            "noTabs": connected && tabs.is_empty() && active_tid.is_none(),
            "chromeReady": chrome_ready,
            "portFile": port_file,
            "activeTargetId": active_tid,
            "dialog": dialog,
            "epoch": epoch,
            "tabs": tabs,
            "tabsHash": t_hash,
            "page": {
                "title": title,
                "url": url,
                "viewportWidth": frame.as_ref().map(|f| f.viewport_width).unwrap_or(1280),
                "viewportHeight": frame.as_ref().map(|f| f.viewport_height).unwrap_or(800),
            },
            "changed": frame.is_some(),
            "unchanged": frame.is_none(),
            "seq": frame.as_ref().map(|f| f.seq).unwrap_or(0),
            "frameHash": frame.as_ref().map(|f| f.hash.as_str()).unwrap_or(""),
            "imageDataUrl": frame.as_ref().map(|f| f.image_data_url.as_str()).unwrap_or(""),
            "imageBytes": frame.as_ref().map(|f| f.image_bytes).unwrap_or(0),
            "qualityApplied": profile,
            "relayVersion": relay_version,
            "relayPid": relay_pid,
            "checkedAt": now_iso(),
        })
    }

    pub(crate) fn notify_frame_waiters(&self) {
        let (lock, cvar) = &self.frame_notify;
        let mut gen = lock.lock().unwrap();
        *gen = gen.wrapping_add(1);
        cvar.notify_all();
    }

    pub(crate) fn wait_for_next_frame(
        &self,
        since_seq: u64,
        timeout: Duration,
        expected_target_id: Option<&str>,
    ) {
        let deadline = Instant::now() + timeout;
        let (lock, cvar) = &self.frame_notify;
        let mut gen = lock.lock().unwrap();
        loop {
            if !self.is_connected() {
                return;
            }
            {
                let mut ts = self.tabs.lock().unwrap();
                if ts.paused {
                    return;
                }
                if let Some(tid) = expected_target_id {
                    if ts.is_recently_closed(tid)
                        || ts.has_dialog(tid)
                        || ts.active_target_id.as_deref() != Some(tid)
                    {
                        return;
                    }
                }
            }
            {
                let cap = self.capture.lock().unwrap();
                if cap.screencast_idle_paused {
                    return;
                }
                if let Some(f) = &cap.latest_frame {
                    let target_ok = expected_target_id
                        .map(|tid| f.target_id == tid)
                        .unwrap_or(true);
                    if target_ok && f.seq > since_seq {
                        return;
                    }
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return;
            }
            let wait = deadline.saturating_duration_since(now);
            let (next_gen, res) = cvar.wait_timeout(gen, wait).unwrap();
            gen = next_gen;
            if res.timed_out() {
                return;
            }
        }
    }

    pub(crate) fn wait_for_notify(&self, timeout: Duration) -> bool {
        let (lock, cvar) = &self.frame_notify;
        let gen = lock.lock().unwrap();
        let start_gen = *gen;
        let (next_gen, res) = cvar.wait_timeout(gen, timeout).unwrap();
        !res.timed_out() && *next_gen != start_gen
    }

    pub fn disconnect(&self) {
        let old_transport = {
            let mut tr = self.transport_state.lock().unwrap();
            tr.connecting = false;
            tr.transport.take()
        };
        self.tabs.lock().unwrap().reset_on_disconnect();
        self.capture.lock().unwrap().reset_on_disconnect();
        self.fail_all_pending("Chrome CDP connection closed");
        self.notify_frame_waiters();
        if let Some(t) = old_transport {
            t.close();
        }
    }

    fn fail_all_pending(&self, reason: &str) {
        let drained: Vec<PendingCall> = {
            let mut map = self.pending.lock().unwrap();
            map.drain().map(|(_, v)| v).collect()
        };
        for p in drained {
            let _ = p.tx.send(Err(CdpError::new(reason)));
        }
    }

    /// Explicitly establishes a CDP WebSocket connection to Chrome.
    /// ONLY invoked by explicit `connect` RPC (never by background polls or input batches).
    pub fn ensure_connected(self: &Arc<Self>) -> Result<(), CdpError> {
        if self.is_connected() {
            self.capture.lock().unwrap().notify_activity();
            return Ok(());
        }
        let _connect_guard = self.connect_lock.lock().unwrap();
        if self.is_connected() {
            self.capture.lock().unwrap().notify_activity();
            return Ok(());
        }

        let ws_url = self
            .ws_url_override
            .clone()
            .or_else(|| resolve_ws_url_candidates(&self.port_files, self.debug_port))
            .ok_or_else(|| CdpError::new(chrome_not_found_error(current_lang())))?;

        self.transport_state.lock().unwrap().connecting = true;

        let weak = Arc::downgrade(self);
        let on_event = Arc::new(move |ev: TransportEvent| {
            let Some(client) = weak.upgrade() else { return };
            match ev {
                TransportEvent::Text(raw) => client.handle_incoming_message(&raw),
                TransportEvent::Closed => client.handle_transport_closed(),
            }
        });

        let connect_res = (self.ws_factory)(&ws_url, CONNECT_TIMEOUT, on_event);
        match connect_res {
            Ok(transport) => {
                {
                    let mut tr = self.transport_state.lock().unwrap();
                    tr.transport = Some(Arc::clone(&transport));
                    tr.connecting = false;
                    tr.last_keepalive_at = Instant::now();
                }
                self.tabs.lock().unwrap().set_paused(false);
                self.capture.lock().unwrap().notify_activity();
                // Subscribe to target lifecycle events so external tab closures & popups sync immediately
                self.send_no_wait("Target.setDiscoverTargets", json!({ "discover": true }), None);
                Ok(())
            }
            Err(msg) => {
                self.transport_state.lock().unwrap().connecting = false;
                Err(CdpError::new(msg))
            }
        }
    }

    fn handle_transport_closed(&self) {
        let should_clean = {
            let mut tr = self.transport_state.lock().unwrap();
            if !tr.is_open() {
                tr.transport = None;
                tr.connecting = false;
                true
            } else {
                false
            }
        };
        if should_clean {
            self.tabs.lock().unwrap().reset_on_disconnect();
            self.capture.lock().unwrap().reset_on_disconnect();
            self.fail_all_pending("Chrome CDP connection closed");
            self.notify_frame_waiters();
        }
    }

    fn handle_incoming_message(&self, raw: &str) {
        let Ok(msg) = serde_json::from_str::<Value>(raw) else {
            return;
        };

        if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
            let entry = { self.pending.lock().unwrap().remove(&id) };
            if let Some(entry) = entry {
                if let Some(err_obj) = msg.get("error") {
                    let message = err_obj
                        .get("message")
                        .and_then(|m| m.as_str())
                        .map(String::from)
                        .unwrap_or_else(|| err_obj.to_string());
                    let code = err_obj.get("code").and_then(|c| c.as_i64());
                    let _ = entry.tx.send(Err(CdpError {
                        message,
                        cdp_code: code,
                        method: Some(entry.method),
                    }));
                } else {
                    let res = msg.get("result").cloned().unwrap_or_else(|| json!({}));
                    let _ = entry.tx.send(Ok(res));
                }
            }
            return;
        }

        let Some(method) = msg.get("method").and_then(|v| v.as_str()) else {
            return;
        };
        let params = msg.get("params");

        match method {
            "Page.screencastFrame" => {
                let Some(p) = params else { return };
                let sid = msg.get("sessionId").and_then(|v| v.as_str());
                let ack_id = p.get("sessionId").and_then(|v| v.as_i64());
                // Immediately ack Chrome so screencast never stalls
                if let (Some(session_id), Some(frame_ack)) = (sid, ack_id) {
                    self.send_no_wait(
                        "Page.screencastFrameAck",
                        json!({ "sessionId": frame_ack }),
                        Some(session_id),
                    );
                }

                let Some(data) = p.get("data").and_then(|v| v.as_str()) else {
                    return;
                };
                if data.is_empty() {
                    return;
                }
                let vw = p
                    .pointer("/metadata/deviceWidth")
                    .and_then(|v| v.as_f64())
                    .map(|n| n.round() as u32)
                    .filter(|&n| n > 0)
                    .unwrap_or(1280);
                let vh = p
                    .pointer("/metadata/deviceHeight")
                    .and_then(|v| v.as_f64())
                    .map(|n| n.round() as u32)
                    .filter(|&n| n > 0)
                    .unwrap_or(800);

                let target_and_epoch = {
                    let mut ts = self.tabs.lock().unwrap();
                    let tid = sid
                        .and_then(|s| ts.target_for_session(s))
                        .or_else(|| self.capture.lock().unwrap().screencast_target_id.clone());
                    tid.and_then(|t| {
                        if ts.active_target_id.as_deref() == Some(t.as_str())
                            && !ts.is_recently_closed(&t)
                        {
                            Some((t, ts.tab_epoch))
                        } else {
                            None
                        }
                    })
                };

                if let Some((tid, epoch)) = target_and_epoch {
                    let committed = self
                        .capture
                        .lock()
                        .unwrap()
                        .ingest_frame(&tid, epoch, data, vw, vh);
                    if committed {
                        self.notify_frame_waiters();
                    }
                }
            }
            "Page.javascriptDialogOpening" => {
                let Some(p) = params else { return };
                let sid = msg.get("sessionId").and_then(|v| v.as_str());
                let dtype = p
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("alert")
                    .to_string();
                let message = p
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let default_prompt = p
                    .get("defaultPrompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let url = p
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let wake = self
                    .tabs
                    .lock()
                    .unwrap()
                    .open_dialog(sid, dtype, message, default_prompt, url);
                if wake {
                    self.notify_frame_waiters();
                }
            }
            "Page.javascriptDialogClosed" => {
                let sid = msg.get("sessionId").and_then(|v| v.as_str());
                let wake = self.tabs.lock().unwrap().close_dialog_for_session(sid);
                if wake {
                    self.notify_frame_waiters();
                }
            }
            "Target.targetDestroyed" => {
                if let Some(tid) = params.and_then(|p| p.get("targetId")).and_then(|v| v.as_str()) {
                    let wake = self.handle_target_gone(tid, true);
                    if wake {
                        self.notify_frame_waiters();
                    }
                }
            }
            "Target.detachedFromTarget" => {
                let sid = params
                    .and_then(|p| p.get("sessionId"))
                    .and_then(|v| v.as_str());
                let param_tid = params
                    .and_then(|p| p.get("targetId"))
                    .and_then(|v| v.as_str());
                if let Some(tid) = self.tabs.lock().unwrap().detach_session(sid, param_tid) {
                    self.capture.lock().unwrap().clear_screencast_if_target(&tid);
                }
            }
            "Target.targetCreated" => {
                let switched = self
                    .tabs
                    .lock()
                    .unwrap()
                    .handle_target_created(params.and_then(|p| p.get("targetInfo")));
                if switched {
                    let mut cap = self.capture.lock().unwrap();
                    cap.clear_latest_frame();
                    cap.notify_activity();
                }
                self.notify_frame_waiters();
            }
            "Target.targetInfoChanged" => {
                self.tabs.lock().unwrap().invalidate_tab_cache();
            }
            _ => {}
        }
    }

    pub fn send_no_wait(&self, method: &str, params: Value, session_id: Option<&str>) {
        let transport = self.transport_state.lock().unwrap().transport.clone();
        let Some(t) = transport else { return };
        if !t.is_open() {
            return;
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut payload = json!({ "id": id, "method": method, "params": params });
        if let Some(sid) = session_id {
            payload["sessionId"] = json!(sid);
        }
        let _ = t.send_text(&payload.to_string());
    }

    pub fn send_cdp(
        self: &Arc<Self>,
        method: &str,
        params: Value,
        session_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, CdpError> {
        let transport = self
            .transport_state
            .lock()
            .unwrap()
            .transport
            .clone()
            .ok_or_else(|| CdpError::new("Chrome CDP connection closed"))?;
        if !transport.is_open() {
            return Err(CdpError::new("Chrome CDP connection closed"));
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut payload = json!({ "id": id, "method": method, "params": params });
        if let Some(sid) = session_id {
            payload["sessionId"] = json!(sid);
        }

        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(
            id,
            PendingCall {
                tx,
                method: method.to_string(),
            },
        );

        if let Err(err_msg) = transport.send_text(&payload.to_string()) {
            self.pending.lock().unwrap().remove(&id);
            return Err(CdpError::new(err_msg));
        }

        match rx.recv_timeout(timeout) {
            Ok(res) => res,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                Err(CdpError::new(format!("CDP timeout on {method}")))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.pending.lock().unwrap().remove(&id);
                Err(CdpError::new("Chrome CDP connection closed"))
            }
        }
    }

    pub fn list_tabs(self: &Arc<Self>, force: bool) -> Result<Vec<TabInfo>, CdpError> {
        targets::list_tabs(self, force)
    }

    pub fn resolve_target_id(
        self: &Arc<Self>,
        requested_id: Option<&str>,
        force_tabs: bool,
        update_active: bool,
    ) -> Result<(Option<String>, Vec<TabInfo>), CdpError> {
        targets::resolve_target_id(self, requested_id, force_tabs, update_active)
    }

    pub fn attach(self: &Arc<Self>, target_id: &str) -> Result<String, CdpError> {
        targets::attach(self, target_id)
    }

    pub fn stop_screencast(self: &Arc<Self>) {
        capture::stop_screencast(self);
    }

    pub fn ensure_screencast(
        self: &Arc<Self>,
        target_id: &str,
        quality_preset: &str,
        rtt_ms: f64,
    ) -> Result<(String, &'static QualityProfile), CdpError> {
        capture::ensure_screencast(self, target_id, quality_preset, rtt_ms, false)
    }

    pub fn warm_active_tab(self: &Arc<Self>, quality_preset: Option<&str>, rtt_ms: f64) {
        capture::warm_active_tab(self, quality_preset, rtt_ms);
    }

    pub fn close_tab(
        self: &Arc<Self>,
        target_id_to_close: Option<&str>,
        quality_preset: Option<&str>,
    ) -> Result<Value, CdpError> {
        targets::close_tab(self, target_id_to_close, quality_preset)
    }

    pub fn poll_frame(self: &Arc<Self>, params: PollFrameParams) -> Result<Value, CdpError> {
        capture::poll_frame(self, params)
    }

    pub fn dispatch_input_batch(
        self: &Arc<Self>,
        params: InputBatchParams,
    ) -> Result<Value, CdpError> {
        InputDispatcher::dispatch_batch(self, params)
    }
}

pub(crate) fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::cdp::protocol::EventCallback;
    use crate::stream::dedup::base64_encode;
    use crate::stream::profiles::{PROFILE_HIGH, PROFILE_ULTRA};
    use std::collections::HashSet;
    use std::sync::atomic::AtomicUsize;

    struct MockState {
        tabs: Vec<TabInfo>,
        sent: Vec<Value>,
        session_to_target: HashMap<String, String>,
        next_sid: u64,
        next_tab: u64,
        shot_counter: u64,
        fail_attach_once: HashSet<String>,
        fail_click_for: HashSet<String>,
        fail_screencast_for: HashSet<String>,
        fail_eval_for: HashSet<String>,
        layout_metrics_vp: Option<(u32, u32)>,
        auto_screencast_b64: Option<String>,
    }

    struct MockTransport {
        open: AtomicBool,
        state: Mutex<MockState>,
        on_event: EventCallback,
    }

    impl MockTransport {
        fn new(tabs: Vec<TabInfo>, on_event: EventCallback) -> Arc<Self> {
            Arc::new(Self {
                open: AtomicBool::new(true),
                state: Mutex::new(MockState {
                    tabs,
                    sent: Vec::new(),
                    session_to_target: HashMap::new(),
                    next_sid: 1,
                    next_tab: 100,
                    shot_counter: 0,
                    fail_attach_once: HashSet::new(),
                    fail_click_for: HashSet::new(),
                    fail_screencast_for: HashSet::new(),
                    fail_eval_for: HashSet::new(),
                    layout_metrics_vp: None,
                    auto_screencast_b64: None,
                }),
                on_event,
            })
        }

        fn push_event(&self, method: &str, params: Value, session_id: Option<&str>) {
            let mut msg = json!({ "method": method, "params": params });
            if let Some(sid) = session_id {
                msg["sessionId"] = json!(sid);
            }
            (self.on_event)(TransportEvent::Text(msg.to_string()));
        }

        fn reply(&self, id: u64, result: Value) {
            (self.on_event)(TransportEvent::Text(
                json!({ "id": id, "result": result }).to_string(),
            ));
        }

        fn error(&self, id: u64, code: i64, message: &str) {
            (self.on_event)(TransportEvent::Text(
                json!({ "id": id, "error": { "code": code, "message": message } }).to_string(),
            ));
        }
    }

    impl CdpTransport for MockTransport {
        fn send_text(&self, text: &str) -> Result<(), String> {
            if !self.is_open() {
                return Err("Chrome CDP connection closed".to_string());
            }
            let msg: Value = serde_json::from_str(text).unwrap();
            let id = msg.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
            let method = msg
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
            let session_id = msg
                .get("sessionId")
                .and_then(|v| v.as_str())
                .map(String::from);

            let mut post_events: Vec<(String, Value, Option<String>)> = Vec::new();
            let reply_val: Result<Value, (i64, String)> = {
                let mut st = self.state.lock().unwrap();
                st.sent.push(msg);

                match method.as_str() {
                    "Target.setDiscoverTargets"
                    | "Browser.getVersion"
                    | "Page.enable"
                    | "Page.handleJavaScriptDialog"
                    | "Page.screencastFrameAck"
                    | "Page.stopScreencast"
                    | "Input.dispatchKeyEvent"
                    | "Input.insertText"
                    | "Page.reload" => Ok(json!({})),

                    "Target.getTargets" => {
                        let infos: Vec<Value> = st
                            .tabs
                            .iter()
                            .map(|t| {
                                json!({
                                    "targetId": t.target_id,
                                    "type": "page",
                                    "title": t.title,
                                    "url": t.url,
                                })
                            })
                            .collect();
                        Ok(json!({ "targetInfos": infos }))
                    }

                    "Target.createTarget" => {
                        st.next_tab += 1;
                        let tid = format!("tab-new-{}", st.next_tab);
                        let url = params
                            .get("url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("about:blank")
                            .to_string();
                        st.tabs.push(TabInfo {
                            target_id: tid.clone(),
                            title: if url == "about:blank" {
                                "New Tab".into()
                            } else {
                                url.clone()
                            },
                            url,
                        });
                        Ok(json!({ "targetId": tid }))
                    }

                    "Target.activateTarget" => {
                        let tid = params
                            .get("targetId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if st.tabs.iter().any(|t| t.target_id == tid) {
                            Ok(json!({}))
                        } else {
                            Err((-32000, "No target with given id found".into()))
                        }
                    }

                    "Target.closeTarget" => {
                        let tid = params
                            .get("targetId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let before = st.tabs.len();
                        st.tabs.retain(|t| t.target_id != tid);
                        if st.tabs.len() == before {
                            Err((-32000, "No target with given id found".into()))
                        } else {
                            post_events.push((
                                "Target.targetDestroyed".into(),
                                json!({ "targetId": tid }),
                                None,
                            ));
                            Ok(json!({ "success": true }))
                        }
                    }

                    "Target.attachToTarget" => {
                        let tid = params
                            .get("targetId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if st.fail_attach_once.remove(tid) {
                            st.tabs.retain(|t| t.target_id != tid);
                            Err((-32000, "No target with given id found".into()))
                        } else if st.tabs.iter().any(|t| t.target_id == tid) {
                            let sid = format!("sid-{}", st.next_sid);
                            st.next_sid += 1;
                            st.session_to_target.insert(sid.clone(), tid.to_string());
                            Ok(json!({ "sessionId": sid }))
                        } else {
                            Err((-32000, "No target with given id found".into()))
                        }
                    }

                    "Page.startScreencast" => {
                        let tid = session_id
                            .as_deref()
                            .and_then(|s| st.session_to_target.get(s))
                            .cloned()
                            .unwrap_or_default();
                        if !st.tabs.iter().any(|t| t.target_id == tid) {
                            Err((-32000, "No target with given id found".into()))
                        } else if st.fail_screencast_for.contains(&tid) {
                            Err((-32000, "Cannot capture chrome:// page".into()))
                        } else {
                            if let Some(b64) = st.auto_screencast_b64.clone() {
                                post_events.push((
                                    "Page.screencastFrame".into(),
                                    json!({
                                        "data": b64,
                                        "metadata": { "deviceWidth": 1280, "deviceHeight": 800 },
                                        "sessionId": 1,
                                    }),
                                    session_id.clone(),
                                ));
                            }
                            Ok(json!({}))
                        }
                    }

                    "Runtime.evaluate" => {
                        let tid = session_id
                            .as_deref()
                            .and_then(|s| st.session_to_target.get(s))
                            .cloned()
                            .unwrap_or_default();
                        if st.fail_eval_for.contains(&tid) {
                            Err((-32000, "Cannot evaluate on chrome:// page".into()))
                        } else {
                            Ok(json!({ "result": { "value": "[1440,900]" } }))
                        }
                    }

                    "Page.getLayoutMetrics" => {
                        let (w, h) = st.layout_metrics_vp.unwrap_or((1366, 768));
                        Ok(json!({
                            "cssVisualViewport": { "clientWidth": w, "clientHeight": h },
                            "cssLayoutViewport": { "clientWidth": w, "clientHeight": h },
                        }))
                    }

                    "Page.captureScreenshot" => {
                        st.shot_counter += 1;
                        let n = st.shot_counter;
                        let tid = session_id
                            .as_deref()
                            .and_then(|s| st.session_to_target.get(s))
                            .cloned()
                            .unwrap_or_default();
                        let q = params.get("quality").and_then(|v| v.as_u64()).unwrap_or(50);
                        let raw = format!("shot:{tid}:{q}:{n}");
                        Ok(json!({ "data": base64_encode(raw.as_bytes()) }))
                    }

                    "Input.dispatchMouseEvent" => {
                        let tid = session_id
                            .as_deref()
                            .and_then(|s| st.session_to_target.get(s))
                            .cloned()
                            .unwrap_or_default();
                        if st.fail_click_for.contains(&tid)
                            || !st.tabs.iter().any(|t| t.target_id == tid)
                        {
                            st.tabs.retain(|t| t.target_id != tid);
                            Err((-32000, "No target with given id found".into()))
                        } else {
                            Ok(json!({}))
                        }
                    }

                    _ => Ok(json!({})),
                }
            };

            match reply_val {
                Ok(v) => self.reply(id, v),
                Err((code, msg)) => self.error(id, code, &msg),
            }
            for (m, p, s) in post_events {
                self.push_event(&m, p, s.as_deref());
            }
            Ok(())
        }

        fn is_open(&self) -> bool {
            self.open.load(Ordering::SeqCst)
        }

        fn close(&self) {
            if self.open.swap(false, Ordering::SeqCst) {
                (self.on_event)(TransportEvent::Closed);
            }
        }
    }

    fn make_unconnected_mock_client(
        tabs: Vec<TabInfo>,
    ) -> (
        Arc<CdpClient>,
        Arc<Mutex<Option<Arc<MockTransport>>>>,
        Arc<AtomicUsize>,
    ) {
        let holder: Arc<Mutex<Option<Arc<MockTransport>>>> = Arc::new(Mutex::new(None));
        let dials = Arc::new(AtomicUsize::new(0));
        let h_clone = Arc::clone(&holder);
        let d_clone = Arc::clone(&dials);
        let factory: TransportFactory = Arc::new(move |_url, _timeout, on_event| {
            d_clone.fetch_add(1, Ordering::SeqCst);
            let t = MockTransport::new(tabs.clone(), on_event);
            *h_clone.lock().unwrap() = Some(Arc::clone(&t));
            Ok(t as Arc<dyn CdpTransport>)
        });
        let client = CdpClient::with_factory(
            PathBuf::from("/tmp/nonexistent-port-file"),
            9222,
            Some("ws://127.0.0.1:9222/devtools/browser/mock".into()),
            factory,
            false,
        );
        (client, holder, dials)
    }

    fn make_mock_client(
        tabs: Vec<TabInfo>,
    ) -> (Arc<CdpClient>, Arc<Mutex<Option<Arc<MockTransport>>>>) {
        let (client, holder, _) = make_unconnected_mock_client(tabs);
        client.ensure_connected().unwrap();
        (client, holder)
    }

    fn tab(id: &str, title: &str, url: &str) -> TabInfo {
        TabInfo {
            target_id: id.into(),
            title: title.into(),
            url: url.into(),
        }
    }

    #[test]
    fn never_auto_dials_chrome_in_poll_frame_or_input_after_disconnect() {
        let (client, _holder, dials) =
            make_unconnected_mock_client(vec![tab("t1", "Home", "https://home.test")]);
        assert_eq!(dials.load(Ordering::SeqCst), 0);

        let f0 = client
            .poll_frame(PollFrameParams {
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(f0["connected"], false);
        assert_eq!(dials.load(Ordering::SeqCst), 0);

        let in0 = client
            .dispatch_input_batch(InputBatchParams {
                target_id: Some("t1".into()),
                events: vec![InputEvent {
                    action: "click".into(),
                    x: Some(100.0),
                    y: Some(120.0),
                    ..Default::default()
                }],
            })
            .unwrap();
        assert_eq!(in0["connected"], false);
        assert_eq!(dials.load(Ordering::SeqCst), 0);

        client.ensure_connected().unwrap();
        assert_eq!(dials.load(Ordering::SeqCst), 1);

        let f1 = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(f1["connected"], true);

        client.disconnect();
        let f2 = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(f2["connected"], false);
        assert_eq!(dials.load(Ordering::SeqCst), 1);
        client.dispose();
    }

    #[test]
    fn tracks_and_resolves_javascript_modal_dialogs_without_deadlocking_poll() {
        let (client, holder) = make_mock_client(vec![tab("t1", "App", "https://app.test")]);
        client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 20,
                ..Default::default()
            })
            .unwrap();

        let sid = client.tabs.lock().unwrap().session_for("t1").unwrap();
        let mock = Arc::clone(holder.lock().unwrap().as_ref().unwrap());
        assert!(mock
            .state
            .lock()
            .unwrap()
            .sent
            .iter()
            .any(|m| m["method"] == "Page.enable"));

        mock.push_event(
            "Page.javascriptDialogOpening",
            json!({
                "type": "prompt",
                "message": "Enter verification code:",
                "defaultPrompt": "000000",
                "url": "https://app.test/login"
            }),
            Some(&sid),
        );

        let poll_with_dlg = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 200,
                force_refresh: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(poll_with_dlg["dialog"]["type"], "prompt");
        assert_eq!(poll_with_dlg["dialog"]["message"], "Enter verification code:");
        assert_eq!(poll_with_dlg["dialog"]["defaultPrompt"], "000000");

        let res = client
            .dispatch_input_batch(InputBatchParams {
                target_id: Some("t1".into()),
                events: vec![InputEvent {
                    action: "dialog".into(),
                    accept: Some(true),
                    prompt_text: Some("123456".into()),
                    ..Default::default()
                }],
            })
            .unwrap();
        assert!(res["dialog"].is_null());
        assert!(res["serverMs"].as_u64().is_some());

        let sent = mock.state.lock().unwrap().sent.clone();
        let dlg_cmd = sent
            .iter()
            .find(|m| m["method"] == "Page.handleJavaScriptDialog")
            .expect("expected Page.handleJavaScriptDialog");
        assert_eq!(dlg_cmd["params"]["accept"], true);
        assert_eq!(dlg_cmd["params"]["promptText"], "123456");
        client.dispose();
    }

    #[test]
    fn stops_screencast_after_12s_idle_even_while_polling_and_resumes_on_input() {
        let (client, holder) = make_mock_client(vec![tab("t1", "Feed", "https://feed.test")]);
        client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 20,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(client.screencast_target_id().as_deref(), Some("t1"));
        assert!(!client.is_screencast_idle_paused());

        client.rewind_capture_idle(Duration::from_secs(13));
        let idle_poll = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 20,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(idle_poll["screencastPausedIdle"], true);
        assert!(client.screencast_target_id().is_none());
        let mock = Arc::clone(holder.lock().unwrap().as_ref().unwrap());
        assert!(mock
            .state
            .lock()
            .unwrap()
            .sent
            .iter()
            .any(|m| m["method"] == "Page.stopScreencast"));

        let wake = client
            .dispatch_input_batch(InputBatchParams {
                target_id: Some("t1".into()),
                events: vec![InputEvent {
                    action: "scroll".into(),
                    x: Some(400.0),
                    y: Some(300.0),
                    delta_y: Some(120.0),
                    ..Default::default()
                }],
            })
            .unwrap();
        assert_eq!(wake["screencastPausedIdle"], false);
        assert_eq!(client.screencast_target_id().as_deref(), Some("t1"));
        client.dispose();
    }

    #[test]
    fn auto_follows_oauth_popups_and_periodically_refreshes_chrome_urls() {
        let (client, holder) = make_mock_client(vec![tab("t1", "Login", "https://login.test")]);
        client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 20,
                ..Default::default()
            })
            .unwrap();

        let mock = Arc::clone(holder.lock().unwrap().as_ref().unwrap());
        mock.state
            .lock()
            .unwrap()
            .tabs
            .push(tab("t-oauth", "OAuth", "chrome://inspect"));
        mock.state
            .lock()
            .unwrap()
            .fail_screencast_for
            .insert("t-oauth".into());
        mock.push_event(
            "Target.targetCreated",
            json!({
                "targetInfo": {
                    "targetId": "t-oauth",
                    "type": "page",
                    "title": "OAuth",
                    "url": "chrome://inspect",
                    "openerId": "t1"
                }
            }),
            None,
        );

        let f_pop = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 20,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(f_pop["activeTargetId"], "t-oauth");
        assert_eq!(f_pop["popupSwitched"], true);
        let h1 = f_pop["frameHash"].as_str().unwrap().to_string();
        assert!(!h1.is_empty());

        client.capture.lock().unwrap().last_fallback_shot_at =
            Some(Instant::now() - Duration::from_secs(2));
        let f_next = client
            .poll_frame(PollFrameParams {
                target_id: Some("t-oauth".into()),
                last_frame_hash: h1.clone(),
                quality_preset: "low".into(),
                wait_ms: 20,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(f_next["changed"], true);
        assert_ne!(f_next["frameHash"].as_str().unwrap(), h1);
        client.dispose();
    }

    #[test]
    fn dispatches_clean_dblclick_drag_shortcuts_and_modifiers() {
        let (client, holder) = make_mock_client(vec![tab("t1", "Editor", "https://editor.test")]);
        client
            .dispatch_input_batch(InputBatchParams {
                target_id: Some("t1".into()),
                events: vec![
                    InputEvent {
                        action: "click".into(),
                        x: Some(100.0),
                        y: Some(200.0),
                        button: Some("left".into()),
                        click_count: Some(2),
                        ..Default::default()
                    },
                    InputEvent {
                        action: "mousedown".into(),
                        x: Some(10.0),
                        y: Some(20.0),
                        button: Some("left".into()),
                        buttons: Some(1),
                        ..Default::default()
                    },
                    InputEvent {
                        action: "mousemove".into(),
                        x: Some(80.0),
                        y: Some(90.0),
                        button: Some("left".into()),
                        buttons: Some(1),
                        ..Default::default()
                    },
                    InputEvent {
                        action: "mouseup".into(),
                        x: Some(80.0),
                        y: Some(90.0),
                        button: Some("left".into()),
                        buttons: Some(0),
                        ..Default::default()
                    },
                    InputEvent {
                        action: "key".into(),
                        key: Some("Enter".into()),
                        modifiers: Some(8),
                        ..Default::default()
                    },
                    InputEvent {
                        action: "key".into(),
                        key: Some("Undo".into()),
                        ..Default::default()
                    },
                    InputEvent {
                        action: "key".into(),
                        key: Some("Redo".into()),
                        ..Default::default()
                    },
                    InputEvent {
                        action: "key".into(),
                        key: Some("Cut".into()),
                        ..Default::default()
                    },
                    InputEvent {
                        action: "key".into(),
                        key: Some("SelectAll".into()),
                        ..Default::default()
                    },
                    InputEvent {
                        action: "key".into(),
                        key: Some("Copy".into()),
                        ..Default::default()
                    },
                    InputEvent {
                        action: "key".into(),
                        key: Some("Paste".into()),
                        ..Default::default()
                    },
                ],
            })
            .unwrap();

        let sent = holder.lock().unwrap().as_ref().unwrap().state.lock().unwrap().sent.clone();
        let dbl_presses: Vec<&Value> = sent
            .iter()
            .filter(|m| {
                m["method"] == "Input.dispatchMouseEvent"
                    && m["params"]["type"] == "mousePressed"
                    && m["params"]["x"] == 100
            })
            .collect();
        assert_eq!(dbl_presses.len(), 1);
        assert_eq!(dbl_presses[0]["params"]["clickCount"], 2);

        assert!(sent.iter().any(|m| {
            m["method"] == "Input.dispatchMouseEvent"
                && m["params"]["type"] == "mouseMoved"
                && m["params"]["x"] == 80
                && m["params"]["buttons"] == 1
        }));
        assert!(sent.iter().any(|m| {
            m["method"] == "Input.dispatchKeyEvent"
                && m["params"]["type"] == "rawKeyDown"
                && m["params"]["key"] == "Enter"
                && m["params"]["modifiers"] == 8
        }));
        for cmd in ["undo", "redo", "cut", "selectAll", "copy", "paste"] {
            assert!(sent.iter().any(|m| {
                m["method"] == "Input.dispatchKeyEvent"
                    && m["params"]["commands"]
                        .as_array()
                        .map(|a| a.iter().any(|v| v == cmd))
                        .unwrap_or(false)
            }));
        }
        assert_eq!(edit_command_def_for_os("Undo", 4).unwrap().3, 4);
        assert_eq!(edit_command_def_for_os("Undo", 2).unwrap().3, 2);
        assert_eq!(edit_command_def_for_os("Redo", 2).unwrap().3, 10);
        client.dispose();
    }

    #[test]
    fn tab_close_lifecycle_handles_non_active_double_active_and_last_tab() {
        let (client, holder) = make_mock_client(vec![
            tab("t1", "One", "https://one.test"),
            tab("t2", "Two", "https://two.test"),
            tab("t3", "Three", "https://three.test"),
        ]);
        let f1 = client
            .poll_frame(PollFrameParams {
                target_id: Some("t2".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(client.active_target_id().as_deref(), Some("t2"));
        assert_eq!(client.screencast_target_id().as_deref(), Some("t2"));

        let r_non_active = client.close_tab(Some("t1"), None).unwrap();
        assert_eq!(r_non_active["connected"], true);
        assert_eq!(r_non_active["activeTargetId"], "t2");
        assert_eq!(client.screencast_target_id().as_deref(), Some("t2"));
        assert_eq!(r_non_active["seq"], f1["seq"]);

        let r_dup = client.close_tab(Some("t-nonexistent"), None).unwrap();
        assert_eq!(r_dup["connected"], true);
        assert_eq!(r_dup["tabs"].as_array().unwrap().len(), 2);
        assert_eq!(r_dup["activeTargetId"], "t2");

        let r_active = client
            .dispatch_input_batch(InputBatchParams {
                target_id: Some("t2".into()),
                events: vec![InputEvent {
                    action: "close_tab".into(),
                    target_id: Some("t2".into()),
                    ..Default::default()
                }],
            })
            .unwrap();
        assert_eq!(r_active["connected"], true);
        assert_eq!(r_active["activeTargetId"], "t3");
        let ids: Vec<&str> = r_active["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["targetId"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["t3"]);
        assert_eq!(client.screencast_target_id().as_deref(), Some("t3"));
        let sent = holder.lock().unwrap().as_ref().unwrap().state.lock().unwrap().sent.clone();
        assert!(sent.iter().any(|m| m["method"] == "Page.stopScreencast"));
        assert!(sent
            .iter()
            .any(|m| m["method"] == "Target.activateTarget" && m["params"]["targetId"] == "t3"));

        let r_last = client.close_tab(Some("t3"), None).unwrap();
        assert_eq!(r_last["connected"], true);
        assert_eq!(r_last["noTabs"], true);
        assert!(r_last["activeTargetId"].is_null());
        assert!(r_last["tabs"].as_array().unwrap().is_empty());
        assert!(client.screencast_target_id().is_none());
        let sent_after = holder
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .state
            .lock()
            .unwrap()
            .sent
            .clone();
        assert!(!sent_after.iter().any(|m| m["method"] == "Target.createTarget"));
        client.dispose();
    }

    #[test]
    fn recovers_from_no_target_with_given_id_found_without_disconnecting() {
        let (client, holder) = make_mock_client(vec![
            tab("t1", "Dying", "https://dying.test"),
            tab("t2", "Live", "https://live.test"),
        ]);
        holder
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .state
            .lock()
            .unwrap()
            .fail_attach_once
            .insert("t1".into());

        let frame = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(frame["connected"], true);
        assert_eq!(frame["recovered"], true);
        assert_eq!(frame["activeTargetId"], "t2");
        assert!(client.is_connected());

        holder
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .state
            .lock()
            .unwrap()
            .fail_click_for
            .insert("t2".into());
        let inp = client
            .dispatch_input_batch(InputBatchParams {
                target_id: Some("t2".into()),
                events: vec![InputEvent {
                    action: "click".into(),
                    x: Some(50.0),
                    y: Some(60.0),
                    button: Some("left".into()),
                    click_count: Some(1),
                    ..Default::default()
                }],
            })
            .unwrap();
        assert_eq!(inp["ok"], true);
        assert_eq!(inp["connected"], true);
        assert_eq!(inp["recovered"], true);
        assert!(client.is_connected());
        client.dispose();
    }

    #[test]
    fn deduplicates_identical_frames_and_elides_unchanged_tabs() {
        let (client, holder) = make_mock_client(vec![tab("t1", "Doc", "https://doc.test")]);
        let same_b64 = base64_encode(b"identical-jpeg-frame-bytes");
        let diff_b64 = base64_encode(b"changed-jpeg-frame-bytes");
        let mock = Arc::clone(holder.lock().unwrap().as_ref().unwrap());
        mock.state.lock().unwrap().auto_screencast_b64 = Some(same_b64.clone());

        let f1 = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(f1["changed"], true);
        assert_eq!(f1["unchanged"], false);
        let h1 = f1["frameHash"].as_str().unwrap().to_string();
        let th1 = f1["tabsHash"].as_str().unwrap().to_string();
        let seq1 = f1["seq"].as_u64().unwrap();

        let sid = client.tabs.lock().unwrap().session_for("t1").unwrap();
        mock.push_event(
            "Page.screencastFrame",
            json!({
                "data": same_b64,
                "metadata": { "deviceWidth": 1280, "deviceHeight": 800 },
                "sessionId": 2,
            }),
            Some(&sid),
        );
        assert_eq!(client.deduped_frames(), 1);

        let f2 = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                since_seq: seq1,
                last_frame_hash: h1.clone(),
                known_tabs_hash: th1,
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(f2["changed"], false);
        assert_eq!(f2["unchanged"], true);
        assert_eq!(f2["imageDataUrl"], "");
        assert_eq!(f2["imageBytes"], 0);
        assert!(f2.get("tabs").is_none());

        mock.push_event(
            "Page.screencastFrame",
            json!({
                "data": diff_b64,
                "metadata": { "deviceWidth": 1280, "deviceHeight": 800 },
                "sessionId": 3,
            }),
            Some(&sid),
        );
        let f3 = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                since_seq: seq1,
                last_frame_hash: h1.clone(),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(f3["changed"], true);
        assert_ne!(f3["frameHash"].as_str().unwrap(), h1);
        client.dispose();
    }

    #[test]
    fn applies_ultra_profile_and_scales_fallback_screenshot() {
        let (client, holder) = make_mock_client(vec![tab("t1", "Heavy", "https://heavy.test")]);
        client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "auto".into(),
                rtt_ms: 900.0,
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        let sent = holder.lock().unwrap().as_ref().unwrap().state.lock().unwrap().sent.clone();
        let sc = sent.iter().find(|m| m["method"] == "Page.startScreencast").unwrap();
        assert_eq!(sc["params"]["quality"], PROFILE_ULTRA.quality);
        assert_eq!(sc["params"]["maxWidth"], PROFILE_ULTRA.max_width);
        assert_eq!(sc["params"]["everyNthFrame"], 3);

        let shot = sent.iter().find(|m| m["method"] == "Page.captureScreenshot").unwrap();
        assert!(shot["params"]["clip"]["scale"].as_f64().unwrap() < 1.0);
        client.dispose();
    }

    #[test]
    fn drops_late_screencast_frame_from_previous_target_while_acking_chrome() {
        let (client, holder) = make_mock_client(vec![
            tab("t1", "One", "https://one.test"),
            tab("t2", "Two", "https://two.test"),
        ]);
        client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        let old_sid = client.tabs.lock().unwrap().session_for("t1").unwrap();
        client
            .dispatch_input_batch(InputBatchParams {
                target_id: Some("t1".into()),
                events: vec![InputEvent {
                    action: "switch_tab".into(),
                    target_id: Some("t2".into()),
                    ..Default::default()
                }],
            })
            .unwrap();
        let seq_before = client.capture.lock().unwrap().frame_seq;

        let mock = Arc::clone(holder.lock().unwrap().as_ref().unwrap());
        mock.push_event(
            "Page.screencastFrame",
            json!({
                "data": base64_encode(b"stale-frame-from-t1"),
                "metadata": { "deviceWidth": 1280, "deviceHeight": 800 },
                "sessionId": 77,
            }),
            Some(&old_sid),
        );

        assert_eq!(client.capture.lock().unwrap().frame_seq, seq_before);
        assert!(client.capture.lock().unwrap().latest_frame.is_none());
        let sent = mock.state.lock().unwrap().sent.clone();
        assert!(sent
            .iter()
            .any(|m| m["method"] == "Page.screencastFrameAck" && m["params"]["sessionId"] == 77));
        client.dispose();
    }

    #[test]
    fn external_last_tab_destruction_enters_no_tabs_state_and_resumes_on_new_tab() {
        let (client, holder) = make_mock_client(vec![tab("t1", "Only", "https://only.test")]);
        client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        let mock = Arc::clone(holder.lock().unwrap().as_ref().unwrap());
        mock.state.lock().unwrap().tabs.clear();
        mock.push_event("Target.targetDestroyed", json!({ "targetId": "t1" }), None);

        let empty = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(empty["connected"], true);
        assert_eq!(empty["noTabs"], true);
        assert!(empty["activeTargetId"].is_null());
        assert!(empty["tabs"].as_array().unwrap().is_empty());
        assert!(client.is_connected());
        let sent = mock.state.lock().unwrap().sent.clone();
        assert!(!sent.iter().any(|m| m["method"] == "Target.createTarget"));

        mock.state
            .lock()
            .unwrap()
            .tabs
            .push(tab("t2", "Fresh", "https://fresh.test"));
        mock.push_event(
            "Target.targetCreated",
            json!({ "targetInfo": { "targetId": "t2", "type": "page" } }),
            None,
        );

        let resumed = client
            .poll_frame(PollFrameParams {
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(resumed["connected"], true);
        assert_eq!(resumed["noTabs"], false);
        assert_eq!(resumed["activeTargetId"], "t2");
        assert_eq!(resumed["page"]["url"], "https://fresh.test");
        client.dispose();
    }

    #[test]
    fn pause_stops_screencast_bumps_pause_epoch_and_resume_restarts() {
        let (client, holder) = make_mock_client(vec![tab("t1", "Live", "https://live.test")]);
        let initial = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(initial["pauseEpoch"], 0);
        assert_eq!(client.screencast_target_id().as_deref(), Some("t1"));

        let res = client
            .dispatch_input_batch(InputBatchParams {
                target_id: Some("t1".into()),
                events: vec![InputEvent {
                    action: "pause".into(),
                    ..Default::default()
                }],
            })
            .unwrap();
        assert_eq!(res["connected"], true);
        assert_eq!(res["paused"], true);
        assert_eq!(res["pauseEpoch"], 1);
        assert!(client.is_connected());
        assert!(client.is_paused());
        assert!(client.screencast_target_id().is_none());

        // Idempotent second pause keeps pause_epoch stable
        client.pause();
        assert_eq!(client.pause_epoch(), 1);

        let mock = Arc::clone(holder.lock().unwrap().as_ref().unwrap());
        let sc_before = mock
            .state
            .lock()
            .unwrap()
            .sent
            .iter()
            .filter(|m| m["method"] == "Page.startScreencast")
            .count();
        let paused_poll = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(paused_poll["paused"], true);
        assert_eq!(paused_poll["pauseEpoch"], 1);
        let sc_after_pause = mock
            .state
            .lock()
            .unwrap()
            .sent
            .iter()
            .filter(|m| m["method"] == "Page.startScreencast")
            .count();
        assert_eq!(sc_before, sc_after_pause);

        client.resume();
        assert!(!client.is_paused());
        assert_eq!(client.pause_epoch(), 2);
        let live_poll = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(live_poll["paused"], false);
        assert_eq!(live_poll["pauseEpoch"], 2);
        assert_eq!(client.screencast_target_id().as_deref(), Some("t1"));
        client.dispose();
    }

    #[test]
    fn boot_snapshot_and_warm_never_downgrade_active_high_screencast() {
        let (client, holder) = make_mock_client(vec![tab("t1", "HD", "https://hd.test")]);
        let f_hi = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "high".into(),
                wait_ms: 20,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(f_hi["qualityApplied"], "high");
        assert_eq!(client.screencast_profile().as_deref(), Some("high"));

        let mock = Arc::clone(holder.lock().unwrap().as_ref().unwrap());
        let starts_before = mock
            .state
            .lock()
            .unwrap()
            .sent
            .iter()
            .filter(|m| m["method"] == "Page.startScreencast")
            .count();
        let stops_before = mock
            .state
            .lock()
            .unwrap()
            .sent
            .iter()
            .filter(|m| m["method"] == "Page.stopScreencast")
            .count();

        // Both boot_snapshot and direct warm_active_tab with "low" MUST NOT downgrade active "high"!
        let snap = client.boot_snapshot(1, "0.3.2", Some("low"), 0.0);
        client.warm_active_tab(Some("low"), 0.0);
        thread::sleep(Duration::from_millis(30));

        assert_eq!(snap["qualityApplied"], "high");
        assert_eq!(client.screencast_profile().as_deref(), Some("high"));
        let sent = mock.state.lock().unwrap().sent.clone();
        assert_eq!(
            sent.iter()
                .filter(|m| m["method"] == "Page.startScreencast")
                .count(),
            starts_before,
            "boot_snapshot/warm_active_tab must not restart active high screencast"
        );
        assert_eq!(
            sent.iter()
                .filter(|m| m["method"] == "Page.stopScreencast")
                .count(),
            stops_before,
            "boot_snapshot/warm_active_tab must not stop active high screencast"
        );
        client.dispose();
    }

    #[test]
    fn decouples_popup_trigger_from_capture_idle_uses_layout_metrics_and_dwell_hysteresis() {
        let (client, holder) = make_mock_client(vec![
            tab("t1", "Main", "https://main.test"),
            tab("t-chrome", "Inspect", "chrome://inspect"),
        ]);
        let mock = Arc::clone(holder.lock().unwrap().as_ref().unwrap());
        {
            let mut st = mock.state.lock().unwrap();
            st.fail_screencast_for.insert("t-chrome".into());
            st.fail_eval_for.insert("t-chrome".into());
            st.layout_metrics_vp = Some((1366, 768));
        }

        // 1. Start high-quality screencast on t1; non-forced downgrade within dwell is suppressed,
        //    whereas forced refresh or post-dwell applies cleanly.
        client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "high".into(),
                wait_ms: 20,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(client.screencast_profile().as_deref(), Some("high"));

        let within_dwell = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 20,
                force_refresh: false,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(within_dwell["qualityApplied"], "high");
        assert_eq!(client.screencast_profile().as_deref(), Some("high"));

        // Rewinding past SCREENCAST_TIER_DWELL allows tier transition
        client.rewind_screencast_started_at(SCREENCAST_TIER_DWELL + Duration::from_secs(1));
        let post_dwell = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 20,
                force_refresh: false,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(post_dwell["qualityApplied"], "low");
        assert_eq!(client.screencast_profile().as_deref(), Some("low"));

        // Explicit force_refresh bypasses dwell immediately (e.g. user quality selector)
        let forced = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "high".into(),
                wait_ms: 20,
                force_refresh: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(forced["qualityApplied"], PROFILE_HIGH.name);
        assert_eq!(client.screencast_profile().as_deref(), Some("high"));

        // 2. Popup trigger is decoupled from capture_idle_at: poll/force_refresh does NOT arm
        //    last_popup_trigger_at, so an unrelated background tab without openerId must NOT steal focus.
        mock.state
            .lock()
            .unwrap()
            .tabs
            .push(tab("t-bg", "Background", "https://bg.test"));
        mock.push_event(
            "Target.targetCreated",
            json!({
                "targetInfo": {
                    "targetId": "t-bg",
                    "type": "page",
                    "title": "Background",
                    "url": "https://bg.test"
                }
            }),
            None,
        );
        assert_eq!(client.active_target_id().as_deref(), Some("t1"));

        // Real user click arms last_popup_trigger_at independently of capture_idle_at rewind
        client
            .dispatch_input_batch(InputBatchParams {
                target_id: Some("t1".into()),
                events: vec![InputEvent {
                    action: "click".into(),
                    x: Some(40.0),
                    y: Some(50.0),
                    ..Default::default()
                }],
            })
            .unwrap();
        client.rewind_capture_idle(Duration::from_secs(15));
        mock.state
            .lock()
            .unwrap()
            .tabs
            .push(tab("t-pop2", "Popup", "https://pop.test"));
        mock.push_event(
            "Target.targetCreated",
            json!({
                "targetInfo": {
                    "targetId": "t-pop2",
                    "type": "page",
                    "title": "Popup",
                    "url": "https://pop.test"
                }
            }),
            None,
        );
        assert_eq!(client.active_target_id().as_deref(), Some("t-pop2"));
        // Consume the popup_switched handoff on t-pop2 before explicitly switching to t-chrome
        let f_pop2 = client
            .poll_frame(PollFrameParams {
                target_id: Some("t-pop2".into()),
                quality_preset: "low".into(),
                wait_ms: 20,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(f_pop2["popupSwitched"], true);

        // 3. Restricted chrome:// page where Runtime.evaluate fails falls back to Page.getLayoutMetrics.
        let restricted = client
            .poll_frame(PollFrameParams {
                target_id: Some("t-chrome".into()),
                quality_preset: "low".into(),
                wait_ms: 20,
                force_refresh: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(restricted["activeTargetId"], "t-chrome");
        assert_eq!(restricted["page"]["viewportWidth"], 1366);
        assert_eq!(restricted["page"]["viewportHeight"], 768);
        let sent = mock.state.lock().unwrap().sent.clone();
        assert!(sent.iter().any(|m| m["method"] == "Page.getLayoutMetrics"));
        client.dispose();
    }
}
