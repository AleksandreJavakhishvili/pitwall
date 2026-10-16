//! Small HTTP pieces both fronts and the client share: JSON and SSE
//! responses, SSE framing and parsing, base64 (A2A `raw` parts), percent
//! encoding (turn ids carry a `/`) and query strings. No dependency beyond
//! `http` and `serde_json`.

use std::io::{BufRead, BufReader, Read};

use crate::net::http::{header, HeaderValue, StatusCode};
use crate::net::{Body, BodyWriter, Response};

pub const JSON: &str = "application/json";
pub const A2A_JSON: &str = "application/a2a+json";
pub const EVENT_STREAM: &str = "text/event-stream";
pub const PATCH: &str = "text/x-patch";

/// A JSON response with `content_type`.
pub fn json_as(status: StatusCode, content_type: &str, value: &serde_json::Value) -> Response {
    let mut res = Response::new(Body::json(value));
    *res.status_mut() = status;
    set(&mut res, header::CONTENT_TYPE.as_str(), content_type);
    res
}

pub fn json(status: StatusCode, value: &serde_json::Value) -> Response {
    json_as(status, JSON, value)
}

/// A status with no body.
pub fn empty(status: StatusCode) -> Response {
    let mut res = Response::new(Body::Empty);
    *res.status_mut() = status;
    res
}

/// A plain-text body.
pub fn text(status: StatusCode, content_type: &str, body: String) -> Response {
    let mut res = Response::new(Body::from(body));
    *res.status_mut() = status;
    set(&mut res, header::CONTENT_TYPE.as_str(), content_type);
    res
}

/// An SSE response streaming `body`: not cached, not buffered by proxies.
pub fn sse(body: Body) -> Response {
    let mut res = Response::new(body);
    set(&mut res, header::CONTENT_TYPE.as_str(), EVENT_STREAM);
    set(&mut res, header::CACHE_CONTROL.as_str(), "no-cache");
    set(&mut res, "x-accel-buffering", "no");
    res
}

pub fn set(res: &mut Response, name: &str, value: &str) {
    if let (Ok(name), Ok(value)) = (header::HeaderName::try_from(name), HeaderValue::from_str(value)) {
        res.headers_mut().insert(name, value);
    }
}

/// A request header as text, trimmed; `None` when absent or not text.
pub fn header_str<'a>(headers: &'a crate::net::http::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim)
}

/// Writes one SSE event per call. `false`: the reader is gone.
pub struct SseWriter(pub BodyWriter);

impl SseWriter {
    /// One event: an optional `event:` name and `data` (split on newlines,
    /// one `data:` line each).
    pub fn event(&self, name: Option<&str>, data: &str) -> bool {
        self.0.send(frame(name, data))
    }

    pub fn json(&self, name: Option<&str>, value: &serde_json::Value) -> bool {
        self.event(name, &value.to_string())
    }

    /// A comment line: keeps the connection alive and finds out whether
    /// the peer is still there.
    pub fn keepalive(&self) -> bool {
        self.0.send(&b":\n\n"[..])
    }
}

/// One SSE event as bytes.
pub fn frame(name: Option<&str>, data: &str) -> Vec<u8> {
    let mut out = String::new();
    if let Some(name) = name {
        out.push_str("event: ");
        out.push_str(name);
        out.push('\n');
    }
    for line in data.split('\n') {
        out.push_str("data: ");
        out.push_str(line.strip_suffix('\r').unwrap_or(line));
        out.push('\n');
    }
    out.push('\n');
    out.into_bytes()
}

/// One event read from an SSE stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// `event:`; `message` when absent.
    pub event: String,
    pub data: String,
}

/// Reads SSE events line by line (comments and `id:`/`retry:` ignored).
pub struct SseReader {
    lines: BufReader<Box<dyn Read + Send>>,
}

impl SseReader {
    pub fn new(body: Body) -> SseReader {
        SseReader { lines: BufReader::new(body.into_reader()) }
    }
}

impl Iterator for SseReader {
    type Item = std::io::Result<SseEvent>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut event = String::new();
        let mut data: Option<String> = None;
        let mut line = String::new();
        loop {
            line.clear();
            match self.lines.read_line(&mut line) {
                Err(e) => return Some(Err(e)),
                Ok(0) => {
                    // End of stream: a pending event without its blank line
                    // is dropped, as the SSE spec says.
                    return None;
                }
                Ok(_) => {}
            }
            let l = line.trim_end_matches(['\n', '\r']);
            if l.is_empty() {
                if let Some(data) = data.take() {
                    let event = if event.is_empty() { "message".to_string() } else { std::mem::take(&mut event) };
                    return Some(Ok(SseEvent { event, data }));
                }
                event.clear();
                continue;
            }
            if l.starts_with(':') {
                continue;
            }
            let (field, value) = l.split_once(':').map_or((l, ""), |(f, v)| (f, v.strip_prefix(' ').unwrap_or(v)));
            match field {
                "event" => event = value.to_string(),
                "data" => match data.as_mut() {
                    Some(d) => {
                        d.push('\n');
                        d.push_str(value);
                    }
                    None => data = Some(value.to_string()),
                },
                _ => {}
            }
        }
    }
}

// ------------------------------------------------------------- base64

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding (protobuf JSON `bytes`).
pub fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Standard or URL-safe base64, padding optional (protobuf JSON accepts
/// both); `None` when it isn't base64.
pub fn b64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace()) {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// ------------------------------------------------------ paths, queries

/// `%XX` decoded (invalid escapes kept as they are); `None` when the
/// result isn't UTF-8.
pub fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// Encoded for one path segment or query value: everything but
/// unreserved characters.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A query string's pairs, decoded (`+` is a space in queries).
pub fn query_pairs(query: Option<&str>) -> Vec<(String, String)> {
    query
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            let dec = |s: &str| percent_decode(&s.replace('+', " ")).unwrap_or_default();
            (dec(k), dec(v))
        })
        .collect()
}

pub fn query_get(query: Option<&str>, key: &str) -> Option<String> {
    query_pairs(query).into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// `1st`, `2nd`, `3rd`, `4th`, `11th`, …
pub fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}
