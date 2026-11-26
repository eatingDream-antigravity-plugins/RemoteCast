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
        if params.events.iter().any(|e| e.action == "disconnect") {
            if client.is_connected() {
                client.stop_screencast();
            }
            client.disconnect();
            return Ok(json!({
                "ok": true,
                "connected": false,
                "paused": false,
                "pauseEpoch": client.pause_epoch(),
                "screencastPausedIdle": false,
                "portFile": client.public_port_file(),
                "locale": current_lang().as_str(),
                "serverMs": t_start.elapsed().as_millis() as u64,
            }));
        }

        if !client.is_connected() {
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

        let is_pause_only = params.events.iter().all(|e| e.action == "pause");
        let was_idle_paused = if !is_pause_only {
            client.capture.lock().unwrap().notify_activity()
        } else {
            client.capture.lock().unwrap().mark_poll();
            false
        };

        // Only real interactive page events arm `last_popup_trigger_at` for OAuth/popup auto-follow.
        if params.events.iter().any(|e| {
            matches!(
                e.action.as_str(),
                "click" | "mousedown" | "mouseup" | "key" | "type" | "navigate"
            )
        }) {
            client.tabs.lock().unwrap().mark_popup_trigger();
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

            if action == "pause" {
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

            if action == "resume" {
                client.resume();
                continue;
            }

            if action == "new_tab" {
                let target_url = normalize_url(ev.url.as_deref().unwrap_or("about:blank"));
                let created = client.send_cdp(
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
                client.tabs.lock().unwrap().activate_new_tab(&new_tid);
                client.capture.lock().unwrap().clear_latest_frame();
                let _ = client.send_cdp(
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
                let res = client.close_tab(tid, None)?;
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
                let (maybe_tid, cur_tabs) = client.resolve_target_id(req, false, true)?;
                let Some(tid) = maybe_tid else {
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
                        client.tabs.lock().unwrap().set_active_target(&tid, false);
                        client.capture.lock().unwrap().clear_latest_frame();
                        tab_mutated = true;
                    }
                    Err(err) if err.is_target_gone() && client.is_connected() => {
                        client.handle_target_gone(&tid, true);
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

            let ask_tid = ev.target_id.as_deref().or(params.target_id.as_deref());
            let mut resolved_tid = String::new();
            let step_res: Result<(), CdpError> = (|| {
                let (maybe_tid, cur_tabs) = client.resolve_target_id(ask_tid, false, true)?;
                let tid = match maybe_tid {
                    Some(t) => t,
                    None if action == "navigate" => {
                        // Explicit URL navigation when 0 tabs exist opens a tab for that URL.
                        let target_url = normalize_url(ev.url.as_deref().unwrap_or("about:blank"));
                        let created = client.send_cdp(
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
                        client.tabs.lock().unwrap().activate_new_tab(&new_tid);
                        client.capture.lock().unwrap().clear_latest_frame();
                        let _ = client.send_cdp(
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
                let sid = client.attach(&tid)?;
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
                        client.notify_frame_waiters();
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

            if let Err(err) = step_res {
                if err.is_target_gone() && client.is_connected() {
                    let dead_id = if !resolved_tid.is_empty() {
                        resolved_tid.as_str()
                    } else {
                        ask_tid.unwrap_or("")
                    };
                    client.handle_target_gone(dead_id, true);
                    let (_, fb_tabs) = client.resolve_target_id(None, true, true)?;
                    latest_tabs = Some(fb_tabs);
                    recovered = true;
                    tab_mutated = true;
                    // Drop remaining stale coordinate/key events intended for the closed tab
                    break;
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
        client.send_cdp("Input.dispatchKeyEvent", down, Some(session_id), CMD_TIMEOUT)?;

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
