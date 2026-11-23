use crate::cdp::discovery::{probe_debug_port_ws_url, read_port_file, resolve_ws_url, PortFileStatus};
use crate::cdp::protocol::{
    default_ws_factory, CdpTransport, TransportEvent, TransportFactory,
};
use crate::stream::dedup::{hash_frame_data, hash_tabs, is_target_gone_error, normalize_url};
use crate::stream::profiles::{resolve_profile, QualityProfile, TabInfo};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(60);
const SCREENCAST_IDLE_STOP: Duration = Duration::from_secs(12);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(65);
const CMD_TIMEOUT: Duration = Duration::from_secs(12);
const CLOSED_TARGET_TTL: Duration = Duration::from_secs(5);
const TAB_CACHE_TTL: Duration = Duration::from_millis(2500);
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputEvent {
    pub action: String,
    pub target_id: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub button: Option<String>,
    pub click_count: Option<u8>,
    pub delta_x: Option<f64>,
    pub delta_y: Option<f64>,
    pub text: Option<String>,
    pub press_enter: Option<bool>,
    pub key: Option<String>,
    pub repeat: Option<u8>,
    pub url: Option<String>,
}

struct PendingCall {
    tx: mpsc::Sender<Result<Value, CdpError>>,
    method: String,
}

struct ClientState {
    transport: Option<Arc<dyn CdpTransport>>,
    connecting: bool,
    paused: bool,
    sessions: HashMap<String, String>,          // targetId -> sessionId
    session_to_target: HashMap<String, String>, // sessionId -> targetId
    active_target_id: Option<String>,
    tab_epoch: u64,
    recently_closed: HashMap<String, Instant>,
    screencast_target_id: Option<String>,
    screencast_quality: String,
    screencast_profile: Option<String>,
    last_poll_at: Instant,
    last_keepalive_at: Instant,
    frame_seq: u64,
    deduped_frames: u64,
    latest_frame: Option<FrameRecord>,
    cached_tabs: Vec<TabInfo>,
    last_tabs_at: Option<Instant>,
}

impl ClientState {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            transport: None,
            connecting: false,
            paused: false,
            sessions: HashMap::new(),
            session_to_target: HashMap::new(),
            active_target_id: None,
            tab_epoch: 1,
            recently_closed: HashMap::new(),
            screencast_target_id: None,
            screencast_quality: "low".to_string(),
            screencast_profile: None,
            last_poll_at: now,
            last_keepalive_at: now,
            frame_seq: 0,
            deduped_frames: 0,
            latest_frame: None,
            cached_tabs: Vec::new(),
            last_tabs_at: None,
        }
    }

    fn prune_closed(&mut self) {
        let now = Instant::now();
        self.recently_closed
            .retain(|_, ts| now.duration_since(*ts) <= CLOSED_TARGET_TTL);
    }

    fn is_recently_closed(&mut self, target_id: &str) -> bool {
        self.prune_closed();
        self.recently_closed.contains_key(target_id)
    }

    fn evict_session(&mut self, target_id: &str) {
        if let Some(old_sid) = self.sessions.remove(target_id) {
            self.session_to_target.remove(&old_sid);
        }
        if self.screencast_target_id.as_deref() == Some(target_id) {
            self.screencast_target_id = None;
            self.screencast_profile = None;
        }
    }

    fn handle_target_gone(&mut self, target_id: &str, mark_closed: bool) -> bool {
        if target_id.is_empty() {
            return false;
        }
        if mark_closed {
            self.recently_closed
                .insert(target_id.to_string(), Instant::now());
        }
        self.evict_session(target_id);
        self.cached_tabs.retain(|t| t.target_id != target_id);
        self.last_tabs_at = None;
        if self
            .latest_frame
            .as_ref()
            .map(|f| f.target_id.as_str() == target_id)
            .unwrap_or(false)
        {
            self.latest_frame = None;
        }
        if self.active_target_id.as_deref() == Some(target_id) {
            self.active_target_id = self.cached_tabs.first().map(|t| t.target_id.clone());
            self.tab_epoch += 1;
            return true;
        }
        false
    }
}

pub struct CdpClient {
    port_file: PathBuf,
    debug_port: u16,
    ws_url_override: Option<String>,
    ws_factory: TransportFactory,
    connect_lock: Mutex<()>,
    state: Mutex<ClientState>,
    pending: Mutex<HashMap<u64, PendingCall>>,
    next_id: AtomicU64,
    frame_notify: (Mutex<u64>, Condvar),
    fallback_port_ready: AtomicBool,
    disposed: AtomicBool,
}

impl CdpClient {
    pub fn new(port_file: PathBuf, debug_port: u16) -> Arc<Self> {
        Self::with_factory(port_file, debug_port, None, default_ws_factory(), true)
    }

    pub fn with_factory(
        port_file: PathBuf,
        debug_port: u16,
        ws_url_override: Option<String>,
        ws_factory: TransportFactory,
        spawn_keepalive: bool,
    ) -> Arc<Self> {
        let client = Arc::new(Self {
            port_file,
            debug_port,
            ws_url_override,
            ws_factory,
            connect_lock: Mutex::new(()),
            state: Mutex::new(ClientState::new()),
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
                        thread::sleep(Duration::from_secs(5));
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
        let (need_ping, need_stop_sc) = {
            let mut st = self.state.lock().unwrap();
            st.prune_closed();
            let connected = st
                .transport
                .as_ref()
                .map(|t| t.is_open())
                .unwrap_or(false);
            let now = Instant::now();
            let ping = connected && now.duration_since(st.last_keepalive_at) >= KEEPALIVE_INTERVAL;
            if ping {
                st.last_keepalive_at = now;
            }
            let stop_sc = st.screencast_target_id.is_some()
                && now.duration_since(st.last_poll_at) > SCREENCAST_IDLE_STOP;
            (ping, stop_sc)
        };
        if need_ping {
            let _ = self.send_cdp("Browser.getVersion", json!({}), None, Duration::from_secs(5));
        }
        if need_stop_sc {
            self.stop_screencast();
        }
    }

    pub fn port_file_status(&self) -> PortFileStatus {
        read_port_file(&self.port_file)
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
        if !pf.valid && self.fallback_port_ready.load(Ordering::Relaxed) {
            summary["valid"] = json!(true);
            summary["port"] = json!(self.debug_port);
            summary["fallback"] = json!(true);
        }
        summary
    }

    pub fn is_connected(&self) -> bool {
        let st = self.state.lock().unwrap();
        st.transport.as_ref().map(|t| t.is_open()).unwrap_or(false)
    }

    pub fn is_connecting(&self) -> bool {
        self.state.lock().unwrap().connecting
    }

    pub fn is_paused(&self) -> bool {
        self.state.lock().unwrap().paused
    }

    pub fn pause(self: &Arc<Self>) {
        {
            let mut st = self.state.lock().unwrap();
            st.paused = true;
        }
        self.stop_screencast();
        self.notify_frame_waiters();
    }

    pub fn resume(&self) {
        {
            let mut st = self.state.lock().unwrap();
            st.paused = false;
            st.last_poll_at = Instant::now();
        }
        self.notify_frame_waiters();
    }

    pub fn active_target_id(&self) -> Option<String> {
        self.state.lock().unwrap().active_target_id.clone()
    }

    pub fn tab_epoch(&self) -> u64 {
        self.state.lock().unwrap().tab_epoch
    }

    #[cfg(test)]
    pub fn deduped_frames(&self) -> u64 {
        self.state.lock().unwrap().deduped_frames
    }

    #[cfg(test)]
    pub fn screencast_target_id(&self) -> Option<String> {
        self.state.lock().unwrap().screencast_target_id.clone()
    }

    pub fn status_snapshot(&self, relay_pid: u32, relay_version: &str) -> Value {
        let st = self.state.lock().unwrap();
        let connected = st.transport.as_ref().map(|t| t.is_open()).unwrap_or(false);
        let port_file = self.public_port_file();
        let chrome_ready = port_file
            .get("valid")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        json!({
            "hostname": "localhost",
            "connected": connected,
            "connecting": st.connecting,
            "paused": connected && st.paused,
            "noTabs": connected && st.cached_tabs.is_empty() && st.active_target_id.is_none(),
            "chromeReady": chrome_ready,
            "portFile": port_file,
            "activeTargetId": st.active_target_id,
            "epoch": st.tab_epoch,
            "hasCachedFrame": st.latest_frame.is_some(),
            "qualityProfile": st.screencast_profile,
            "dedupedFrames": st.deduped_frames,
            "relayVersion": relay_version,
            "relayPid": relay_pid,
            "checkedAt": now_iso(),
        })
    }

    pub fn boot_snapshot(
        self: &Arc<Self>,
        relay_pid: u32,
        relay_version: &str,
        quality_preset: &str,
        rtt_ms: f64,
    ) -> Value {
        let (connected, paused, tabs, active_tid, epoch, frame, profile) = {
            let st = self.state.lock().unwrap();
            let connected = st.transport.as_ref().map(|t| t.is_open()).unwrap_or(false);
            let paused = connected && st.paused;
            let tabs = if connected {
                st.cached_tabs.clone()
            } else {
                Vec::new()
            };
            let active = st.active_target_id.clone();
            let frame = if connected {
                st.latest_frame.as_ref().and_then(|f| {
                    if active.as_deref() == Some(f.target_id.as_str())
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
                connected,
                paused,
                tabs,
                active,
                st.tab_epoch,
                frame,
                st.screencast_profile.clone().unwrap_or_else(|| "low".into()),
            )
        };

        if connected && !paused && !tabs.is_empty() {
            let c = Arc::clone(self);
            let qp = quality_preset.to_string();
            let _ = thread::spawn(move || {
                c.warm_active_tab(&qp, rtt_ms);
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
            "connected": connected,
            "connecting": self.is_connecting(),
            "paused": paused,
            "noTabs": connected && tabs.is_empty() && active_tid.is_none(),
            "chromeReady": chrome_ready,
            "portFile": port_file,
            "activeTargetId": active_tid,
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

    fn notify_frame_waiters(&self) {
        let (lock, cvar) = &self.frame_notify;
        let mut gen = lock.lock().unwrap();
        *gen = gen.wrapping_add(1);
        cvar.notify_all();
    }

    fn wait_for_next_frame(
        &self,
        since_seq: u64,
        timeout: Duration,
        expected_target_id: Option<&str>,
    ) {
        let deadline = Instant::now() + timeout;
        let (lock, cvar) = &self.frame_notify;
        let mut gen = lock.lock().unwrap();
        loop {
            {
                let mut st = self.state.lock().unwrap();
                if st.paused || !st.transport.as_ref().map(|t| t.is_open()).unwrap_or(false) {
                    return;
                }
                if let Some(tid) = expected_target_id {
                    if st.is_recently_closed(tid) {
                        return;
                    }
                }
                if let Some(f) = &st.latest_frame {
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

    fn wait_for_notify(&self, timeout: Duration) -> bool {
        let (lock, cvar) = &self.frame_notify;
        let gen = lock.lock().unwrap();
        let start_gen = *gen;
        let (next_gen, res) = cvar.wait_timeout(gen, timeout).unwrap();
        !res.timed_out() && *next_gen != start_gen
    }

    pub fn disconnect(&self) {
        let old_transport = {
            let mut st = self.state.lock().unwrap();
            st.connecting = false;
            st.paused = false;
            st.sessions.clear();
            st.session_to_target.clear();
            st.recently_closed.clear();
            st.active_target_id = None;
            st.screencast_target_id = None;
            st.screencast_profile = None;
            st.latest_frame = None;
            st.cached_tabs.clear();
            st.last_tabs_at = None;
            st.transport.take()
        };
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

    pub fn ensure_connected(self: &Arc<Self>) -> Result<(), CdpError> {
        if self.is_connected() {
            return Ok(());
        }
        let _connect_guard = self.connect_lock.lock().unwrap();
        if self.is_connected() {
            return Ok(());
        }

        let ws_url = self
            .ws_url_override
            .clone()
            .or_else(|| resolve_ws_url(&self.port_file, self.debug_port))
            .ok_or_else(|| {
                CdpError::new(
                    "未检测到本机 Chrome 远程调试端口（DevToolsActivePort）。请确保 Google Chrome 正在运行，并在 chrome://inspect/#remote-debugging 开启远程调试。",
                )
            })?;

        {
            let mut st = self.state.lock().unwrap();
            st.connecting = true;
        }

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
                    let mut st = self.state.lock().unwrap();
                    st.transport = Some(Arc::clone(&transport));
                    st.connecting = false;
                    st.paused = false;
                    st.last_keepalive_at = Instant::now();
                }
                // Subscribe to target lifecycle events so external tab closures recover immediately
                self.send_no_wait("Target.setDiscoverTargets", json!({ "discover": true }), None);
                Ok(())
            }
            Err(msg) => {
                let mut st = self.state.lock().unwrap();
                st.connecting = false;
                Err(CdpError::new(msg))
            }
        }
    }

    fn handle_transport_closed(&self) {
        let should_clean = {
            let mut st = self.state.lock().unwrap();
            if st
                .transport
                .as_ref()
                .map(|t| !t.is_open())
                .unwrap_or(true)
            {
                st.transport = None;
                st.connecting = false;
                st.paused = false;
                st.sessions.clear();
                st.session_to_target.clear();
                st.recently_closed.clear();
                st.active_target_id = None;
                st.screencast_target_id = None;
                st.screencast_profile = None;
                st.latest_frame = None;
                st.cached_tabs.clear();
                st.last_tabs_at = None;
                true
            } else {
                false
            }
        };
        if should_clean {
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

                let hash = hash_frame_data(data);
                let mut should_notify = false;
                {
                    let mut st = self.state.lock().unwrap();
                    let target_id = sid
                        .and_then(|s| st.session_to_target.get(s).cloned())
                        .or_else(|| st.screencast_target_id.clone());

                    if let Some(tid) = target_id {
                        if st.active_target_id.as_deref() == Some(tid.as_str())
                            && !st.is_recently_closed(&tid)
                        {
                            let is_dup = st
                                .latest_frame
                                .as_ref()
                                .map(|f| f.target_id == tid && f.hash == hash)
                                .unwrap_or(false);
                            if is_dup {
                                st.deduped_frames += 1;
                                if let Some(f) = st.latest_frame.as_mut() {
                                    f.captured_at = now_iso();
                                }
                            } else {
                                st.frame_seq += 1;
                                let seq = st.frame_seq;
                                let epoch = st.tab_epoch;
                                st.latest_frame = Some(FrameRecord {
                                    seq,
                                    epoch,
                                    target_id: tid,
                                    hash,
                                    image_data_url: format!("data:image/jpeg;base64,{data}"),
                                    image_bytes: (data.len() * 3) / 4,
                                    viewport_width: vw,
                                    viewport_height: vh,
                                    captured_at: now_iso(),
                                });
                                should_notify = true;
                            }
                        }
                    }
                }
                if should_notify {
                    self.notify_frame_waiters();
                }
            }
            "Target.targetDestroyed" => {
                if let Some(tid) = params.and_then(|p| p.get("targetId")).and_then(|v| v.as_str()) {
                    let wake = {
                        let mut st = self.state.lock().unwrap();
                        st.handle_target_gone(tid, true)
                    };
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
                let mut st = self.state.lock().unwrap();
                let mapped_tid = sid.and_then(|s| st.session_to_target.remove(s));
                let tid = param_tid.map(String::from).or(mapped_tid);
                if let Some(tid) = tid {
                    let should_remove = match (sid, st.sessions.get(&tid)) {
                        (None, _) => true,
                        (Some(s), Some(cur)) if cur == s => true,
                        _ => false,
                    };
                    if should_remove {
                        st.sessions.remove(&tid);
                    }
                    if st.screencast_target_id.as_deref() == Some(tid.as_str()) {
                        st.screencast_target_id = None;
                        st.screencast_profile = None;
                    }
                }
            }
            "Target.targetCreated" => {
                {
                    let mut st = self.state.lock().unwrap();
                    st.last_tabs_at = None;
                }
                self.notify_frame_waiters();
            }
            "Target.targetInfoChanged" => {
                let mut st = self.state.lock().unwrap();
                st.last_tabs_at = None;
            }
            _ => {}
        }
    }

    pub fn send_no_wait(&self, method: &str, params: Value, session_id: Option<&str>) {
        let transport = { self.state.lock().unwrap().transport.clone() };
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
        self.ensure_connected()?;
        let transport = {
            self.state
                .lock()
                .unwrap()
                .transport
                .clone()
                .ok_or_else(|| CdpError::new("Chrome CDP connection closed"))?
        };
        if !transport.is_open() {
            return Err(CdpError::new("Chrome CDP connection closed"));
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut payload = json!({ "id": id, "method": method, "params": params });
        if let Some(sid) = session_id {
            payload["sessionId"] = json!(sid);
        }

        let (tx, rx) = mpsc::channel();
        {
            self.pending.lock().unwrap().insert(
                id,
                PendingCall {
                    tx,
                    method: method.to_string(),
                },
            );
        }

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
        {
            let mut st = self.state.lock().unwrap();
            st.prune_closed();
            if !force && !st.cached_tabs.is_empty() {
                if let Some(ts) = st.last_tabs_at {
                    if ts.elapsed() < TAB_CACHE_TTL {
                        let closed = st.recently_closed.clone();
                        return Ok(st
                            .cached_tabs
                            .iter()
                            .filter(|t| !closed.contains_key(&t.target_id))
                            .cloned()
                            .collect());
                    }
                }
            }
        }

        let res = self.send_cdp("Target.getTargets", json!({}), None, CMD_TIMEOUT)?;
        let raw = res
            .get("targetInfos")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut st = self.state.lock().unwrap();
        st.prune_closed();
        let mut tabs = Vec::new();
        for item in raw {
            if item.get("type").and_then(|v| v.as_str()) != Some("page") {
                continue;
            }
            let Some(tid) = item.get("targetId").and_then(|v| v.as_str()) else {
                continue;
            };
            if st.recently_closed.contains_key(tid) {
                continue;
            }
            let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("");
            if url.starts_with("devtools://") {
                continue;
            }
            if url.starts_with("chrome-extension://") && title.is_empty() {
                continue;
            }
            tabs.push(TabInfo {
                target_id: tid.to_string(),
                title: if !title.is_empty() {
                    title.to_string()
                } else if !url.is_empty() {
                    url.to_string()
                } else {
                    "New Tab".to_string()
                },
                url: if !url.is_empty() {
                    url.to_string()
                } else {
                    "about:blank".to_string()
                },
            });
        }
        st.cached_tabs = tabs.clone();
        st.last_tabs_at = Some(Instant::now());
        Ok(tabs)
    }

    pub fn resolve_target_id(
        self: &Arc<Self>,
        requested_id: Option<&str>,
        force_tabs: bool,
        update_active: bool,
    ) -> Result<(Option<String>, Vec<TabInfo>), CdpError> {
        let tabs = self.list_tabs(force_tabs)?;
        if tabs.is_empty() {
            // Never auto-create `about:blank`: let the user close the last tab / Chrome window cleanly.
            let need_stop_sc = {
                let mut st = self.state.lock().unwrap();
                if update_active {
                    if st.active_target_id.is_some() {
                        st.tab_epoch += 1;
                    }
                    st.active_target_id = None;
                    st.latest_frame = None;
                }
                st.screencast_target_id.is_some()
            };
            if need_stop_sc {
                self.stop_screencast();
            }
            return Ok((None, Vec::new()));
        }

        let mut st = self.state.lock().unwrap();
        let safe_req = requested_id.filter(|id| !id.is_empty() && !st.is_recently_closed(id));
        let candidate = safe_req.or(st.active_target_id.as_deref());
        let chosen = candidate
            .and_then(|c| tabs.iter().find(|t| t.target_id == c))
            .map(|t| t.target_id.clone())
            .unwrap_or_else(|| tabs[0].target_id.clone());

        if update_active {
            if st
                .active_target_id
                .as_deref()
                .map(|cur| cur != chosen.as_str())
                .unwrap_or(false)
            {
                st.tab_epoch += 1;
            }
            st.active_target_id = Some(chosen.clone());
        }
        Ok((Some(chosen), tabs))
    }

    pub fn attach(self: &Arc<Self>, target_id: &str) -> Result<String, CdpError> {
        {
            let mut st = self.state.lock().unwrap();
            if st.is_recently_closed(target_id) {
                return Err(CdpError {
                    message: "No target with given id found".to_string(),
                    cdp_code: Some(-32000),
                    method: Some("Target.attachToTarget".to_string()),
                });
            }
            if let Some(sid) = st.sessions.get(target_id) {
                return Ok(sid.clone());
            }
        }

        let res = self.send_cdp(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
            CMD_TIMEOUT,
        )?;
        let sid = res
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CdpError::new("Target.attachToTarget missing sessionId"))?
            .to_string();

        let mut st = self.state.lock().unwrap();
        st.sessions.insert(target_id.to_string(), sid.clone());
        st.session_to_target
            .insert(sid.clone(), target_id.to_string());
        Ok(sid)
    }

    pub fn stop_screencast(self: &Arc<Self>) {
        let prev_sid = {
            let mut st = self.state.lock().unwrap();
            let Some(prev_tid) = st.screencast_target_id.take() else {
                return;
            };
            st.screencast_profile = None;
            st.sessions.get(&prev_tid).cloned()
        };
        if let Some(sid) = prev_sid {
            if self.is_connected() {
                let _ = self.send_cdp(
                    "Page.stopScreencast",
                    json!({}),
                    Some(&sid),
                    Duration::from_secs(3),
                );
            }
        }
    }

    pub fn ensure_screencast(
        self: &Arc<Self>,
        target_id: &str,
        quality_preset: &str,
        rtt_ms: f64,
    ) -> Result<(String, &'static QualityProfile), CdpError> {
        let qp = resolve_profile(quality_preset, rtt_ms);
        {
            let mut st = self.state.lock().unwrap();
            st.last_poll_at = Instant::now();
            if st.screencast_target_id.as_deref() == Some(target_id)
                && st.screencast_profile.as_deref() == Some(qp.name)
            {
                if let Some(sid) = st.sessions.get(target_id).cloned() {
                    return Ok((sid, qp));
                }
            }
        }

        self.stop_screencast();
        let sid = self.attach(target_id)?;
        {
            let mut st = self.state.lock().unwrap();
            st.screencast_target_id = Some(target_id.to_string());
            st.screencast_quality = quality_preset.to_string();
            st.screencast_profile = Some(qp.name.to_string());
            if st
                .latest_frame
                .as_ref()
                .map(|f| f.target_id.as_str() != target_id)
                .unwrap_or(false)
            {
                st.latest_frame = None;
            }
        }

        let start_res = self.send_cdp(
            "Page.startScreencast",
            json!({
                "format": "jpeg",
                "quality": qp.quality,
                "maxWidth": qp.max_width,
                "maxHeight": qp.max_height,
                "everyNthFrame": qp.every_nth_frame,
            }),
            Some(&sid),
            Duration::from_secs(5),
        );

        if let Err(err) = start_res {
            let mut st = self.state.lock().unwrap();
            st.screencast_target_id = None;
            st.screencast_profile = None;
            if err.is_target_gone() {
                st.evict_session(target_id);
                return Err(err);
            }
            // Restricted pages (e.g. chrome://) disallow screencast; fallback screenshot handles them.
        }

        Ok((sid, qp))
    }

    pub fn warm_active_tab(self: &Arc<Self>, quality_preset: &str, rtt_ms: f64) {
        if !self.is_connected() || self.is_paused() {
            return;
        }
        let active = self.active_target_id();
        if let Ok((Some(tid), _)) = self.resolve_target_id(active.as_deref(), false, true) {
            let _ = self.ensure_screencast(&tid, quality_preset, rtt_ms);
        }
    }

    fn capture_fallback_frame(
        self: &Arc<Self>,
        target_id: &str,
        session_id: &str,
        quality_preset: &str,
        rtt_ms: f64,
    ) -> Result<(), CdpError> {
        let qp = resolve_profile(quality_preset, rtt_ms);
        let mut vw: u32 = 1280;
        let mut vh: u32 = 800;
        let mut got_vp = false;

        match self.send_cdp(
            "Runtime.evaluate",
            json!({
                "expression": "JSON.stringify([window.innerWidth||1280,window.innerHeight||800])",
                "returnByValue": true,
            }),
            Some(session_id),
            Duration::from_millis(1500),
        ) {
            Ok(ev) => {
                if let Some(raw) = ev.pointer("/result/value").and_then(|v| v.as_str()) {
                    if let Ok(arr) = serde_json::from_str::<Vec<f64>>(raw) {
                        if arr.len() >= 2 && arr[0] > 0.0 && arr[1] > 0.0 {
                            vw = arr[0].round() as u32;
                            vh = arr[1].round() as u32;
                            got_vp = true;
                        }
                    }
                }
            }
            Err(err) if err.is_target_gone() => return Err(err),
            Err(_) => {}
        }

        let mut capture_params = json!({
            "format": "jpeg",
            "quality": qp.quality.min(55),
            "fromSurface": true,
        });
        if got_vp {
            let scale_w = qp.max_width as f64 / vw as f64;
            let scale_h = qp.max_height as f64 / vh as f64;
            let scale = scale_w.min(scale_h).min(1.0);
            let scale_rounded = (scale * 1000.0).round() / 1000.0;
            capture_params["clip"] = json!({
                "x": 0,
                "y": 0,
                "width": vw,
                "height": vh,
                "scale": scale_rounded,
            });
        }

        let shot = match self.send_cdp(
            "Page.captureScreenshot",
            capture_params.clone(),
            Some(session_id),
            Duration::from_secs(8),
        ) {
            Ok(v) => v,
            Err(err) if err.is_target_gone() => return Err(err),
            Err(err) => {
                if capture_params.get("clip").is_some() {
                    self.send_cdp(
                        "Page.captureScreenshot",
                        json!({
                            "format": "jpeg",
                            "quality": qp.quality.min(46),
                            "fromSurface": true,
                        }),
                        Some(session_id),
                        Duration::from_secs(8),
                    )?
                } else {
                    return Err(err);
                }
            }
        };

        let Some(data) = shot.get("data").and_then(|v| v.as_str()) else {
            return Ok(());
        };
        if data.is_empty() {
            return Ok(());
        }

        let hash = hash_frame_data(data);
        let mut st = self.state.lock().unwrap();
        if st.is_recently_closed(target_id) {
            return Ok(());
        }
        let is_dup = st
            .latest_frame
            .as_ref()
            .map(|f| f.target_id == target_id && f.hash == hash)
            .unwrap_or(false);
        if is_dup {
            st.deduped_frames += 1;
            if let Some(f) = st.latest_frame.as_mut() {
                f.captured_at = now_iso();
            }
            return Ok(());
        }

        st.frame_seq += 1;
        let seq = st.frame_seq;
        let epoch = st.tab_epoch;
        st.latest_frame = Some(FrameRecord {
            seq,
            epoch,
            target_id: target_id.to_string(),
            hash,
            image_data_url: format!("data:image/jpeg;base64,{data}"),
            image_bytes: (data.len() * 3) / 4,
            viewport_width: vw,
            viewport_height: vh,
            captured_at: now_iso(),
        });
        Ok(())
    }

    pub fn close_tab(
        self: &Arc<Self>,
        target_id_to_close: Option<&str>,
        quality_preset: Option<&str>,
    ) -> Result<Value, CdpError> {
        self.ensure_connected()?;
        {
            self.state.lock().unwrap().last_poll_at = Instant::now();
        }

        let before_tabs = self.list_tabs(true)?;
        let close_id = target_id_to_close
            .filter(|s| !s.is_empty())
            .map(String::from)
            .or_else(|| self.active_target_id());

        let close_idx = close_id
            .as_deref()
            .and_then(|id| before_tabs.iter().position(|t| t.target_id == id));
        let was_active = close_id.as_deref().is_some()
            && self.active_target_id().as_deref() == close_id.as_deref();

        if let Some(cid) = close_id.as_deref() {
            let need_stop_sc = {
                let mut st = self.state.lock().unwrap();
                st.recently_closed.insert(cid.to_string(), Instant::now());
                st.screencast_target_id.as_deref() == Some(cid)
            };
            if need_stop_sc {
                self.stop_screencast();
            }
            {
                let mut st = self.state.lock().unwrap();
                st.evict_session(cid);
                if st
                    .latest_frame
                    .as_ref()
                    .map(|f| f.target_id.as_str() == cid)
                    .unwrap_or(false)
                {
                    st.latest_frame = None;
                }
                if was_active {
                    st.active_target_id = None;
                    st.tab_epoch += 1;
                }
            }

            // Close in Chrome only if target actually existed (prevents double-close hitting innocent tabs!)
            if close_idx.is_some() {
                if let Err(err) = self.send_cdp(
                    "Target.closeTarget",
                    json!({ "targetId": cid }),
                    None,
                    Duration::from_secs(5),
                ) {
                    if !err.is_target_gone() {
                        return Err(err);
                    }
                }
            }
        }

        // Refresh surviving targets (retry briefly while Chrome tears down renderer)
        let mut remaining = Vec::new();
        for attempt in 0..4 {
            {
                self.state.lock().unwrap().last_tabs_at = None;
            }
            remaining = self.list_tabs(true)?;
            if !remaining.is_empty() || attempt >= 1 {
                break;
            }
            thread::sleep(Duration::from_millis(35));
        }

        if remaining.is_empty() {
            // Respect closing the last tab / Chrome window without spawning `about:blank`.
            self.stop_screencast();
            let mut st = self.state.lock().unwrap();
            if st.active_target_id.is_some() {
                st.tab_epoch += 1;
            }
            st.active_target_id = None;
            st.latest_frame = None;
            st.cached_tabs.clear();
        } else {
            let cur_active = self.active_target_id();
            let active_still_valid = cur_active
                .as_deref()
                .map(|id| remaining.iter().any(|t| t.target_id == id))
                .unwrap_or(false);
            if was_active || !active_still_valid {
                let next_idx = close_idx
                    .map(|idx| idx.min(remaining.len() - 1))
                    .unwrap_or(0);
                let next_tid = remaining[next_idx].target_id.clone();
                let _ = self.send_cdp(
                    "Target.activateTarget",
                    json!({ "targetId": next_tid }),
                    None,
                    Duration::from_secs(3),
                );
                let mut st = self.state.lock().unwrap();
                st.active_target_id = Some(next_tid);
                st.tab_epoch += 1;
            }
        }

        let (active_tid, sc_tid, sc_q, paused) = {
            let st = self.state.lock().unwrap();
            (
                st.active_target_id.clone(),
                st.screencast_target_id.clone(),
                st.screencast_quality.clone(),
                st.paused,
            )
        };
        if !paused {
            if let Some(tid) = active_tid.as_deref() {
                if was_active || sc_tid.as_deref() != Some(tid) {
                    let q = quality_preset.unwrap_or(sc_q.as_str());
                    let _ = self.ensure_screencast(tid, q, 0.0);
                }
            }
        }

        self.notify_frame_waiters();
        let (epoch, seq) = {
            let st = self.state.lock().unwrap();
            (st.tab_epoch, st.frame_seq)
        };
        let t_hash = hash_tabs(&remaining);

        Ok(json!({
            "ok": true,
            "connected": self.is_connected(),
            "paused": paused,
            "noTabs": remaining.is_empty(),
            "activeTargetId": active_tid,
            "epoch": epoch,
            "tabs": remaining,
            "tabsHash": t_hash,
            "seq": seq,
        }))
    }

    pub fn poll_frame(self: &Arc<Self>, params: PollFrameParams) -> Result<Value, CdpError> {
        let t_start = Instant::now();
        self.ensure_connected()?;
        let paused = {
            let mut st = self.state.lock().unwrap();
            st.last_poll_at = Instant::now();
            st.paused
        };

        let mut target_id = String::new();
        let mut tabs: Vec<TabInfo> = Vec::new();
        let mut profile = resolve_profile(&params.quality_preset, params.rtt_ms);
        let mut recovered = false;

        for attempt in 0..2 {
            let ask_tid = if attempt == 0 {
                params.target_id.as_deref()
            } else {
                None
            };
            let force_tabs = params.force_refresh || attempt > 0;
            let (mut resolved_tid, mut resolved_tabs) =
                self.resolve_target_id(ask_tid, force_tabs, true)?;

            if resolved_tid.is_none() {
                // Chrome has 0 open windows/tabs: wait gently for a user-opened tab without spawning about:blank.
                if !params.force_refresh && params.wait_ms > 0 && !paused {
                    let wait = params.wait_ms.clamp(40, 1200);
                    if self.wait_for_notify(Duration::from_millis(wait)) && self.is_connected() {
                        let (next_tid, next_tabs) = self.resolve_target_id(None, true, true)?;
                        resolved_tid = next_tid;
                        resolved_tabs = next_tabs;
                    }
                }
            }

            let Some(tid) = resolved_tid else {
                let (epoch, seq, is_paused) = {
                    let st = self.state.lock().unwrap();
                    (st.tab_epoch, st.frame_seq, st.paused)
                };
                let t_hash = hash_tabs(&resolved_tabs);
                let include_tabs =
                    params.known_tabs_hash.is_empty() || params.known_tabs_hash != t_hash;
                let mut resp = json!({
                    "connected": self.is_connected(),
                    "paused": is_paused,
                    "noTabs": true,
                    "activeTargetId": Value::Null,
                    "epoch": epoch,
                    "tabsHash": t_hash,
                    "page": {
                        "title": "",
                        "url": "",
                        "viewportWidth": 1280,
                        "viewportHeight": 800,
                    },
                    "changed": false,
                    "unchanged": true,
                    "seq": seq,
                    "frameHash": "",
                    "imageDataUrl": "",
                    "imageBytes": 0,
                    "qualityApplied": profile.name,
                    "serverMs": t_start.elapsed().as_millis() as u64,
                    "capturedAt": now_iso(),
                });
                if include_tabs {
                    resp["tabs"] = json!(resolved_tabs);
                }
                return Ok(resp);
            };

            target_id = tid;
            tabs = resolved_tabs;

            let attempt_res: Result<(), CdpError> = (|| {
                if paused {
                    // While paused, never restart continuous screencast; only capture one-shot snapshot if requested.
                    if params.force_refresh {
                        let sid = self.attach(&target_id)?;
                        self.capture_fallback_frame(
                            &target_id,
                            &sid,
                            &params.quality_preset,
                            params.rtt_ms,
                        )?;
                    }
                    return Ok(());
                }

                let (sid, qp) =
                    self.ensure_screencast(&target_id, &params.quality_preset, params.rtt_ms)?;
                profile = qp;

                let (has_valid_frame, same_as_client_hash, cur_seq) = {
                    let st = self.state.lock().unwrap();
                    match &st.latest_frame {
                        Some(f) if f.target_id == target_id => (
                            true,
                            !params.last_frame_hash.is_empty() && f.hash == params.last_frame_hash,
                            f.seq,
                        ),
                        _ => (false, false, st.frame_seq),
                    }
                };

                if params.force_refresh {
                    let prev_seq = self.state.lock().unwrap().frame_seq;
                    self.wait_for_next_frame(
                        prev_seq,
                        Duration::from_millis(160),
                        Some(&target_id),
                    );
                    let need_fallback = {
                        let st = self.state.lock().unwrap();
                        st.frame_seq == prev_seq
                            || st
                                .latest_frame
                                .as_ref()
                                .map(|f| f.target_id != target_id)
                                .unwrap_or(true)
                    };
                    if need_fallback {
                        self.capture_fallback_frame(
                            &target_id,
                            &sid,
                            &params.quality_preset,
                            params.rtt_ms,
                        )?;
                    }
                } else if !has_valid_frame {
                    self.wait_for_next_frame(0, Duration::from_millis(300), Some(&target_id));
                    let need_fallback = {
                        let st = self.state.lock().unwrap();
                        st.latest_frame
                            .as_ref()
                            .map(|f| f.target_id != target_id)
                            .unwrap_or(true)
                    };
                    if need_fallback {
                        self.capture_fallback_frame(
                            &target_id,
                            &sid,
                            &params.quality_preset,
                            params.rtt_ms,
                        )?;
                    }
                } else if (cur_seq <= params.since_seq || same_as_client_hash) && params.wait_ms > 0
                {
                    let wait = params.wait_ms.clamp(40, 1200);
                    self.wait_for_next_frame(
                        params.since_seq.max(cur_seq),
                        Duration::from_millis(wait),
                        Some(&target_id),
                    );
                }

                Ok(())
            })();

            match attempt_res {
                Ok(()) => {
                    let gone = { self.state.lock().unwrap().is_recently_closed(&target_id) };
                    if gone && attempt == 0 {
                        recovered = true;
                        continue;
                    }
                    break;
                }
                Err(err) if err.is_target_gone() && self.is_connected() && attempt == 0 => {
                    let wake = {
                        self.state
                            .lock()
                            .unwrap()
                            .handle_target_gone(&target_id, true)
                    };
                    if wake {
                        self.notify_frame_waiters();
                    }
                    recovered = true;
                    continue;
                }
                Err(err) => return Err(err),
            }
        }

        let (epoch, frame, cur_paused) = {
            let st = self.state.lock().unwrap();
            let f = st
                .latest_frame
                .as_ref()
                .filter(|f| f.target_id == target_id)
                .cloned();
            (st.tab_epoch, f, st.paused)
        };

        let same_hash = frame
            .as_ref()
            .map(|f| !params.last_frame_hash.is_empty() && f.hash == params.last_frame_hash)
            .unwrap_or(false);
        let diff_hash = frame
            .as_ref()
            .map(|f| !params.last_frame_hash.is_empty() && f.hash != params.last_frame_hash)
            .unwrap_or(false);
        let changed = frame
            .as_ref()
            .map(|f| !same_hash && (params.force_refresh || f.seq > params.since_seq || diff_hash))
            .unwrap_or(false);

        let t_hash = hash_tabs(&tabs);
        let include_tabs = params.known_tabs_hash.is_empty() || params.known_tabs_hash != t_hash;
        let tab_meta = tabs.iter().find(|t| t.target_id == target_id);

        let mut resp = json!({
            "connected": self.is_connected(),
            "paused": cur_paused,
            "noTabs": false,
            "activeTargetId": target_id,
            "epoch": epoch,
            "tabsHash": t_hash,
            "page": {
                "title": tab_meta.map(|t| t.title.as_str()).unwrap_or(""),
                "url": tab_meta.map(|t| t.url.as_str()).unwrap_or(""),
                "viewportWidth": frame.as_ref().map(|f| f.viewport_width).unwrap_or(1280),
                "viewportHeight": frame.as_ref().map(|f| f.viewport_height).unwrap_or(800),
            },
            "changed": changed,
            "unchanged": !changed,
            "seq": frame.as_ref().map(|f| f.seq).unwrap_or(params.since_seq),
            "frameHash": frame.as_ref().map(|f| f.hash.as_str()).unwrap_or(""),
            "imageDataUrl": if changed { frame.as_ref().map(|f| f.image_data_url.as_str()).unwrap_or("") } else { "" },
            "imageBytes": if changed { frame.as_ref().map(|f| f.image_bytes).unwrap_or(0) } else { 0 },
            "qualityApplied": profile.name,
            "serverMs": t_start.elapsed().as_millis() as u64,
            "capturedAt": frame.as_ref().map(|f| f.captured_at.as_str()).unwrap_or(&now_iso()),
        });
        if include_tabs {
            resp["tabs"] = json!(tabs);
        }
        if recovered {
            resp["recovered"] = json!(true);
        }
        Ok(resp)
    }

    pub fn dispatch_input_batch(
        self: &Arc<Self>,
        params: InputBatchParams,
    ) -> Result<Value, CdpError> {
        self.ensure_connected()?;
        {
            self.state.lock().unwrap().last_poll_at = Instant::now();
        }

        let mut tab_mutated = false;
        let mut nav_mutated = false;
        let mut recovered = false;
        let mut latest_tabs: Option<Vec<TabInfo>> = None;

        for ev in &params.events {
            let action = ev.action.as_str();
            if action.is_empty() {
                continue;
            }

            if action == "disconnect" {
                self.stop_screencast();
                self.disconnect();
                return Ok(json!({
                    "ok": true,
                    "connected": false,
                    "paused": false,
                    "portFile": self.public_port_file(),
                }));
            }

            if action == "pause" {
                self.pause();
                return Ok(json!({
                    "ok": true,
                    "connected": self.is_connected(),
                    "paused": true,
                    "portFile": self.public_port_file(),
                }));
            }

            if action == "resume" {
                self.resume();
                continue;
            }

            if action == "new_tab" {
                let target_url = normalize_url(ev.url.as_deref().unwrap_or("about:blank"));
                let created = self.send_cdp(
                    "Target.createTarget",
                    json!({ "url": target_url }),
                    None,
                    CMD_TIMEOUT,
                )?;
                let new_tid = created
                    .get("targetId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| CdpError::new("Target.createTarget missing targetId"))?
                    .to_string();
                {
                    let mut st = self.state.lock().unwrap();
                    st.recently_closed.remove(&new_tid);
                    st.active_target_id = Some(new_tid.clone());
                    st.tab_epoch += 1;
                    st.latest_frame = None;
                    st.last_tabs_at = None;
                }
                let _ = self.send_cdp(
                    "Target.activateTarget",
                    json!({ "targetId": new_tid }),
                    None,
                    Duration::from_secs(3),
                );
                tab_mutated = true;
                continue;
            }

            if action == "close_tab" {
                let tid = ev.target_id.as_deref().or(params.target_id.as_deref());
                let res = self.close_tab(tid, None)?;
                if let Some(arr) = res.get("tabs") {
                    if let Ok(parsed) = serde_json::from_value::<Vec<TabInfo>>(arr.clone()) {
                        latest_tabs = Some(parsed);
                    }
                }
                tab_mutated = true;
                continue;
            }

            if action == "switch_tab" {
                let req = ev.target_id.as_deref().or(params.target_id.as_deref());
                let (maybe_tid, cur_tabs) = self.resolve_target_id(req, false, true)?;
                let Some(tid) = maybe_tid else {
                    latest_tabs = Some(cur_tabs);
                    tab_mutated = true;
                    continue;
                };
                match self.send_cdp(
                    "Target.activateTarget",
                    json!({ "targetId": tid }),
                    None,
                    Duration::from_secs(3),
                ) {
                    Ok(_) => {
                        let mut st = self.state.lock().unwrap();
                        st.active_target_id = Some(tid);
                        st.latest_frame = None;
                        tab_mutated = true;
                    }
                    Err(err) if err.is_target_gone() && self.is_connected() => {
                        {
                            self.state.lock().unwrap().handle_target_gone(&tid, true);
                        }
                        let (fb_tid, fb_tabs) = self.resolve_target_id(None, true, true)?;
                        if let Some(fb) = fb_tid {
                            let _ = self.send_cdp(
                                "Target.activateTarget",
                                json!({ "targetId": fb }),
                                None,
                                Duration::from_secs(3),
                            );
                        }
                        latest_tabs = Some(fb_tabs);
                        recovered = true;
                        tab_mutated = true;
                    }
                    Err(err) => return Err(err),
                }
                continue;
            }

            let ask_tid = ev.target_id.as_deref().or(params.target_id.as_deref());
            let mut resolved_tid = String::new();
            let step_res: Result<(), CdpError> = (|| {
                let (maybe_tid, cur_tabs) = self.resolve_target_id(ask_tid, false, true)?;
                let tid = match maybe_tid {
                    Some(t) => t,
                    None if action == "navigate" => {
                        // Explicit URL navigation when 0 tabs exist opens a tab for that URL.
                        let target_url = normalize_url(ev.url.as_deref().unwrap_or("about:blank"));
                        let created = self.send_cdp(
                            "Target.createTarget",
                            json!({ "url": target_url }),
                            None,
                            CMD_TIMEOUT,
                        )?;
                        let new_tid = created
                            .get("targetId")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| CdpError::new("Target.createTarget missing targetId"))?
                            .to_string();
                        {
                            let mut st = self.state.lock().unwrap();
                            st.recently_closed.remove(&new_tid);
                            st.active_target_id = Some(new_tid.clone());
                            st.tab_epoch += 1;
                            st.latest_frame = None;
                            st.last_tabs_at = None;
                        }
                        let _ = self.send_cdp(
                            "Target.activateTarget",
                            json!({ "targetId": new_tid }),
                            None,
                            Duration::from_secs(3),
                        );
                        tab_mutated = true;
                        nav_mutated = true;
                        return Ok(());
                    }
                    None => {
                        latest_tabs = Some(cur_tabs);
                        tab_mutated = true;
                        return Ok(());
                    }
                };
                resolved_tid = tid.clone();
                let sid = self.attach(&tid)?;

                match action {
                    "mousemove" => {
                        let mx = ev.x.unwrap_or(0.0).max(0.0).round() as i64;
                        let my = ev.y.unwrap_or(0.0).max(0.0).round() as i64;
                        self.send_cdp(
                            "Input.dispatchMouseEvent",
                            json!({ "type": "mouseMoved", "x": mx, "y": my }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                    }
                    "click" => {
                        let cx = ev.x.unwrap_or(0.0).max(0.0).round() as i64;
                        let cy = ev.y.unwrap_or(0.0).max(0.0).round() as i64;
                        let btn = match ev.button.as_deref() {
                            Some("right") => "right",
                            Some("middle") => "middle",
                            _ => "left",
                        };
                        let count = if ev.click_count == Some(2) { 2 } else { 1 };
                        self.send_cdp(
                            "Input.dispatchMouseEvent",
                            json!({ "type": "mouseMoved", "x": cx, "y": cy }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                        self.send_cdp(
                            "Input.dispatchMouseEvent",
                            json!({ "type": "mousePressed", "x": cx, "y": cy, "button": btn, "clickCount": 1 }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                        self.send_cdp(
                            "Input.dispatchMouseEvent",
                            json!({ "type": "mouseReleased", "x": cx, "y": cy, "button": btn, "clickCount": 1 }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                        if count == 2 {
                            self.send_cdp(
                                "Input.dispatchMouseEvent",
                                json!({ "type": "mousePressed", "x": cx, "y": cy, "button": btn, "clickCount": 2 }),
                                Some(&sid),
                                CMD_TIMEOUT,
                            )?;
                            self.send_cdp(
                                "Input.dispatchMouseEvent",
                                json!({ "type": "mouseReleased", "x": cx, "y": cy, "button": btn, "clickCount": 2 }),
                                Some(&sid),
                                CMD_TIMEOUT,
                            )?;
                        }
                    }
                    "scroll" => {
                        let sx = ev.x.unwrap_or(500.0).max(0.0).round() as i64;
                        let sy = ev.y.unwrap_or(350.0).max(0.0).round() as i64;
                        self.send_cdp(
                            "Input.dispatchMouseEvent",
                            json!({
                                "type": "mouseWheel",
                                "x": sx,
                                "y": sy,
                                "deltaX": ev.delta_x.unwrap_or(0.0),
                                "deltaY": ev.delta_y.unwrap_or(0.0),
                            }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                    }
                    "type" => {
                        if let Some(txt) = ev.text.as_deref() {
                            if !txt.is_empty() {
                                self.send_cdp(
                                    "Input.insertText",
                                    json!({ "text": txt }),
                                    Some(&sid),
                                    CMD_TIMEOUT,
                                )?;
                            }
                        }
                        if ev.press_enter == Some(true) {
                            self.dispatch_key(&sid, "Enter")?;
                        }
                    }
                    "key" => {
                        let key_name = ev.key.as_deref().unwrap_or("Enter");
                        let repeat = ev.repeat.unwrap_or(1).clamp(1, 20);
                        for _ in 0..repeat {
                            if let Some((k, code, vk, cmd)) = edit_command_def(key_name) {
                                self.send_cdp(
                                    "Input.dispatchKeyEvent",
                                    json!({
                                        "type": "rawKeyDown",
                                        "key": k,
                                        "code": code,
                                        "windowsVirtualKeyCode": vk,
                                        "modifiers": 4,
                                        "commands": [cmd],
                                    }),
                                    Some(&sid),
                                    CMD_TIMEOUT,
                                )?;
                                self.send_cdp(
                                    "Input.dispatchKeyEvent",
                                    json!({
                                        "type": "keyUp",
                                        "key": k,
                                        "code": code,
                                        "windowsVirtualKeyCode": vk,
                                        "modifiers": 4,
                                    }),
                                    Some(&sid),
                                    CMD_TIMEOUT,
                                )?;
                            } else {
                                self.dispatch_key(&sid, key_name)?;
                            }
                        }
                    }
                    "navigate" => {
                        let url = normalize_url(ev.url.as_deref().unwrap_or("about:blank"));
                        self.send_cdp(
                            "Page.navigate",
                            json!({ "url": url }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                        nav_mutated = true;
                    }
                    "back" | "forward" => {
                        let hist = self.send_cdp(
                            "Page.getNavigationHistory",
                            json!({}),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                        let cur_idx = hist
                            .get("currentIndex")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        let next_idx = cur_idx + if action == "back" { -1 } else { 1 };
                        if let Some(entries) = hist.get("entries").and_then(|v| v.as_array()) {
                            if next_idx >= 0 && (next_idx as usize) < entries.len() {
                                if let Some(entry_id) = entries[next_idx as usize].get("id") {
                                    self.send_cdp(
                                        "Page.navigateToHistoryEntry",
                                        json!({ "entryId": entry_id }),
                                        Some(&sid),
                                        CMD_TIMEOUT,
                                    )?;
                                    nav_mutated = true;
                                }
                            }
                        }
                    }
                    "reload" => {
                        self.send_cdp("Page.reload", json!({}), Some(&sid), CMD_TIMEOUT)?;
                        nav_mutated = true;
                    }
                    _ => {}
                }
                Ok(())
            })();

            if let Err(err) = step_res {
                if err.is_target_gone() && self.is_connected() {
                    let dead_id = if !resolved_tid.is_empty() {
                        resolved_tid.as_str()
                    } else {
                        ask_tid.unwrap_or("")
                    };
                    {
                        self.state.lock().unwrap().handle_target_gone(dead_id, true);
                    }
                    let (_, fb_tabs) = self.resolve_target_id(None, true, true)?;
                    latest_tabs = Some(fb_tabs);
                    recovered = true;
                    tab_mutated = true;
                    // Drop remaining stale coordinate/key events intended for the closed tab
                    break;
                }
                return Err(err);
            }
        }

        let tabs = if tab_mutated || nav_mutated {
            match latest_tabs {
                Some(t) => t,
                None => self.list_tabs(true)?,
            }
        } else {
            self.state.lock().unwrap().cached_tabs.clone()
        };

        let (active_tid, epoch, seq, paused) = {
            let st = self.state.lock().unwrap();
            (st.active_target_id.clone(), st.tab_epoch, st.frame_seq, st.paused)
        };
        let t_hash = hash_tabs(&tabs);

        let mut resp = json!({
            "ok": true,
            "connected": self.is_connected(),
            "paused": paused,
            "noTabs": tabs.is_empty() && active_tid.is_none(),
            "recovered": recovered,
            "activeTargetId": active_tid,
            "epoch": epoch,
            "tabsHash": t_hash,
            "seq": seq,
        });
        if tab_mutated || nav_mutated {
            resp["tabs"] = json!(tabs);
        }
        Ok(resp)
    }

    fn dispatch_key(self: &Arc<Self>, session_id: &str, key_name: &str) -> Result<(), CdpError> {
        let (key, code, key_code, text) = key_def(key_name);
        self.send_cdp(
            "Input.dispatchKeyEvent",
            json!({
                "type": "rawKeyDown",
                "key": key,
                "code": code,
                "windowsVirtualKeyCode": key_code,
                "nativeVirtualKeyCode": key_code,
            }),
            Some(session_id),
            CMD_TIMEOUT,
        )?;
        if let Some(txt) = text {
            self.send_cdp(
                "Input.dispatchKeyEvent",
                json!({
                    "type": "char",
                    "text": txt,
                    "key": key,
                    "code": code,
                    "windowsVirtualKeyCode": key_code,
                }),
                Some(session_id),
                CMD_TIMEOUT,
            )?;
        }
        self.send_cdp(
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyUp",
                "key": key,
                "code": code,
                "windowsVirtualKeyCode": key_code,
                "nativeVirtualKeyCode": key_code,
            }),
            Some(session_id),
            CMD_TIMEOUT,
        )?;
        Ok(())
    }
}

fn edit_command_def(name: &str) -> Option<(&'static str, &'static str, i32, &'static str)> {
    match name {
        "SelectAll" => Some(("a", "KeyA", 65, "selectAll")),
        "Copy" => Some(("c", "KeyC", 67, "copy")),
        "Paste" => Some(("v", "KeyV", 86, "paste")),
        _ => None,
    }
}

fn key_def(name: &str) -> (&'static str, &'static str, i32, Option<&'static str>) {
    match name {
        "Backspace" => ("Backspace", "Backspace", 8, None),
        "Delete" => ("Delete", "Delete", 46, None),
        "Tab" => ("Tab", "Tab", 9, None),
        "Escape" => ("Escape", "Escape", 27, None),
        "ArrowUp" => ("ArrowUp", "ArrowUp", 38, None),
        "ArrowDown" => ("ArrowDown", "ArrowDown", 40, None),
        "ArrowLeft" => ("ArrowLeft", "ArrowLeft", 37, None),
        "ArrowRight" => ("ArrowRight", "ArrowRight", 39, None),
        "PageUp" => ("PageUp", "PageUp", 33, None),
        "PageDown" => ("PageDown", "PageDown", 34, None),
        "Home" => ("Home", "Home", 36, None),
        "End" => ("End", "End", 35, None),
        "Space" => (" ", "Space", 32, Some(" ")),
        _ => ("Enter", "Enter", 13, Some("\r")),
    }
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::cdp::protocol::EventCallback;
    use crate::stream::dedup::base64_encode;
    use crate::stream::profiles::PROFILE_ULTRA;
    use std::collections::HashSet;

    struct MockState {
        tabs: Vec<TabInfo>,
        sent: Vec<Value>,
        session_to_target: HashMap<String, String>,
        next_sid: u64,
        next_tab: u64,
        fail_attach_once: HashSet<String>,
        fail_click_for: HashSet<String>,
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
                    fail_attach_once: HashSet::new(),
                    fail_click_for: HashSet::new(),
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
                        Ok(json!({ "result": { "value": "[1440,900]" } }))
                    }

                    "Page.captureScreenshot" => {
                        let tid = session_id
                            .as_deref()
                            .and_then(|s| st.session_to_target.get(s))
                            .cloned()
                            .unwrap_or_default();
                        let q = params.get("quality").and_then(|v| v.as_u64()).unwrap_or(50);
                        let raw = format!("shot:{tid}:{q}");
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

    fn make_mock_client(
        tabs: Vec<TabInfo>,
    ) -> (Arc<CdpClient>, Arc<Mutex<Option<Arc<MockTransport>>>>) {
        let holder: Arc<Mutex<Option<Arc<MockTransport>>>> = Arc::new(Mutex::new(None));
        let h_clone = Arc::clone(&holder);
        let factory: TransportFactory = Arc::new(move |_url, _timeout, on_event| {
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
    fn closes_active_tab_atomically_and_warms_surviving_neighbor() {
        let (client, holder) = make_mock_client(vec![
            tab("t1", "One", "https://one.test"),
            tab("t2", "Two", "https://two.test"),
            tab("t3", "Three", "https://three.test"),
        ]);
        client
            .poll_frame(PollFrameParams {
                target_id: Some("t2".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(client.active_target_id().as_deref(), Some("t2"));
        assert_eq!(client.screencast_target_id().as_deref(), Some("t2"));

        let res = client
            .dispatch_input_batch(InputBatchParams {
                target_id: Some("t2".into()),
                events: vec![InputEvent {
                    action: "close_tab".into(),
                    target_id: Some("t2".into()),
                    x: None,
                    y: None,
                    button: None,
                    click_count: None,
                    delta_x: None,
                    delta_y: None,
                    text: None,
                    press_enter: None,
                    key: None,
                    repeat: None,
                    url: None,
                }],
            })
            .unwrap();

        assert_eq!(res["connected"], true);
        assert_eq!(res["activeTargetId"], "t3");
        let ids: Vec<&str> = res["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["targetId"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["t1", "t3"]);
        assert_eq!(client.screencast_target_id().as_deref(), Some("t3"));
        let sent = holder.lock().unwrap().as_ref().unwrap().state.lock().unwrap().sent.clone();
        assert!(sent.iter().any(|m| m["method"] == "Page.stopScreencast"));
        assert!(sent
            .iter()
            .any(|m| m["method"] == "Target.activateTarget" && m["params"]["targetId"] == "t3"));
        client.dispose();
    }

    #[test]
    fn closes_non_active_tab_without_disturbing_active_session_or_frame() {
        let (client, _) = make_mock_client(vec![
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

        let res = client.close_tab(Some("t3"), None).unwrap();
        assert_eq!(res["connected"], true);
        assert_eq!(res["activeTargetId"], "t2");
        assert_eq!(client.screencast_target_id().as_deref(), Some("t2"));
        assert_eq!(res["seq"], f1["seq"]);
        client.dispose();
    }

    #[test]
    fn double_close_does_not_close_innocent_tab() {
        let (client, _) = make_mock_client(vec![
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

        let res = client.close_tab(Some("t-nonexistent"), None).unwrap();
        assert_eq!(res["connected"], true);
        assert_eq!(res["tabs"].as_array().unwrap().len(), 2);
        assert_eq!(res["activeTargetId"], "t1");
        client.dispose();
    }

    #[test]
    fn closing_last_tab_leaves_zero_tabs_without_creating_about_blank() {
        let (client, holder) = make_mock_client(vec![tab("t1", "Only", "https://only.test")]);
        client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();

        let res = client.close_tab(Some("t1"), None).unwrap();
        assert_eq!(res["connected"], true);
        assert_eq!(res["noTabs"], true);
        assert!(res["activeTargetId"].is_null());
        assert!(res["tabs"].as_array().unwrap().is_empty());
        assert!(client.screencast_target_id().is_none());
        let sent = holder
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .state
            .lock()
            .unwrap()
            .sent
            .clone();
        assert!(
            !sent.iter().any(|m| m["method"] == "Target.createTarget"),
            "closing last tab must NOT auto-create about:blank"
        );
        client.dispose();
    }

    #[test]
    fn recovers_from_no_target_with_given_id_found_without_disconnecting() {
        let (client, holder) = make_mock_client(vec![
            tab("t1", "Dying", "https://dying.test"),
            tab("t2", "Live", "https://live.test"),
        ]);
        client.ensure_connected().unwrap();
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

        // External target destruction during input dispatch also recovers without disconnecting
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
                    target_id: None,
                    x: Some(50.0),
                    y: Some(60.0),
                    button: Some("left".into()),
                    click_count: Some(1),
                    delta_x: None,
                    delta_y: None,
                    text: None,
                    press_enter: None,
                    key: None,
                    repeat: None,
                    url: None,
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
        client.ensure_connected().unwrap();
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

        let sid = client.state.lock().unwrap().sessions.get("t1").cloned().unwrap();
        // Push byte-identical frame -> deduped at ingress!
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

        // Poll with matching frameHash & tabsHash -> zero imageDataUrl, no tabs array!
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

        // Push distinct frame -> delivered with new hash!
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
        let old_sid = client.state.lock().unwrap().sessions.get("t1").cloned().unwrap();
        client
            .dispatch_input_batch(InputBatchParams {
                target_id: Some("t1".into()),
                events: vec![InputEvent {
                    action: "switch_tab".into(),
                    target_id: Some("t2".into()),
                    x: None,
                    y: None,
                    button: None,
                    click_count: None,
                    delta_x: None,
                    delta_y: None,
                    text: None,
                    press_enter: None,
                    key: None,
                    repeat: None,
                    url: None,
                }],
            })
            .unwrap();
        let seq_before = client.state.lock().unwrap().frame_seq;

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

        assert_eq!(client.state.lock().unwrap().frame_seq, seq_before);
        assert!(client.state.lock().unwrap().latest_frame.is_none());
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
        assert!(
            !sent.iter().any(|m| m["method"] == "Target.createTarget"),
            "external last-tab close must NOT force-create about:blank"
        );

        // When user later opens a new tab in Chrome, polling attaches to it cleanly.
        mock.state
            .lock()
            .unwrap()
            .tabs
            .push(tab("t2", "Fresh", "https://fresh.test"));
        mock.push_event("Target.targetCreated", json!({ "targetInfo": { "targetId": "t2", "type": "page" } }), None);

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
    fn pause_stops_screencast_while_keeping_connection_and_resume_restarts() {
        let (client, holder) = make_mock_client(vec![tab("t1", "Live", "https://live.test")]);
        client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(client.screencast_target_id().as_deref(), Some("t1"));

        let res = client
            .dispatch_input_batch(InputBatchParams {
                target_id: Some("t1".into()),
                events: vec![InputEvent {
                    action: "pause".into(),
                    target_id: None,
                    x: None,
                    y: None,
                    button: None,
                    click_count: None,
                    delta_x: None,
                    delta_y: None,
                    text: None,
                    press_enter: None,
                    key: None,
                    repeat: None,
                    url: None,
                }],
            })
            .unwrap();
        assert_eq!(res["connected"], true);
        assert_eq!(res["paused"], true);
        assert!(client.is_connected());
        assert!(client.is_paused());
        assert!(client.screencast_target_id().is_none());

        // Polling while paused must NOT restart Page.startScreencast
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
        let sc_after_pause = mock
            .state
            .lock()
            .unwrap()
            .sent
            .iter()
            .filter(|m| m["method"] == "Page.startScreencast")
            .count();
        assert_eq!(sc_before, sc_after_pause);

        // Resuming restores screencast without reconnecting
        client.resume();
        assert!(!client.is_paused());
        let live_poll = client
            .poll_frame(PollFrameParams {
                target_id: Some("t1".into()),
                quality_preset: "low".into(),
                wait_ms: 40,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(live_poll["paused"], false);
        assert_eq!(client.screencast_target_id().as_deref(), Some("t1"));
        client.dispose();
    }

    #[test]
    fn dispatches_copy_paste_and_select_all_edit_commands() {
        let (client, holder) = make_mock_client(vec![tab("t1", "Doc", "https://doc.test")]);
        client
            .dispatch_input_batch(InputBatchParams {
                target_id: Some("t1".into()),
                events: ["SelectAll", "Copy", "Paste"]
                    .into_iter()
                    .map(|k| InputEvent {
                        action: "key".into(),
                        target_id: None,
                        x: None,
                        y: None,
                        button: None,
                        click_count: None,
                        delta_x: None,
                        delta_y: None,
                        text: None,
                        press_enter: None,
                        key: Some(k.into()),
                        repeat: None,
                        url: None,
                    })
                    .collect(),
            })
            .unwrap();
        let sent = holder.lock().unwrap().as_ref().unwrap().state.lock().unwrap().sent.clone();
        for cmd in ["selectAll", "copy", "paste"] {
            assert!(sent.iter().any(|m| {
                m["method"] == "Input.dispatchKeyEvent"
                    && m["params"]["commands"]
                        .as_array()
                        .map(|a| a.iter().any(|v| v == cmd))
                        .unwrap_or(false)
            }));
        }
        client.dispose();
    }
}
