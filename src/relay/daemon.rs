use crate::cdp::client::{CdpClient, InputBatchParams, InputEvent, PollFrameParams};
use crate::config::{Config, RELAY_VERSION};
use crate::local_io::{
    create_private_directory, effective_uid, owned_directory, read_bounded_line, socket_parent,
};
use crate::relay::ipc::{probe_relay_ping, verify_same_user};
use crate::stream::dedup::hash_tabs;
use serde_json::{json, Value};
use std::fs::{self, File, Metadata, OpenOptions};
#[cfg(test)]
use std::io::BufRead;
use std::io::{self, BufReader, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const DISCONNECTED_IDLE_EXIT: Duration = Duration::from_secs(15 * 60);
const MAX_RELAY_CONCURRENT_RPCS: usize = 32;
const RELAY_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
struct RelayLimits {
    clients: usize,
    request_bytes: usize,
    rpcs: usize,
    disconnected_idle_exit: Duration,
}

impl Default for RelayLimits {
    fn default() -> Self {
        Self {
            clients: 64,
            request_bytes: 1024 * 1024,
            rpcs: MAX_RELAY_CONCURRENT_RPCS,
            disconnected_idle_exit: DISCONNECTED_IDLE_EXIT,
        }
    }
}

struct RelaySlot(Arc<AtomicUsize>);

impl RelaySlot {
    fn acquire(counter: &Arc<AtomicUsize>, limit: usize) -> Option<Self> {
        let mut count = counter.load(Ordering::Acquire);
        while count < limit {
            match counter.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Self(Arc::clone(counter))),
                Err(current) => count = current,
            }
        }
        None
    }
}

impl Drop for RelaySlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

#[derive(Debug)]
enum ServeError {
    Owned,
    Failed(String),
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Owned => f.write_str("Another relay already owns this socket endpoint"),
            Self::Failed(reason) => f.write_str(reason),
        }
    }
}

struct OwnedPath<'a> {
    path: &'a Path,
    metadata: Metadata,
}

fn remove_matching_path(path: &Path, expected: &Metadata) -> io::Result<()> {
    owned_directory(socket_parent(path), false)?;
    match fs::symlink_metadata(path) {
        Ok(current)
            if current.file_type() == expected.file_type()
                && current.dev() == expected.dev()
                && current.ino() == expected.ino() =>
        {
            fs::remove_file(path)
        }
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

impl Drop for OwnedPath<'_> {
    fn drop(&mut self) {
        let _ = remove_matching_path(self.path, &self.metadata);
    }
}

static SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_signal(_sig: libc::c_int) {
    SHUTDOWN_FLAG.store(true, Ordering::SeqCst);
}

pub fn run_relay_daemon(cfg: &Config) -> Result<(), String> {
    SHUTDOWN_FLAG.store(false, Ordering::SeqCst);
    unsafe {
        // Restrict newly created socket entries, lock files and PID files to the owner.
        libc::umask(0o077);
        libc::signal(
            libc::SIGTERM,
            handle_signal as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            handle_signal as *const () as libc::sighandler_t,
        );
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    create_private_directory(&cfg.data_dir)
        .map_err(|e| format!("Could not prepare relay data directory: {e}"))?;

    let cdp = CdpClient::new_with_candidates(cfg.port_files.clone(), cfg.debug_port);
    match serve_relay_socket(
        &cfg.relay_sock,
        &cfg.data_dir,
        Some(&cfg.relay_pid_file),
        cdp,
        Arc::new(AtomicBool::new(false)),
        true,
    ) {
        Ok(()) => Ok(()),
        Err(ServeError::Owned) => {
            let deadline = Instant::now() + Duration::from_secs(1);
            while probe_relay_ping(&cfg.relay_sock).is_err() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(40));
            }
            Ok(())
        }
        Err(err) => Err(err.to_string()),
    }
}

fn lock_endpoint(sock_path: &Path) -> Result<File, ServeError> {
    let mut lock_path = sock_path.as_os_str().to_os_string();
    lock_path.push(".lock");
    // Leave the lock file in place so every process locks the same inode.
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&lock_path)
        .map_err(|err| ServeError::Failed(format!("Failed opening relay ownership lock: {err}")))?;
    if !file
        .metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.uid() == effective_uid())
    {
        return Err(ServeError::Failed(
            "Relay ownership lock must be a regular file".into(),
        ));
    }
    // SAFETY: flock borrows the valid fd owned by file. The lock is released when file drops.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let err = io::Error::last_os_error();
        return Err(if err.kind() == io::ErrorKind::WouldBlock {
            ServeError::Owned
        } else {
            ServeError::Failed(format!("Failed locking relay endpoint: {err}"))
        });
    }
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|err| {
            ServeError::Failed(format!("Failed setting relay lock permissions: {err}"))
        })?;
    Ok(file)
}

fn write_pid_file_safely(path: &Path) -> Option<(OwnedPath<'_>, File)> {
    owned_directory(socket_parent(path), false).ok()?;
    // Unlink the old entry, then use O_CREAT|O_EXCL to avoid following symlinks.
    let _ = fs::remove_file(path);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .ok()?;
    let pid_file = OwnedPath {
        path,
        metadata: file.metadata().ok()?,
    };
    writeln!(file, "{}", std::process::id()).ok()?;
    Some((pid_file, file))
}

fn serve_relay_socket(
    sock_path: &Path,
    data_dir: &Path,
    pid_file: Option<&Path>,
    cdp: Arc<CdpClient>,
    stop_flag: Arc<AtomicBool>,
    use_global_signals: bool,
) -> Result<(), ServeError> {
    serve_relay_socket_with_limits(
        sock_path,
        data_dir,
        pid_file,
        cdp,
        stop_flag,
        use_global_signals,
        RelayLimits::default(),
    )
}

fn serve_relay_socket_with_limits(
    sock_path: &Path,
    data_dir: &Path,
    pid_file: Option<&Path>,
    cdp: Arc<CdpClient>,
    stop_flag: Arc<AtomicBool>,
    use_global_signals: bool,
    limits: RelayLimits,
) -> Result<(), ServeError> {
    owned_directory(socket_parent(sock_path), true).map_err(|err| {
        ServeError::Failed(format!("Could not prepare relay socket directory: {err}"))
    })?;
    let _endpoint_lock = lock_endpoint(sock_path)?;
    match fs::symlink_metadata(sock_path) {
        Ok(metadata) if metadata.file_type().is_socket() && metadata.uid() == effective_uid() => {
            match probe_relay_ping(sock_path) {
                Ok(_) => return Err(ServeError::Owned),
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                    ) => {}
                Err(err) => {
                    return Err(ServeError::Failed(format!(
                        "Could not verify the existing relay socket: {err}"
                    )))
                }
            }
            remove_matching_path(sock_path, &metadata).map_err(|err| {
                ServeError::Failed(format!("Failed removing stale relay socket: {err}"))
            })?;
        }
        Ok(_) => {
            return Err(ServeError::Failed(
                "Relay endpoint must be a Unix socket owned by the current user".into(),
            ))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(ServeError::Failed(format!(
                "Failed checking relay endpoint: {err}"
            )))
        }
    }

    let listener = UnixListener::bind(sock_path)
        .map_err(|e| ServeError::Failed(format!("Failed binding relay socket: {e}")))?;
    let metadata = fs::symlink_metadata(sock_path).map_err(|err| {
        ServeError::Failed(format!("Failed identifying bound relay socket: {err}"))
    })?;
    if !metadata.file_type().is_socket() || metadata.uid() != effective_uid() {
        return Err(ServeError::Failed(
            "Bound relay endpoint is no longer a Unix socket".into(),
        ));
    }
    let _owned_socket = OwnedPath {
        path: sock_path,
        metadata,
    };
    fs::set_permissions(sock_path, fs::Permissions::from_mode(0o600))
        .map_err(|e| ServeError::Failed(format!("Failed setting 0600 on the relay socket: {e}")))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| ServeError::Failed(format!("Failed configuring relay listener: {err}")))?;

    let _owned_pid = pid_file.and_then(write_pid_file_safely);
    if use_global_signals {
        let cdp = Arc::clone(&cdp);
        let _ = thread::Builder::new()
            .name("chrome-stream-probe".to_string())
            .spawn(move || cdp.prewarm_readiness());
    }

    let data_dir: Arc<Path> = Arc::from(data_dir);
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
            Ok((mut stream, _)) => {
                if verify_same_user(&stream).is_err() {
                    continue;
                }
                let Some(slot) = RelaySlot::acquire(&active_clients, limits.clients) else {
                    if stream.set_nonblocking(true).is_ok() {
                        let _ = writeln!(
                            stream,
                            "{}",
                            json!({ "id": null, "error": "Relay busy: too many client connections" })
                        );
                    }
                    continue;
                };
                if stream.set_nonblocking(false).is_err()
                    || stream.set_write_timeout(Some(RELAY_WRITE_TIMEOUT)).is_err()
                {
                    continue;
                }
                *last_activity.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                let cdp = Arc::clone(&cdp);
                let data_dir = Arc::clone(&data_dir);
                let rpcs = Arc::clone(&in_flight_rpcs);
                let activity = Arc::clone(&last_activity);
                let _ = thread::Builder::new()
                    .name("chrome-stream-conn".to_string())
                    .spawn(move || {
                        handle_client_conn(stream, cdp, data_dir, rpcs, activity, limits, slot)
                    });
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                let idle_elapsed = last_activity
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .elapsed();
                if !cdp.is_connected()
                    && !cdp.is_connecting()
                    && active_clients.load(Ordering::Acquire) == 0
                    && in_flight_rpcs.load(Ordering::Acquire) == 0
                    && idle_elapsed > limits.disconnected_idle_exit
                {
                    break;
                }
                thread::sleep(Duration::from_millis(40));
            }
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }

    cdp.dispose();
    Ok(())
}

struct ClientWriter {
    stream: Mutex<UnixStream>,
    _connection_slot: RelaySlot,
}

impl ClientWriter {
    fn write_response(&self, response: Value) {
        if let Ok(mut stream) = self.stream.lock() {
            if writeln!(stream, "{response}").is_err() {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
    }
}

fn handle_client_conn(
    stream: UnixStream,
    cdp: Arc<CdpClient>,
    data_dir: Arc<Path>,
    in_flight_rpcs: Arc<AtomicUsize>,
    last_activity: Arc<Mutex<Instant>>,
    limits: RelayLimits,
    connection_slot: RelaySlot,
) {
    let Ok(writer_stream) = stream.try_clone() else {
        return;
    };
    let writer = Arc::new(ClientWriter {
        stream: Mutex::new(writer_stream),
        _connection_slot: connection_slot,
    });
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();

    loop {
        match read_bounded_line(&mut reader, &mut line, limits.request_bytes) {
            Ok(0) => break,
            Ok(_) => {}
            Err(err) => {
                if err.kind() == io::ErrorKind::InvalidData {
                    writer.write_response(
                        json!({ "id": null, "error": "Request exceeds maximum allowed size" }),
                    );
                    let _ = reader.get_ref().shutdown(Shutdown::Both);
                }
                break;
            }
        }
        let Ok(Value::Object(mut req)) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        let id = req.remove("id").unwrap_or(Value::Null);
        let params = req.remove("params").unwrap_or(Value::Null);
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        *last_activity.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        // The client reader has reserved execution capacity for connection controls.
        let is_control = |action| matches!(action, "disconnect" | "pause" | "resume");
        if is_control(method)
            || (method == "input"
                && params["events"].as_array().is_some_and(|events| {
                    events.len() == 1 && events[0]["action"].as_str().is_some_and(is_control)
                }))
        {
            writer.write_response(rpc_response(&cdp, &data_dir, method, params, id));
            continue;
        }
        let Some(slot) = RelaySlot::acquire(&in_flight_rpcs, limits.rpcs) else {
            writer.write_response(
                json!({ "id": id, "error": "Relay busy: too many concurrent requests" }),
            );
            continue;
        };

        // Keep fast input independent of long polls, including on a shared connection.
        let cdp = Arc::clone(&cdp);
        let data_dir = Arc::clone(&data_dir);
        let rpc_writer = Arc::clone(&writer);
        let activity = Arc::clone(&last_activity);
        let failed_id = id.clone();
        let spawn = thread::Builder::new()
            .name("chrome-stream-rpc".to_string())
            .spawn(move || {
                let _slot = slot;
                let method = req.get("method").and_then(Value::as_str).unwrap_or("");
                let response = rpc_response(&cdp, &data_dir, method, params, id);
                *activity.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                rpc_writer.write_response(response);
            });
        if spawn.is_err() {
            writer.write_response(
                json!({ "id": failed_id, "error": "Relay busy: unable to start request worker" }),
            );
        }
    }
}

fn rpc_response(
    cdp: &Arc<CdpClient>,
    data_dir: &Path,
    method: &str,
    params: Value,
    id: Value,
) -> Value {
    match dispatch_rpc(cdp, data_dir, method, params) {
        Ok(result) => json!({ "id": id, "result": result }),
        Err(err) => json!({ "id": id, "error": err }),
    }
}

fn dispatch_rpc(
    cdp: &Arc<CdpClient>,
    data_dir: &Path,
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
            cdp.warm_in_background(qp, rtt);
            Ok(json!({ "ok": true, "connected": cdp.is_connected() }))
        }
        "connect" | "launch_remote" => {
            let qp = params
                .get("qualityPreset")
                .and_then(Value::as_str)
                .unwrap_or("low");
            let rtt = params.get("rttMs").and_then(Value::as_f64).unwrap_or(0.0);
            if method == "launch_remote" {
                cdp.launch_remote(data_dir)
            } else {
                match params.get("mode").map(|mode| mode.as_str().unwrap_or("")) {
                    None => cdp.ensure_connected(),
                    Some(mode) => cdp.ensure_connected_with_mode(mode),
                }
            }
            .map_err(|e| e.message)?;
            cdp.resume();
            let (active_tid, tabs) = cdp
                .resolve_target_id(cdp.active_target_id().as_deref(), true, true)
                .map_err(|e| e.message)?;
            if let Some(tid) = active_tid.as_deref() {
                if cdp.active_dialog().is_null() {
                    let _ = cdp.ensure_screencast(tid, qp, rtt);
                }
            }
            if method == "connect" {
                let connection_mode = cdp.connection_mode();
                return Ok(json!({
                    "ok": connection_mode.is_some(),
                    "connected": connection_mode.is_some(),
                    "connectionMode": connection_mode,
                    "paused": cdp.is_paused(),
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
                }));
            }
            let mut state = cdp.boot_snapshot(pid, RELAY_VERSION, Some(qp), rtt);
            state["ok"] = state["connected"].clone();
            Ok(state)
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
                "close" => close_requested_tab(cdp, &params),
                "switch" => cdp
                    .dispatch_input_batch(InputBatchParams {
                        target_id: target_id.clone(),
                        target_epoch: None,
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
                        target_epoch: None,
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
            let parsed: PollFrameParams = serde_json::from_value(params).unwrap_or_default();
            cdp.poll_frame(parsed).map_err(|e| e.message)
        }
        "input" => {
            let parsed: InputBatchParams = serde_json::from_value(params)
                .map_err(|err| format!("Invalid input batch: {err}"))?;
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
                target_epoch: params.get("targetEpoch").and_then(Value::as_u64),
                events: vec![InputEvent {
                    action: mapped.into(),
                    target_id,
                    url,
                    ..Default::default()
                }],
            })
            .map_err(|e| e.message)
        }
        "close_tab" => close_requested_tab(cdp, &params),
        "pause" | "resume" => {
            if method == "pause" {
                cdp.pause();
            } else {
                cdp.resume();
            }
            let (connected, paused, pause_epoch) = cdp.pause_status();
            Ok(json!({
                "ok": true,
                "connected": connected,
                "paused": paused,
                "pauseEpoch": pause_epoch,
                "portFile": cdp.public_port_file(),
            }))
        }
        "disconnect" => {
            let mode = params
                .get("mode")
                .map_or("revoke", |mode| mode.as_str().unwrap_or(""));
            match mode {
                "standby" => cdp.pause(),
                "revoke" => cdp.disconnect(),
                _ => return Err("Invalid disconnect mode. Expected standby or revoke.".into()),
            }
            let (connected, paused, pause_epoch) = cdp.pause_status();
            Ok(json!({
                "ok": true,
                "connected": connected,
                "connectionMode": cdp.connection_mode(),
                "paused": paused,
                "standby": mode == "standby" && paused,
                "mode": mode,
                "pauseEpoch": pause_epoch,
                "screencastPausedIdle": false,
                "portFile": cdp.public_port_file(),
                "locale": crate::i18n::current_lang().as_str(),
            }))
        }
        other => Err(format!("Unknown relay method: {other}")),
    }
}

fn close_requested_tab(cdp: &Arc<CdpClient>, params: &Value) -> Result<Value, String> {
    let target_id = params["targetId"].as_str();
    let epoch = match &params["targetEpoch"] {
        Value::Null => None,
        value => Some(value.as_u64().ok_or("Invalid targetEpoch for close_tab")?),
    };
    let quality = params["qualityPreset"].as_str();
    cdp.close_tab(target_id, epoch, quality)
        .map_err(|err| err.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdp::protocol::{CdpTransport, TransportEvent, TransportFactory};
    use crate::relay::ipc::{RelayBackend, RelayClient};
    use std::path::PathBuf;
    use std::sync::mpsc;

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

    struct TestEndpoint {
        dir: PathBuf,
        socket: PathBuf,
        pid_file: PathBuf,
    }

    impl TestEndpoint {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "csm-relay-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            Self {
                socket: dir.join("relay.sock"),
                pid_file: dir.join("relay.pid"),
                dir,
            }
        }

        fn start(&self, cdp: Arc<CdpClient>, limits: RelayLimits) -> TestServer {
            let stop = Arc::new(AtomicBool::new(false));
            let stop_worker = Arc::clone(&stop);
            let socket = self.socket.clone();
            let pid_file = self.pid_file.clone();
            let data_dir = self.dir.clone();
            let worker = thread::spawn(move || {
                serve_relay_socket_with_limits(
                    &socket,
                    &data_dir,
                    Some(&pid_file),
                    cdp,
                    stop_worker,
                    false,
                    limits,
                )
                .unwrap();
            });
            let server = TestServer { stop, worker };
            let deadline = Instant::now() + Duration::from_secs(3);
            // The PID file is created after the socket is bound and its mode has been set.
            while !self.pid_file.exists() {
                assert!(Instant::now() < deadline, "relay listener did not start");
                thread::sleep(Duration::from_millis(5));
            }
            server
        }

        fn open_connection(&self) -> BufReader<UnixStream> {
            let stream = UnixStream::connect(&self.socket).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            BufReader::new(stream)
        }
    }

    impl Drop for TestEndpoint {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    struct TestServer {
        stop: Arc<AtomicBool>,
        worker: thread::JoinHandle<()>,
    }

    impl TestServer {
        fn stop(self) {
            self.stop.store(true, Ordering::Release);
            self.worker.join().unwrap();
        }
    }

    fn new_test_cdp(factory: TransportFactory) -> Arc<CdpClient> {
        CdpClient::with_factory(
            PathBuf::from("/tmp/none"),
            9222,
            Some("ws://127.0.0.1:9222/devtools/browser/test".into()),
            factory,
            false,
        )
    }

    fn mock_transport(
        on_event: Arc<dyn Fn(TransportEvent) + Send + Sync>,
    ) -> Arc<dyn CdpTransport> {
        Arc::new(SimpleMockWs {
            open: AtomicBool::new(true),
            on_event,
        })
    }

    fn mock_cdp() -> Arc<CdpClient> {
        new_test_cdp(Arc::new(|_, _, on_event| Ok(mock_transport(on_event))))
    }

    fn send_request(connection: &mut BufReader<UnixStream>, id: u64, method: &str) {
        writeln!(
            connection.get_mut(),
            "{}",
            json!({ "id": id, "method": method })
        )
        .unwrap();
    }

    fn read_response(connection: &mut BufReader<UnixStream>) -> Value {
        let mut line = String::new();
        connection.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn standby_preserves_connected_relay_past_the_disconnected_idle_deadline() {
        let endpoint = TestEndpoint::new();
        let dial_count = Arc::new(AtomicUsize::new(0));
        let dialed = Arc::clone(&dial_count);
        let cdp = new_test_cdp(Arc::new(move |_, _, on_event| {
            dialed.fetch_add(1, Ordering::Relaxed);
            Ok(mock_transport(on_event))
        }));
        let idle_threshold = Duration::from_millis(150);
        let server = endpoint.start(
            Arc::clone(&cdp),
            RelayLimits {
                disconnected_idle_exit: idle_threshold,
                ..RelayLimits::default()
            },
        );
        let call = |connection: &mut BufReader<UnixStream>, method, params| {
            writeln!(
                connection.get_mut(),
                "{}",
                json!({ "id": 1, "method": method, "params": params })
            )
            .unwrap();
            read_response(connection)["result"].clone()
        };
        let mut client = endpoint.open_connection();
        assert_eq!(
            call(&mut client, "connect", json!({}))["connectionMode"],
            "personal"
        );
        assert!(cdp.screencast_target_id().is_some());
        let standby = call(&mut client, "disconnect", json!({ "mode": "standby" }));
        assert_eq!(standby["mode"], "standby");
        assert_eq!(standby["standby"], true);
        assert_eq!(standby["connected"], true);
        assert_eq!(standby["paused"], true);
        assert!(standby["pauseEpoch"].as_u64().unwrap() > 0);
        assert!(cdp.screencast_target_id().is_none());
        drop(client);

        thread::sleep(idle_threshold * 3);
        assert!(!server.worker.is_finished());
        assert!(cdp.is_connected() && cdp.is_paused());

        let mut client = endpoint.open_connection();
        let resumed = call(&mut client, "connect", json!({ "mode": "personal" }));
        assert_eq!(resumed["paused"], false);
        assert_eq!(resumed["connectionMode"], "personal");
        assert_eq!(dial_count.load(Ordering::Relaxed), 1);
        assert!(cdp.screencast_target_id().is_some());
        let revoke = call(&mut client, "disconnect", json!({}));
        assert_eq!(revoke["mode"], "revoke");
        assert_eq!(revoke["connected"], false);
        assert_eq!(revoke["paused"], false);
        assert!(revoke["connectionMode"].is_null());
        drop(client);

        let deadline = Instant::now() + Duration::from_secs(2);
        while !server.worker.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(server.worker.is_finished());
        server.stop();
    }

    #[test]
    fn launch_remote_reuses_a_loopback_endpoint_and_switches_from_personal() {
        use std::net::TcpListener;

        let endpoint = TestEndpoint::new();
        let personal = TcpListener::bind("127.0.0.1:0").unwrap();
        let port_file = endpoint.dir.join("DevToolsActivePort");
        let remote_profile = endpoint.dir.join("remote-profile");
        fs::create_dir(&remote_profile).unwrap();
        let remote = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = remote.local_addr().unwrap().port();
        fs::write(
            remote_profile.join("DevToolsActivePort"),
            format!("{port}\n/devtools/browser/remote\n"),
        )
        .unwrap();
        let url = format!("ws://127.0.0.1:{port}/devtools/browser/remote");
        let served_url = url.clone();
        let responder = thread::spawn(move || {
            let (mut stream, _) = remote.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = BufReader::new(&stream);
            let mut line = String::new();
            request.read_line(&mut line).unwrap();
            assert_eq!(line, "GET /json/version HTTP/1.1\r\n");
            loop {
                line.clear();
                assert_ne!(request.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
            }
            let body = json!({ "webSocketDebuggerUrl": served_url }).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let (dialed, urls) = mpsc::channel();
        let cdp = CdpClient::with_factory(
            port_file.clone(),
            port,
            None,
            Arc::new(move |endpoint, _, on_event| {
                dialed.send(endpoint.to_string()).unwrap();
                Ok(mock_transport(on_event))
            }),
            false,
        );
        assert!(cdp.ensure_connected_with_mode("personal").is_err());
        assert!(urls.try_recv().is_err());
        fs::write(
            &port_file,
            format!(
                "{}\n/devtools/browser/personal\n",
                personal.local_addr().unwrap().port()
            ),
        )
        .unwrap();
        let personal = dispatch_rpc(
            &cdp,
            &endpoint.dir,
            "connect",
            json!({ "mode": "personal" }),
        )
        .unwrap();
        assert_eq!(personal["connectionMode"], "personal");
        assert!(urls.recv().unwrap().ends_with("/personal"));

        let launched = dispatch_rpc(
            &cdp,
            &endpoint.dir,
            "launch_remote",
            json!({ "qualityPreset": "high" }),
        )
        .unwrap();
        assert_eq!(launched["connectionMode"], "remote");
        assert_eq!(launched["qualityApplied"], "high");
        assert_eq!(launched["activeTargetId"], "t1");
        assert_eq!(launched["connected"], true);
        assert_eq!(urls.recv().unwrap(), url);
        assert_eq!(cdp.status_snapshot(1, "test")["connectionMode"], "remote");
        let poll = cdp
            .poll_frame(
                serde_json::from_value(json!({ "qualityPreset": "high", "waitMs": 0 })).unwrap(),
            )
            .unwrap();
        assert_eq!(poll["connectionMode"], "remote");
        responder.join().unwrap();
    }

    #[test]
    fn a_second_relay_cannot_replace_the_owner_and_normal_exit_allows_restart() {
        let endpoint = TestEndpoint::new();
        let first = endpoint.start(mock_cdp(), RelayLimits::default());
        let socket_inode = fs::symlink_metadata(&endpoint.socket).unwrap().ino();
        let locked = serve_relay_socket(
            &endpoint.socket,
            &endpoint.dir,
            Some(&endpoint.pid_file),
            mock_cdp(),
            Arc::new(AtomicBool::new(true)),
            false,
        );
        assert!(matches!(locked, Err(ServeError::Owned)));
        assert_eq!(
            fs::symlink_metadata(&endpoint.socket).unwrap().ino(),
            socket_inode
        );
        assert_eq!(probe_relay_ping(&endpoint.socket).unwrap()["ok"], true);
        assert_eq!(
            fs::metadata(endpoint.dir.join("relay.sock.lock"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        first.stop();
        assert!(!endpoint.socket.exists());
        assert!(!endpoint.pid_file.exists());
        let restarted = endpoint.start(mock_cdp(), RelayLimits::default());
        assert_eq!(probe_relay_ping(&endpoint.socket).unwrap()["ok"], true);
        restarted.stop();
        assert!(!endpoint.socket.exists());
    }

    #[test]
    fn relay_exit_keeps_replacement_socket_and_regular_file() {
        for replace_with_socket in [true, false] {
            let endpoint = TestEndpoint::new();
            let server = endpoint.start(mock_cdp(), RelayLimits::default());
            let original = fs::symlink_metadata(&endpoint.socket).unwrap();
            fs::rename(&endpoint.socket, endpoint.dir.join("original.sock")).unwrap();
            let replacement_listener = if replace_with_socket {
                Some(UnixListener::bind(&endpoint.socket).unwrap())
            } else {
                fs::write(&endpoint.socket, "replacement").unwrap();
                None
            };
            let replacement = fs::symlink_metadata(&endpoint.socket).unwrap();
            assert_ne!(replacement.ino(), original.ino());
            fs::remove_file(&endpoint.pid_file).unwrap();
            fs::write(&endpoint.pid_file, "replacement PID file").unwrap();

            server.stop();
            let retained = fs::symlink_metadata(&endpoint.socket).unwrap();
            assert_eq!(retained.ino(), replacement.ino());
            assert_eq!(retained.file_type().is_socket(), replace_with_socket);
            assert_eq!(
                fs::read_to_string(&endpoint.pid_file).unwrap(),
                "replacement PID file"
            );
            drop(replacement_listener);
        }
    }

    #[test]
    fn relay_rejects_excess_connections_and_keeps_the_accepted_client() {
        let endpoint = TestEndpoint::new();
        let server = endpoint.start(
            mock_cdp(),
            RelayLimits {
                clients: 1,
                ..RelayLimits::default()
            },
        );
        let mut accepted = endpoint.open_connection();
        send_request(&mut accepted, 1, "ping");
        assert_eq!(read_response(&mut accepted)["result"]["ok"], true);

        let mut excess = endpoint.open_connection();
        assert_eq!(
            read_response(&mut excess)["error"],
            "Relay busy: too many client connections"
        );
        assert_eq!(excess.read_line(&mut String::new()).unwrap(), 0);
        send_request(&mut accepted, 2, "ping");
        assert_eq!(read_response(&mut accepted)["id"], 2);
        drop(accepted);
        server.stop();
    }

    #[test]
    fn relay_bounds_a_request_line_and_closes_only_the_offending_connection() {
        let endpoint = TestEndpoint::new();
        let server = endpoint.start(
            mock_cdp(),
            RelayLimits {
                request_bytes: 64,
                ..RelayLimits::default()
            },
        );
        let mut offending = endpoint.open_connection();
        let request = json!({ "id": 1, "method": "ping" }).to_string();
        // A newline is the delimiter, so exactly 64 request bytes are accepted.
        writeln!(offending.get_mut(), "{request:63}").unwrap();
        assert_eq!(read_response(&mut offending)["id"], 1);
        offending.get_mut().write_all(&[b'X'; 65]).unwrap();
        assert_eq!(
            read_response(&mut offending)["error"],
            "Request exceeds maximum allowed size"
        );
        assert_eq!(offending.read_line(&mut String::new()).unwrap(), 0);

        let mut fresh = endpoint.open_connection();
        send_request(&mut fresh, 2, "ping");
        assert_eq!(read_response(&mut fresh)["result"]["ok"], true);
        drop(fresh);
        server.stop();
    }

    #[test]
    fn relay_replies_busy_while_a_half_closed_client_still_has_an_in_flight_rpc() {
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let cdp = new_test_cdp(Arc::new(move |_, _, on_event| {
            entered_tx.send(()).map_err(|err| err.to_string())?;
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .map_err(|err| err.to_string())?;
            Ok(mock_transport(on_event))
        }));
        let endpoint = TestEndpoint::new();
        let server = endpoint.start(
            cdp,
            RelayLimits {
                clients: 1,
                rpcs: 1,
                ..RelayLimits::default()
            },
        );
        let mut connection = endpoint.open_connection();
        send_request(&mut connection, 1, "connect");
        entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        send_request(&mut connection, 2, "ping");
        assert_eq!(
            read_response(&mut connection),
            json!({
                "id": 2,
                "error": "Relay busy: too many concurrent requests",
            })
        );
        connection.get_ref().shutdown(Shutdown::Write).unwrap();
        let mut excess = endpoint.open_connection();
        assert_eq!(
            read_response(&mut excess)["error"],
            "Relay busy: too many client connections",
        );
        release_tx.send(()).unwrap();
        let result = read_response(&mut connection);
        assert_eq!(result["id"], 1);
        assert_eq!(result["result"]["connected"], true);
        drop(connection);
        server.stop();
    }

    #[test]
    fn saturated_rpc_slots_do_not_block_connection_controls_or_revoke() {
        let cdp = mock_cdp();
        cdp.ensure_connected().unwrap();
        let limits = RelayLimits::default();
        let rpcs = Arc::new(AtomicUsize::new(0));
        let slots: Vec<_> = (0..limits.rpcs)
            .map(|_| RelaySlot::acquire(&rpcs, limits.rpcs).unwrap())
            .collect();
        let clients = Arc::new(AtomicUsize::new(0));
        let slot = RelaySlot::acquire(&clients, limits.clients).unwrap();
        let (server, peer) = UnixStream::pair().unwrap();
        let worker_cdp = Arc::clone(&cdp);
        let worker_rpcs = Arc::clone(&rpcs);
        let worker = thread::spawn(move || {
            handle_client_conn(
                server,
                worker_cdp,
                Arc::from(Path::new(".")),
                worker_rpcs,
                Arc::new(Mutex::new(Instant::now())),
                limits,
                slot,
            );
        });
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut peer = BufReader::new(peer);
        send_request(&mut peer, 1, "ping");
        assert_eq!(
            read_response(&mut peer)["error"],
            "Relay busy: too many concurrent requests"
        );
        for (id, control, paused, connected) in [
            (2, "pause", true, true),
            (3, "resume", false, true),
            (4, "disconnect", false, false),
        ] {
            send_request(&mut peer, id, control);
            let reply = read_response(&mut peer);
            assert_eq!(reply["id"], id);
            assert_eq!(reply["result"]["ok"], true);
            assert_eq!(reply["result"]["paused"], paused);
            assert_eq!(reply["result"]["connected"], connected);
        }
        assert!(!cdp.is_connected());
        assert_eq!(rpcs.load(Ordering::SeqCst), 32);
        drop(slots);
        drop(peer);
        worker.join().unwrap();
    }

    #[test]
    fn unix_socket_relay_serves_multiplexed_ipc_with_0600_permissions() {
        let endpoint = TestEndpoint::new();
        let server = endpoint.start(mock_cdp(), RelayLimits::default());

        let mode = fs::metadata(&endpoint.socket).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let client = RelayClient::connect_only(endpoint.socket.clone());
        let ping = client
            .call("ping", json!({}), Duration::from_secs(2))
            .unwrap();
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

        // Concurrent fast input does NOT block behind a 350ms long-poll on the same socket
        let seq = frame["seq"].as_u64().unwrap();
        let hash = frame["frameHash"].as_str().unwrap().to_string();
        let c_poll = Arc::clone(&client);
        let poll_handle = thread::spawn(move || {
            c_poll
                .call(
                    "poll_frame",
                    json!({ "sinceSeq": seq, "lastFrameHash": hash, "qualityPreset": "low", "waitMs": 350 }),
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

        server.stop();
        assert!(!endpoint.socket.exists());
    }
}
