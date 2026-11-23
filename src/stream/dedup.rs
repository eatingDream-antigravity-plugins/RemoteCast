use crate::stream::profiles::TabInfo;

/// Pure-Rust FIPS 180-4 SHA-1 used for RFC 6455 WebSocket handshake verification
/// and fast content deduplication hashing of screencast frames & tab lists.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xEFCD_AB89;
    let mut h2: u32 = 0x98BA_DCFE;
    let mut h3: u32 = 0x1032_5476;
    let mut h4: u32 = 0xC3D2_E1F0;

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(data.len() + 72);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0u32; 80];
    for chunk in padded.chunks_exact(64) {
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDCu32),
                _ => (b ^ c ^ d, 0xCA62_C1D6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

const B64_STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64_URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn base64_encode(input: &[u8]) -> String {
    encode_b64_with(input, B64_STD, true)
}

pub fn base64url_no_pad(input: &[u8]) -> String {
    encode_b64_with(input, B64_URL, false)
}

fn encode_b64_with(input: &[u8], alphabet: &[u8; 64], pad: bool) -> String {
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        out.push(alphabet[((n >> 18) & 0x3F) as usize] as char);
        out.push(alphabet[((n >> 12) & 0x3F) as usize] as char);
        out.push(alphabet[((n >> 6) & 0x3F) as usize] as char);
        out.push(alphabet[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.push(alphabet[((n >> 18) & 0x3F) as usize] as char);
        out.push(alphabet[((n >> 12) & 0x3F) as usize] as char);
        if pad {
            out.push('=');
            out.push('=');
        }
    } else if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(alphabet[((n >> 18) & 0x3F) as usize] as char);
        out.push(alphabet[((n >> 12) & 0x3F) as usize] as char);
        out.push(alphabet[((n >> 6) & 0x3F) as usize] as char);
        if pad {
            out.push('=');
        }
    }
    out
}

pub fn hash_frame_data(data: &str) -> String {
    if data.is_empty() {
        return String::new();
    }
    let raw = if data.starts_with("data:") {
        match data.find(',') {
            Some(idx) => &data[idx + 1..],
            None => data,
        }
    } else {
        data
    };
    let digest = sha1(raw.as_bytes());
    let encoded = base64url_no_pad(&digest);
    encoded.chars().take(16).collect()
}

pub fn hash_tabs(tabs: &[TabInfo]) -> String {
    if tabs.is_empty() {
        return "empty".to_string();
    }
    let mut buf = String::with_capacity(tabs.len() * 64);
    for (idx, t) in tabs.iter().enumerate() {
        if idx > 0 {
            buf.push('\n');
        }
        buf.push_str(&t.target_id);
        buf.push('\t');
        buf.push_str(&t.title);
        buf.push('\t');
        buf.push_str(&t.url);
    }
    let digest = sha1(buf.as_bytes());
    base64url_no_pad(&digest).chars().take(12).collect()
}

pub fn is_target_gone_error(msg: &str, cdp_code: Option<i64>) -> bool {
    if cdp_code == Some(-32001) {
        return true;
    }
    let low = msg.to_ascii_lowercase();
    if low.contains("no target with given id")
        || low.contains("target closed")
        || low.contains("session with given id not found")
        || low.contains("inspected target navigated or closed")
        || low.contains("cannot find context with specified id")
        || low.contains("detached while handling command")
    {
        return true;
    }
    if low.contains("target")
        && (low.contains("not found") || low.contains("destroyed") || low.contains("closed"))
    {
        return true;
    }
    false
}

#[cfg(test)]
pub fn is_disconnected_error(msg: &str) -> bool {
    let low = msg.to_ascii_lowercase();
    low.contains("chrome cdp connection closed")
        || low.contains("relay connection closed")
        || low.contains("websocket closed")
        || low.contains("econnrefused")
        || low.contains("connection refused")
        || low.contains("devtoolsactiveport")
}

pub fn normalize_url(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return "about:blank".to_string();
    }
    let low = s.to_ascii_lowercase();
    if low.starts_with("http:")
        || low.starts_with("https:")
        || low.starts_with("chrome:")
        || low.starts_with("about:")
        || low.starts_with("file:")
    {
        return s.to_string();
    }
    if low == "localhost"
        || low.starts_with("localhost:")
        || low.starts_with("localhost/")
        || looks_like_host(s)
    {
        return format!("https://{s}");
    }
    format!("https://www.google.com/search?q={}", url_encode_query(s))
}

fn looks_like_host(s: &str) -> bool {
    if s.contains(' ') {
        return false;
    }
    let host_part = s.split(['/', '?', '#']).next().unwrap_or("");
    let host = host_part.split(':').next().unwrap_or("");
    let Some((prefix, tld)) = host.rsplit_once('.') else {
        return false;
    };
    !prefix.is_empty()
        && tld.len() >= 2
        && tld.chars().all(|c| c.is_ascii_alphabetic())
        && prefix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

fn url_encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => {
                out.push('%');
                let hi = b >> 4;
                let lo = b & 0x0F;
                out.push(hex_char(hi));
                out.push(hex_char(lo));
            }
        }
    }
    out
}

fn hex_char(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'A' + (n - 10)) as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_matches_rfc6455_and_nist_vectors() {
        // RFC 6455 Section 1.3 handshake accept vector
        let combined = b"dGhlIHNhbXBsZSBub25jZQ==258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
        assert_eq!(base64_encode(&sha1(combined)), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");

        // NIST empty string
        let empty = sha1(b"");
        let hex: String = empty.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn frame_and_tab_hashing_deduplicates_reliably() {
        let h1 = hash_frame_data("data:image/jpeg;base64,AAAA1234");
        let h2 = hash_frame_data("AAAA1234");
        let h3 = hash_frame_data("BBBB5678");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
        assert_ne!(h1, h3);

        let tabs = vec![TabInfo {
            target_id: "t1".into(),
            title: "Docs".into(),
            url: "https://example.com".into(),
        }];
        let th1 = hash_tabs(&tabs);
        let th2 = hash_tabs(&tabs);
        assert_eq!(th1, th2);
        assert_eq!(hash_tabs(&[]), "empty");
    }

    #[test]
    fn error_classification_and_url_normalization() {
        assert!(is_target_gone_error("No target with given id found", Some(-32000)));
        assert!(is_target_gone_error("Session with given id not found", Some(-32001)));
        assert!(!is_target_gone_error("Chrome CDP connection closed", None));
        assert!(is_disconnected_error("Chrome CDP connection closed"));

        assert_eq!(normalize_url("example.com/docs"), "https://example.com/docs");
        assert_eq!(normalize_url("localhost:3000"), "https://localhost:3000");
        assert_eq!(normalize_url("http://127.0.0.1:8080"), "http://127.0.0.1:8080");
        assert_eq!(
            normalize_url("rust cdp"),
            "https://www.google.com/search?q=rust%20cdp"
        );
    }
}
