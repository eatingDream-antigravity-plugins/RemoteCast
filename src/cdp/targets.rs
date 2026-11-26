use crate::cdp::client::{CdpClient, CdpError, JsDialogInfo, CMD_TIMEOUT};
use crate::i18n::current_lang;
use crate::stream::dedup::hash_tabs;
use crate::stream::profiles::TabInfo;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub const CLOSED_TARGET_TTL: Duration = Duration::from_secs(5);
const TAB_CACHE_TTL: Duration = Duration::from_millis(2500);
pub const POPUP_TRIGGER_WINDOW: Duration = Duration::from_millis(2500);

/// Authoritative owner of Chrome target sessions, modal JS dialogs, active tab focus,
/// monotonic `tab_epoch` and `pause_epoch`, and the dedicated popup auto-follow timer.
pub struct TabStateStore {
    pub(crate) sessions: HashMap<String, String>,          // targetId -> sessionId
    pub(crate) session_to_target: HashMap<String, String>, // sessionId -> targetId
    pub(crate) dialogs: HashMap<String, JsDialogInfo>,     // targetId -> open modal JS dialog
    pub(crate) active_target_id: Option<String>,
    pub(crate) popup_switched_target: Option<String>,
    pub(crate) tab_epoch: u64,
    pub(crate) paused: bool,
    pub(crate) pause_epoch: u64,
    pub(crate) recently_closed: HashMap<String, Instant>,
    pub(crate) cached_tabs: Vec<TabInfo>,
    pub(crate) last_tabs_at: Option<Instant>,
    pub(crate) last_popup_trigger_at: Option<Instant>,
}

impl TabStateStore {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            session_to_target: HashMap::new(),
            dialogs: HashMap::new(),
            active_target_id: None,
            popup_switched_target: None,
            tab_epoch: 1,
            paused: false,
            pause_epoch: 0,
            recently_closed: HashMap::new(),
            cached_tabs: Vec::new(),
            last_tabs_at: None,
            last_popup_trigger_at: None,
        }
    }

    pub fn reset_on_disconnect(&mut self) {
        if self.paused {
            self.paused = false;
            self.pause_epoch += 1;
        }
        self.sessions.clear();
        self.session_to_target.clear();
        self.dialogs.clear();
        self.recently_closed.clear();
        self.active_target_id = None;
        self.popup_switched_target = None;
        self.cached_tabs.clear();
        self.last_tabs_at = None;
        self.last_popup_trigger_at = None;
    }

    pub fn prune_closed(&mut self) {
        let now = Instant::now();
        self.recently_closed
            .retain(|_, ts| now.duration_since(*ts) <= CLOSED_TARGET_TTL);
    }

    pub fn is_recently_closed(&mut self, target_id: &str) -> bool {
        self.prune_closed();
        self.recently_closed.contains_key(target_id)
    }

    pub fn mark_recently_closed(&mut self, target_id: &str) {
        self.recently_closed
            .insert(target_id.to_string(), Instant::now());
    }

    pub fn set_paused(&mut self, paused: bool) -> u64 {
        if self.paused != paused {
            self.paused = paused;
            self.pause_epoch += 1;
        }
        self.pause_epoch
    }

    pub fn mark_popup_trigger(&mut self) {
        self.last_popup_trigger_at = Some(Instant::now());
    }

    pub fn is_popup_trigger_recent(&self) -> bool {
        self.last_popup_trigger_at
            .map(|t| t.elapsed() < POPUP_TRIGGER_WINDOW)
            .unwrap_or(false)
    }

    pub fn record_attach(&mut self, target_id: &str, session_id: &str) {
        self.sessions
            .insert(target_id.to_string(), session_id.to_string());
        self.session_to_target
            .insert(session_id.to_string(), target_id.to_string());
    }

    pub fn session_for(&self, target_id: &str) -> Option<String> {
        self.sessions.get(target_id).cloned()
    }

    pub fn target_for_session(&self, session_id: &str) -> Option<String> {
        self.session_to_target.get(session_id).cloned()
    }

    pub fn evict_session(&mut self, target_id: &str) {
        if let Some(old_sid) = self.sessions.remove(target_id) {
            self.session_to_target.remove(&old_sid);
        }
        self.dialogs.remove(target_id);
    }

    pub fn detach_session(&mut self, sid: Option<&str>, param_tid: Option<&str>) -> Option<String> {
        let mapped_tid = sid.and_then(|s| self.session_to_target.remove(s));
        let tid = param_tid.map(String::from).or(mapped_tid)?;
        let should_remove = match (sid, self.sessions.get(&tid)) {
            (None, _) => true,
            (Some(s), Some(cur)) if cur == s => true,
            _ => false,
        };
        if should_remove {
            self.sessions.remove(&tid);
            self.dialogs.remove(&tid);
        }
        Some(tid)
    }

    pub fn open_dialog(
        &mut self,
        sid: Option<&str>,
        dialog_type: String,
        message: String,
        default_prompt: String,
        url: String,
    ) -> bool {
        let tid = sid
            .and_then(|s| self.session_to_target.get(s).cloned())
            .or_else(|| self.active_target_id.clone());
        let Some(target_id) = tid else {
            return false;
        };
        self.dialogs.insert(
            target_id.clone(),
            JsDialogInfo {
                target_id,
                dialog_type,
                message,
                default_prompt,
                url,
            },
        );
        true
    }

    pub fn close_dialog_for_session(&mut self, sid: Option<&str>) -> bool {
        let tid = sid
            .and_then(|s| self.session_to_target.get(s).cloned())
            .or_else(|| self.active_target_id.clone());
        tid.map(|id| self.dialogs.remove(&id).is_some())
            .unwrap_or(false)
    }

    pub fn clear_dialog(&mut self, target_id: &str) {
        self.dialogs.remove(target_id);
    }

    pub fn has_dialog(&self, target_id: &str) -> bool {
        self.dialogs.contains_key(target_id)
    }

    pub fn active_dialog_value(&self) -> Value {
        self.active_target_id
            .as_deref()
            .and_then(|tid| self.dialogs.get(tid))
            .and_then(|d| serde_json::to_value(d).ok())
            .unwrap_or(Value::Null)
    }

    pub fn dialog_value_for(&self, target_id: &str) -> Value {
        self.dialogs
            .get(target_id)
            .and_then(|d| serde_json::to_value(d).ok())
            .unwrap_or(Value::Null)
    }

    /// Evicts a dead/closed target from session/dialog/tab maps.
    /// Returns `true` if the active target changed (and `tab_epoch` was incremented).
    pub fn mark_target_gone(&mut self, target_id: &str, mark_closed: bool) -> bool {
        if target_id.is_empty() {
            return false;
        }
        if mark_closed {
            self.mark_recently_closed(target_id);
        }
        self.evict_session(target_id);
        self.cached_tabs.retain(|t| t.target_id != target_id);
        self.last_tabs_at = None;
        if self.active_target_id.as_deref() == Some(target_id) {
            self.active_target_id = self.cached_tabs.first().map(|t| t.target_id.clone());
            self.tab_epoch += 1;
            return true;
        }
        false
    }

    pub fn clear_no_tabs(&mut self) {
        if self.active_target_id.is_some() {
            self.tab_epoch += 1;
        }
        self.active_target_id = None;
        self.cached_tabs.clear();
    }

    pub fn activate_new_tab(&mut self, target_id: &str) {
        self.recently_closed.remove(target_id);
        self.active_target_id = Some(target_id.to_string());
        self.tab_epoch += 1;
        self.last_tabs_at = None;
    }

    pub fn set_active_target(&mut self, target_id: &str, bump_epoch: bool) {
        if bump_epoch
            && self
                .active_target_id
                .as_deref()
                .map(|cur| cur != target_id)
                .unwrap_or(false)
        {
            self.tab_epoch += 1;
        }
        self.active_target_id = Some(target_id.to_string());
    }

    pub fn take_popup_switched(&mut self, target_id: &str) -> bool {
        if self.popup_switched_target.as_deref() == Some(target_id) {
            self.popup_switched_target = None;
            true
        } else {
            false
        }
    }

    /// Processes `Target.targetCreated` using opener matching or dedicated `last_popup_trigger_at`.
    /// Returns `true` if focus switched to the newly created page target.
    pub fn handle_target_created(&mut self, info: Option<&Value>) -> bool {
        self.last_tabs_at = None;
        let Some(info) = info else {
            return false;
        };
        let is_page = info.get("type").and_then(|v| v.as_str()) == Some("page");
        let new_tid = info.get("targetId").and_then(|v| v.as_str()).unwrap_or("");
        let url = info.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let title = info.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let opener_id = info.get("openerId").and_then(|v| v.as_str());

        let valid_page = is_page
            && !new_tid.is_empty()
            && !url.starts_with("devtools://")
            && !(url.starts_with("chrome-extension://") && title.is_empty())
            && !self.is_recently_closed(new_tid);

        if !valid_page || self.active_target_id.as_deref() == Some(new_tid) {
            return false;
        }

        let opener_matches = opener_id.is_some() && opener_id == self.active_target_id.as_deref();
        let recent_popup_trigger = !self.paused && self.is_popup_trigger_recent();
        if self.active_target_id.is_none() || opener_matches || recent_popup_trigger {
            let was_popup = self.active_target_id.is_some();
            self.active_target_id = Some(new_tid.to_string());
            self.tab_epoch += 1;
            if was_popup {
                self.popup_switched_target = Some(new_tid.to_string());
            }
            return true;
        }
        false
    }

    pub fn invalidate_tab_cache(&mut self) {
        self.last_tabs_at = None;
    }

    pub fn cached_tabs_if_fresh(&mut self, force: bool) -> Option<Vec<TabInfo>> {
        self.prune_closed();
        if !force && !self.cached_tabs.is_empty() {
            if let Some(ts) = self.last_tabs_at {
                if ts.elapsed() < TAB_CACHE_TTL {
                    let closed = &self.recently_closed;
                    return Some(
                        self.cached_tabs
                            .iter()
                            .filter(|t| !closed.contains_key(&t.target_id))
                            .cloned()
                            .collect(),
                    );
                }
            }
        }
        None
    }

    pub fn update_cached_tabs(&mut self, raw_targets: Vec<Value>) -> Vec<TabInfo> {
        self.prune_closed();
        let mut tabs = Vec::new();
        for item in raw_targets {
            if item.get("type").and_then(|v| v.as_str()) != Some("page") {
                continue;
            }
            let Some(tid) = item.get("targetId").and_then(|v| v.as_str()) else {
                continue;
            };
            if self.recently_closed.contains_key(tid) {
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
        self.cached_tabs = tabs.clone();
        self.last_tabs_at = Some(Instant::now());
        tabs
    }

    pub fn resolve_chosen_target(
        &mut self,
        requested_id: Option<&str>,
        tabs: &[TabInfo],
        update_active: bool,
    ) -> String {
        let popup_switched = self.popup_switched_target.as_deref();
        let safe_req = if popup_switched.is_some()
            && popup_switched == self.active_target_id.as_deref()
        {
            self.active_target_id.as_deref()
        } else {
            requested_id
                .filter(|id| !id.is_empty() && !self.is_recently_closed(id))
                .or(self.active_target_id.as_deref())
        };
        let chosen = safe_req
            .and_then(|c| tabs.iter().find(|t| t.target_id == c))
            .map(|t| t.target_id.clone())
            .unwrap_or_else(|| tabs[0].target_id.clone());

        if update_active {
            self.set_active_target(&chosen, true);
        }
        chosen
    }
}

pub(crate) fn list_tabs(client: &Arc<CdpClient>, force: bool) -> Result<Vec<TabInfo>, CdpError> {
    if let Some(fresh) = client.tabs.lock().unwrap().cached_tabs_if_fresh(force) {
        return Ok(fresh);
    }

    let res = client.send_cdp("Target.getTargets", json!({}), None, CMD_TIMEOUT)?;
    let raw = res
        .get("targetInfos")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(client.tabs.lock().unwrap().update_cached_tabs(raw))
}

pub(crate) fn resolve_target_id(
    client: &Arc<CdpClient>,
    requested_id: Option<&str>,
    force_tabs: bool,
    update_active: bool,
) -> Result<(Option<String>, Vec<TabInfo>), CdpError> {
    let tabs = list_tabs(client, force_tabs)?;
    if tabs.is_empty() {
        // Never auto-create `about:blank`: let the user close the last tab / Chrome window cleanly.
        if update_active {
            client.tabs.lock().unwrap().clear_no_tabs();
            client.capture.lock().unwrap().clear_latest_frame();
        }
        let need_stop_sc = client
            .capture
            .lock()
            .unwrap()
            .screencast_target_id
            .is_some();
        if need_stop_sc {
            client.stop_screencast();
        }
        return Ok((None, Vec::new()));
    }

    let chosen = client
        .tabs
        .lock()
        .unwrap()
        .resolve_chosen_target(requested_id, &tabs, update_active);
    Ok((Some(chosen), tabs))
}

pub(crate) fn attach(client: &Arc<CdpClient>, target_id: &str) -> Result<String, CdpError> {
    {
        let mut ts = client.tabs.lock().unwrap();
        if ts.is_recently_closed(target_id) {
            return Err(CdpError {
                message: "No target with given id found".to_string(),
                cdp_code: Some(-32000),
                method: Some("Target.attachToTarget".to_string()),
            });
        }
        if let Some(sid) = ts.session_for(target_id) {
            return Ok(sid);
        }
    }

    let res = client.send_cdp(
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

    client.tabs.lock().unwrap().record_attach(target_id, &sid);
    // Enable Page domain non-blockingly so JS modal dialogs (`alert`/`confirm`/`prompt`/`beforeunload`)
    // fire `Page.javascriptDialogOpening` instead of silently freezing the tab.
    client.send_no_wait("Page.enable", json!({}), Some(&sid));
    Ok(sid)
}

pub(crate) fn close_tab(
    client: &Arc<CdpClient>,
    target_id_to_close: Option<&str>,
    quality_preset: Option<&str>,
) -> Result<Value, CdpError> {
    if !client.is_connected() {
        return Ok(json!({
            "ok": false,
            "connected": false,
            "paused": false,
            "pauseEpoch": client.pause_epoch(),
            "portFile": client.public_port_file(),
            "locale": current_lang().as_str(),
        }));
    }
    client.capture.lock().unwrap().notify_activity();

    let before_tabs = list_tabs(client, true)?;
    let close_id = target_id_to_close
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| client.active_target_id());

    let close_idx = close_id
        .as_deref()
        .and_then(|id| before_tabs.iter().position(|t| t.target_id == id));
    let was_active = close_id.as_deref().is_some()
        && client.active_target_id().as_deref() == close_id.as_deref();

    if let Some(cid) = close_id.as_deref() {
        client.tabs.lock().unwrap().mark_recently_closed(cid);
        let need_stop_sc = client
            .capture
            .lock()
            .unwrap()
            .screencast_target_id
            .as_deref()
            == Some(cid);
        if need_stop_sc {
            client.stop_screencast();
        }
        {
            let mut ts = client.tabs.lock().unwrap();
            ts.evict_session(cid);
            if was_active {
                ts.active_target_id = None;
                ts.tab_epoch += 1;
            }
        }
        {
            let mut cap = client.capture.lock().unwrap();
            cap.clear_screencast_if_target(cid);
            cap.clear_frame_if_target(cid);
        }

        // Close in Chrome only if target actually existed (prevents double-close hitting innocent tabs!)
        if close_idx.is_some() {
            if let Err(err) = client.send_cdp(
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
        client.tabs.lock().unwrap().invalidate_tab_cache();
        remaining = list_tabs(client, true)?;
        if !remaining.is_empty() || attempt >= 1 {
            break;
        }
        thread::sleep(Duration::from_millis(35));
    }

    if remaining.is_empty() {
        // Respect closing the last tab / Chrome window without spawning `about:blank`.
        client.stop_screencast();
        client.tabs.lock().unwrap().clear_no_tabs();
        client.capture.lock().unwrap().clear_latest_frame();
    } else {
        let cur_active = client.active_target_id();
        let active_still_valid = cur_active
            .as_deref()
            .map(|id| remaining.iter().any(|t| t.target_id == id))
            .unwrap_or(false);
        if was_active || !active_still_valid {
            let next_idx = close_idx
                .map(|idx| idx.min(remaining.len() - 1))
                .unwrap_or(0);
            let next_tid = remaining[next_idx].target_id.clone();
            let _ = client.send_cdp(
                "Target.activateTarget",
                json!({ "targetId": next_tid }),
                None,
                Duration::from_secs(3),
            );
            let mut ts = client.tabs.lock().unwrap();
            ts.active_target_id = Some(next_tid);
            ts.tab_epoch += 1;
        }
    }

    let (active_tid, paused) = {
        let ts = client.tabs.lock().unwrap();
        (ts.active_target_id.clone(), ts.paused)
    };
    let (sc_tid, sc_q) = {
        let cap = client.capture.lock().unwrap();
        (cap.screencast_target_id.clone(), cap.screencast_quality.clone())
    };
    if !paused {
        if let Some(tid) = active_tid.as_deref() {
            if was_active || sc_tid.as_deref() != Some(tid) {
                let q = quality_preset.unwrap_or(sc_q.as_str());
                let _ = client.ensure_screencast(tid, q, 0.0);
            }
        }
    }

    client.notify_frame_waiters();
    let (epoch, pause_epoch, dialog) = {
        let ts = client.tabs.lock().unwrap();
        (ts.tab_epoch, ts.pause_epoch, ts.active_dialog_value())
    };
    let seq = client.capture.lock().unwrap().frame_seq;
    let t_hash = hash_tabs(&remaining);

    Ok(json!({
        "ok": true,
        "connected": client.is_connected(),
        "paused": paused,
        "pauseEpoch": pause_epoch,
        "screencastPausedIdle": false,
        "noTabs": remaining.is_empty(),
        "activeTargetId": active_tid,
        "dialog": dialog,
        "epoch": epoch,
        "tabs": remaining,
        "tabsHash": t_hash,
        "seq": seq,
    }))
}
