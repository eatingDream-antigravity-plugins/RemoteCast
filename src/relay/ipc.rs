use crate::config::Config;
#[cfg(not(test))]
use crate::local_io::create_private_directory;
use crate::local_io::{effective_uid, owned_directory, read_bounded_line, socket_parent};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
#[cfg(not(test))]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
#[cfg(not(test))]
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub trait RelayBackend: Send + Sync {
    fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String>;
    fn warm(&self) -> Result<(), String> {
        Ok(())
    }
}

const MAX_PROBE_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_RPC_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

pub fn verify_same_user(stream: &UnixStream) -> io::Result<()> {
    verify_peer_uid(stream, effective_uid())
}

fn verify_peer_uid(stream: &UnixStream, uid: libc::uid_t) -> io::Result<()> {
    if peer_uid(stream)? != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Relay peer must belong to the current user",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn peer_uid(stream: &UnixStream) -> io::Result<libc::uid_t> {
    let mut cred = std::mem::MaybeUninit::<libc::ucred>::uninit();
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: cred points to len writable bytes and the socket fd is valid.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            cred.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    if len as usize != std::mem::size_of::<libc::ucred>() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid relay peer credentials",
        ));
    }
    // SAFETY: successful getsockopt initialized the complete structure.
    Ok(unsafe { cred.assume_init() }.uid)
}

#[cfg(not(target_os = "linux"))]
fn peer_uid(stream: &UnixStream) -> io::Result<libc::uid_t> {
    let mut uid = 0;
    let mut gid = 0;
    // SAFETY: both output pointers and the socket fd are valid.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(uid)
}

fn connect_socket(sock_path: &Path) -> io::Result<UnixStream> {
    owned_directory(socket_parent(sock_path), false)?;
    let metadata = fs::symlink_metadata(sock_path)?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Unsafe relay socket",
        ));
    }
    let stream = UnixStream::connect(sock_path)?;
    verify_same_user(&stream)?;
    Ok(stream)
}

/// Quick one-shot probe used by doctor and relay startup deduplication.
pub fn probe_relay_ping(sock_path: &Path) -> io::Result<Value> {
    let mut stream = connect_socket(sock_path)?;
    stream.set_read_timeout(Some(Duration::from_millis(600)))?;
    stream.set_write_timeout(Some(Duration::from_millis(400)))?;
    writeln!(
        stream,
        "{}",
        json!({ "id": 0, "method": "ping", "params": {} })
    )?;
    read_probe_reply(&mut BufReader::new(stream))
}

fn read_probe_reply(reader: &mut impl BufRead) -> io::Result<Value> {
    let mut line = Vec::new();
    read_bounded_line(reader, &mut line, MAX_PROBE_RESPONSE_BYTES)?;
    let mut parsed: Value = serde_json::from_slice(&line)?;
    if parsed["id"] == 0 && parsed["result"]["ok"] == true {
        return Ok(parsed["result"].take());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid relay probe reply",
    ))
}

pub(crate) fn should_recycle_relay(ping: &Value, expected_version: &str) -> bool {
    let connected = ping
        .get("connected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if connected {
        return false;
    }
    let version = ping.get("version").and_then(Value::as_str).unwrap_or("");
    version != expected_version
}

enum ShutdownOutcome {
    Stopping,
    ActiveSession,
    UnsupportedOrError,
}

fn send_shutdown_rpc(sock_path: &Path) -> io::Result<ShutdownOutcome> {
    let mut stream = connect_socket(sock_path)?;
    stream.set_read_timeout(Some(Duration::from_millis(600)))?;
    stream.set_write_timeout(Some(Duration::from_millis(400)))?;
    writeln!(
        stream,
        "{}",
        json!({ "id": 0, "method": "shutdown", "params": {} })
    )?;
    let mut line = Vec::new();
    read_bounded_line(
        &mut BufReader::new(stream),
        &mut line,
        MAX_PROBE_RESPONSE_BYTES,
    )?;
    let parsed: Value = serde_json::from_slice(&line)?;
    if parsed["id"] != 0 || parsed.get("error").is_some_and(|e| !e.is_null()) {
        return Ok(ShutdownOutcome::UnsupportedOrError);
    }
    let result = &parsed["result"];
    if result["ok"] == true && result["stopping"] == true {
        return Ok(ShutdownOutcome::Stopping);
    }
    if result["reason"] == "active_session" || result["connected"] == true {
        return Ok(ShutdownOutcome::ActiveSession);
    }
    Ok(ShutdownOutcome::UnsupportedOrError)
}

fn wait_for_relay_exit(sock_path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let is_gone = || {
        !sock_path.exists()
            || matches!(
                probe_relay_ping(sock_path),
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    )
            )
    };
    while Instant::now() < deadline {
        if is_gone() {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    is_gone()
}

#[cfg(not(test))]
fn verify_relay_process(pid: libc::pid_t) -> bool {
    let Ok(output) = Command::new("ps")
        .args(["-o", "uid=,command=", "-p", &pid.to_string()])
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(line) = stdout.lines().find(|l| !l.trim().is_empty()) else {
        return false;
    };
    let mut parts = line.trim_start().splitn(2, char::is_whitespace);
    let Some(uid_str) = parts.next() else {
        return false;
    };
    let Some(cmd) = parts.next() else {
        return false;
    };
    let Ok(uid) = uid_str.trim().parse::<u32>() else {
        return false;
    };
    if uid != effective_uid() as u32 {
        return false;
    }
    cmd.contains("chrome-stream-mcp") && cmd.split_whitespace().any(|arg| arg == "relay")
}

#[cfg(not(test))]
fn terminate_legacy_relay(ping: &Value) -> bool {
    let Some(raw_pid) = ping.get("pid").and_then(Value::as_u64) else {
        return false;
    };
    let Ok(pid) = libc::pid_t::try_from(raw_pid) else {
        return false;
    };
    if pid <= 1 || pid as u32 == std::process::id() {
        return false;
    }
    if !verify_relay_process(pid) {
        return false;
    }
    // SAFETY: pid > 1, != current PID, and verified via ps to match effective_uid() and `chrome-stream-mcp` `relay`.
    unsafe { libc::kill(pid, libc::SIGTERM) == 0 }
}

type PendingReplies = HashMap<u64, mpsc::Sender<Result<Value, String>>>;

struct ActiveConn {
    writer: Mutex<UnixStream>,
    open: AtomicBool,
    pending: Mutex<PendingReplies>,
}

/// Persistent multiplexed Unix-socket JSON-RPC client to the detached `chrome-stream-mcp relay` daemon.
/// Concurrent callers (e.g. long-polling `chrome_stream_frame` + fast `chrome_stream_input`) share one socket
/// without head-of-line blocking.
pub struct RelayClient {
    #[cfg(not(test))]
    data_dir: PathBuf,
    sock_path: PathBuf,
    #[cfg(not(test))]
    port_file: Option<PathBuf>,
    #[cfg(not(test))]
    debug_port: u16,
    auto_spawn: bool,
    connect_lock: Mutex<()>,
    conn: Mutex<Option<Arc<ActiveConn>>>,
    next_id: AtomicU64,
}

impl RelayClient {
    pub fn from_config(cfg: &Config) -> Arc<Self> {
        Arc::new(Self {
            #[cfg(not(test))]
            data_dir: cfg.data_dir.clone(),
            sock_path: cfg.relay_sock.clone(),
            #[cfg(not(test))]
            port_file: cfg.port_files.first().cloned(),
            #[cfg(not(test))]
            debug_port: cfg.debug_port,
            auto_spawn: true,
            connect_lock: Mutex::new(()),
            conn: Mutex::new(None),
            next_id: AtomicU64::new(1),
        })
    }

    pub fn connect_only(sock_path: PathBuf) -> Arc<Self> {
        #[cfg(not(test))]
        let data_dir = sock_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Arc::new(Self {
            #[cfg(not(test))]
            data_dir,
            sock_path,
            #[cfg(not(test))]
            port_file: None,
            #[cfg(not(test))]
            debug_port: 9222,
            auto_spawn: false,
            connect_lock: Mutex::new(()),
            conn: Mutex::new(None),
            next_id: AtomicU64::new(1),
        })
    }

    #[cfg(test)]
    fn spawn_relay(&self) -> Result<(), String> {
        Err("Starting a relay process is disabled in test builds".into())
    }

    #[cfg(not(test))]
    fn spawn_relay(&self) -> Result<(), String> {
        create_private_directory(&self.data_dir)
            .map_err(|e| format!("Could not prepare relay data directory: {e}"))?;
        let exe = std::env::current_exe()
            .map_err(|e| format!("Failed locating current executable for relay spawn: {e}"))?;

        let mut cmd = Command::new(exe);
        cmd.arg("relay")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .env("CHROME_STREAM_DATA_DIR", &self.data_dir)
            .env("CHROME_STREAM_RELAY_SOCK", &self.sock_path)
            .env(
                "CHROME_STREAM_CHROME_DEBUG_PORT",
                self.debug_port.to_string(),
            );
        if let Some(port_file) = &self.port_file {
            cmd.env("CHROME_STREAM_CHROME_PORT_FILE", port_file);
        }

        // Detach into a new session so `chrome-stream-mcp relay` survives MCP server reloads.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }

        cmd.spawn()
            .map_err(|e| format!("Failed spawning chrome-stream-mcp relay daemon: {e}"))?;
        Ok(())
    }

    fn attach_stream(&self, stream: UnixStream) -> Result<Arc<ActiveConn>, String> {
        let reader_stream = stream
            .try_clone()
            .map_err(|e| format!("Failed cloning relay Unix socket: {e}"))?;
        let active = Arc::new(ActiveConn {
            writer: Mutex::new(stream),
            open: AtomicBool::new(true),
            pending: Mutex::new(HashMap::new()),
        });

        let connection = Arc::clone(&active);
        thread::Builder::new()
            .name("chrome-stream-ipc".to_string())
            .spawn(move || {
                let mut reader = BufReader::new(reader_stream);
                let mut line = Vec::new();
                let reason = loop {
                    match read_bounded_line(&mut reader, &mut line, MAX_RPC_RESPONSE_BYTES) {
                        Ok(0) => break "Local CDP relay connection closed".to_string(),
                        Err(err) => break format!("Local CDP relay response read failed: {err}"),
                        Ok(_) => {}
                    }
                    let Ok(mut msg) = serde_json::from_slice::<Value>(&line) else {
                        continue;
                    };
                    let Some(id) = msg["id"].as_u64() else {
                        continue;
                    };
                    let tx = connection.pending.lock().unwrap().remove(&id);
                    if let Some(tx) = tx {
                        let reply = match msg["error"].as_str() {
                            Some(err) => Err(err.to_string()),
                            None => Ok(msg["result"].take()),
                        };
                        let _ = tx.send(reply);
                    }
                };
                connection.open.store(false, Ordering::SeqCst);
                let _ = reader.get_ref().shutdown(Shutdown::Both);
                let drained = std::mem::take(&mut *connection.pending.lock().unwrap());
                for tx in drained.into_values() {
                    let _ = tx.send(Err(reason.clone()));
                }
            })
            .map_err(|e| format!("Failed spawning IPC reader thread: {e}"))?;

        *self.conn.lock().unwrap() = Some(Arc::clone(&active));
        Ok(active)
    }

    fn recycle_stale_relay(&self, ping: &Value) -> bool {
        match send_shutdown_rpc(&self.sock_path) {
            Ok(ShutdownOutcome::ActiveSession) => return true,
            Ok(ShutdownOutcome::Stopping) => {
                wait_for_relay_exit(&self.sock_path, Duration::from_millis(1500));
                return false;
            }
            Ok(ShutdownOutcome::UnsupportedOrError) => {}
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                return false;
            }
            Err(_) => {}
        }

        #[cfg(not(test))]
        if terminate_legacy_relay(ping) {
            wait_for_relay_exit(&self.sock_path, Duration::from_millis(1500));
        }
        #[cfg(test)]
        let _ = ping;
        false
    }

    pub fn warm_connection(&self) -> Result<(), String> {
        self.ensure_connected().map(|_| ())
    }

    fn ensure_connected(&self) -> Result<Arc<ActiveConn>, String> {
        self.ensure_connected_for_version(crate::config::RELAY_VERSION)
    }

    fn ensure_connected_for_version(
        &self,
        expected_version: &str,
    ) -> Result<Arc<ActiveConn>, String> {
        {
            let guard = self.conn.lock().unwrap();
            if let Some(active) = guard.as_ref() {
                if active.open.load(Ordering::SeqCst) {
                    return Ok(Arc::clone(active));
                }
            }
        }

        let _lock = self.connect_lock.lock().unwrap();
        {
            let guard = self.conn.lock().unwrap();
            if let Some(active) = guard.as_ref() {
                if active.open.load(Ordering::SeqCst) {
                    return Ok(Arc::clone(active));
                }
            }
        }

        if self.auto_spawn {
            match probe_relay_ping(&self.sock_path) {
                Ok(ping) => {
                    let keep_existing = if should_recycle_relay(&ping, expected_version) {
                        self.recycle_stale_relay(&ping)
                    } else {
                        true
                    };
                    if keep_existing {
                        let stream = connect_socket(&self.sock_path)
                            .map_err(|err| format!("Unable to connect to relay socket: {err}"))?;
                        return self.attach_stream(stream);
                    }
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    ) => {}
                Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
                    return Err(format!("Unable to connect to relay socket: {err}"));
                }
                Err(_) => match connect_socket(&self.sock_path) {
                    Ok(stream) => return self.attach_stream(stream),
                    Err(err)
                        if matches!(
                            err.kind(),
                            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                        ) => {}
                    Err(err) => return Err(format!("Unable to connect to relay socket: {err}")),
                },
            }
        } else {
            let stream = connect_socket(&self.sock_path)
                .map_err(|err| format!("Unable to connect to relay socket: {err}"))?;
            return self.attach_stream(stream);
        }

        self.spawn_relay()?;
        let deadline = Instant::now() + Duration::from_millis(2500);
        let mut last_err = String::from("Failed to start local CDP relay daemon");
        let mut attempt: u32 = 0;
        while Instant::now() < deadline {
            let sleep_ms = if attempt < 12 { 10 } else { 25 };
            attempt += 1;
            thread::sleep(Duration::from_millis(sleep_ms));
            match connect_socket(&self.sock_path) {
                Ok(stream) => return self.attach_stream(stream),
                Err(e) => last_err = e.to_string(),
            }
        }
        Err(last_err)
    }
}

impl RelayBackend for RelayClient {
    fn warm(&self) -> Result<(), String> {
        self.warm_connection()
    }

    fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        for attempt in 0..2 {
            let active = self.ensure_connected()?;
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let (tx, rx) = mpsc::channel();
            {
                let mut pending = active.pending.lock().unwrap();
                if !active.open.load(Ordering::SeqCst) {
                    continue;
                }
                pending.insert(id, tx);
            }

            let payload = format!(
                "{}\n",
                json!({ "id": id, "method": method, "params": params })
            );
            let write_res = {
                let mut w = active.writer.lock().unwrap();
                let result = w
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .and_then(|_| w.write_all(payload.as_bytes()));
                if result.is_err() {
                    let _ = w.shutdown(Shutdown::Both);
                }
                result
            };
            if let Err(e) = write_res {
                active.open.store(false, Ordering::SeqCst);
                active.pending.lock().unwrap().remove(&id);
                // Safe single retry on stale/restarted socket before request was delivered.
                if attempt == 0 {
                    continue;
                }
                return Err(format!("Local CDP relay connection closed: {e}"));
            }

            return match rx.recv_timeout(timeout) {
                Ok(res) => res,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    active.pending.lock().unwrap().remove(&id);
                    Err(format!("Relay timeout on {method}"))
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    active.pending.lock().unwrap().remove(&id);
                    Err("Local CDP relay connection closed".to_string())
                }
            };
        }
        Err("Local CDP relay connection closed".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn rejects_untrusted_socket_parents_and_checks_peer_uid() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("csm-ipc-parent-{}", std::process::id()));
        let shared = root.join("shared");
        fs::create_dir_all(&shared).unwrap();
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o777)).unwrap();
        let unsafe_socket = shared.join("relay.sock");
        assert_eq!(
            connect_socket(&unsafe_socket).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );

        let link = root.join("link");
        symlink(&shared, &link).unwrap();
        assert!(connect_socket(&link.join("relay.sock")).is_err());
        fs::remove_dir_all(root).unwrap();

        let (client, server) = UnixStream::pair().unwrap();
        for side in [&client, &server] {
            verify_same_user(side).unwrap();
            assert_eq!(
                verify_peer_uid(side, effective_uid().wrapping_add(1))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::PermissionDenied
            );
        }
    }

    #[test]
    fn oversize_relay_replies_fail_with_an_explicit_limit_error() {
        let mut probe = Cursor::new(vec![b'x'; MAX_PROBE_RESPONSE_BYTES + 1]);
        let err = read_probe_reply(&mut probe).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("65536-byte limit"));

        let relay = RelayClient::connect_only(PathBuf::new());
        let (client, mut peer) = UnixStream::pair().unwrap();
        relay.attach_stream(client).unwrap();
        let responder = thread::spawn(move || {
            let mut request = String::new();
            BufReader::new(&peer).read_line(&mut request).unwrap();
            peer.write_all(&vec![b'x'; MAX_RPC_RESPONSE_BYTES + 1])
                .unwrap();
        });
        let err = relay
            .call("poll_frame", json!({}), Duration::from_secs(3))
            .unwrap_err();
        assert!(err.contains("16777216-byte limit"));
        responder.join().unwrap();
    }

    #[test]
    fn old_reader_eof_cannot_cancel_a_new_connections_pending_call() {
        fn read_request(stream: &UnixStream) -> Value {
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            serde_json::from_str(&line).unwrap()
        }

        let relay = RelayClient::connect_only(PathBuf::new());
        let (old_socket, old_peer) = UnixStream::pair().unwrap();
        relay.attach_stream(old_socket).unwrap();
        let old = Arc::clone(&relay);
        let old_rpc = thread::spawn(move || old.call("status", json!({}), Duration::from_secs(3)));
        assert_eq!(read_request(&old_peer)["method"], "status");

        let (new_socket, mut new_peer) = UnixStream::pair().unwrap();
        relay.attach_stream(new_socket).unwrap();
        let new_rpc = thread::spawn(move || relay.call("ping", json!({}), Duration::from_secs(3)));
        let req = read_request(&new_peer);
        assert_eq!(req["method"], "ping");
        drop(old_peer);
        assert!(old_rpc
            .join()
            .unwrap()
            .unwrap_err()
            .contains("connection closed"));

        writeln!(
            new_peer,
            "{}",
            json!({ "id": req["id"], "result": { "ok": true } })
        )
        .unwrap();
        assert_eq!(new_rpc.join().unwrap().unwrap()["ok"], true);
    }

    #[test]
    fn should_recycle_relay_checks_version_and_connected_state() {
        assert!(should_recycle_relay(
            &json!({ "ok": true, "version": "0.5.1", "connected": false }),
            "0.6.0"
        ));
        assert!(should_recycle_relay(
            &json!({ "ok": true, "connected": false }),
            "0.6.0"
        ));
        assert!(!should_recycle_relay(
            &json!({ "ok": true, "version": "0.5.1", "connected": true }),
            "0.6.0"
        ));
        assert!(!should_recycle_relay(
            &json!({ "ok": true, "version": "0.6.0", "connected": false }),
            "0.6.0"
        ));
        assert!(!should_recycle_relay(
            &json!({ "ok": true, "version": "0.6.0", "connected": true }),
            "0.6.0"
        ));
    }
}
