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

pub fn tool_definitions() -> Value {
    json!([
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
                    "rttMs": { "type": "number", "minimum": 0 }
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
            "description": "Stop active screencast streaming and close the Chrome DevTools session to save battery.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        }
    ])
}

#[cfg(test)]
pub fn dispatch_tool(relay: &Arc<dyn RelayBackend>, name: &str, args: Value) -> Value {
    dispatch_tool_with_cache(relay, None, name, args)
}

pub fn dispatch_tool_with_cache(
    relay: &Arc<dyn RelayBackend>,
    swr_cache: Option<&Arc<SwrBootCache>>,
    name: &str,
    args: Value,
) -> Value {
    let mut out = match name {
        "chrome_stream_stage" => handle_stage(relay, swr_cache),
        "chrome_stream_connect" => handle_connect(relay, args),
        "chrome_stream_frame" => handle_frame(relay, args),
        "chrome_stream_input" => handle_input(relay, args),
        "chrome_stream_disconnect" => handle_disconnect(relay),
        _ => json!({
            "isError": true,
            "content": [{ "type": "text", "text": format!("Unknown tool: {name}") }]
        }),
    };

    if let Some(sc) = out.get_mut("structuredContent") {
        redact_relay_payload(sc);
        if let Some(cache) = swr_cache {
            if name != "chrome_stream_stage" {
                cache.update_from_tool_result(name, sc);
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
    let mut status = if let Some(cache) = swr_cache {
        match cache.get_swr(Duration::from_millis(1200)) {
            Some(snap) => stage_status_from_snapshot(&snap),
            None => match relay.call("status", json!({}), Duration::from_secs(3)) {
                Ok(mut s) => {
                    redact_relay_payload(&mut s);
                    s
                }
                Err(err) => {
                    let msg = match lang {
                        Lang::En => format!("Chrome Stream relay unavailable: {err}"),
                        Lang::ZhCn => format!("Chrome Stream 本地服务暂不可用：{err}"),
                    };
                    return json!({
                        "isError": false,
                        "content": [{ "type": "text", "text": msg }],
                        "structuredContent": { "connected": false, "locale": lang.as_str(), "error": err }
                    });
                }
            },
        }
    } else {
        match relay.call("status", json!({}), Duration::from_secs(5)) {
            Ok(mut s) => {
                redact_relay_payload(&mut s);
                s
            }
            Err(err) => {
                let msg = match lang {
                    Lang::En => format!("Chrome Stream relay unavailable: {err}"),
                    Lang::ZhCn => format!("Chrome Stream 本地服务暂不可用：{err}"),
                };
                return json!({
                    "isError": false,
                    "content": [{ "type": "text", "text": msg }],
                    "structuredContent": { "connected": false, "locale": lang.as_str(), "error": err }
                });
            }
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
            let _ = r.call("warm", json!({ "qualityPreset": "low" }), Duration::from_secs(2));
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

fn handle_connect(relay: &Arc<dyn RelayBackend>, args: Value) -> Value {
    let lang = current_lang();
    match relay.call("connect", args, Duration::from_secs(70)) {
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
        .unwrap_or("")
        .to_string();
    let frame_hash = frame
        .get("frameHash")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| hash_frame_data(&img_url));

    let active_tid = frame
        .get("activeTargetId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let changed = frame
        .get("changed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if changed
        && !img_url.is_empty()
        && !last_frame_hash.is_empty()
        && frame_hash == last_frame_hash
        && (req_target_id.is_none() || active_tid.is_empty() || req_target_id == Some(active_tid))
    {
        frame["changed"] = json!(false);
        frame["unchanged"] = json!(true);
        frame["imageDataUrl"] = json!("");
        frame["imageBytes"] = json!(0);
    } else {
        frame["unchanged"] = json!(!changed);
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
    if frame.get("qualityApplied").and_then(|v| v.as_str()).is_none() {
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
        "qualityPreset": profile.name,
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
                    "qualityPreset": profile.name,
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

            let post = if still_connected { safe_status(relay) } else { st };
            let live = post
                .get("connected")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut sc = json!({
                "connected": live,
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

fn handle_input(relay: &Arc<dyn RelayBackend>, args: Value) -> Value {
    match relay.call("input", args, Duration::from_secs(30)) {
        Ok(res) => json!({
            "content": [{ "type": "text", "text": "ok" }],
            "structuredContent": res
        }),
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
                        json!({ "sinceSeq": 0, "waitMs": 0, "forceRefresh": true, "qualityPreset": "low" }),
                        Duration::from_secs(15),
                    )
                    .ok();
                return json!({
                    "isError": false,
                    "content": [{ "type": "text", "text": "recovered" }],
                    "structuredContent": {
                        "ok": true,
                        "recovered": true,
                        "connected": true,
                        "activeTargetId": rec.as_ref().and_then(|r| r.get("activeTargetId")).cloned().unwrap_or_else(|| st.get("activeTargetId").cloned().unwrap_or(Value::Null)),
                        "dialog": rec.as_ref().and_then(|r| r.get("dialog")).cloned().unwrap_or(Value::Null),
                        "epoch": rec.as_ref().and_then(|r| r.get("epoch")).cloned().unwrap_or(Value::Null),
                        "tabs": rec.as_ref().and_then(|r| r.get("tabs")).cloned().unwrap_or(Value::Null),
                        "tabsHash": rec.as_ref().and_then(|r| r.get("tabsHash")).cloned().unwrap_or(Value::Null),
                        "seq": rec.as_ref().and_then(|r| r.get("seq")).cloned().unwrap_or(json!(0)),
                        "portFile": st.get("portFile").cloned().unwrap_or(Value::Null),
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
                }
            })
        }
    }
}

fn handle_disconnect(relay: &Arc<dyn RelayBackend>) -> Value {
    let lang = current_lang();
    let ok_text = match lang {
        Lang::En => "Disconnected from Chrome.",
        Lang::ZhCn => "已断开本机 Chrome 连接。",
    };
    match relay.call("disconnect", json!({}), Duration::from_secs(10)) {
        Ok(mut res) => {
            res["locale"] = json!(lang.as_str());
            json!({
                "content": [{ "type": "text", "text": ok_text }],
                "structuredContent": res
            })
        }
        Err(err) => json!({
            "isError": false,
            "content": [{ "type": "text", "text": format!("Disconnect note: {err}") }],
            "structuredContent": { "ok": true, "connected": false, "locale": lang.as_str() }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FlakyRelay {
        poll_attempts: AtomicUsize,
        connected: bool,
    }

    impl RelayBackend for FlakyRelay {
        fn call(&self, method: &str, params: Value, _timeout: Duration) -> Result<Value, String> {
            match method {
                "status" => Ok(json!({
                    "hostname": "test-host",
                    "connected": self.connected,
                    "portFile": { "exists": true, "valid": true, "port": 9222 },
                    "activeTargetId": "t2",
                })),
                "poll_frame" => {
                    let n = self.poll_attempts.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
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
                            "qualityApplied": params["qualityPreset"],
                        }))
                    }
                }
                _ => Ok(json!({ "ok": true, "connected": self.connected })),
            }
        }
    }

    #[test]
    fn chrome_stream_frame_recovers_from_no_target_found_without_disconnecting() {
        let relay: Arc<dyn RelayBackend> = Arc::new(FlakyRelay {
            poll_attempts: AtomicUsize::new(0),
            connected: true,
        });
        let res = dispatch_tool(
            &relay,
            "chrome_stream_frame",
            json!({ "targetId": "t-closed", "qualityPreset": "auto", "rttMs": 820 }),
        );
        let sc = &res["structuredContent"];
        assert_eq!(sc["connected"], true);
        assert_eq!(sc["recovered"], true);
        assert_eq!(sc["activeTargetId"], "t2");
        assert_eq!(sc["qualityApplied"], "ultra");
    }

    #[test]
    fn exposes_five_clean_english_tools_without_dead_app_tools() {
        let defs = tool_definitions();
        let arr = defs.as_array().unwrap();
        let names: Vec<&str> = arr.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            vec![
                "chrome_stream_stage",
                "chrome_stream_connect",
                "chrome_stream_frame",
                "chrome_stream_input",
                "chrome_stream_disconnect",
            ]
        );
        for tool in arr {
            let desc = tool["description"].as_str().unwrap();
            assert!(desc.is_ascii(), "tool description must be clean English ASCII: {desc}");
        }
    }
}
