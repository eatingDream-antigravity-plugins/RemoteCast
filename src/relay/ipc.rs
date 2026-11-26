use crate::config::Config;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
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

/// Quick one-shot probe used by `chrome-stream-mcp doctor` and relay startup deduplication.
pub fn probe_relay_ping(sock_path: &Path) -> Option<Value> {
    let mut stream = UnixStream::connect(sock_path).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(600)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(400)));
    let req = format!("{}\n", json!({ "id": 0, "method": "ping", "params": {} }));
    stream.write_all(req.as_bytes()).ok()?;
    stream.flush().ok()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let n = reader.read_line(&mut line).ok()?;
    if n == 0 {
        return None;
    }
    let parsed: Value = serde_json::from_str(line.trim()).ok()?;
    parsed.get("result").cloned()
}

struct ActiveConn {
    writer: Arc<Mutex<UnixStream>>,
    open: Arc<AtomicBool>,
}

type PendingReplies = Arc<Mutex<HashMap<u64, mpsc::Sender<Result<Value, String>>>>>;

/// Persistent multiplexed Unix-socket JSON-RPC client to the detached `chrome-stream-mcp relay` daemon.
/// Concurrent callers (e.g. long-polling `chrome_stream_frame` + fast `chrome_stream_input`) share one socket
/// without head-of-line blocking.
pub struct RelayClient {
    data_dir: PathBuf,
    sock_path: PathBuf,
    port_file: PathBuf,
    debug_port: u16,
    auto_spawn: bool,
    connect_lock: Mutex<()>,
    conn: Mutex<Option<Arc<ActiveConn>>>,
    pending: PendingReplies,
    next_id: AtomicU64,
}

impl RelayClient {
    pub fn from_config(cfg: &Config) -> Arc<Self> {
        Arc::new(Self {
            data_dir: cfg.data_dir.clone(),
            sock_path: cfg.relay_sock.clone(),
            port_file: cfg.port_file.clone(),
            debug_port: cfg.debug_port,
            auto_spawn: true,
            connect_lock: Mutex::new(()),
            conn: Mutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
        })
    }

    pub fn connect_only(sock_path: PathBuf) -> Arc<Self> {
        let data_dir = sock_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Arc::new(Self {
            data_dir,
            sock_path,
            port_file: PathBuf::new(),
            debug_port: 9222,
            auto_spawn: false,
            connect_lock: Mutex::new(()),
            conn: Mutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: AtomicU64::new(1),
        })
    }

    fn spawn_relay(&self) -> Result<(), String> {
        let _ = fs::create_dir_all(&self.data_dir);
        let _ = fs::set_permissions(&self.data_dir, fs::Permissions::from_mode(0o700));
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
        if !self.port_file.as_os_str().is_empty() {
            cmd.env("CHROME_STREAM_CHROME_PORT_FILE", &self.port_file);
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
            writer: Arc::new(Mutex::new(stream)),
            open: Arc::new(AtomicBool::new(true)),
        });

        let open_flag = Arc::clone(&active.open);
        let pending_map = Arc::clone(&self.pending);
        thread::Builder::new()
            .name("chrome-stream-ipc".to_string())
            .spawn(move || {
                let mut reader = BufReader::new(reader_stream);
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
                            let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
                                continue;
                            };
                            let Some(id) = msg.get("id").and_then(|v| v.as_u64()) else {
                                continue;
                            };
                            let tx = { pending_map.lock().unwrap().remove(&id) };
                            if let Some(tx) = tx {
                                if let Some(err) = msg.get("error").and_then(|v| v.as_str()) {
                                    let _ = tx.send(Err(err.to_string()));
                                } else {
                                    let res =
                                        msg.get("result").cloned().unwrap_or_else(|| json!({}));
                                    let _ = tx.send(Ok(res));
                                }
                            }
                        }
                    }
                }
                open_flag.store(false, Ordering::SeqCst);
                let drained: Vec<_> = {
                    let mut map = pending_map.lock().unwrap();
                    map.drain().map(|(_, tx)| tx).collect()
                };
                for tx in drained {
                    let _ = tx.send(Err("Local CDP relay connection closed".to_string()));
                }
            })
            .map_err(|e| format!("Failed spawning IPC reader thread: {e}"))?;

        *self.conn.lock().unwrap() = Some(Arc::clone(&active));
        Ok(active)
    }

    pub fn warm_connection(&self) -> Result<(), String> {
        self.ensure_connected().map(|_| ())
    }

    fn ensure_connected(&self) -> Result<Arc<ActiveConn>, String> {
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

        if let Ok(stream) = UnixStream::connect(&self.sock_path) {
            return self.attach_stream(stream);
        }

        if !self.auto_spawn {
            return Err(format!(
                "Unable to connect to relay socket {}",
                self.sock_path.display()
            ));
        }

        self.spawn_relay()?;
        let deadline = Instant::now() + Duration::from_millis(2500);
        let mut last_err = String::from("Failed to start local CDP relay daemon");
        let mut attempt: u32 = 0;
        while Instant::now() < deadline {
            let sleep_ms = if attempt < 12 { 10 } else { 25 };
            attempt += 1;
            thread::sleep(Duration::from_millis(sleep_ms));
            match UnixStream::connect(&self.sock_path) {
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
                self.pending.lock().unwrap().insert(id, tx);
            }

            let payload = format!(
                "{}\n",
                json!({ "id": id, "method": method, "params": params })
            );
            let write_res = {
                let mut w = active.writer.lock().unwrap();
                let _ = w.set_write_timeout(Some(Duration::from_secs(5)));
                w.write_all(payload.as_bytes()).and_then(|_| w.flush())
            };
            if let Err(e) = write_res {
                active.open.store(false, Ordering::SeqCst);
                self.pending.lock().unwrap().remove(&id);
                // Safe single retry on stale/restarted socket before request was delivered.
                if attempt == 0 {
                    continue;
                }
                return Err(format!("Local CDP relay connection closed: {e}"));
            }

            return match rx.recv_timeout(timeout) {
                Ok(res) => res,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.pending.lock().unwrap().remove(&id);
                    Err(format!("Relay timeout on {method}"))
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.pending.lock().unwrap().remove(&id);
                    Err("Local CDP relay connection closed".to_string())
                }
            };
        }
        Err("Local CDP relay connection closed".to_string())
    }
}
