use crate::i18n::{current_lang, Lang};
use crate::mcp::server::{stage_status_from_snapshot, SwrBootCache};
use crate::relay::ipc::RelayBackend;
use crate::stream::dedup::{hash_frame_data, hash_tabs, is_target_gone_error};
use crate::stream::profiles::{resolve_profile, TabInfo};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const STAGE_URI: &str = "ui://chrome-stream-mcp/stage";
pub const APP_MIME: &str = "text/html;profile=mcp-app";

/// Strips sensitive local path / DevTools WS token fields and personal hostnames from any relay JSON.
pub fn redact_relay_payload(val: &mut Value) {
    if let Some(obj) = val.as_object_mut() {
        if obj.contains_key("hostname") {
            obj.insert("hostname".to_string(), json!("localhost"));
        }
        if let Some(pf) = obj.get_mut("portFile").and_then(|v| v.as_object_mut()) {
            pf.remove("path");
            pf.remove("wsUrl");
        }
    }
}

pub fn tool_definitions(include_app_only: bool) -> Value {
    let mut tools = json!([
        {
            "name": "chrome_stream_stage",
            "title": "Chrome Stream",
            "description": "Open the Chrome Stream live browser stage to view and interactively control local Google Chrome tabs without auto-connecting.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            },
            "_meta": {
                "ui": {
                    "resourceUri": STAGE_URI
                },
                "com.google.antigravity/ui": {
                    "entrypoints": [
                        { "type": "global", "title": "Chrome Stream" },
                        { "type": "aux_pane", "title": "Chrome Stream" }
                    ]
                }
            }
        },
        {
            "name": "chrome_stream_connect",
            "title": "Connect Chrome Stream",
            "description": "Connect or verify the persistent local Chrome DevTools session and warm the active tab screencast.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "qualityPreset": { "type": "string", "enum": ["auto", "ultra", "low", "medium", "high"] },
                    "rttMs": { "type": "number", "minimum": 0 },
                    "mode": { "type": "string", "enum": ["auto", "personal", "remote"], "default": "auto" }
                },
                "additionalProperties": false
            }
        },
        {
            "name": "chrome_stream_frame",
            "title": "Poll Screencast Frame",
            "description": "App-only tool: poll the active tab screencast frame, tab state, and modal JS dialog status without auto-dialing when disconnected.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "targetId": { "type": "string" },
                    "sinceSeq": { "type": "integer", "minimum": 0 },
                    "lastFrameHash": { "type": "string" },
                    "knownTabsHash": { "type": "string" },
                    "qualityPreset": { "type": "string", "enum": ["auto", "ultra", "low", "medium", "high"] },
                    "rttMs": { "type": "number", "minimum": 0 },
                    "waitMs": { "type": "integer", "minimum": 0, "maximum": 1500 },
                    "forceRefresh": { "type": "boolean" }
                },
                "additionalProperties": false
            },
            "_meta": {
                "ui": {
                    "visibility": ["app"]
                }
            }
        },
        {
            "name": "chrome_stream_input",
            "title": "Dispatch Interactive Input Batch",
            "description": "App-only tool: dispatch coalesced mouse, drag, wheel, touch, keyboard, shortcut, IME, dialog, navigation, and tab events.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "targetId": { "type": "string" },
                    "targetEpoch": { "type": "integer", "minimum": 0 },
                    "events": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "action": {
                                    "type": "string",
                                    "enum": [
                                        "click",
                                        "mousedown",
                                        "mouseup",
                                        "mousemove",
                                        "scroll",
                                        "type",
                                        "key",
                                        "dialog",
                                        "navigate",
                                        "back",
                                        "forward",
                                        "reload",
                                        "switch_tab",
                                        "new_tab",
                                        "close_tab",
                                        "pause",
                                        "resume",
                                        "disconnect"
                                    ]
                                },
                                "targetId": { "type": "string" },
                                "x": { "type": "number" },
                                "y": { "type": "number" },
                                "button": { "type": "string", "enum": ["left", "right", "middle", "none"] },
                                "buttons": { "type": "integer", "minimum": 0, "maximum": 7 },
                                "clickCount": { "type": "integer", "minimum": 1, "maximum": 3 },
                                "modifiers": { "type": "integer", "minimum": 0, "maximum": 15 },
                                "deltaX": { "type": "number" },
                                "deltaY": { "type": "number" },
                                "text": { "type": "string" },
                                "pressEnter": { "type": "boolean" },
                                "key": { "type": "string" },
                                "repeat": { "type": "integer", "minimum": 1, "maximum": 20 },
                                "url": { "type": "string" },
                                "accept": { "type": "boolean" },
                                "promptText": { "type": "string" }
                            },
                            "required": ["action"]
                        }
                    }
                },
                "required": ["events"],
                "additionalProperties": false
            },
            "_meta": {
                "ui": {
                    "visibility": ["app"]
                }
            }
        },
        {
            "name": "chrome_stream_disconnect",
            "title": "Disconnect Chrome Stream",
            "description": "Stop screencast streaming and either revoke the Chrome DevTools session or keep it authorized in standby.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["standby", "revoke"], "default": "revoke" }
                },
                "additionalProperties": false
            }
        },
        {
            "name": "chrome_stream_launch_remote",
            "title": "Launch Remote Chrome",
            "description": "App-only tool: launch or reuse dedicated Remote Chrome on loopback and warm the active tab screencast.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "qualityPreset": { "type": "string", "enum": ["auto", "ultra", "low", "medium", "high"] },
                    "rttMs": { "type": "number", "minimum": 0 }
                },
                "additionalProperties": false
            },
            "_meta": {
                "ui": {
                    "visibility": ["app"]
                }
            }
        }
    ]);
    if !include_app_only {
        tools.as_array_mut().unwrap().retain(|tool| {
            tool.pointer("/_meta/ui/visibility")
                .and_then(Value::as_array)
                .is_none_or(|visibility| {
                    !visibility.iter().any(|scope| scope == "app")
                        || visibility.iter().any(|scope| scope == "model")
                })
        });
    }
    tools
}

pub fn dispatch_tool_with_cache(
    relay: &Arc<dyn RelayBackend>,
    swr_cache: Option<&Arc<SwrBootCache>>,
    name: &str,
    args: Value,
) -> Value {
    let revoking = match name {
        "chrome_stream_disconnect" => args.get("mode").is_none_or(|mode| mode == "revoke"),
        "chrome_stream_input" => args["events"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["action"] == "disconnect")),
        _ => false,
    };
    let generation = swr_cache.map(|cache| {
        if revoking {
            cache.begin_revoke();
        }
        cache.generation()
    });
    let mut out = match name {
        "chrome_stream_stage" => handle_stage(relay, swr_cache),
        "chrome_stream_connect" => handle_connect(relay, "connect", args, Duration::from_secs(70)),
        "chrome_stream_frame" => handle_frame(relay, args),
        "chrome_stream_input" => handle_input(relay, args, revoking),
        "chrome_stream_disconnect" => handle_disconnect(relay, args),
        "chrome_stream_launch_remote" => {
            handle_connect(relay, "launch_remote", args, Duration::from_secs(85))
        }
        _ => json!({
            "isError": true,
            "content": [{ "type": "text", "text": format!("Unknown tool: {name}") }]
        }),
    };

    if let Some(sc) = out.get_mut("structuredContent") {
        redact_relay_payload(sc);
    }
    let sc = out.get("structuredContent");
    if let Some((cache, generation)) = swr_cache.zip(generation) {
        if revoking {
            cache.finish_revoke(sc.filter(|sc| sc["ok"] == true && sc["connected"] == false));
        } else if name != "chrome_stream_stage" {
            if let Some(sc) = sc.filter(|sc| sc["unconfirmed"] != true) {
                cache.update_from_tool_result(generation, sc);
            }
        }
    }
    out
}

fn safe_status(relay: &Arc<dyn RelayBackend>) -> Value {
    let mut st = relay
        .call("status", json!({}), Duration::from_secs(3))
        .unwrap_or_else(|_| {
            json!({
                "connected": false,
                "locale": current_lang().as_str(),
                "portFile": { "exists": false, "valid": false, "port": Value::Null }
            })
        });
    redact_relay_payload(&mut st);
    st
}

fn handle_stage(relay: &Arc<dyn RelayBackend>, swr_cache: Option<&Arc<SwrBootCache>>) -> Value {
    let lang = current_lang();
    let status = swr_cache
        .and_then(|cache| cache.stage_status(Duration::from_millis(1200)))
        .map(Ok)
        .unwrap_or_else(|| {
            relay
                .call("status", json!({}), Duration::from_secs(3))
                .map(|status| stage_status_from_snapshot(&status))
        });
    let mut status = match status {
        Ok(status) => status,
        Err(_) => {
            let text = match lang {
                Lang::En => "Chrome Stream relay unavailable.",
                Lang::ZhCn => "Chrome Stream 本地中继暂不可用。",
            };
            return json!({
                "isError": false,
                "content": [{ "type": "text", "text": text }],
                "structuredContent": { "connected": false, "locale": lang.as_str(), "error": text }
            });
        }
    };
    status["locale"] = json!(lang.as_str());

    let connected = status
        .get("connected")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if connected && swr_cache.is_none() {
        let r = Arc::clone(relay);
        let _ = std::thread::spawn(move || {
            let _ = r.call("warm", json!({}), Duration::from_secs(2));
        });
    }
    let port_ready = status
        .pointer("/portFile/valid")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let port_num = status
        .pointer("/portFile/port")
        .and_then(|v| v.as_u64())
        .unwrap_or(9222);

    let text = match lang {
        Lang::En => {
            let port_desc = if port_ready {
                format!("ready (port {port_num})")
            } else {
                "not detected".to_string()
            };
            format!("Chrome Stream: connected={connected}, remote debugging={port_desc}.")
        }
        Lang::ZhCn => {
            let port_desc = if port_ready {
                format!("已就绪（端口 {port_num}）")
            } else {
                "未检测到".to_string()
            };
            format!("Chrome Stream：已连接={connected}，远程调试={port_desc}。")
        }
    };

    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": status
    })
}

fn handle_connect(
    relay: &Arc<dyn RelayBackend>,
    method: &str,
    args: Value,
    timeout: Duration,
) -> Value {
    let lang = current_lang();
    match relay.call(method, args, timeout) {
        Ok(mut res) => {
            res["locale"] = json!(lang.as_str());
            let count = res
                .get("tabs")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let text = match lang {
                Lang::En => format!("Connected to Chrome ({count} tab(s))."),
                Lang::ZhCn => format!("已连接到本机 Chrome（共 {count} 个标签页）。"),
            };
            json!({
                "content": [{ "type": "text", "text": text }],
                "structuredContent": res
            })
        }
        Err(err) => {
            let st = safe_status(relay);
            let connected = st
                .get("connected")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let text = match lang {
                Lang::En => format!("Connect error: {err}"),
                Lang::ZhCn => format!("连接失败：{err}"),
            };
            json!({
                "isError": false,
                "content": [{ "type": "text", "text": text }],
                "structuredContent": {
                    "ok": false,
                    "connected": connected,
                    "connectionMode": st.get("connectionMode").cloned().unwrap_or(Value::Null),
                    "locale": lang.as_str(),
                    "error": err,
                    "portFile": st.get("portFile").cloned().unwrap_or(Value::Null),
                }
            })
        }
    }
}

fn normalize_frame(
    mut frame: Value,
    req_target_id: Option<&str>,
    last_frame_hash: &str,
    known_tabs_hash: &str,
    profile_name: &str,
    elapsed_ms: u64,
) -> Value {
    let img_url = frame
        .get("imageDataUrl")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let frame_hash = frame
        .get("frameHash")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| hash_frame_data(img_url));

    let active_tid = frame
        .get("activeTargetId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let changed = frame
        .get("changed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let same_frame = !img_url.is_empty()
        && !last_frame_hash.is_empty()
        && frame_hash == last_frame_hash
        && (req_target_id.is_none() || active_tid.is_empty() || req_target_id == Some(active_tid));
    let changed = changed && !same_frame;
    frame["changed"] = json!(changed);
    frame["unchanged"] = json!(!changed);
    if !changed {
        if frame["crisp"] == true {
            frame["imageDataUrl"] = json!("");
        } else if let Some(object) = frame.as_object_mut() {
            object.remove("imageDataUrl");
        }
        frame["imageBytes"] = json!(0);
    }

    let tabs_hash = frame
        .get("tabsHash")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| {
            frame
                .get("tabs")
                .and_then(|v| serde_json::from_value::<Vec<TabInfo>>(v.clone()).ok())
                .map(|t| hash_tabs(&t))
                .unwrap_or_default()
        });

    if !known_tabs_hash.is_empty() && !tabs_hash.is_empty() && known_tabs_hash == tabs_hash {
        if let Some(obj) = frame.as_object_mut() {
            obj.remove("tabs");
        }
    }

    frame["frameHash"] = json!(frame_hash);
    if !tabs_hash.is_empty() {
        frame["tabsHash"] = json!(tabs_hash);
    }
    if frame
        .get("qualityApplied")
        .and_then(|v| v.as_str())
        .is_none()
    {
        frame["qualityApplied"] = json!(profile_name);
    }
    if frame.get("serverMs").and_then(|v| v.as_u64()).is_none() {
        frame["serverMs"] = json!(elapsed_ms);
    }
    frame
}

fn handle_frame(relay: &Arc<dyn RelayBackend>, args: Value) -> Value {
    let t_start = Instant::now();
    let req_target_id = args
        .get("targetId")
        .and_then(|v| v.as_str())
        .map(String::from);
    let since_seq = args.get("sinceSeq").and_then(|v| v.as_u64()).unwrap_or(0);
    let last_hash = args
        .get("lastFrameHash")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let known_tabs_hash = args
        .get("knownTabsHash")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let q_preset = args
        .get("qualityPreset")
        .and_then(|v| v.as_str())
        .unwrap_or("auto");
    let rtt_ms = args.get("rttMs").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let wait_ms = args.get("waitMs").and_then(|v| v.as_u64()).unwrap_or(350);
    let force_refresh = args
        .get("forceRefresh")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let profile = resolve_profile(q_preset, rtt_ms);
    let relay_args = json!({
        "targetId": req_target_id,
        "sinceSeq": since_seq,
        "lastFrameHash": last_hash,
        "knownTabsHash": known_tabs_hash,
        "qualityPreset": q_preset,
        "rttMs": rtt_ms,
        "waitMs": wait_ms,
        "forceRefresh": force_refresh,
    });

    match relay.call("poll_frame", relay_args, Duration::from_secs(70)) {
        Ok(frame) => {
            let norm = normalize_frame(
                frame,
                req_target_id.as_deref(),
                &last_hash,
                &known_tabs_hash,
                profile.name,
                t_start.elapsed().as_millis() as u64,
            );
            let label = if norm.get("changed").and_then(|v| v.as_bool()) == Some(true) {
                format!("Frame #{}", norm["seq"].as_u64().unwrap_or(0))
            } else {
                "No change".to_string()
            };
            json!({
                "content": [{ "type": "text", "text": label }],
                "structuredContent": norm
            })
        }
        Err(err) => {
            let st = safe_status(relay);
            let still_connected = st
                .get("connected")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            // Auto-recover transparently when a target vanished while browser WS stays open!
            if still_connected && is_target_gone_error(&err, None) {
                let retry_args = json!({
                    "sinceSeq": 0,
                    "lastFrameHash": "",
                    "knownTabsHash": "",
                    "qualityPreset": q_preset,
                    "rttMs": rtt_ms,
                    "waitMs": 0,
                    "forceRefresh": true,
                });
                if let Ok(mut recovered) =
                    relay.call("poll_frame", retry_args, Duration::from_secs(30))
                {
                    recovered["recovered"] = json!(true);
                    let norm = normalize_frame(
                        recovered,
                        None,
                        "",
                        "",
                        profile.name,
                        t_start.elapsed().as_millis() as u64,
                    );
                    return json!({
                        "content": [{ "type": "text", "text": format!("Recovered frame #{}", norm["seq"].as_u64().unwrap_or(0)) }],
                        "structuredContent": norm
                    });
                }
            }

            let post = if still_connected {
                safe_status(relay)
            } else {
                st
            };
            let live = post
                .get("connected")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut sc = json!({
                "connected": live,
                "connectionMode": post.get("connectionMode").cloned().unwrap_or(Value::Null),
                "recoverable": live,
                "error": err,
                "portFile": post.get("portFile").cloned().unwrap_or(Value::Null),
                "activeTargetId": post.get("activeTargetId").cloned().unwrap_or(Value::Null),
                "dialog": post.get("dialog").cloned().unwrap_or(Value::Null),
                "changed": false,
                "unchanged": true,
                "seq": since_seq,
                "serverMs": t_start.elapsed().as_millis() as u64,
            });
            if !live {
                sc["tabs"] = json!([]);
            }
            json!({
                "isError": false,
                "content": [{ "type": "text", "text": format!("Poll error: {err}") }],
                "structuredContent": sc
            })
        }
    }
}

fn handle_input(relay: &Arc<dyn RelayBackend>, args: Value, revoking: bool) -> Value {
    let t_start = Instant::now();
    match relay.call("input", args, Duration::from_secs(30)) {
        Ok(mut res) => {
            if revoking && (res["ok"] != true || res["connected"] != false) {
                return unconfirmed_control(
                    current_lang(),
                    "The relay did not confirm revoking the Chrome connection",
                );
            }
            if res.get("serverMs").and_then(|v| v.as_u64()).is_none() {
                res["serverMs"] = json!(t_start.elapsed().as_millis() as u64);
            }
            let label = res.get("error").and_then(Value::as_str).unwrap_or("ok");
            json!({
                "content": [{ "type": "text", "text": label }],
                "structuredContent": res
            })
        }
        Err(err) if revoking => {
            unconfirmed_control(current_lang(), sanitized_control_error("input", &err))
        }
        Err(err) => {
            let st = safe_status(relay);
            let live = st
                .get("connected")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if live && is_target_gone_error(&err, None) {
                let rec = relay
                    .call(
                        "poll_frame",
                        json!({ "sinceSeq": 0, "waitMs": 0, "forceRefresh": true }),
                        Duration::from_secs(15),
                    )
                    .ok();
                return json!({
                    "isError": false,
                    "content": [{ "type": "text", "text": "The input target closed. Refresh the panel before sending input again." }],
                    "structuredContent": {
                        "ok": false,
                        "staleTarget": true,
                        "needsRefresh": true,
                        "recoverable": true,
                        "connected": true,
                        "error": "The input target closed. Refresh the panel before sending input again.",
                        "activeTargetId": rec.as_ref().and_then(|r| r.get("activeTargetId")).cloned().unwrap_or_else(|| st.get("activeTargetId").cloned().unwrap_or(Value::Null)),
                        "dialog": rec.as_ref().and_then(|r| r.get("dialog")).cloned().unwrap_or(Value::Null),
                        "epoch": rec.as_ref().and_then(|r| r.get("epoch")).cloned().unwrap_or(Value::Null),
                        "pauseEpoch": rec.as_ref().and_then(|r| r.get("pauseEpoch")).cloned().unwrap_or_else(|| st.get("pauseEpoch").cloned().unwrap_or(json!(0))),
                        "paused": rec.as_ref().and_then(|r| r.get("paused")).cloned().unwrap_or_else(|| st.get("paused").cloned().unwrap_or(json!(false))),
                        "tabs": rec.as_ref().and_then(|r| r.get("tabs")).cloned().unwrap_or(Value::Null),
                        "tabsHash": rec.as_ref().and_then(|r| r.get("tabsHash")).cloned().unwrap_or(Value::Null),
                        "seq": rec.as_ref().and_then(|r| r.get("seq")).cloned().unwrap_or(json!(0)),
                        "portFile": st.get("portFile").cloned().unwrap_or(Value::Null),
                        "serverMs": t_start.elapsed().as_millis() as u64,
                    }
                });
            }
            json!({
                "isError": false,
                "content": [{ "type": "text", "text": format!("Input error: {err}") }],
                "structuredContent": {
                    "ok": false,
                    "connected": live,
                    "recoverable": live,
                    "error": err,
                    "portFile": st.get("portFile").cloned().unwrap_or(Value::Null),
                    "serverMs": t_start.elapsed().as_millis() as u64,
                }
            })
        }
    }
}

fn handle_disconnect(relay: &Arc<dyn RelayBackend>, args: Value) -> Value {
    let lang = current_lang();
    let standby = match args.get("mode").map(Value::as_str) {
        Some(Some("standby")) => true,
        None | Some(Some("revoke")) => false,
        _ => {
            return json!({
                "isError": true,
                "content": [{ "type": "text", "text": "Invalid disconnect mode. Expected standby or revoke." }]
            })
        }
    };
    // Older relays implement pause and interpret disconnect as an unconditional revoke.
    let method = if standby { "pause" } else { "disconnect" };
    let mut res = match relay.call(method, json!({}), Duration::from_secs(10)) {
        Ok(res) if res.is_object() => res,
        Ok(_) => {
            return unconfirmed_control(
                lang,
                "The relay returned an invalid connection control response",
            )
        }
        Err(err) => return unconfirmed_control(lang, sanitized_control_error(method, &err)),
    };
    if !standby && (res["ok"] != true || res["connected"] != false) {
        return unconfirmed_control(
            lang,
            "The relay did not confirm revoking the Chrome connection",
        );
    }
    let standby_active =
        standby && res["ok"] == true && res["connected"] == true && res["paused"] == true;
    let text = match (lang, standby, standby_active, res["connected"].as_bool()) {
        (Lang::En, false, _, _) => "Disconnected from Chrome.",
        (Lang::ZhCn, false, _, _) => "已断开本机 Chrome 连接。",
        (Lang::En, true, true, _) => "Screencast paused. Chrome authorization stays active.",
        (Lang::ZhCn, true, true, _) => "已暂停画面传输，并保留 Chrome 授权。",
        (Lang::En, true, false, Some(false)) => "Chrome is disconnected.",
        (Lang::ZhCn, true, false, Some(false)) => "Chrome 当前已断开。",
        (Lang::En, true, false, _) => "The relay did not confirm standby.",
        (Lang::ZhCn, true, false, _) => "中继未确认进入待机。",
    };
    res["locale"] = json!(lang.as_str());
    res["mode"] = json!(if standby { "standby" } else { "revoke" });
    res["standby"] = json!(standby_active);
    if standby {
        res["ok"] = json!(standby_active);
    }
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": res
    })
}

fn unconfirmed_control(lang: Lang, error: &str) -> Value {
    json!({
        "isError": true,
        "content": [{ "type": "text", "text": error }],
        "structuredContent": { "ok": false, "unconfirmed": true, "error": error, "locale": lang.as_str() }
    })
}

// Relay errors can contain local paths and WebSocket tokens, including on older versions.
fn sanitized_control_error(method: &str, err: &str) -> &'static str {
    if err == format!("Relay timeout on {method}") {
        "The relay timed out while processing the Chrome connection control"
    } else if err.starts_with("Relay busy:") {
        "The relay is busy"
    } else if err.starts_with("Local CDP relay connection closed") {
        "The local relay connection closed"
    } else {
        "Chrome connection control failed. Query live status or retry."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ControlRelay(Result<Value, String>, &'static str);

    impl RelayBackend for ControlRelay {
        fn call(&self, method: &str, _params: Value, _timeout: Duration) -> Result<Value, String> {
            assert_eq!(method, self.1);
            self.0.clone()
        }
    }

    #[test]
    fn failed_revoke_reports_unconfirmed_and_omits_raw_relay_details() {
        let relay: Arc<dyn RelayBackend> = Arc::new(ControlRelay(
            Err("Unable to connect to /tmp/example/relay.sock, ws://127.0.0.1:9222/devtools/browser/example".into()),
            "disconnect",
        ));
        let result = handle_disconnect(&relay, json!({ "mode": "revoke" }));
        assert_eq!(result["isError"], true);
        let status = &result["structuredContent"];
        assert_eq!(status["ok"], false);
        assert_eq!(status["unconfirmed"], true);
        assert!(status.get("connected").is_none());
        assert!(status["error"].is_string());
        let rendered = result.to_string();
        assert!(!rendered.contains("/tmp/example"));
        assert!(!rendered.contains("ws://"));
    }

    #[test]
    fn standby_uses_legacy_pause_and_checks_connection_and_pause_state() {
        for (connected, paused) in [(true, true), (false, true), (true, false)] {
            let relay: Arc<dyn RelayBackend> = Arc::new(ControlRelay(
                Ok(
                    json!({ "ok": true, "connected": connected, "paused": paused, "pauseEpoch": 3 }),
                ),
                "pause",
            ));
            let result = handle_disconnect(&relay, json!({ "mode": "standby" }));
            let status = &result["structuredContent"];
            assert_eq!(status["connected"], connected);
            assert_eq!(status["paused"], paused);
            assert_eq!(status["standby"], connected && paused);
            assert_eq!(status["ok"], connected && paused);
            let text = result["content"][0]["text"].as_str().unwrap();
            assert_eq!(
                text.contains("authorization stays active") || text.contains("保留 Chrome 授权"),
                connected && paused
            );
        }
    }

    #[test]
    fn multi_event_input_with_disconnect_surfaces_unconfirmed_on_stale_target() {
        let relay: Arc<dyn RelayBackend> = Arc::new(ControlRelay(
            Ok(json!({
                "ok": false,
                "staleTarget": true,
                "connected": true,
                "error": "The tab epoch changed. Refresh the panel before sending input again."
            })),
            "input",
        ));
        let result = dispatch_tool_with_cache(
            &relay,
            None,
            "chrome_stream_input",
            json!({
                "targetEpoch": 1,
                "events": [
                    { "action": "click", "x": 10, "y": 20 },
                    { "action": "disconnect" }
                ]
            }),
        );
        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["ok"], false);
        assert_eq!(result["structuredContent"]["unconfirmed"], true);
    }
}
