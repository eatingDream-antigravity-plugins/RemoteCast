use crate::cdp::client::{now_iso, CdpClient, CdpError, FrameRecord, PollFrameParams};
use crate::cdp::targets::TabStateStore;
use crate::i18n::current_lang;
use crate::stream::dedup::{hash_frame_data, hash_tabs};
use crate::stream::profiles::{
    profile_by_name, resolve_profile, AdaptiveProfile, QualityProfile, TabInfo,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CRISP_SETTLE_DELAY: Duration = Duration::from_millis(380);
const MAX_SETTLE_ATTEMPTS: u8 = 3;
const SETTLE_RETRY_BACKOFF: Duration = Duration::from_millis(150);
pub const SCREENCAST_IDLE_STOP: Duration = Duration::from_secs(12);
pub const SCREENCAST_TIER_DWELL: Duration = Duration::from_secs(2);
const FALLBACK_ACTIVE_WINDOW: Duration = Duration::from_millis(2500);
const FALLBACK_ACTIVE_INTERVAL: Duration = Duration::from_millis(450);
const FALLBACK_IDLE_INTERVAL: Duration = Duration::from_millis(1100);

/// Authoritative owner of screencast/fallback frame capture, SHA-1 dedup state,
/// minimum tier dwell hysteresis, and dedicated capture idle / fallback activity clocks.
pub struct FramePipeline {
    pub(crate) screencast_target_id: Option<String>,
    pub(crate) screencast_quality: String,
    pub(crate) screencast_profile: Option<String>,
    pub(crate) screencast_started_at: Option<Instant>,
    pub(crate) screencast_idle_paused: bool,
    pub(crate) capture_idle_at: Instant,
    pub(crate) fallback_active_at: Option<Instant>,
    pub(crate) last_fallback_shot_at: Option<Instant>,
    pub(crate) last_poll_at: Instant,
    pub(crate) frame_seq: u64,
    pub(crate) deduped_frames: u64,
    pub(crate) latest_frame: Option<Arc<FrameRecord>>,
    latest_frame_captured_at: String,
    adaptive_profile: AdaptiveProfile,
    last_motion_hash: String,
    settle_at: Option<Instant>,
    settle_failures: u8,
    settle_in_flight: bool,
    inputs_in_flight: usize,
    input_generation: u64,
}

impl FramePipeline {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            screencast_target_id: None,
            screencast_quality: "low".to_string(),
            screencast_profile: None,
            screencast_started_at: None,
            screencast_idle_paused: false,
            capture_idle_at: now,
            fallback_active_at: None,
            last_fallback_shot_at: None,
            last_poll_at: now,
            frame_seq: 0,
            deduped_frames: 0,
            latest_frame: None,
            latest_frame_captured_at: String::new(),
            adaptive_profile: AdaptiveProfile::default(),
            last_motion_hash: String::new(),
            settle_at: None,
            settle_failures: 0,
            settle_in_flight: false,
            inputs_in_flight: 0,
            input_generation: 0,
        }
    }

    pub fn reset_on_disconnect(&mut self) {
        self.screencast_idle_paused = false;
        self.screencast_target_id = None;
        self.screencast_profile = None;
        self.screencast_started_at = None;
        self.clear_latest_frame();
        self.adaptive_profile = AdaptiveProfile::default();
        self.last_fallback_shot_at = None;
        self.fallback_active_at = None;
    }

    /// Explicit activity notification from input dispatch, resume, connect, or forced refresh.
    /// Returns `true` if capture was previously idle-paused.
    pub fn notify_activity(&mut self) -> bool {
        let now = Instant::now();
        self.last_poll_at = now;
        self.capture_idle_at = now;
        self.fallback_active_at = Some(now);
        std::mem::replace(&mut self.screencast_idle_paused, false)
    }

    pub fn mark_poll(&mut self) {
        self.last_poll_at = Instant::now();
    }

    pub fn clear_screencast_if_target(&mut self, target_id: &str) {
        if self.screencast_target_id.as_deref() == Some(target_id) {
            self.screencast_target_id = None;
            self.screencast_profile = None;
            self.screencast_started_at = None;
        }
    }

    pub fn clear_frame_if_target(&mut self, target_id: &str) {
        if self
            .latest_frame
            .as_ref()
            .map(|f| f.target_id.as_str() == target_id)
            .unwrap_or(false)
        {
            self.clear_latest_frame();
        }
    }

    pub fn clear_latest_frame(&mut self) {
        self.latest_frame = None;
        self.last_motion_hash.clear();
        self.settle_at = None;
        self.settle_failures = 0;
    }

    fn select_profile(&mut self, preset: &str, rtt_ms: Option<f64>) -> &'static QualityProfile {
        if preset == "auto" {
            match rtt_ms {
                Some(sample) => self.adaptive_profile.observe(sample),
                None => self.adaptive_profile.profile(),
            }
        } else {
            profile_by_name(preset)
        }
    }

    pub fn begin_input(&mut self) -> bool {
        self.inputs_in_flight += 1;
        self.input_generation += 1;
        self.notify_activity()
    }

    pub fn finish_input(&mut self, still_active: bool) {
        self.inputs_in_flight -= 1;
        if still_active {
            self.notify_activity();
            self.schedule_settle();
        }
    }

    fn schedule_settle(&mut self) {
        self.settle_at = Some(Instant::now() + CRISP_SETTLE_DELAY);
        self.settle_failures = 0;
    }

    pub fn latest_frame_is_crisp(&self) -> bool {
        self.settle_at.is_none() && self.latest_frame.as_ref().is_some_and(|frame| frame.crisp)
    }

    pub fn settle_deadline(&self, target_id: &str) -> Option<Instant> {
        self.settle_at.filter(|_| {
            self.inputs_in_flight == 0
                && self.settle_failures < MAX_SETTLE_ATTEMPTS
                && !self.settle_in_flight
                && !self.screencast_idle_paused
                && self
                    .latest_frame
                    .as_ref()
                    .is_some_and(|frame| frame.target_id == target_id)
        })
    }

    pub fn is_fallback_due(&self) -> bool {
        if self.settle_in_flight {
            return false;
        }
        let active_recently = self
            .fallback_active_at
            .map(|t| t.elapsed() < FALLBACK_ACTIVE_WINDOW)
            .unwrap_or(false);
        let interval = if active_recently {
            FALLBACK_ACTIVE_INTERVAL
        } else {
            FALLBACK_IDLE_INTERVAL
        };
        self.last_fallback_shot_at
            .map(|t| t.elapsed() >= interval)
            .unwrap_or(true)
    }

    pub fn check_idle_timeout(&mut self, connected: bool, paused: bool) -> bool {
        if !connected || paused {
            false
        } else if self.capture_idle_at.elapsed() >= SCREENCAST_IDLE_STOP {
            let had_sc = self.screencast_target_id.is_some();
            self.screencast_idle_paused = true;
            had_sc
        } else {
            false
        }
    }

    /// Keeps the last motion hash across the WebP upgrade, so a duplicate
    /// screencast frame cannot replace a settled still.
    pub fn ingest_motion_frame(&mut self, target_id: &str, data: &str, vw: u32, vh: u32) -> bool {
        let hash = hash_frame_data(data);
        let same_frame = self.latest_frame.as_ref().is_some_and(|frame| {
            frame.target_id == target_id
                && (frame.viewport_width, frame.viewport_height) == (vw, vh)
        });
        if same_frame && self.last_motion_hash == hash {
            self.deduped_frames += 1;
            if self.latest_frame.as_ref().is_some_and(|f| !f.crisp) {
                self.latest_frame_captured_at = now_iso();
            }
            return false;
        }
        self.last_motion_hash.clone_from(&hash);
        self.schedule_settle();
        self.store_frame(target_id, data, hash, (vw, vh), false);
        true
    }

    fn ingest_crisp_frame(&mut self, target_id: &str, data: &str, viewport: (u32, u32)) {
        // Include the format in the hash domain when switching encodings.
        let hash = format!("webp-{}", hash_frame_data(data));
        self.settle_at = None;
        if self.latest_frame.as_ref().is_some_and(|f| {
            f.target_id == target_id
                && f.hash == hash
                && (f.viewport_width, f.viewport_height) == viewport
        }) {
            self.deduped_frames += 1;
            self.latest_frame_captured_at = now_iso();
            return;
        }
        self.store_frame(target_id, data, hash, viewport, true);
    }

    fn store_frame(
        &mut self,
        target_id: &str,
        data: &str,
        hash: String,
        (viewport_width, viewport_height): (u32, u32),
        crisp: bool,
    ) {
        let format = if crisp { "webp" } else { "jpeg" };
        self.latest_frame_captured_at = now_iso();
        self.frame_seq += 1;
        self.latest_frame = Some(Arc::new(FrameRecord {
            seq: self.frame_seq,
            target_id: target_id.to_string(),
            hash,
            image_data_url: format!("data:image/{format};base64,{data}"),
            image_bytes: (data.len() * 3) / 4,
            crisp,
            format: format.into(),
            viewport_width,
            viewport_height,
        }));
    }
}

pub(crate) fn check_idle_screencast_stop(client: &Arc<CdpClient>) -> bool {
    let connected = client.is_connected();
    let paused = client.is_paused();
    let should_stop = client
        .capture
        .lock()
        .unwrap()
        .check_idle_timeout(connected, paused);
    if should_stop {
        stop_screencast(client);
        client.notify_frame_waiters();
    }
    should_stop
}

pub(crate) fn stop_screencast(client: &Arc<CdpClient>) {
    let _guard = client.screencast_lock.lock().unwrap();
    stop_screencast_locked(client);
}

fn stop_screencast_locked(client: &Arc<CdpClient>) {
    let prev_tid = {
        let mut cap = client.capture.lock().unwrap();
        let Some(tid) = cap.screencast_target_id.take() else {
            return;
        };
        cap.screencast_profile = None;
        cap.screencast_started_at = None;
        tid
    };
    let prev_sid = client.tabs.lock().unwrap().session_for(&prev_tid);
    if let Some(sid) = prev_sid {
        if client.is_connected() {
            let _ = client.send_cdp(
                "Page.stopScreencast",
                json!({}),
                Some(&sid),
                Duration::from_secs(3),
            );
        }
    }
}

pub(crate) fn ensure_screencast(
    client: &Arc<CdpClient>,
    target_id: &str,
    quality_preset: &str,
    rtt_ms: Option<f64>,
    force_profile: bool,
) -> Result<(String, &'static QualityProfile), CdpError> {
    let _guard = client.screencast_lock.lock().unwrap();
    let qp = {
        let tabs = client.tabs.lock().unwrap();
        let existing_sid = tabs.session_for(target_id);
        let mut cap = client.capture.lock().unwrap();
        cap.mark_poll();
        let qp = cap.select_profile(quality_preset, rtt_ms);
        if tabs.active_target_id.as_deref() != Some(target_id) {
            return Ok((existing_sid.unwrap_or_default(), qp));
        }
        if cap.screencast_target_id.as_deref() == Some(target_id) {
            if let (Some(cur_name), Some(sid)) = (cap.screencast_profile.as_deref(), existing_sid) {
                if cur_name == qp.name {
                    cap.screencast_quality = quality_preset.to_string();
                    return Ok((sid, qp));
                }
                // Minimum tier dwell-time hysteresis prevents tearing down & restarting Chrome's
                // GPU screencast on noisy back-to-back non-forced tier oscillations.
                let in_dwell = !force_profile
                    && quality_preset != "auto"
                    && cap
                        .screencast_started_at
                        .map(|t| t.elapsed() < SCREENCAST_TIER_DWELL)
                        .unwrap_or(false);
                if in_dwell {
                    return Ok((sid, profile_by_name(cur_name)));
                }
            }
        }
        qp
    };

    stop_screencast_locked(client);
    let sid = client.attach(target_id)?;
    {
        let tabs = client.tabs.lock().unwrap();
        let mut cap = client.capture.lock().unwrap();
        if tabs.paused
            || tabs.active_target_id.as_deref() != Some(target_id)
            || tabs.has_dialog(target_id)
            || cap.screencast_idle_paused
        {
            return Ok((sid, qp));
        }
        cap.screencast_target_id = Some(target_id.to_string());
        cap.screencast_quality = quality_preset.to_string();
        cap.screencast_profile = Some(qp.name.to_string());
        cap.screencast_started_at = Some(Instant::now());
        if cap
            .latest_frame
            .as_ref()
            .map(|f| f.target_id.as_str() != target_id)
            .unwrap_or(false)
        {
            cap.clear_latest_frame();
        }
    }

    let start_res = client.send_cdp(
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
        {
            let mut cap = client.capture.lock().unwrap();
            cap.screencast_target_id = None;
            cap.screencast_profile = None;
            cap.screencast_started_at = None;
        }
        if err.is_target_gone() {
            client.tabs.lock().unwrap().evict_session(target_id);
            return Err(err);
        }
        // Restricted pages (e.g. chrome://) disallow screencast; fallback screenshot handles them.
    }

    Ok((sid, qp))
}

/// Warms the active tab screencast ONLY when no screencast is currently active.
/// Never stops or downgrades an already-active high/medium stream (cuts DP7 -> FR3 side-effect).
pub(crate) fn warm_active_tab(client: &Arc<CdpClient>, quality_preset: Option<&str>, rtt_ms: f64) {
    if !client.is_connected() || client.is_paused() {
        return;
    }
    let effective_preset = {
        let cap = client.capture.lock().unwrap();
        if cap.screencast_target_id.is_some() || cap.screencast_idle_paused {
            return;
        }
        quality_preset
            .filter(|s| !s.is_empty())
            .map(String::from)
            .unwrap_or_else(|| cap.screencast_quality.clone())
    };
    let active = client.active_target_id();
    if let Ok((Some(tid), _)) = client.resolve_target_id(active.as_deref(), false, true) {
        let _ = ensure_screencast(
            client,
            &tid,
            &effective_preset,
            (rtt_ms > 0.0).then_some(rtt_ms),
            false,
        );
    }
}

fn extract_layout_viewport(metrics: &Value) -> Option<(u32, u32)> {
    for ptr_pair in [
        (
            "/cssVisualViewport/clientWidth",
            "/cssVisualViewport/clientHeight",
        ),
        (
            "/visualViewport/clientWidth",
            "/visualViewport/clientHeight",
        ),
        (
            "/cssLayoutViewport/clientWidth",
            "/cssLayoutViewport/clientHeight",
        ),
        (
            "/layoutViewport/clientWidth",
            "/layoutViewport/clientHeight",
        ),
    ] {
        let w = metrics.pointer(ptr_pair.0).and_then(|v| v.as_f64());
        let h = metrics.pointer(ptr_pair.1).and_then(|v| v.as_f64());
        if let (Some(wf), Some(hf)) = (w, h) {
            if wf > 0.0 && hf > 0.0 {
                return Some((wf.round() as u32, hf.round() as u32));
            }
        }
    }
    None
}

struct CaptureSnapshot<'a> {
    session_id: &'a str,
    target_id: &'a str,
    target_epoch: u64,
    pause_epoch: u64,
    seq: u64,
    input_generation: u64,
}

impl<'a> CaptureSnapshot<'a> {
    fn new(
        tabs: &TabStateStore,
        cap: &FramePipeline,
        target_id: &'a str,
        session_id: &'a str,
    ) -> Self {
        Self {
            session_id,
            target_id,
            target_epoch: tabs.tab_epoch,
            pause_epoch: tabs.pause_epoch,
            seq: cap.frame_seq,
            input_generation: cap.input_generation,
        }
    }

    fn is_current(&self, tabs: &TabStateStore, cap: &FramePipeline) -> bool {
        tabs.active_target_id.as_deref() == Some(self.target_id)
            && tabs.sessions.get(self.target_id).map(String::as_str) == Some(self.session_id)
            && tabs.tab_epoch == self.target_epoch
            && tabs.pause_epoch == self.pause_epoch
            && cap.frame_seq == self.seq
            && cap.input_generation == self.input_generation
            && cap.inputs_in_flight == 0
            && !tabs.has_dialog(self.target_id)
    }
}

pub(crate) fn capture_fallback_frame(
    client: &Arc<CdpClient>,
    target_id: &str,
    session_id: &str,
    qp: &QualityProfile,
) -> Result<(), CdpError> {
    let (snapshot, viewport) = {
        let tabs = client.tabs.lock().unwrap();
        let mut cap = client.capture.lock().unwrap();
        let snapshot = CaptureSnapshot::new(&tabs, &cap, target_id, session_id);
        if !snapshot.is_current(&tabs, &cap) {
            return Ok(());
        }
        cap.last_fallback_shot_at = Some(Instant::now());
        let viewport = cap
            .latest_frame
            .as_ref()
            .filter(|f| f.target_id == target_id && f.viewport_width > 0 && f.viewport_height > 0)
            .map(|f| (f.viewport_width, f.viewport_height))
            .unwrap_or((1280, 800));
        (snapshot, viewport)
    };

    let shot = request_fallback_screenshot(client, session_id, qp, viewport);
    let changed = {
        let tabs = client.tabs.lock().unwrap();
        let mut cap = client.capture.lock().unwrap();
        if !snapshot.is_current(&tabs, &cap) {
            return Ok(());
        }
        let (shot, (vw, vh)) = shot?;
        shot["data"]
            .as_str()
            .filter(|data| !data.is_empty())
            .is_some_and(|data| cap.ingest_motion_frame(target_id, data, vw, vh))
    };
    if changed {
        client.notify_frame_waiters();
    }
    Ok(())
}

fn request_fallback_screenshot(
    client: &Arc<CdpClient>,
    session_id: &str,
    qp: &QualityProfile,
    (mut vw, mut vh): (u32, u32),
) -> Result<(Value, (u32, u32)), CdpError> {
    let mut got_vp = false;

    match client.send_cdp(
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

    // Fallback to Page.getLayoutMetrics on restricted `chrome://` pages where Runtime.evaluate fails.
    if !got_vp {
        match client.send_cdp(
            "Page.getLayoutMetrics",
            json!({}),
            Some(session_id),
            Duration::from_millis(1500),
        ) {
            Ok(metrics) => {
                if let Some((mw, mh)) = extract_layout_viewport(&metrics) {
                    vw = mw;
                    vh = mh;
                    got_vp = true;
                }
            }
            Err(err) if err.is_target_gone() => return Err(err),
            Err(_) => {}
        }
    }

    let mut capture_params = json!({
        "format": "jpeg",
        "quality": qp.quality,
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

    let shot = match client.send_cdp(
        "Page.captureScreenshot",
        capture_params.clone(),
        Some(session_id),
        Duration::from_secs(8),
    ) {
        Ok(v) => v,
        Err(err) if err.is_target_gone() => return Err(err),
        Err(err) => {
            if capture_params.get("clip").is_some() {
                client.send_cdp(
                    "Page.captureScreenshot",
                    json!({
                        "format": "jpeg",
                        "quality": qp.quality,
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

    Ok((shot, (vw, vh)))
}

fn capture_crisp_if_due(
    client: &Arc<CdpClient>,
    target_id: &str,
    session_id: &str,
) -> Result<(), CdpError> {
    let (snapshot, deadline, viewport) = {
        let tabs = client.tabs.lock().unwrap();
        let mut cap = client.capture.lock().unwrap();
        let snapshot = CaptureSnapshot::new(&tabs, &cap, target_id, session_id);
        if tabs.paused || !snapshot.is_current(&tabs, &cap) {
            return Ok(());
        }
        let Some(deadline) = cap
            .settle_deadline(target_id)
            .filter(|due| *due <= Instant::now())
        else {
            return Ok(());
        };
        cap.settle_in_flight = true;
        let frame = cap.latest_frame.as_ref().unwrap();
        (
            snapshot,
            deadline,
            (frame.viewport_width, frame.viewport_height),
        )
    };

    let shot = client.send_cdp(
        "Page.captureScreenshot",
        json!({ "format": "webp", "quality": 86 }),
        Some(session_id),
        Duration::from_secs(8),
    );

    let result = {
        let tabs = client.tabs.lock().unwrap();
        let mut cap = client.capture.lock().unwrap();
        cap.settle_in_flight = false;
        if cap.settle_at != Some(deadline)
            || !snapshot.is_current(&tabs, &cap)
            || cap.screencast_idle_paused
        {
            Ok(())
        } else {
            match shot {
                Err(err) if err.is_target_gone() => Err(err),
                shot => {
                    let data = shot.as_ref().ok().and_then(|value| value["data"].as_str());
                    if let Some(data) = data.filter(|data| !data.is_empty()) {
                        cap.ingest_crisp_frame(target_id, data, viewport);
                    } else {
                        cap.settle_failures += 1;
                        cap.settle_at = Some(
                            Instant::now() + SETTLE_RETRY_BACKOFF * u32::from(cap.settle_failures),
                        );
                    }
                    Ok(())
                }
            }
        }
    };
    client.notify_frame_waiters();
    result
}

/// Polls the active tab frame and tab state WITHOUT ever auto-dialing Chrome when disconnected.
pub(crate) fn poll_frame(
    client: &Arc<CdpClient>,
    params: PollFrameParams,
) -> Result<Value, CdpError> {
    let t_start = Instant::now();
    let mut profile = resolve_profile(&params.quality_preset, params.rtt_ms);

    if !client.is_connected() {
        let port_file = client.public_port_file();
        let chrome_ready = port_file
            .get("valid")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let (epoch, pause_epoch) = {
            let ts = client.tabs.lock().unwrap();
            (ts.tab_epoch, ts.pause_epoch)
        };
        return Ok(json!({
            "connected": false,
            "connectionMode": Value::Null,
            "crisp": false,
            "format": Value::Null,
            "paused": false,
            "pauseEpoch": pause_epoch,
            "screencastPausedIdle": false,
            "noTabs": false,
            "chromeReady": chrome_ready,
            "portFile": port_file,
            "activeTargetId": Value::Null,
            "dialog": Value::Null,
            "epoch": epoch,
            "tabs": [],
            "tabsHash": "empty",
            "changed": false,
            "unchanged": true,
            "seq": params.since_seq,
            "frameHash": "",
            "imageBytes": 0,
            "qualityApplied": profile.name,
            "locale": current_lang().as_str(),
            "serverMs": t_start.elapsed().as_millis() as u64,
            "capturedAt": now_iso(),
        }));
    }

    if params.force_refresh {
        client.capture.lock().unwrap().notify_activity();
    } else {
        client.capture.lock().unwrap().mark_poll();
        check_idle_screencast_stop(client);
    }

    let paused = client.is_paused();
    let idle_paused = client.capture.lock().unwrap().screencast_idle_paused;

    let mut tabs: Vec<TabInfo> = Vec::new();
    let mut recovered = false;

    for attempt in 0..2 {
        let ask_tid = if attempt == 0 {
            params.target_id.as_deref()
        } else {
            None
        };
        let force_tabs = params.force_refresh || attempt > 0;
        let (mut resolved_tid, mut resolved_tabs) =
            client.resolve_target_id(ask_tid, force_tabs, false)?;

        if resolved_tid.is_none() {
            // Chrome has 0 open windows/tabs: wait gently for a user-opened tab without spawning about:blank.
            if !params.force_refresh && params.wait_ms > 0 && !paused && !idle_paused {
                let wait = params.wait_ms.clamp(40, 1200);
                if client.wait_for_notify(Duration::from_millis(wait)) && client.is_connected() {
                    let (next_tid, next_tabs) = client.resolve_target_id(None, true, false)?;
                    resolved_tid = next_tid;
                    resolved_tabs = next_tabs;
                }
            }
        }

        let Some(tid) = resolved_tid else {
            let (active, epoch, is_paused, pause_epoch) = {
                let ts = client.tabs.lock().unwrap();
                (
                    ts.active_target_id.clone(),
                    ts.tab_epoch,
                    ts.paused,
                    ts.pause_epoch,
                )
            };
            let (seq, is_idle) = {
                let cap = client.capture.lock().unwrap();
                (cap.frame_seq, cap.screencast_idle_paused)
            };
            let t_hash = hash_tabs(&resolved_tabs);
            let include_tabs =
                params.known_tabs_hash.is_empty() || params.known_tabs_hash != t_hash;
            let connection_mode = client.connection_mode();
            let mut resp = json!({
                "connected": connection_mode.is_some(),
                "connectionMode": connection_mode,
                "crisp": false,
                "format": Value::Null,
                "paused": is_paused,
                "pauseEpoch": pause_epoch,
                "screencastPausedIdle": is_idle,
                "noTabs": true,
                "activeTargetId": active,
                "dialog": Value::Null,
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
                "imageBytes": 0,
                "qualityApplied": profile.name,
                "locale": current_lang().as_str(),
                "serverMs": t_start.elapsed().as_millis() as u64,
                "capturedAt": now_iso(),
            });
            if include_tabs {
                resp["tabs"] = json!(resolved_tabs);
            }
            return Ok(resp);
        };

        let target_id = tid;
        tabs = resolved_tabs;

        let attempt_res: Result<(), CdpError> = (|| {
            let has_open_dialog = client.tabs.lock().unwrap().has_dialog(&target_id);

            if paused || idle_paused {
                // While paused or idle-stopped, never restart continuous screencast.
                if params.force_refresh && !has_open_dialog {
                    let sid = client.attach(&target_id)?;
                    capture_fallback_frame(client, &target_id, &sid, profile)?;
                }
                return Ok(());
            }

            if has_open_dialog {
                // Do not block on captureScreenshot/Runtime.evaluate while a modal JS dialog holds the renderer.
                return Ok(());
            }

            let (sid, qp) = ensure_screencast(
                client,
                &target_id,
                &params.quality_preset,
                Some(params.rtt_ms),
                params.force_refresh,
            )?;
            profile = qp;

            let has_dlg = client.tabs.lock().unwrap().has_dialog(&target_id);
            if has_dlg {
                return Ok(());
            }

            let (has_valid_frame, same_as_client_hash, cur_seq, sc_running, fb_due, settle_due) = {
                let cap = client.capture.lock().unwrap();
                let sc_on = cap.screencast_target_id.as_deref() == Some(target_id.as_str());
                let due = cap.is_fallback_due();
                let settle_due = cap.settle_deadline(&target_id);
                match &cap.latest_frame {
                    Some(f) if f.target_id == target_id => (
                        true,
                        !params.last_frame_hash.is_empty() && f.hash == params.last_frame_hash,
                        f.seq,
                        sc_on,
                        due,
                        settle_due,
                    ),
                    _ => (false, false, cap.frame_seq, sc_on, due, settle_due),
                }
            };

            if params.force_refresh {
                let prev_seq = client.capture.lock().unwrap().frame_seq;
                if sc_running {
                    client.wait_for_next_frame(
                        prev_seq,
                        Duration::from_millis(160),
                        Some(&target_id),
                        false,
                    );
                }
                let has_dlg_now = client.tabs.lock().unwrap().has_dialog(&target_id);
                let need_fallback = !has_dlg_now && {
                    let cap = client.capture.lock().unwrap();
                    cap.frame_seq == prev_seq
                        || cap
                            .latest_frame
                            .as_ref()
                            .map(|f| f.target_id != target_id)
                            .unwrap_or(true)
                };
                if need_fallback {
                    capture_fallback_frame(client, &target_id, &sid, profile)?;
                }
            } else if !has_valid_frame {
                if sc_running {
                    client.wait_for_next_frame(
                        0,
                        Duration::from_millis(300),
                        Some(&target_id),
                        false,
                    );
                }
                let has_dlg_now = client.tabs.lock().unwrap().has_dialog(&target_id);
                let need_fallback = !has_dlg_now && {
                    let cap = client.capture.lock().unwrap();
                    cap.latest_frame
                        .as_ref()
                        .map(|f| f.target_id != target_id)
                        .unwrap_or(true)
                };
                if need_fallback {
                    capture_fallback_frame(client, &target_id, &sid, profile)?;
                }
            } else if settle_due.is_some_and(|due| due <= Instant::now()) {
                capture_crisp_if_due(client, &target_id, &sid)?;
            } else if !sc_running && fb_due {
                // Restricted pages periodically refresh through the screenshot fallback.
                capture_fallback_frame(client, &target_id, &sid, profile)?;
            } else if (cur_seq <= params.since_seq || same_as_client_hash) && params.wait_ms > 0 {
                let wait = params.wait_ms.clamp(40, 1200);
                client.wait_for_next_frame(
                    params.since_seq.max(cur_seq),
                    Duration::from_millis(wait),
                    Some(&target_id),
                    true,
                );
                capture_crisp_if_due(client, &target_id, &sid)?;
            }

            Ok(())
        })();

        match attempt_res {
            Ok(()) => {
                let gone = client.tabs.lock().unwrap().is_recently_closed(&target_id);
                if gone && attempt == 0 {
                    recovered = true;
                    continue;
                }
                break;
            }
            Err(err) if err.is_target_gone() && client.is_connected() && attempt == 0 => {
                let wake = client.handle_target_gone(&target_id, true);
                if wake {
                    client.notify_frame_waiters();
                }
                recovered = true;
                continue;
            }
            Err(err) => return Err(err),
        }
    }

    let connection_mode = client.connection_mode();
    let (
        active,
        epoch,
        cur_paused,
        pause_epoch,
        dialog,
        popup_switched,
        frame,
        crisp,
        cur_idle_paused,
        captured_at,
    ) = {
        let mut ts = client.tabs.lock().unwrap();
        let active = ts
            .active_target_id
            .clone()
            .filter(|_| connection_mode.is_some());
        let dialog = ts.active_dialog_value();
        let popped = active
            .as_deref()
            .is_some_and(|id| ts.take_popup_switched(id));
        let cap = client.capture.lock().unwrap();
        let frame = cap
            .latest_frame
            .as_ref()
            .filter(|f| active.as_deref() == Some(f.target_id.as_str()))
            .cloned();
        let captured_at = frame.as_ref().map(|_| cap.latest_frame_captured_at.clone());
        let crisp = frame.is_some() && cap.latest_frame_is_crisp();
        (
            active,
            ts.tab_epoch,
            ts.paused,
            ts.pause_epoch,
            dialog,
            popped,
            frame,
            crisp,
            cap.screencast_idle_paused,
            captured_at,
        )
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

    if connection_mode.is_none() {
        tabs.clear();
    }
    let t_hash = hash_tabs(&tabs);
    let include_tabs = params.known_tabs_hash.is_empty() || params.known_tabs_hash != t_hash;
    let tab_meta = tabs
        .iter()
        .find(|t| active.as_deref() == Some(t.target_id.as_str()));
    let mut resp = json!({
        "connected": connection_mode.is_some(),
        "connectionMode": connection_mode,
        "crisp": crisp,
        "format": frame.as_ref().map(|f| f.format.as_str()),
        "paused": cur_paused,
        "pauseEpoch": pause_epoch,
        "screencastPausedIdle": cur_idle_paused,
        "noTabs": false,
        "activeTargetId": active,
        "dialog": dialog,
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
        "imageBytes": if changed { frame.as_ref().map(|f| f.image_bytes).unwrap_or(0) } else { 0 },
        "qualityApplied": profile.name,
        "locale": current_lang().as_str(),
        "serverMs": t_start.elapsed().as_millis() as u64,
        "capturedAt": captured_at.unwrap_or_else(now_iso),
    });
    if let Some(frame) = frame.as_ref().filter(|_| changed) {
        resp["imageDataUrl"] = json!(frame.image_data_url.as_str());
    } else if crisp {
        resp["imageDataUrl"] = json!("");
    }
    if include_tabs {
        resp["tabs"] = json!(tabs);
    }
    if recovered {
        resp["recovered"] = json!(true);
    }
    if popup_switched {
        resp["popupSwitched"] = json!(true);
    }
    Ok(resp)
}
