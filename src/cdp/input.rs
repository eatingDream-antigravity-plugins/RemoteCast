use crate::cdp::client::{CdpClient, CdpError, InputBatchParams, CMD_TIMEOUT};
use crate::i18n::current_lang;
use crate::stream::dedup::{hash_tabs, normalize_url};
use crate::stream::profiles::TabInfo;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Stateless input/navigation dispatcher operating on `CdpClient`'s orthogonal
/// `TabStateStore` and `FramePipeline` via explicit methods.
pub struct InputDispatcher;

impl InputDispatcher {
    /// Dispatches batched input/tab/dialog events WITHOUT ever auto-dialing Chrome when disconnected.
    pub fn dispatch_batch(
        client: &Arc<CdpClient>,
        params: InputBatchParams,
    ) -> Result<Value, CdpError> {
        let t_start = Instant::now();
        if !client.is_connected() {
            if params.events.iter().any(|ev| ev.action == "disconnect") {
                return Ok(Self::disconnect(client, t_start));
            }
            return Ok(json!({
                "ok": false,
                "connected": false,
                "paused": false,
                "pauseEpoch": client.pause_epoch(),
                "screencastPausedIdle": false,
                "portFile": client.public_port_file(),
                "locale": current_lang().as_str(),
                "serverMs": t_start.elapsed().as_millis() as u64,
            }));
        }

        let mut was_idle_paused = false;
        let mut tab_mutated = false;
        let mut nav_mutated = false;
        let mut recovered = false;
        let mut latest_tabs: Option<Vec<TabInfo>> = None;
        let mut batch_epoch = params.target_epoch;
        let mut batch_target: Option<String> = None;

        for ev in &params.events {
            let action = ev.action.as_str();
            if action.is_empty() {
                continue;
            }
            if action == "disconnect" {
                return Ok(Self::disconnect(client, t_start));
            }

            if action == "pause" {
                client.capture.lock().unwrap().mark_poll();
                client.pause();
                return Ok(json!({
                    "ok": true,
                    "connected": client.is_connected(),
                    "paused": true,
                    "pauseEpoch": client.pause_epoch(),
                    "screencastPausedIdle": false,
                    "portFile": client.public_port_file(),
                    "serverMs": t_start.elapsed().as_millis() as u64,
                }));
            }

            if matches!(action, "resume" | "new_tab" | "switch_tab") {
                was_idle_paused |= client.capture.lock().unwrap().notify_activity();
            }
            if action == "resume" {
                client.resume();
                continue;
            }

            if action == "new_tab" {
                Self::open_tab(client, ev.url.as_deref())?;
                tab_mutated = true;
                continue;
            }

            if action == "close_tab" {
                let tid = ev.target_id.as_deref().or(params.target_id.as_deref());
                let mut res = client.close_tab(tid, batch_epoch, None)?;
                if res["ok"] == false {
                    res["serverMs"] = json!(t_start.elapsed().as_millis() as u64);
                    return Ok(res);
                }
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
                let cur_tabs = client.list_tabs(false)?;
                let active = client.active_target_id();
                let chosen = req
                    .and_then(|id| cur_tabs.iter().find(|tab| tab.target_id == id))
                    .or_else(|| {
                        cur_tabs
                            .iter()
                            .find(|tab| Some(tab.target_id.as_str()) == active.as_deref())
                    })
                    .or_else(|| cur_tabs.first());
                let Some(tid) = chosen.map(|tab| tab.target_id.as_str()) else {
                    client.stop_screencast();
                    client.tabs.lock().unwrap().clear_no_tabs();
                    client.capture.lock().unwrap().clear_latest_frame();
                    latest_tabs = Some(cur_tabs);
                    tab_mutated = true;
                    continue;
                };
                match client.send_cdp(
                    "Target.activateTarget",
                    json!({ "targetId": tid }),
                    None,
                    Duration::from_secs(3),
                ) {
                    Ok(_) => {
                        client.tabs.lock().unwrap().set_active_target(tid, true);
                        client.capture.lock().unwrap().clear_latest_frame();
                        tab_mutated = true;
                    }
                    Err(err) if err.is_target_gone() && client.is_connected() => {
                        client.handle_target_gone(tid, true);
                        let (fb_tid, fb_tabs) = client.resolve_target_id(None, true, true)?;
                        if let Some(fb) = fb_tid {
                            let _ = client.send_cdp(
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

            if !matches!(
                action,
                "click"
                    | "mousedown"
                    | "mouseup"
                    | "mousemove"
                    | "scroll"
                    | "type"
                    | "key"
                    | "dialog"
                    | "navigate"
                    | "back"
                    | "forward"
                    | "reload"
            ) {
                continue;
            }

            let current_epoch = client.tab_epoch();
            let expected_epoch = *batch_epoch.get_or_insert(current_epoch);
            if expected_epoch != current_epoch {
                return Ok(Self::stale_target(client, "The tab epoch changed", t_start));
            }
            let bound_target = params.target_id.as_deref().or(batch_target.as_deref());
            if let (Some(event_target), Some(bound_target)) =
                (ev.target_id.as_deref(), bound_target)
            {
                if event_target != bound_target {
                    return Ok(Self::stale_target(
                        client,
                        "The event target differs from the batch target",
                        t_start,
                    ));
                }
            }
            client.list_tabs(batch_target.is_none())?;
            let ask_tid = ev.target_id.as_deref().or(bound_target);
            let target = client
                .tabs
                .lock()
                .unwrap()
                .validate_input_target(ask_tid, expected_epoch);
            let tid = match target {
                Ok(Some(tid)) => tid,
                Ok(None) if action == "navigate" => {
                    was_idle_paused |= client.capture.lock().unwrap().notify_activity();
                    Self::open_tab(client, ev.url.as_deref())?;
                    tab_mutated = true;
                    continue;
                }
                Ok(None) => return Ok(Self::stale_target(client, "No active tab", t_start)),
                Err(reason) => return Ok(Self::stale_target(client, reason, t_start)),
            };
            batch_target.get_or_insert_with(|| tid.clone());
            let sid = match client.attach(&tid) {
                Ok(sid) => sid,
                Err(err) if err.is_target_gone() && client.is_connected() => {
                    client.handle_target_gone(&tid, true);
                    return Ok(Self::stale_target(
                        client,
                        "The input target closed",
                        t_start,
                    ));
                }
                Err(err) => return Err(err),
            };
            let check = {
                let mut tabs = client.tabs.lock().unwrap();
                let check = tabs.validate_input_target(Some(&tid), expected_epoch);
                if check.is_ok()
                    && matches!(
                        action,
                        "click" | "mousedown" | "mouseup" | "key" | "type" | "navigate"
                    )
                {
                    tabs.mark_popup_trigger();
                }
                check
            };
            if let Err(reason) = check {
                return Ok(Self::stale_target(client, reason, t_start));
            }
            was_idle_paused |= client.capture.lock().unwrap().begin_input();

            let step_res: Result<(), CdpError> = (|| {
                let mods = ev.modifiers.unwrap_or(0).clamp(0, 15);

                match action {
                    "dialog" => {
                        let accept = ev.accept.unwrap_or(true);
                        let mut dlg_params = json!({ "accept": accept });
                        if let Some(pt) = ev.prompt_text.as_deref().or(ev.text.as_deref()) {
                            dlg_params["promptText"] = json!(pt);
                        }
                        let res = client.send_cdp(
                            "Page.handleJavaScriptDialog",
                            dlg_params,
                            Some(&sid),
                            Duration::from_secs(5),
                        );
                        client.tabs.lock().unwrap().clear_dialog(&tid);
                        if let Err(err) = res {
                            if !err.message.to_ascii_lowercase().contains("no dialog") {
                                return Err(err);
                            }
                        }
                    }
                    "mousemove" => {
                        let mx = ev.x.unwrap_or(0.0).max(0.0).round() as i64;
                        let my = ev.y.unwrap_or(0.0).max(0.0).round() as i64;
                        let btns = ev.buttons.unwrap_or(0);
                        let btn = match ev.button.as_deref() {
                            Some("left") => "left",
                            Some("right") => "right",
                            Some("middle") => "middle",
                            _ if btns & 1 != 0 => "left",
                            _ if btns & 2 != 0 => "right",
                            _ if btns & 4 != 0 => "middle",
                            _ => "none",
                        };
                        client.send_cdp(
                            "Input.dispatchMouseEvent",
                            json!({
                                "type": "mouseMoved",
                                "x": mx,
                                "y": my,
                                "button": btn,
                                "buttons": btns,
                                "modifiers": mods,
                            }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                    }
                    "mousedown" => {
                        let cx = ev.x.unwrap_or(0.0).max(0.0).round() as i64;
                        let cy = ev.y.unwrap_or(0.0).max(0.0).round() as i64;
                        let (btn, default_mask) = mouse_button_and_mask(ev.button.as_deref());
                        let btns = ev.buttons.unwrap_or(default_mask);
                        let count = ev.click_count.unwrap_or(1).clamp(1, 3);
                        client.send_cdp(
                            "Input.dispatchMouseEvent",
                            json!({
                                "type": "mousePressed",
                                "x": cx,
                                "y": cy,
                                "button": btn,
                                "buttons": btns,
                                "clickCount": count,
                                "modifiers": mods,
                            }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                    }
                    "mouseup" => {
                        let cx = ev.x.unwrap_or(0.0).max(0.0).round() as i64;
                        let cy = ev.y.unwrap_or(0.0).max(0.0).round() as i64;
                        let (btn, _) = mouse_button_and_mask(ev.button.as_deref());
                        let btns = ev.buttons.unwrap_or(0);
                        let count = ev.click_count.unwrap_or(1).clamp(1, 3);
                        client.send_cdp(
                            "Input.dispatchMouseEvent",
                            json!({
                                "type": "mouseReleased",
                                "x": cx,
                                "y": cy,
                                "button": btn,
                                "buttons": btns,
                                "clickCount": count,
                                "modifiers": mods,
                            }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                    }
                    "click" => {
                        let cx = ev.x.unwrap_or(0.0).max(0.0).round() as i64;
                        let cy = ev.y.unwrap_or(0.0).max(0.0).round() as i64;
                        let (btn, mask) = mouse_button_and_mask(ev.button.as_deref());
                        let count = ev.click_count.unwrap_or(1).clamp(1, 3);
                        client.send_cdp(
                            "Input.dispatchMouseEvent",
                            json!({
                                "type": "mouseMoved",
                                "x": cx,
                                "y": cy,
                                "button": "none",
                                "buttons": 0,
                                "modifiers": mods,
                            }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                        client.send_cdp(
                            "Input.dispatchMouseEvent",
                            json!({
                                "type": "mousePressed",
                                "x": cx,
                                "y": cy,
                                "button": btn,
                                "buttons": mask,
                                "clickCount": count,
                                "modifiers": mods,
                            }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                        client.send_cdp(
                            "Input.dispatchMouseEvent",
                            json!({
                                "type": "mouseReleased",
                                "x": cx,
                                "y": cy,
                                "button": btn,
                                "buttons": 0,
                                "clickCount": count,
                                "modifiers": mods,
                            }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                    }
                    "scroll" => {
                        let sx = ev.x.unwrap_or(500.0).max(0.0).round() as i64;
                        let sy = ev.y.unwrap_or(350.0).max(0.0).round() as i64;
                        client.send_cdp(
                            "Input.dispatchMouseEvent",
                            json!({
                                "type": "mouseWheel",
                                "x": sx,
                                "y": sy,
                                "deltaX": ev.delta_x.unwrap_or(0.0),
                                "deltaY": ev.delta_y.unwrap_or(0.0),
                                "modifiers": mods,
                            }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                    }
                    "type" => {
                        if let Some(txt) = ev.text.as_deref() {
                            if !txt.is_empty() {
                                client.send_cdp(
                                    "Input.insertText",
                                    json!({ "text": txt }),
                                    Some(&sid),
                                    CMD_TIMEOUT,
                                )?;
                            }
                        }
                        if ev.press_enter == Some(true) {
                            Self::dispatch_key(client, &sid, "Enter", 0)?;
                        }
                    }
                    "key" => {
                        let key_name = ev.key.as_deref().unwrap_or("Enter");
                        let repeat = ev.repeat.unwrap_or(1).clamp(1, 20);
                        let base_cmd_mod = platform_cmd_modifier();
                        for _ in 0..repeat {
                            if let Some((k, code, vk, cmd_mods, cmd)) =
                                edit_command_def_for_os(key_name, base_cmd_mod)
                            {
                                client.send_cdp(
                                    "Input.dispatchKeyEvent",
                                    json!({
                                        "type": "rawKeyDown",
                                        "key": k,
                                        "code": code,
                                        "windowsVirtualKeyCode": vk,
                                        "modifiers": cmd_mods,
                                        "commands": [cmd],
                                    }),
                                    Some(&sid),
                                    CMD_TIMEOUT,
                                )?;
                                client.send_cdp(
                                    "Input.dispatchKeyEvent",
                                    json!({
                                        "type": "keyUp",
                                        "key": k,
                                        "code": code,
                                        "windowsVirtualKeyCode": vk,
                                        "modifiers": cmd_mods,
                                    }),
                                    Some(&sid),
                                    CMD_TIMEOUT,
                                )?;
                            } else {
                                Self::dispatch_key(client, &sid, key_name, mods)?;
                            }
                        }
                    }
                    "navigate" => {
                        let url = normalize_url(ev.url.as_deref().unwrap_or("about:blank"));
                        client.send_cdp(
                            "Page.navigate",
                            json!({ "url": url }),
                            Some(&sid),
                            CMD_TIMEOUT,
                        )?;
                        nav_mutated = true;
                    }
                    "back" | "forward" => {
                        let hist = client.send_cdp(
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
                                    client.send_cdp(
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
                        client.send_cdp("Page.reload", json!({}), Some(&sid), CMD_TIMEOUT)?;
                        nav_mutated = true;
                    }
                    _ => {}
                }
                Ok(())
            })();

            {
                let tabs = client.tabs.lock().unwrap();
                let still_active = tabs.tab_epoch == expected_epoch
                    && tabs.active_target_id.as_deref() == Some(&tid);
                client
                    .capture
                    .lock()
                    .unwrap()
                    .finish_input(still_active && step_res.is_ok());
            }
            client.notify_frame_waiters();

            if let Err(err) = step_res {
                if err.is_target_gone() && client.is_connected() {
                    client.handle_target_gone(&tid, true);
                    return Ok(Self::stale_target(
                        client,
                        "The input target closed",
                        t_start,
                    ));
                }
                return Err(err);
            }
        }

        // Immediately resume GPU screencast if this interaction woke an idle-paused session
        // or switched tabs while unpaused.
        let (active_now, is_paused_now) = {
            let ts = client.tabs.lock().unwrap();
            (ts.active_target_id.clone(), ts.paused)
        };
        let (wake_tid, wake_q) = {
            let cap = client.capture.lock().unwrap();
            let need_sc = !is_paused_now
                && (was_idle_paused
                    || (tab_mutated
                        && active_now.is_some()
                        && cap.screencast_target_id != active_now));
            let tid = if need_sc { active_now } else { None };
            (tid, cap.screencast_quality.clone())
        };
        if !is_paused_now {
            if let Some(tid) = wake_tid {
                if !client.tabs.lock().unwrap().has_dialog(&tid) {
                    let _ = client.ensure_screencast(&tid, &wake_q, 0.0);
                }
            }
        }

        let tabs = if tab_mutated || nav_mutated {
            match latest_tabs {
                Some(t) => t,
                None => client.list_tabs(true)?,
            }
        } else {
            client.tabs.lock().unwrap().cached_tabs.clone()
        };

        let (active_tid, epoch, paused, pause_epoch, dialog) = {
            let ts = client.tabs.lock().unwrap();
            (
                ts.active_target_id.clone(),
                ts.tab_epoch,
                ts.paused,
                ts.pause_epoch,
                ts.active_dialog_value(),
            )
        };
        let seq = client.capture.lock().unwrap().frame_seq;
        let t_hash = hash_tabs(&tabs);

        let mut resp = json!({
            "ok": true,
            "connected": client.is_connected(),
            "paused": paused,
            "pauseEpoch": pause_epoch,
            "screencastPausedIdle": false,
            "noTabs": tabs.is_empty() && active_tid.is_none(),
            "recovered": recovered,
            "activeTargetId": active_tid,
            "dialog": dialog,
            "epoch": epoch,
            "tabsHash": t_hash,
            "seq": seq,
            "serverMs": t_start.elapsed().as_millis() as u64,
        });
        if tab_mutated || nav_mutated {
            resp["tabs"] = json!(tabs);
        }
        Ok(resp)
    }

    fn disconnect(client: &Arc<CdpClient>, started_at: Instant) -> Value {
        client.disconnect();
        json!({
            "ok": true,
            "connected": false,
            "paused": false,
            "pauseEpoch": client.pause_epoch(),
            "screencastPausedIdle": false,
            "portFile": client.public_port_file(),
            "locale": current_lang().as_str(),
            "serverMs": started_at.elapsed().as_millis() as u64,
        })
    }

    fn open_tab(client: &Arc<CdpClient>, url: Option<&str>) -> Result<(), CdpError> {
        let target_url = normalize_url(url.unwrap_or("about:blank"));
        let created = client.send_cdp(
            "Target.createTarget",
            json!({ "url": target_url }),
            None,
            CMD_TIMEOUT,
        )?;
        let tid = created
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or_else(|| CdpError::new("Target.createTarget missing targetId"))?;
        client.tabs.lock().unwrap().activate_new_tab(tid);
        client.capture.lock().unwrap().clear_latest_frame();
        let _ = client.send_cdp(
            "Target.activateTarget",
            json!({ "targetId": tid }),
            None,
            Duration::from_secs(3),
        );
        Ok(())
    }

    pub(crate) fn stale_target(
        client: &Arc<CdpClient>,
        reason: &str,
        started_at: Instant,
    ) -> Value {
        let _ = client.list_tabs(false);
        let (active, epoch, tabs, dialog, pause_epoch, paused) = {
            let state = client.tabs.lock().unwrap();
            (
                state.active_target_id.clone(),
                state.tab_epoch,
                state.cached_tabs.clone(),
                state.active_dialog_value(),
                state.pause_epoch,
                state.paused,
            )
        };
        json!({
            "ok": false,
            "staleTarget": true,
            "needsRefresh": true,
            "recoverable": true,
            "connected": client.is_connected(),
            "error": format!("{reason}. Refresh the panel before sending input again."),
            "activeTargetId": active,
            "epoch": epoch,
            "tabsHash": hash_tabs(&tabs),
            "tabs": tabs,
            "dialog": dialog,
            "pauseEpoch": pause_epoch,
            "paused": paused,
            "seq": client.capture.lock().unwrap().frame_seq,
            "serverMs": started_at.elapsed().as_millis() as u64,
        })
    }

    fn dispatch_key(
        client: &Arc<CdpClient>,
        session_id: &str,
        key_name: &str,
        modifiers: i32,
    ) -> Result<(), CdpError> {
        let (key, code, key_code, text) = key_def(key_name);
        let mut down = json!({
            "type": "rawKeyDown",
            "key": key,
            "code": code,
            "windowsVirtualKeyCode": key_code,
            "nativeVirtualKeyCode": key_code,
            "modifiers": modifiers,
        });
        // Do not emit printable `char` when Ctrl or Meta is held.
        let emit_char = (modifiers & (2 | 4)) == 0;
        if emit_char {
            if let Some(ref txt) = text {
                down["text"] = json!(txt);
                down["unmodifiedText"] = json!(txt.to_ascii_lowercase());
            }
        }
        client.send_cdp(
            "Input.dispatchKeyEvent",
            down,
            Some(session_id),
            CMD_TIMEOUT,
        )?;

        if emit_char {
            if let Some(txt) = text {
                client.send_cdp(
                    "Input.dispatchKeyEvent",
                    json!({
                        "type": "char",
                        "text": txt,
                        "key": key,
                        "code": code,
                        "windowsVirtualKeyCode": key_code,
                        "modifiers": modifiers,
                    }),
                    Some(session_id),
                    CMD_TIMEOUT,
                )?;
            }
        }

        client.send_cdp(
            "Input.dispatchKeyEvent",
            json!({
                "type": "keyUp",
                "key": key,
                "code": code,
                "windowsVirtualKeyCode": key_code,
                "nativeVirtualKeyCode": key_code,
                "modifiers": modifiers,
            }),
            Some(session_id),
            CMD_TIMEOUT,
        )?;
        Ok(())
    }
}

pub fn mouse_button_and_mask(raw: Option<&str>) -> (&'static str, u8) {
    match raw {
        Some("right") => ("right", 2),
        Some("middle") => ("middle", 4),
        _ => ("left", 1),
    }
}

pub const fn platform_cmd_modifier() -> i32 {
    #[cfg(target_os = "macos")]
    {
        4 // Meta (Command)
    }
    #[cfg(not(target_os = "macos"))]
    {
        2 // Control
    }
}

pub fn edit_command_def_for_os(
    name: &str,
    base_cmd_mod: i32,
) -> Option<(&'static str, &'static str, i32, i32, &'static str)> {
    match name {
        "SelectAll" => Some(("a", "KeyA", 65, base_cmd_mod, "selectAll")),
        "Copy" => Some(("c", "KeyC", 67, base_cmd_mod, "copy")),
        "Paste" => Some(("v", "KeyV", 86, base_cmd_mod, "paste")),
        "Cut" => Some(("x", "KeyX", 88, base_cmd_mod, "cut")),
        "Undo" => Some(("z", "KeyZ", 90, base_cmd_mod, "undo")),
        "Redo" => Some(("z", "KeyZ", 90, base_cmd_mod | 8, "redo")),
        _ => None,
    }
}

pub fn key_def(name: &str) -> (String, String, i32, Option<String>) {
    match name {
        "Backspace" => ("Backspace".into(), "Backspace".into(), 8, None),
        "Delete" => ("Delete".into(), "Delete".into(), 46, None),
        "Tab" => ("Tab".into(), "Tab".into(), 9, None),
        "Escape" => ("Escape".into(), "Escape".into(), 27, None),
        "ArrowUp" => ("ArrowUp".into(), "ArrowUp".into(), 38, None),
        "ArrowDown" => ("ArrowDown".into(), "ArrowDown".into(), 40, None),
        "ArrowLeft" => ("ArrowLeft".into(), "ArrowLeft".into(), 37, None),
        "ArrowRight" => ("ArrowRight".into(), "ArrowRight".into(), 39, None),
        "PageUp" => ("PageUp".into(), "PageUp".into(), 33, None),
        "PageDown" => ("PageDown".into(), "PageDown".into(), 34, None),
        "Home" => ("Home".into(), "Home".into(), 36, None),
        "End" => ("End".into(), "End".into(), 35, None),
        "Space" | " " => (" ".into(), "Space".into(), 32, Some(" ".into())),
        "Enter" => ("Enter".into(), "Enter".into(), 13, Some("\r".into())),
        s if s.len() == 1 && s.is_ascii() => {
            let b = s.as_bytes()[0];
            let (code, vk): (String, i32) = match b {
                b'a'..=b'z' => (format!("Key{}", (b - 32) as char), (b - 32) as i32),
                b'A'..=b'Z' => (format!("Key{}", b as char), b as i32),
                b'0'..=b'9' => (format!("Digit{}", b as char), b as i32),
                b'/' | b'?' => ("Slash".into(), 191),
                b'.' | b'>' => ("Period".into(), 190),
                b',' | b'<' => ("Comma".into(), 188),
                b';' | b':' => ("Semicolon".into(), 186),
                b'\'' | b'"' => ("Quote".into(), 222),
                b'[' | b'{' => ("BracketLeft".into(), 219),
                b']' | b'}' => ("BracketRight".into(), 221),
                b'\\' | b'|' => ("Backslash".into(), 220),
                b'-' | b'_' => ("Minus".into(), 189),
                b'=' | b'+' => ("Equal".into(), 187),
                b'`' | b'~' => ("Backquote".into(), 192),
                b'!' => ("Digit1".into(), 49),
                b'@' => ("Digit2".into(), 50),
                b'#' => ("Digit3".into(), 51),
                b'$' => ("Digit4".into(), 52),
                b'%' => ("Digit5".into(), 53),
                b'^' => ("Digit6".into(), 54),
                b'&' => ("Digit7".into(), 55),
                b'*' => ("Digit8".into(), 56),
                b'(' => ("Digit9".into(), 57),
                b')' => ("Digit0".into(), 48),
                _ => ("Unidentified".into(), b as i32),
            };
            let txt = if (0x20..=0x7E).contains(&b) {
                Some(s.to_string())
            } else {
                None
            };
            (s.to_string(), code, vk, txt)
        }
        _ => ("Enter".into(), "Enter".into(), 13, Some("\r".into())),
    }
}
