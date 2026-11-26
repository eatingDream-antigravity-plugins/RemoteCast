use crate::cdp::client::{CdpClient, InputBatchParams, InputEvent, PollFrameParams};
use crate::config::{Config, RELAY_VERSION};
use crate::relay::ipc::probe_relay_ping;
use crate::stream::dedup::hash_tabs;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const DISCONNECTED_IDLE_EXIT: Duration = Duration::from_secs(15 * 60);
const MAX_RELAY_CONCURRENT_RPCS: usize = 32;

static SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_signal(_sig: libc::c_int) {
    SHUTDOWN_FLAG.store(true, Ordering::SeqCst);
}

pub fn run_relay_daemon(cfg: &Config) -> Result<(), String> {
    SHUTDOWN_FLAG.store(false, Ordering::SeqCst);
    unsafe {
        // Ensure Unix socket and PID file are created 0600 atomically with zero permission window.
        libc::umask(0o077);
        libc::signal(libc::SIGTERM, handle_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, handle_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    fs::create_dir_all(&cfg.data_dir)
        .map_err(|e| format!("Failed creating data dir {}: {e}", cfg.data_dir.display()))?;
    let _ = fs::set_permissions(&cfg.data_dir, fs::Permissions::from_mode(0o700));

    if cfg.relay_sock.exists() {
        if probe_relay_ping(&cfg.relay_sock).is_some() {
            // An existing healthy relay daemon already owns the socket.
            return Ok(());
        }
        let _ = fs::remove_file(&cfg.relay_sock);
    }

    let cdp = CdpClient::new_with_candidates(cfg.port_files.clone(), cfg.debug_port);
    let cdp_warm = Arc::clone(&cdp);
    let _ = thread::Builder::new()
        .name("chrome-stream-probe".to_string())
        .spawn(move || cdp_warm.prewarm_readiness());

    serve_relay_socket(
        &cfg.relay_sock,
        Some(&cfg.relay_pid_file),
        cdp,
        Arc::new(AtomicBool::new(false)),
        true,
    )
}

fn write_pid_file_safely(pid_path: &Path) {
    // Avoid symlink-following writes: unlink first, then O_CREAT|O_EXCL at mode 0600.
    let _ = fs::remove_file(pid_path);
    if let Ok(mut f) = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(pid_path)
    {
        let _ = writeln!(f, "{}", std::process::id());
    }
}

pub fn serve_relay_socket(
    sock_path: &Path,
    pid_file: Option<&Path>,
    cdp: Arc<CdpClient>,
    stop_flag: Arc<AtomicBool>,
    use_global_signals: bool,
) -> Result<(), String> {
    if let Some(parent) = sock_path.parent() {
        let _ = fs::create_dir_all(parent);
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    if fs::symlink_metadata(sock_path).is_ok() {
        let _ = fs::remove_file(sock_path);
    }

    let listener = UnixListener::bind(sock_path)
        .map_err(|e| format!("Failed binding relay socket {}: {e}", sock_path.display()))?;
    fs::set_permissions(sock_path, fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("Failed setting 0600 on {}: {e}", sock_path.display()))?;
    let _ = listener.set_nonblocking(true);

    if let Some(pf) = pid_file {
        write_pid_file_safely(pf);
    }

    let active_clients = Arc::new(AtomicUsize::new(0));
    let in_flight_rpcs = Arc::new(AtomicUsize::new(0));
    let last_activity = Arc::new(Mutex::new(Instant::now()));

    loop {
        if stop_flag.load(Ordering::SeqCst)
            || (use_global_signals && SHUTDOWN_FLAG.load(Ordering::SeqCst))
        {
            break;
        }

        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                *last_activity.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                active_clients.fetch_add(1, Ordering::SeqCst);

                let cdp_c = Arc::clone(&cdp);
                let clients_c = Arc::clone(&active_clients);
                let rpcs_c = Arc::clone(&in_flight_rpcs);
                let act_c = Arc::clone(&last_activity);
                let _ = thread::Builder::new()
                    .name("chrome-stream-conn".to_string())
                    .spawn(move || {
                        handle_client_conn(stream, cdp_c, rpcs_c, act_c);
                        clients_c.fetch_sub(1, Ordering::SeqCst);
                    });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                // Idle auto-exit ONLY when no browser WebSocket is held and no clients are attached.
                let idle_elapsed = last_activity
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .elapsed();
                if !cdp.is_connected()
                    && !cdp.is_connecting()
                    && active_clients.load(Ordering::SeqCst) == 0
                    && idle_elapsed > DISCONNECTED_IDLE_EXIT
                {
                    break;
                }
                thread::sleep(Duration::from_millis(40));
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(50));
            }
        }
    }

    cdp.dispose();
    let _ = fs::remove_file(sock_path);
    if let Some(pf) = pid_file {
        let _ = fs::remove_file(pf);
    }
    Ok(())
}

fn handle_client_conn(
    stream: UnixStream,
    cdp: Arc<CdpClient>,
    in_flight_rpcs: Arc<AtomicUsize>,
    last_activity: Arc<Mutex<Instant>>,
) {
    let Ok(writer_stream) = stream.try_clone() else {
        return;
    };
    let writer = Arc::new(Mutex::new(writer_stream));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Ok(req) = serde_json::from_str::<Value>(trimmed) else {
                    continue;
                };
                let id = req.get("id").cloned().unwrap_or(Value::Null);
                let method = req
                    .get("method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
                *last_activity.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();

                let cdp_c = Arc::clone(&cdp);
                let writer_c = Arc::clone(&writer);
                let act_c = Arc::clone(&last_activity);

                if in_flight_rpcs.load(Ordering::SeqCst) >= MAX_RELAY_CONCURRENT_RPCS {
                    let resp = match dispatch_rpc(&cdp_c, &method, params) {
                        Ok(result) => json!({ "id": id, "result": result }),
                        Err(err) => json!({ "id": id, "error": err }),
                    };
                    *act_c.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                    if let Ok(mut w) = writer_c.lock() {
                        let _ = writeln!(w, "{}", resp);
                        let _ = w.flush();
                    }
                } else {
                    in_flight_rpcs.fetch_add(1, Ordering::SeqCst);
                    let rpcs_c = Arc::clone(&in_flight_rpcs);
                    // Dispatch each RPC on its own thread so `input`/`close_tab` never waits
                    // behind an in-flight `poll_frame` long-poll on the same socket.
                    let _ = thread::spawn(move || {
                        let resp = match dispatch_rpc(&cdp_c, &method, params) {
                            Ok(result) => json!({ "id": id, "result": result }),
                            Err(err) => json!({ "id": id, "error": err }),
                        };
                        *act_c.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                        if let Ok(mut w) = writer_c.lock() {
                            let _ = writeln!(w, "{}", resp);
                            let _ = w.flush();
                        }
                        rpcs_c.fetch_sub(1, Ordering::SeqCst);
                    });
                }
            }
        }
    }
}

pub fn dispatch_rpc(
    cdp: &Arc<CdpClient>,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let pid = std::process::id();
    match method {
        "ping" => Ok(json!({
            "ok": true,
            "pid": pid,
            "version": RELAY_VERSION,
            "connected": cdp.is_connected(),
        })),
        "status" => Ok(cdp.status_snapshot(pid, RELAY_VERSION)),
        "boot_snapshot" => {
            let qp = params
                .get("qualityPreset")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            let rtt = params.get("rttMs").and_then(|v| v.as_f64()).unwrap_or(0.0);
            Ok(cdp.boot_snapshot(pid, RELAY_VERSION, qp, rtt))
        }
        "warm" => {
            let qp = params
                .get("qualityPreset")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            let rtt = params.get("rttMs").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if cdp.is_connected() {
                let c = Arc::clone(cdp);
                let _ = thread::spawn(move || c.warm_active_tab(qp.as_deref(), rtt));
            } else {
                let c = Arc::clone(cdp);
                let _ = thread::spawn(move || c.prewarm_readiness());
            }
            Ok(json!({ "ok": true, "connected": cdp.is_connected() }))
        }
        "connect" => {
            let qp = params
                .get("qualityPreset")
                .and_then(|v| v.as_str())
                .unwrap_or("low");
            let rtt = params.get("rttMs").and_then(|v| v.as_f64()).unwrap_or(0.0);
            cdp.ensure_connected().map_err(|e| e.message)?;
            cdp.resume();
            let (active_tid, tabs) = cdp
                .resolve_target_id(cdp.active_target_id().as_deref(), true, true)
                .map_err(|e| e.message)?;
            if let Some(tid) = active_tid.as_deref() {
                let _ = cdp.ensure_screencast(tid, qp, rtt);
            }
            Ok(json!({
                "ok": true,
                "connected": cdp.is_connected(),
                "paused": false,
                "pauseEpoch": cdp.pause_epoch(),
                "screencastPausedIdle": false,
                "noTabs": active_tid.is_none(),
                "hostname": "localhost",
                "locale": crate::i18n::current_lang().as_str(),
                "activeTargetId": active_tid,
                "dialog": cdp.active_dialog(),
                "epoch": cdp.tab_epoch(),
                "tabs": tabs,
                "tabsHash": hash_tabs(&tabs),
                "portFile": cdp.public_port_file(),
            }))
        }
        "tabs" => {
            let action = params
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("list");
            let target_id = params
                .get("targetId")
                .and_then(|v| v.as_str())
                .map(String::from);
            let url = params.get("url").and_then(|v| v.as_str()).map(String::from);

            match action {
                "close" => cdp
                    .close_tab(target_id.as_deref(), None)
                    .map_err(|e| e.message),
                "switch" => cdp
                    .dispatch_input_batch(InputBatchParams {
                        target_id: target_id.clone(),
                        events: vec![InputEvent {
                            action: "switch_tab".into(),
                            target_id,
                            ..Default::default()
                        }],
                    })
                    .map_err(|e| e.message),
                "new" => cdp
                    .dispatch_input_batch(InputBatchParams {
                        target_id: None,
                        events: vec![InputEvent {
                            action: "new_tab".into(),
                            url: url.or_else(|| Some("about:blank".into())),
                            ..Default::default()
                        }],
                    })
                    .map_err(|e| e.message),
                _ => {
                    if !cdp.is_connected() {
                        return Ok(json!({
                            "ok": false,
                            "connected": false,
                            "activeTargetId": Value::Null,
                            "epoch": cdp.tab_epoch(),
                            "tabs": [],
                            "tabsHash": "empty",
                        }));
                    }
                    let tabs = cdp.list_tabs(true).map_err(|e| e.message)?;
                    Ok(json!({
                        "ok": true,
                        "connected": cdp.is_connected(),
                        "activeTargetId": cdp.active_target_id(),
                        "epoch": cdp.tab_epoch(),
                        "tabs": tabs,
                        "tabsHash": hash_tabs(&tabs),
                    }))
                }
            }
        }
        "poll_frame" => {
            let parsed: PollFrameParams =
                serde_json::from_value(params).unwrap_or_default();
            cdp.poll_frame(parsed).map_err(|e| e.message)
        }
        "input" => {
            let parsed: InputBatchParams =
                serde_json::from_value(params).unwrap_or_default();
            cdp.dispatch_input_batch(parsed).map_err(|e| e.message)
        }
        "navigate" => {
            let nav_action = params
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("goto");
            let target_id = params
                .get("targetId")
                .and_then(|v| v.as_str())
                .map(String::from);
            let url = params.get("url").and_then(|v| v.as_str()).map(String::from);
            let mapped = match nav_action {
                "back" => "back",
                "forward" => "forward",
                "reload" => "reload",
                _ => "navigate",
            };
            cdp.dispatch_input_batch(InputBatchParams {
                target_id: target_id.clone(),
                events: vec![InputEvent {
                    action: mapped.into(),
                    target_id,
                    url,
                    ..Default::default()
                }],
            })
            .map_err(|e| e.message)
        }
        "close_tab" => {
            let tid = params.get("targetId").and_then(|v| v.as_str());
            let qp = params.get("qualityPreset").and_then(|v| v.as_str());
            cdp.close_tab(tid, qp).map_err(|e| e.message)
        }
        "pause" => {
            cdp.pause();
            Ok(json!({
                "ok": true,
                "connected": cdp.is_connected(),
                "paused": true,
                "pauseEpoch": cdp.pause_epoch(),
                "portFile": cdp.public_port_file(),
            }))
        }
        "resume" => {
            cdp.resume();
            Ok(json!({
                "ok": true,
                "connected": cdp.is_connected(),
                "paused": false,
                "pauseEpoch": cdp.pause_epoch(),
                "portFile": cdp.public_port_file(),
            }))
        }
        "disconnect" => {
            cdp.stop_screencast();
            cdp.disconnect();
            Ok(json!({
                "ok": true,
                "connected": false,
                "paused": false,
                "pauseEpoch": cdp.pause_epoch(),
                "portFile": cdp.public_port_file(),
            }))
        }
        other => Err(format!("Unknown relay method: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::protocol::{CdpTransport, TransportEvent, TransportFactory};
    use crate::relay::ipc::{RelayBackend, RelayClient};
    use std::path::PathBuf;

    struct SimpleMockWs {
        open: AtomicBool,
        on_event: Arc<dyn Fn(TransportEvent) + Send + Sync>,
    }

    impl CdpTransport for SimpleMockWs {
        fn send_text(&self, text: &str) -> Result<(), String> {
            let msg: Value = serde_json::from_str(text).unwrap();
            let id = msg["id"].as_u64().unwrap();
            let method = msg["method"].as_str().unwrap_or("");
            let res = match method {
                "Target.getTargets" => json!({
                    "targetInfos": [
                        { "targetId": "t1", "type": "page", "title": "Home", "url": "https://home.test" }
                    ]
                }),
                "Target.attachToTarget" => json!({ "sessionId": "s1" }),
                "Runtime.evaluate" => json!({ "result": { "value": "[1280,800]" } }),
                "Page.captureScreenshot" => json!({ "data": "QUFBQQ==" }),
                _ => json!({}),
            };
            (self.on_event)(TransportEvent::Text(
                json!({ "id": id, "result": res }).to_string(),
            ));
            Ok(())
        }
        fn is_open(&self) -> bool {
            self.open.load(Ordering::SeqCst)
        }
        fn close(&self) {
            self.open.store(false, Ordering::SeqCst);
        }
    }

    #[test]
    fn unix_socket_relay_serves_multiplexed_ipc_with_0600_permissions() {
        let dir = std::env::temp_dir().join(format!(
            "chrome-stream-ipc-test-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let _ = fs::create_dir_all(&dir);
        let sock = dir.join("relay.sock");
        let pid_f = dir.join("relay.pid");

        let factory: TransportFactory = Arc::new(|_, _, on_event| {
            Ok(Arc::new(SimpleMockWs {
                open: AtomicBool::new(true),
                on_event,
            }) as Arc<dyn CdpTransport>)
        });
        let cdp = CdpClient::with_factory(
            PathBuf::from("/tmp/none"),
            9222,
            Some("ws://127.0.0.1:9222/devtools/browser/test".into()),
            factory,
            false,
        );

        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = Arc::clone(&stop);
        let sock_t = sock.clone();
        let pid_t = pid_f.clone();
        let handle = thread::spawn(move || {
            serve_relay_socket(&sock_t, Some(&pid_t), cdp, stop_t, false).unwrap();
        });

        for _ in 0..50 {
            if sock.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        let mode = fs::metadata(&sock).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let client = RelayClient::connect_only(sock.clone());
        let ping = client.call("ping", json!({}), Duration::from_secs(2)).unwrap();
        assert_eq!(ping["ok"], true);

        let pre_frame = client
            .call(
                "poll_frame",
                json!({ "qualityPreset": "low", "waitMs": 0 }),
                Duration::from_secs(2),
            )
            .unwrap();
        assert_eq!(pre_frame["connected"], false);

        let conn_res = client
            .call(
                "connect",
                json!({ "qualityPreset": "low" }),
                Duration::from_secs(3),
            )
            .unwrap();
        assert_eq!(conn_res["connected"], true);

        let frame = client
            .call(
                "poll_frame",
                json!({ "qualityPreset": "low", "waitMs": 40 }),
                Duration::from_secs(3),
            )
            .unwrap();
        assert_eq!(frame["connected"], true);
        assert_eq!(frame["activeTargetId"], "t1");
        assert_eq!(frame["changed"], true);

        // Concurrent fast input does NOT block behind a 450ms long-poll on the same socket
        let seq = frame["seq"].as_u64().unwrap();
        let hash = frame["frameHash"].as_str().unwrap().to_string();
        let c_poll = Arc::clone(&client);
        let poll_handle = thread::spawn(move || {
            c_poll
                .call(
                    "poll_frame",
                    json!({ "sinceSeq": seq, "lastFrameHash": hash, "qualityPreset": "low", "waitMs": 450 }),
                    Duration::from_secs(3),
                )
                .unwrap()
        });
        thread::sleep(Duration::from_millis(30));
        let t_in = Instant::now();
        let in_res = client
            .call(
                "input",
                json!({ "targetId": "t1", "events": [{ "action": "click", "x": 10, "y": 20 }] }),
                Duration::from_secs(2),
            )
            .unwrap();
        let input_ms = t_in.elapsed().as_millis();
        assert_eq!(in_res["ok"], true);
        assert!(
            input_ms < 200,
            "input RPC should bypass concurrent long-poll (took {input_ms}ms)"
        );
        let poll_res = poll_handle.join().unwrap();
        assert_eq!(poll_res["unchanged"], true);

        let disc = client
            .call("disconnect", json!({}), Duration::from_secs(2))
            .unwrap();
        assert_eq!(disc["connected"], false);

        stop.store(true, Ordering::SeqCst);
        handle.join().unwrap();
        assert!(!sock.exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
