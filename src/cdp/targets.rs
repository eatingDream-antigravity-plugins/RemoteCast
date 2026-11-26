use crate::cdp::client::{CdpClient, CdpError, JsDialogInfo, CMD_TIMEOUT};
use crate::cdp::input::InputDispatcher;
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
    pub(crate) sessions: HashMap<String, String>, // targetId -> sessionId
    pub(crate) session_to_target: HashMap<String, String>, // sessionId -> targetId
    pub(crate) dialogs: HashMap<String, JsDialogInfo>, // targetId -> open modal JS dialog
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

    pub fn reset_on_disconnect(&mut self, had_transport: bool) {
        if had_transport {
            self.set_paused(false);
            self.tab_epoch += 1;
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
        if let Some(old) = self
            .sessions
            .insert(target_id.to_string(), session_id.to_string())
        {
            self.session_to_target.remove(&old);
        }
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
        if self.popup_switched_target.as_deref() != Some(target_id) {
            self.popup_switched_target = None;
        }
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
        let safe_req = if !update_active
            || (popup_switched.is_some() && popup_switched == self.active_target_id.as_deref())
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

    pub fn validate_input_target(
        &mut self,
        requested_id: Option<&str>,
        expected_epoch: u64,
    ) -> Result<Option<String>, &'static str> {
        if expected_epoch != self.tab_epoch {
            return Err("The tab epoch changed");
        }
        if requested_id.is_none() && self.cached_tabs.is_empty() {
            return Ok(None);
        }
        let target = requested_id
            .or(self.active_target_id.as_deref())
            .or_else(|| self.cached_tabs.first().map(|tab| tab.target_id.as_str()))
            .map(String::from);
        let Some(target) = target else {
            return Ok(None);
        };
        if self.is_recently_closed(&target)
            || !self.cached_tabs.iter().any(|tab| tab.target_id == target)
        {
            return Err("The input target is missing or closed");
        }
        if self
            .active_target_id
            .as_ref()
            .is_some_and(|active| active != &target)
        {
            return Err("The active tab switched away from the input target");
        }
        if self.active_target_id.is_none() {
            self.set_active_target(&target, false);
        }
        Ok(Some(target))
    }
}

pub(crate) fn list_tabs(client: &Arc<CdpClient>, force: bool) -> Result<Vec<TabInfo>, CdpError> {
    if let Some(fresh) = client.tabs.lock().unwrap().cached_tabs_if_fresh(force) {
        return Ok(fresh);
    }

    let generation = client.transport_generation();
    let mut res = client.send_cdp_at_generation(
        Some(generation),
        "Target.getTargets",
        json!({}),
        None,
        CMD_TIMEOUT,
    )?;
    let raw = match res
        .as_object_mut()
        .and_then(|obj| obj.remove("targetInfos"))
    {
        Some(Value::Array(tabs)) => tabs,
        _ => Vec::new(),
    };
    client.with_current_tabs(generation, |tabs| tabs.update_cached_tabs(raw))
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
            if client
                .capture
                .lock()
                .unwrap()
                .screencast_target_id
                .is_some()
            {
                client.stop_screencast();
            }
        }
        return Ok((None, Vec::new()));
    }

    let chosen =
        client
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

    let generation = client.transport_generation();
    let res = client.send_cdp_at_generation(
        Some(generation),
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

    client.with_current_tabs(generation, |tabs| {
        if tabs.is_recently_closed(target_id) {
            return Err(CdpError::new("No target with given id found"));
        }
        tabs.record_attach(target_id, &sid);
        Ok(())
    })??;
    // Enable Page domain non-blockingly so JS modal dialogs (`alert`/`confirm`/`prompt`/`beforeunload`)
    // fire `Page.javascriptDialogOpening` instead of silently freezing the tab.
    client.send_no_wait("Page.enable", json!({}), Some(&sid));
    Ok(sid)
}

pub(crate) fn close_tab(
    client: &Arc<CdpClient>,
    target_id_to_close: Option<&str>,
    target_epoch: Option<u64>,
    quality_preset: Option<&str>,
) -> Result<Value, CdpError> {
    let started_at = Instant::now();
    let stale = |reason| InputDispatcher::stale_target(client, reason, started_at);
    let Some(close_id) = target_id_to_close.filter(|id| !id.is_empty()) else {
        return Ok(stale("Closing a tab requires an explicit targetId"));
    };
    let expected_epoch = target_epoch.unwrap_or_else(|| client.tab_epoch());
    if expected_epoch != client.tab_epoch() {
        return Ok(stale("The tab epoch changed"));
    }
    if !client.is_connected() {
        if target_epoch.is_some() {
            return Ok(stale("The tab to close is missing or closed"));
        }
        return Ok(json!({
            "ok": false,
            "connected": false,
            "paused": false,
            "pauseEpoch": client.pause_epoch(),
            "portFile": client.public_port_file(),
            "locale": current_lang().as_str(),
        }));
    }
    // Validate once before the refresh and again against the tab set at reservation.
    let generation = client.transport_generation();
    let before_tabs = match list_tabs(client, true) {
        Err(_) if client.tab_epoch() != expected_epoch => {
            return Ok(stale("The tab epoch changed"))
        }
        tabs => tabs?,
    };
    let Some(close_idx) = before_tabs.iter().position(|tab| tab.target_id == close_id) else {
        return Ok(stale("The tab to close is missing or closed"));
    };
    let reservation = client.with_current_tabs(generation, |tabs| {
        if tabs.tab_epoch != expected_epoch {
            return Err("The tab epoch changed");
        }
        if tabs.is_recently_closed(close_id)
            || !tabs.cached_tabs.iter().any(|tab| tab.target_id == close_id)
        {
            return Err("The tab to close is missing or closed");
        }
        tabs.mark_recently_closed(close_id);
        Ok(tabs.active_target_id.as_deref() == Some(close_id))
    });
    let was_active = match reservation {
        Ok(Ok(was_active)) => was_active,
        Ok(Err(reason)) => return Ok(stale(reason)),
        Err(_) => return Ok(stale("The Chrome connection changed")),
    };

    let result = client
        .send_cdp_at_generation(
            Some(generation),
            "Target.closeTarget",
            json!({ "targetId": close_id }),
            None,
            Duration::from_secs(5),
        )
        .and_then(|reply| {
            if reply["success"] == false {
                Err(CdpError::new("Chrome did not close the tab"))
            } else {
                Ok(reply)
            }
        });
    if let Err(err) = result {
        if !err.is_target_gone() {
            let _ = client.with_current_tabs(generation, |tabs| {
                tabs.recently_closed.remove(close_id);
                tabs.invalidate_tab_cache();
            });
            return Err(err);
        }
    }
    client.with_current_tabs(generation, |tabs| {
        tabs.mark_target_gone(close_id, true);
        let mut capture = client.capture.lock().unwrap();
        capture.clear_screencast_if_target(close_id);
        capture.clear_frame_if_target(close_id);
    })?;
    client.capture.lock().unwrap().notify_activity();

    // Refresh surviving targets (retry briefly while Chrome tears down renderer)
    let mut remaining = Vec::new();
    for attempt in 0..2 {
        remaining = list_tabs(client, true)?;
        if !remaining.is_empty() || attempt == 1 {
            break;
        }
        thread::sleep(Duration::from_millis(35));
    }

    if remaining.is_empty() {
        // Respect closing the last tab / Chrome window without spawning `about:blank`.
        client.with_current_tabs(generation, |tabs| {
            if tabs.active_target_id.is_none() && tabs.cached_tabs.is_empty() {
                tabs.clear_no_tabs();
                client.capture.lock().unwrap().clear_latest_frame();
            }
        })?;
    } else {
        let (active, epoch) = client.with_current_tabs(generation, |tabs| {
            (tabs.active_target_id.clone(), tabs.tab_epoch)
        })?;
        let active_still_valid = remaining
            .iter()
            .any(|tab| active.as_deref() == Some(&tab.target_id));
        if !active_still_valid || (was_active && epoch == expected_epoch + 1) {
            let next_tid = &remaining[close_idx.min(remaining.len() - 1)].target_id;
            let activation = client.send_cdp_at_generation(
                Some(generation),
                "Target.activateTarget",
                json!({ "targetId": next_tid }),
                None,
                Duration::from_secs(3),
            );
            client.with_current_tabs(generation, |tabs| {
                if activation.is_ok() && tabs.tab_epoch == epoch {
                    tabs.set_active_target(next_tid, true);
                }
            })?;
        }
    }

    let (active_tid, paused) = {
        let ts = client.tabs.lock().unwrap();
        (ts.active_target_id.clone(), ts.paused)
    };
    let (sc_tid, sc_q) = {
        let cap = client.capture.lock().unwrap();
        (
            cap.screencast_target_id.clone(),
            cap.screencast_quality.clone(),
        )
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
    let (active_tid, epoch, pause_epoch, dialog) = {
        let ts = client.tabs.lock().unwrap();
        (
            ts.active_target_id.clone(),
            ts.tab_epoch,
            ts.pause_epoch,
            ts.active_dialog_value(),
        )
    };
    let seq = client.capture.lock().unwrap().frame_seq;
    let t_hash = hash_tabs(&remaining);

    Ok(json!({
        "ok": true,
        "connected": client.is_connected(),
        "paused": paused,
        "pauseEpoch": pause_epoch,
        "screencastPausedIdle": false,
        "noTabs": remaining.is_empty() && active_tid.is_none(),
        "activeTargetId": active_tid,
        "dialog": dialog,
        "epoch": epoch,
        "tabs": remaining,
        "tabsHash": t_hash,
        "seq": seq,
    }))
}
