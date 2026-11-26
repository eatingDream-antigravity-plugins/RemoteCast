use crate::cdp::discovery::validate_loopback_ws_url;
use crate::stream::dedup::{base64_encode, sha1};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_WS_MESSAGE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone)]
pub enum TransportEvent {
    Text(String),
    Closed,
}

pub type EventCallback = Arc<dyn Fn(TransportEvent) + Send + Sync>;

pub trait CdpTransport: Send + Sync {
    fn send_text(&self, text: &str) -> Result<(), String>;
    fn is_open(&self) -> bool;
    fn close(&self);
}

pub type TransportFactory = Arc<
    dyn Fn(&str, Duration, EventCallback) -> Result<Arc<dyn CdpTransport>, String> + Send + Sync,
>;

pub fn default_ws_factory() -> TransportFactory {
    Arc::new(|url, timeout, on_event| {
        let ws = WsTransport::connect(url, timeout, on_event)?;
        Ok(ws as Arc<dyn CdpTransport>)
    })
}

pub struct WsTransport {
    writer: Arc<Mutex<TcpStream>>,
    open: Arc<AtomicBool>,
}

impl WsTransport {
    pub fn connect(
        ws_url: &str,
        handshake_timeout: Duration,
        on_event: EventCallback,
    ) -> Result<Arc<Self>, String> {
        let (host, port, path) = validate_loopback_ws_url(ws_url)?;
        let addr_str = if host == "::1" {
            format!("[::1]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let addr: SocketAddr = addr_str
            .parse()
            .map_err(|e| format!("Invalid loopback address {addr_str}: {e}"))?;

        let mut stream =
            TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| {
                format!("Unable to connect to Chrome DevTools WebSocket ({addr_str}): {e}")
            })?;
        let _ = stream.set_nodelay(true);
        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
        // Chrome holds the HTTP upgrade response open while waiting for the user's "Allow" click.
        let _ = stream.set_read_timeout(Some(handshake_timeout));

        let mut key_bytes = [0u8; 16];
        getrandom::fill(&mut key_bytes)
            .map_err(|e| format!("Failed generating WebSocket key: {e}"))?;
        let sec_key = base64_encode(&key_bytes);

        let host_header = if host == "::1" {
            format!("[::1]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let req = format!(
            "GET {path} HTTP/1.1\r\n\
             Host: {host_header}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: {sec_key}\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        );
        stream
            .write_all(req.as_bytes())
            .map_err(|e| format!("WebSocket upgrade write failed: {e}"))?;
        stream
            .flush()
            .map_err(|e| format!("WebSocket upgrade flush failed: {e}"))?;

        let (headers, residual) = read_http_response_headers(&mut stream).map_err(|e| {
            if e.contains("timed out") || e.contains("Resource temporarily unavailable") {
                format!(
                    "Timed out waiting for Chrome DevTools handshake ({}s). If Chrome displayed an Allow remote debugging dialog, click Allow and retry.",
                    handshake_timeout.as_secs()
                )
            } else {
                format!("WebSocket handshake failed: {e}")
            }
        })?;

        verify_handshake_response(&headers, &sec_key)?;

        // Clear read timeout for persistent idle WebSocket keepalive.
        let _ = stream.set_read_timeout(None);
        let reader_stream = stream
            .try_clone()
            .map_err(|e| format!("Failed cloning WebSocket TCP stream: {e}"))?;

        let writer = Arc::new(Mutex::new(stream));
        let open = Arc::new(AtomicBool::new(true));

        let reader_writer = Arc::clone(&writer);
        let reader_open = Arc::clone(&open);
        thread::Builder::new()
            .name("chrome-stream-ws".to_string())
            .spawn(move || {
                run_reader_loop(
                    reader_stream,
                    residual,
                    reader_writer,
                    reader_open,
                    on_event,
                );
            })
            .map_err(|e| format!("Failed spawning WebSocket reader thread: {e}"))?;

        Ok(Arc::new(Self { writer, open }))
    }
}

impl CdpTransport for WsTransport {
    fn send_text(&self, text: &str) -> Result<(), String> {
        if !self.is_open() {
            return Err("Chrome CDP connection closed".to_string());
        }
        let mut guard = self
            .writer
            .lock()
            .map_err(|_| "WebSocket writer lock poisoned".to_string())?;
        if let Err(e) = write_client_frame(&mut *guard, 0x1, text.as_bytes()) {
            self.open.store(false, Ordering::SeqCst);
            return Err(format!("Chrome CDP connection closed: {e}"));
        }
        Ok(())
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }

    fn close(&self) {
        if self.open.swap(false, Ordering::SeqCst) {
            if let Ok(mut guard) = self.writer.lock() {
                let _ = write_client_frame(&mut *guard, 0x8, &[]);
                let _ = guard.shutdown(Shutdown::Both);
            }
        }
    }
}

fn read_http_response_headers(stream: &mut TcpStream) -> Result<(String, Vec<u8>), String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("EOF during WebSocket handshake".to_string());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_crlf_crlf(&buf) {
            let headers = String::from_utf8_lossy(&buf[..pos]).to_string();
            let residual = buf[pos + 4..].to_vec();
            return Ok((headers, residual));
        }
        if buf.len() > 16_384 {
            return Err("WebSocket handshake header exceeded 16 KiB".to_string());
        }
    }
}

fn find_crlf_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn verify_handshake_response(headers: &str, sec_key: &str) -> Result<(), String> {
    let mut lines = headers.lines();
    let status = lines.next().unwrap_or("");
    if !status.contains(" 101 ") && !status.ends_with(" 101") {
        return Err(format!(
            "Expected HTTP 101 Switching Protocols, got: {status}"
        ));
    }

    let expected_accept = expected_ws_accept(sec_key);
    let mut got_accept = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("sec-websocket-accept") {
                got_accept = Some(v.trim().to_string());
            }
        }
    }
    match got_accept {
        Some(actual) if actual == expected_accept => Ok(()),
        Some(actual) => Err(format!(
            "Sec-WebSocket-Accept mismatch: expected {expected_accept}, got {actual}"
        )),
        None => Err("Missing Sec-WebSocket-Accept header in handshake response".to_string()),
    }
}

pub fn expected_ws_accept(sec_key: &str) -> String {
    let mut input = Vec::with_capacity(sec_key.len() + WS_GUID.len());
    input.extend_from_slice(sec_key.as_bytes());
    input.extend_from_slice(WS_GUID.as_bytes());
    base64_encode(&sha1(&input))
}

fn write_client_frame<W: Write>(writer: &mut W, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut mask = [0u8; 4];
    getrandom::fill(&mut mask).map_err(|e| std::io::Error::other(format!("CSPRNG failed: {e}")))?;

    let len = payload.len();
    let mut header = Vec::with_capacity(14);
    header.push(0x80 | (opcode & 0x0F)); // FIN=1
    if len < 126 {
        header.push(0x80 | (len as u8));
    } else if len <= 65_535 {
        header.push(0x80 | 126);
        header.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        header.push(0x80 | 127);
        header.extend_from_slice(&(len as u64).to_be_bytes());
    }
    header.extend_from_slice(&mask);

    let mut masked = Vec::with_capacity(len);
    for (i, &b) in payload.iter().enumerate() {
        masked.push(b ^ mask[i & 3]);
    }
    writer.write_all(&header)?;
    writer.write_all(&masked)?;
    writer.flush()
}

struct PrefixedReader<R: Read> {
    prefix: Vec<u8>,
    pos: usize,
    inner: R,
}

impl<R: Read> PrefixedReader<R> {
    fn new(prefix: Vec<u8>, inner: R) -> Self {
        Self {
            prefix,
            pos: 0,
            inner,
        }
    }

    fn read_exact_bytes(&mut self, mut dst: &mut [u8]) -> std::io::Result<()> {
        while !dst.is_empty() {
            if self.pos < self.prefix.len() {
                let avail = self.prefix.len() - self.pos;
                let take = avail.min(dst.len());
                dst[..take].copy_from_slice(&self.prefix[self.pos..self.pos + take]);
                self.pos += take;
                dst = &mut dst[take..];
                continue;
            }
            let n = self.inner.read(dst)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "EOF reading WebSocket frame",
                ));
            }
            dst = &mut dst[n..];
        }
        Ok(())
    }
}

struct WsFrame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

fn read_frame<R: Read>(reader: &mut PrefixedReader<R>) -> std::io::Result<WsFrame> {
    let mut head = [0u8; 2];
    reader.read_exact_bytes(&mut head)?;
    let fin = (head[0] & 0x80) != 0;
    let opcode = head[0] & 0x0F;
    let masked = (head[1] & 0x80) != 0;
    let len_code = head[1] & 0x7F;

    let payload_len: usize = match len_code {
        0..=125 => len_code as usize,
        126 => {
            let mut ext = [0u8; 2];
            reader.read_exact_bytes(&mut ext)?;
            u16::from_be_bytes(ext) as usize
        }
        _ => {
            let mut ext = [0u8; 8];
            reader.read_exact_bytes(&mut ext)?;
            let raw_len = u64::from_be_bytes(ext);
            if raw_len > MAX_WS_MESSAGE_BYTES as u64 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "WebSocket frame exceeded safety limit",
                ));
            }
            raw_len as usize
        }
    };

    if payload_len > MAX_WS_MESSAGE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "WebSocket frame exceeded safety limit",
        ));
    }

    let mask_key = if masked {
        let mut m = [0u8; 4];
        reader.read_exact_bytes(&mut m)?;
        Some(m)
    } else {
        None
    };

    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        reader.read_exact_bytes(&mut payload)?;
    }
    if let Some(m) = mask_key {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= m[i & 3];
        }
    }

    Ok(WsFrame {
        fin,
        opcode,
        payload,
    })
}

fn run_reader_loop(
    stream: TcpStream,
    residual: Vec<u8>,
    writer: Arc<Mutex<TcpStream>>,
    open: Arc<AtomicBool>,
    on_event: EventCallback,
) {
    let mut reader = PrefixedReader::new(residual, stream);
    let mut frag_buf: Vec<u8> = Vec::new();
    let mut frag_opcode: Option<u8> = None;

    while let Ok(frame) = read_frame(&mut reader) {
        match frame.opcode {
            0x8 => {
                // Close frame
                if let Ok(mut guard) = writer.lock() {
                    let _ = write_client_frame(&mut *guard, 0x8, &[]);
                }
                break;
            }
            0x9 => {
                // Ping -> reply with Pong
                if let Ok(mut guard) = writer.lock() {
                    let _ = write_client_frame(&mut *guard, 0xA, &frame.payload);
                }
            }
            0xA => {
                // Pong -> ignore
            }
            0x1 | 0x2 => {
                if frame.fin {
                    if frame.opcode == 0x1 {
                        if let Ok(text) = String::from_utf8(frame.payload) {
                            on_event(TransportEvent::Text(text));
                        }
                    }
                } else {
                    frag_opcode = Some(frame.opcode);
                    frag_buf.clear();
                    frag_buf.extend_from_slice(&frame.payload);
                }
            }
            // Continuation frame
            0x0 if frag_opcode.is_some() => {
                if frag_buf.len().saturating_add(frame.payload.len()) > MAX_WS_MESSAGE_BYTES {
                    break;
                }
                frag_buf.extend_from_slice(&frame.payload);
                if frame.fin {
                    let op = frag_opcode.take();
                    let bytes = std::mem::take(&mut frag_buf);
                    if op == Some(0x1) {
                        if let Ok(text) = String::from_utf8(bytes) {
                            on_event(TransportEvent::Text(text));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    open.store(false, Ordering::SeqCst);
    on_event(TransportEvent::Closed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;

    fn write_server_frame(stream: &mut TcpStream, fin: bool, opcode: u8, payload: &[u8]) {
        let mut head = Vec::new();
        head.push(if fin { 0x80 } else { 0x00 } | (opcode & 0x0F));
        let len = payload.len();
        if len < 126 {
            head.push(len as u8);
        } else if len <= 65_535 {
            head.push(126);
            head.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            head.push(127);
            head.extend_from_slice(&(len as u64).to_be_bytes());
        }
        stream.write_all(&head).unwrap();
        stream.write_all(payload).unwrap();
        stream.flush().unwrap();
    }

    #[test]
    fn rfc6455_loopback_handshake_fragmented_frames_and_ping_pong() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let (req, _) = read_http_response_headers(&mut sock).unwrap();
            let key = req
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    if k.trim().eq_ignore_ascii_case("sec-websocket-key") {
                        Some(v.trim().to_string())
                    } else {
                        None
                    }
                })
                .unwrap();
            let accept = expected_ws_accept(&key);
            let resp = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            );
            sock.write_all(resp.as_bytes()).unwrap();
            sock.flush().unwrap();

            // Read one masked text frame from client
            let mut pr = PrefixedReader::new(Vec::new(), sock.try_clone().unwrap());
            let f = read_frame(&mut pr).unwrap();
            assert_eq!(f.opcode, 0x1);
            assert_eq!(String::from_utf8(f.payload).unwrap(), r#"{"id":1}"#);

            // Send Ping and verify Pong comes back
            write_server_frame(&mut sock, true, 0x9, b"hi");
            let pong = read_frame(&mut pr).unwrap();
            assert_eq!(pong.opcode, 0xA);
            assert_eq!(pong.payload, b"hi");

            // Send fragmented text message in 2 frames
            write_server_frame(&mut sock, false, 0x1, b"{\"ok\":");
            write_server_frame(&mut sock, true, 0x0, b"true}");
            write_server_frame(&mut sock, true, 0x8, &[]);
        });

        let (tx, rx) = mpsc::channel();
        let ws = WsTransport::connect(
            &format!("ws://127.0.0.1:{port}/devtools/browser/test"),
            Duration::from_secs(5),
            Arc::new(move |ev| {
                let _ = tx.send(ev);
            }),
        )
        .unwrap();

        ws.send_text(r#"{"id":1}"#).unwrap();
        let mut got_text = None;
        let mut got_closed = false;
        for _ in 0..4 {
            match rx.recv_timeout(Duration::from_secs(3)).unwrap() {
                TransportEvent::Text(t) => got_text = Some(t),
                TransportEvent::Closed => {
                    got_closed = true;
                    break;
                }
            }
        }
        assert_eq!(got_text.as_deref(), Some(r#"{"ok":true}"#));
        assert!(got_closed);
        assert!(!ws.is_open());
        server.join().unwrap();
    }
}
