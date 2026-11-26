use crate::cdp::client::{now_iso, CdpClient, CdpError, FrameRecord, PollFrameParams};
use crate::i18n::current_lang;
use crate::stream::dedup::{hash_frame_data, hash_tabs};
use crate::stream::profiles::{profile_by_name, resolve_profile, QualityProfile, TabInfo};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SCREENCAST_IDLE_STOP: Duration = Duration::from_secs(12);
pub const SCREENCAST_TIER_DWELL: Duration = Duration::from_secs(2);
const FALLBACK_ACTIVE_WINDOW: Duration = Duration::from_millis(2500);
const FALLBACK_ACTIVE_INTERVAL: Duration = Duration::from_millis(350);
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
    pub(crate) latest_frame: Option<FrameRecord>,
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
        }
    }

    pub fn reset_on_disconnect(&mut self) {
        self.screencast_idle_paused = false;
        self.screencast_target_id = None;
        self.screencast_profile = None;
        self.screencast_started_at = None;
        self.latest_frame = None;
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
            self.latest_frame = None;
        }
    }

    pub fn clear_latest_frame(&mut self) {
        self.latest_frame = None;
    }

    pub fn is_fallback_due(&self) -> bool {
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

    /// SHA-1 deduplicates and stores an incoming screencast or fallback JPEG frame.
    /// Returns `true` if a new unique frame was committed.
    pub fn ingest_frame(
        &mut self,
        target_id: &str,
        epoch: u64,
        data: &str,
        vw: u32,
        vh: u32,
    ) -> bool {
        let hash = hash_frame_data(data);
        let is_dup = self
            .latest_frame
            .as_ref()
            .map(|f| f.target_id == target_id && f.hash == hash)
            .unwrap_or(false);
        if is_dup {
            self.deduped_frames += 1;
            if let Some(f) = self.latest_frame.as_mut() {
                f.captured_at = now_iso();
            }
            return false;
        }
        self.frame_seq += 1;
        let seq = self.frame_seq;
        self.latest_frame = Some(FrameRecord {
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
        true
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
    rtt_ms: f64,
    force_profile: bool,
) -> Result<(String, &'static QualityProfile), CdpError> {
    let qp = resolve_profile(quality_preset, rtt_ms);
    {
        let existing_sid = client.tabs.lock().unwrap().session_for(target_id);
        let mut cap = client.capture.lock().unwrap();
        cap.mark_poll();
        if cap.screencast_target_id.as_deref() == Some(target_id) {
            if let (Some(cur_name), Some(sid)) = (cap.screencast_profile.as_deref(), existing_sid) {
                if cur_name == qp.name {
                    return Ok((sid, qp));
                }
                // Minimum tier dwell-time hysteresis prevents tearing down & restarting Chrome's
                // GPU screencast on noisy back-to-back non-forced tier oscillations.
                let in_dwell = !force_profile
                    && cap
                        .screencast_started_at
                        .map(|t| t.elapsed() < SCREENCAST_TIER_DWELL)
                        .unwrap_or(false);
                if in_dwell {
                    return Ok((sid, profile_by_name(cur_name)));
                }
            }
        }
    }

    stop_screencast(client);
    let sid = client.attach(target_id)?;
    {
        let mut cap = client.capture.lock().unwrap();
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
            cap.latest_frame = None;
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
pub(crate) fn warm_active_tab(
    client: &Arc<CdpClient>,
    quality_preset: Option<&str>,
    rtt_ms: f64,
) {
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
        let _ = ensure_screencast(client, &tid, &effective_preset, rtt_ms, false);
    }
}

fn extract_layout_viewport(metrics: &Value) -> Option<(u32, u32)> {
    for ptr_pair in [
        ("/cssVisualViewport/clientWidth", "/cssVisualViewport/clientHeight"),
        ("/visualViewport/clientWidth", "/visualViewport/clientHeight"),
        ("/cssLayoutViewport/clientWidth", "/cssLayoutViewport/clientHeight"),
        ("/layoutViewport/clientWidth", "/layoutViewport/clientHeight"),
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

pub(crate) fn capture_fallback_frame(
    client: &Arc<CdpClient>,
    target_id: &str,
    session_id: &str,
    quality_preset: &str,
    rtt_ms: f64,
) -> Result<(), CdpError> {
    if client.tabs.lock().unwrap().has_dialog(target_id) {
        return Ok(());
    }
    let (mut vw, mut vh): (u32, u32) = {
        let mut cap = client.capture.lock().unwrap();
        cap.last_fallback_shot_at = Some(Instant::now());
        cap.latest_frame
            .as_ref()
            .filter(|f| f.target_id == target_id && f.viewport_width > 0 && f.viewport_height > 0)
            .map(|f| (f.viewport_width, f.viewport_height))
            .unwrap_or((1280, 800))
    };

    let qp = resolve_profile(quality_preset, rtt_ms);
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

    let (closed, epoch) = {
        let mut ts = client.tabs.lock().unwrap();
        (ts.is_recently_closed(target_id), ts.tab_epoch)
    };
    if closed {
        return Ok(());
    }
    client
        .capture
        .lock()
        .unwrap()
        .ingest_frame(target_id, epoch, data, vw, vh);
    Ok(())
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
            "imageDataUrl": "",
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

    let mut target_id = String::new();
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
            client.resolve_target_id(ask_tid, force_tabs, true)?;

        if resolved_tid.is_none() {
            // Chrome has 0 open windows/tabs: wait gently for a user-opened tab without spawning about:blank.
            if !params.force_refresh && params.wait_ms > 0 && !paused && !idle_paused {
                let wait = params.wait_ms.clamp(40, 1200);
                if client.wait_for_notify(Duration::from_millis(wait)) && client.is_connected() {
                    let (next_tid, next_tabs) = client.resolve_target_id(None, true, true)?;
                    resolved_tid = next_tid;
                    resolved_tabs = next_tabs;
                }
            }
        }

        let Some(tid) = resolved_tid else {
            let (epoch, is_paused, pause_epoch) = {
                let ts = client.tabs.lock().unwrap();
                (ts.tab_epoch, ts.paused, ts.pause_epoch)
            };
            let (seq, is_idle) = {
                let cap = client.capture.lock().unwrap();
                (cap.frame_seq, cap.screencast_idle_paused)
            };
            let t_hash = hash_tabs(&resolved_tabs);
            let include_tabs =
                params.known_tabs_hash.is_empty() || params.known_tabs_hash != t_hash;
            let mut resp = json!({
                "connected": client.is_connected(),
                "paused": is_paused,
                "pauseEpoch": pause_epoch,
                "screencastPausedIdle": is_idle,
                "noTabs": true,
                "activeTargetId": Value::Null,
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
                "imageDataUrl": "",
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

        target_id = tid;
        tabs = resolved_tabs;

        let attempt_res: Result<(), CdpError> = (|| {
            let has_open_dialog = client.tabs.lock().unwrap().has_dialog(&target_id);

            if paused || idle_paused {
                // While paused or idle-stopped, never restart continuous screencast.
                if params.force_refresh && !has_open_dialog {
                    let sid = client.attach(&target_id)?;
                    capture_fallback_frame(
                        client,
                        &target_id,
                        &sid,
                        &params.quality_preset,
                        params.rtt_ms,
                    )?;
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
                params.rtt_ms,
                params.force_refresh,
            )?;
            profile = qp;

            let has_dlg = client.tabs.lock().unwrap().has_dialog(&target_id);
            if has_dlg {
                return Ok(());
            }

            let (has_valid_frame, same_as_client_hash, cur_seq, sc_running, fb_due) = {
                let cap = client.capture.lock().unwrap();
                let sc_on = cap.screencast_target_id.as_deref() == Some(target_id.as_str());
                let due = cap.is_fallback_due();
                match &cap.latest_frame {
                    Some(f) if f.target_id == target_id => (
                        true,
                        !params.last_frame_hash.is_empty() && f.hash == params.last_frame_hash,
                        f.seq,
                        sc_on,
                        due,
                    ),
                    _ => (false, false, cap.frame_seq, sc_on, due),
                }
            };

            if params.force_refresh {
                let prev_seq = client.capture.lock().unwrap().frame_seq;
                if sc_running {
                    client.wait_for_next_frame(
                        prev_seq,
                        Duration::from_millis(160),
                        Some(&target_id),
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
                    capture_fallback_frame(
                        client,
                        &target_id,
                        &sid,
                        &params.quality_preset,
                        params.rtt_ms,
                    )?;
                }
            } else if !has_valid_frame {
                if sc_running {
                    client.wait_for_next_frame(0, Duration::from_millis(300), Some(&target_id));
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
                    capture_fallback_frame(
                        client,
                        &target_id,
                        &sid,
                        &params.quality_preset,
                        params.rtt_ms,
                    )?;
                }
            } else if !sc_running {
                // Restricted page (e.g. `chrome://`): periodically refresh via throttled screenshot fallback.
                if fb_due {
                    capture_fallback_frame(
                        client,
                        &target_id,
                        &sid,
                        &params.quality_preset,
                        params.rtt_ms,
                    )?;
                } else if params.wait_ms > 0 {
                    let _ = client.wait_for_notify(Duration::from_millis(params.wait_ms.min(350)));
                }
            } else if (cur_seq <= params.since_seq || same_as_client_hash) && params.wait_ms > 0 {
                let wait = params.wait_ms.clamp(40, 1200);
                client.wait_for_next_frame(
                    params.since_seq.max(cur_seq),
                    Duration::from_millis(wait),
                    Some(&target_id),
                );
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

    let (epoch, cur_paused, pause_epoch, dialog, popup_switched) = {
        let mut ts = client.tabs.lock().unwrap();
        let dlg = ts.dialog_value_for(&target_id);
        let popped = ts.take_popup_switched(&target_id);
        (ts.tab_epoch, ts.paused, ts.pause_epoch, dlg, popped)
    };
    let (frame, cur_idle_paused) = {
        let cap = client.capture.lock().unwrap();
        let f = cap
            .latest_frame
            .as_ref()
            .filter(|f| f.target_id == target_id)
            .cloned();
        (f, cap.screencast_idle_paused)
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
        "connected": client.is_connected(),
        "paused": cur_paused,
        "pauseEpoch": pause_epoch,
        "screencastPausedIdle": cur_idle_paused,
        "noTabs": false,
        "activeTargetId": target_id,
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
        "imageDataUrl": if changed { frame.as_ref().map(|f| f.image_data_url.as_str()).unwrap_or("") } else { "" },
        "imageBytes": if changed { frame.as_ref().map(|f| f.image_bytes).unwrap_or(0) } else { 0 },
        "qualityApplied": profile.name,
        "locale": current_lang().as_str(),
        "serverMs": t_start.elapsed().as_millis() as u64,
        "capturedAt": frame.as_ref().map(|f| f.captured_at.as_str()).unwrap_or(&now_iso()),
    });
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
