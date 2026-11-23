use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

const MAX_PORT_FILE_BYTES: u64 = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortFileStatus {
    pub exists: bool,
    pub valid: bool,
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
            "port": self.port,
            "updatedAt": self.updated_at,
        })
    }
}

pub fn read_port_file(path: &Path) -> PortFileStatus {
    let path_str = path.to_string_lossy().to_string();
    let Ok(meta) = fs::metadata(path) else {
        return PortFileStatus {
            exists: false,
            valid: false,
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
    let valid = ws_path.starts_with('/')
        && candidate
            .as_deref()
            .map(|u| validate_loopback_ws_url(u).is_ok())
            .unwrap_or(false);

    PortFileStatus {
        exists: true,
        valid,
        port: if valid { port } else { None },
        ws_url: if valid { candidate } else { None },
        updated_at,
        path: path_str,
    }
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

pub fn resolve_ws_url(port_file: &Path, debug_port: u16) -> Option<String> {
    let status = read_port_file(port_file);
    if status.exists && status.valid {
        if let Some(url) = status.ws_url {
            return Some(url);
        }
    }
    probe_debug_port_ws_url(debug_port)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parses_port_file_safely_and_redacts_public_summary() {
        let dir = std::env::temp_dir().join(format!("chrome-stream-port-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let p = dir.join("DevToolsActivePort");
        fs::write(&p, "9222\n/devtools/browser/test-id\n").unwrap();
        let st = read_port_file(&p);
        assert!(st.exists && st.valid);
        assert_eq!(st.port, Some(9222));
        assert_eq!(
            st.ws_url.as_deref(),
            Some("ws://127.0.0.1:9222/devtools/browser/test-id")
        );
        let summary = st.public_summary();
        assert_eq!(summary["exists"], true);
        assert_eq!(summary["valid"], true);
        assert_eq!(summary["port"], 9222);
        assert!(summary.get("path").is_none());
        assert!(summary.get("wsUrl").is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
