use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_PORT_FILE_BYTES: u64 = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortFileStatus {
    pub exists: bool,
    pub valid: bool,
    #[serde(default)]
    pub stale: bool,
    pub port: Option<u16>,
    pub ws_url: Option<String>,
    pub updated_at: Option<String>,
    pub path: String,
}

impl PortFileStatus {
    /// Privacy-safe summary for MCP responses and SSR boot payloads:
    /// omits local filesystem paths and raw DevTools browser WebSocket tokens.
    pub fn public_summary(&self) -> Value {
        json!({
            "exists": self.exists,
            "valid": self.valid,
            "stale": self.stale,
            "port": self.port,
            "updatedAt": self.updated_at,
        })
    }
}

/// Fast loopback TCP liveness check (`127.0.0.1:<port>` then `[::1]:<port>`).
/// Prevents stale `DevToolsActivePort` files left behind after Chrome exits from
/// reporting false readiness or blocking fallback to `127.0.0.1:9222`.
pub fn is_loopback_port_alive(port: u16) -> bool {
    if port == 0 {
        return false;
    }
    let v4 = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    if TcpStream::connect_timeout(&v4, Duration::from_millis(120)).is_ok() {
        return true;
    }
    let v6 = SocketAddr::from((Ipv6Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&v6, Duration::from_millis(80)).is_ok()
}

pub fn read_port_file(path: &Path) -> PortFileStatus {
    let path_str = path.to_string_lossy().to_string();
    let Ok(meta) = fs::metadata(path) else {
        return PortFileStatus {
            exists: false,
            valid: false,
            stale: false,
            port: None,
            ws_url: None,
            updated_at: None,
            path: path_str,
        };
    };

    let updated_at = meta
        .modified()
        .ok()
        .map(|t| DateTime::<Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Millis, true));

    let Ok(file) = File::open(path) else {
        return PortFileStatus {
            exists: true,
            valid: false,
            stale: false,
            port: None,
            ws_url: None,
            updated_at,
            path: path_str,
        };
    };
    let mut raw = String::new();
    if file.take(MAX_PORT_FILE_BYTES).read_to_string(&mut raw).is_err() {
        return PortFileStatus {
            exists: true,
            valid: false,
            stale: false,
            port: None,
            ws_url: None,
            updated_at,
            path: path_str,
        };
    }

    let lines: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let port = lines.first().and_then(|s| s.parse::<u16>().ok()).filter(|&p| p > 0);
    let ws_path = lines.get(1).copied().unwrap_or("");
    let candidate = port.map(|p| format!("ws://127.0.0.1:{p}{ws_path}"));
    let syntax_valid = ws_path.starts_with('/')
        && candidate
            .as_deref()
            .map(|u| validate_loopback_ws_url(u).is_ok())
            .unwrap_or(false);

    let alive = syntax_valid && port.map(is_loopback_port_alive).unwrap_or(false);
    let stale = syntax_valid && !alive;

    PortFileStatus {
        exists: true,
        valid: alive,
        stale,
        port: if syntax_valid { port } else { None },
        ws_url: if alive { candidate } else { None },
        updated_at,
        path: path_str,
    }
}

/// Scans candidate `DevToolsActivePort` locations in priority order and returns the first live
/// Chrome instance (or the first existing/stale status if none is listening).
pub fn discover_port_file(candidates: &[PathBuf]) -> PortFileStatus {
    let mut first_existing: Option<PortFileStatus> = None;
    let mut first_status: Option<PortFileStatus> = None;

    for path in candidates {
        let st = read_port_file(path);
        if st.valid {
            return st;
        }
        if st.exists && first_existing.is_none() {
            first_existing = Some(st.clone());
        }
        if first_status.is_none() {
            first_status = Some(st);
        }
    }

    first_existing
        .or(first_status)
        .unwrap_or_else(|| read_port_file(Path::new("DevToolsActivePort")))
}

/// Strictly validates that a CDP WebSocket URL targets loopback only (SSRF guard).
pub fn validate_loopback_ws_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url
        .strip_prefix("ws://")
        .ok_or_else(|| format!("Only loopback ws:// CDP URLs are allowed: {url}"))?;
    let (authority, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };

    let (host, port_str) = if authority.starts_with('[') {
        let end = authority
            .find(']')
            .ok_or_else(|| format!("Invalid IPv6 authority in URL: {url}"))?;
        let host = &authority[1..end];
        let port_part = authority[end + 1..]
            .strip_prefix(':')
            .ok_or_else(|| format!("Missing port in URL: {url}"))?;
        (host, port_part)
    } else {
        authority
            .rsplit_once(':')
            .ok_or_else(|| format!("Missing port in URL: {url}"))?
    };

    let is_loopback = host == "127.0.0.1" || host.eq_ignore_ascii_case("localhost") || host == "::1";
    if !is_loopback {
        return Err(format!("Refusing non-loopback CDP host: {host}"));
    }

    let port = port_str
        .parse::<u16>()
        .ok()
        .filter(|&p| p > 0)
        .ok_or_else(|| format!("Invalid CDP port: {port_str}"))?;

    if path
        .chars()
        .any(|c| c.is_ascii_control() || c.is_whitespace())
    {
        return Err("Invalid characters in CDP WebSocket path".to_string());
    }

    let connect_host = if host == "::1" { "::1" } else { "127.0.0.1" };
    Ok((connect_host.to_string(), port, path.to_string()))
}

/// Fallback discovery against a dedicated `--remote-debugging-port=<port>` Chrome instance.
/// Tries IPv4 loopback first and falls back to IPv6 loopback for dual-stack hygiene.
pub fn probe_debug_port_ws_url(port: u16) -> Option<String> {
    if port == 0 {
        return None;
    }
    for (host, addr_str) in [
        ("127.0.0.1", format!("127.0.0.1:{port}")),
        ("[::1]", format!("[::1]:{port}")),
    ] {
        let Ok(addr) = addr_str.parse::<SocketAddr>() else {
            continue;
        };
        let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(250)) else {
            continue;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(300)));

        let req = format!(
            "GET /json/version HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nAccept: application/json\r\n\r\n"
        );
        if stream.write_all(req.as_bytes()).is_err() {
            continue;
        }

        let mut resp = Vec::with_capacity(4096);
        let mut chunk = [0u8; 2048];
        while resp.len() < 32_768 {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => resp.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }
        let text = String::from_utf8_lossy(&resp);
        let Some((headers, body)) = text.split_once("\r\n\r\n") else {
            continue;
        };
        if !headers.starts_with("HTTP/1.1 200") && !headers.starts_with("HTTP/1.0 200") {
            continue;
        }
        let Ok(json) = serde_json::from_str::<Value>(body.trim()) else {
            continue;
        };
        let Some(ws_url) = json.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) else {
            continue;
        };
        if validate_loopback_ws_url(ws_url).is_ok() {
            return Some(ws_url.to_string());
        }
    }
    None
}

pub fn resolve_ws_url_candidates(candidates: &[PathBuf], debug_port: u16) -> Option<String> {
    let status = discover_port_file(candidates);
    if status.exists && status.valid {
        if let Some(url) = status.ws_url {
            return Some(url);
        }
    }
    probe_debug_port_ws_url(debug_port)
}

#[cfg(test)]
pub fn resolve_ws_url(port_file: &Path, debug_port: u16) -> Option<String> {
    resolve_ws_url_candidates(&[port_file.to_path_buf()], debug_port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn validates_only_loopback_ws_urls() {
        assert_eq!(
            validate_loopback_ws_url("ws://127.0.0.1:9222/devtools/browser/abc").unwrap(),
            ("127.0.0.1".into(), 9222, "/devtools/browser/abc".into())
        );
        assert_eq!(
            validate_loopback_ws_url("ws://localhost:9222/devtools/browser/abc").unwrap(),
            ("127.0.0.1".into(), 9222, "/devtools/browser/abc".into())
        );
        assert_eq!(
            validate_loopback_ws_url("ws://[::1]:9222/devtools/browser/abc").unwrap(),
            ("::1".into(), 9222, "/devtools/browser/abc".into())
        );
        // Reject remote / private non-loopback targets (SSRF) & header injection
        assert!(validate_loopback_ws_url("ws://169.254.169.254:80/latest").is_err());
        assert!(validate_loopback_ws_url("ws://10.0.0.5:9222/devtools/browser/x").is_err());
        assert!(validate_loopback_ws_url("ws://[::ffff:169.254.169.254]:80/x").is_err());
        assert!(validate_loopback_ws_url("ws://evil.com@127.0.0.1:9222/x").is_err());
        assert!(validate_loopback_ws_url("wss://example.com:443/ws").is_err());
        assert!(validate_loopback_ws_url("ws://127.0.0.1:9222/a\r\nX-Injected: 1").is_err());
        assert!(validate_loopback_ws_url("ws://127.0.0.1:9222/a\t/b").is_err());
    }

    #[test]
    fn parses_port_file_probes_liveness_and_falls_back_when_stale() {
        let dir = std::env::temp_dir().join(format!("chrome-stream-port-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let live_pf = dir.join("DevToolsActivePort");

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let live_port = listener.local_addr().unwrap().port();
        fs::write(&live_pf, format!("{live_port}\n/devtools/browser/test-id\n")).unwrap();

        let st = read_port_file(&live_pf);
        assert!(st.exists && st.valid && !st.stale);
        assert_eq!(st.port, Some(live_port));
        assert_eq!(
            st.ws_url.as_deref(),
            Some(format!("ws://127.0.0.1:{live_port}/devtools/browser/test-id").as_str())
        );
        let summary = st.public_summary();
        assert_eq!(summary["exists"], true);
        assert_eq!(summary["valid"], true);
        assert_eq!(summary["stale"], false);
        assert_eq!(summary["port"], live_port);
        assert!(summary.get("path").is_none());
        assert!(summary.get("wsUrl").is_none());

        // Drop listener -> port becomes dead/stale!
        drop(listener);
        let stale_st = read_port_file(&live_pf);
        assert!(stale_st.exists);
        assert!(!stale_st.valid, "stale DevToolsActivePort must NOT report valid=true");
        assert!(stale_st.stale);
        assert!(stale_st.ws_url.is_none());

        // Spin up a mock :debug_port `/json/version` server and verify stale port file falls back!
        let fb_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let fb_port = fb_listener.local_addr().unwrap().port();
        let expected_fb_ws = format!("ws://127.0.0.1:{fb_port}/devtools/browser/fallback-ok");
        let expected_clone = expected_fb_ws.clone();
        let srv = thread::spawn(move || {
            if let Ok((mut conn, _)) = fb_listener.accept() {
                let mut buf = [0u8; 512];
                let _ = conn.read(&mut buf);
                let body = format!(r#"{{"webSocketDebuggerUrl":"{expected_clone}"}}"#);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = conn.write_all(resp.as_bytes());
            }
        });

        let resolved = resolve_ws_url(&live_pf, fb_port);
        assert_eq!(resolved.as_deref(), Some(expected_fb_ws.as_str()));
        let _ = srv.join();

        // Multi-path scan skips stale first candidate and picks live second candidate!
        let live2_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let live2_port = live2_listener.local_addr().unwrap().port();
        let snap_pf = dir.join("SnapDevToolsActivePort");
        fs::write(&snap_pf, format!("{live2_port}\n/devtools/browser/snap-id\n")).unwrap();
        let multi = discover_port_file(&[live_pf.clone(), snap_pf]);
        assert!(multi.valid && !multi.stale);
        assert_eq!(multi.port, Some(live2_port));
        drop(live2_listener);

        let _ = fs::remove_dir_all(&dir);
    }
}
