//! Just enough HTTP/1.1 for collaboration's listener (protocol.md §8.1):
//! request heads parsed with `httparse`, bodies by `Content-Length` or
//! chunked, responses with a length or chunked as their body is read (so
//! an SSE stream goes out event by event), keep-alive.
//!
//! Why not `tiny_http` (the spec's candidate, to be confirmed in C1a): it
//! accepts and parses connections on its own threads, so a request is
//! read before the backend can run `whois` on the connection, and a
//! request dropped unanswered gets a `500`. The backend must authenticate
//! a connection before reading a byte and close it without one when that
//! fails (§15.4), so it owns the socket and uses this instead.

use std::io::{self, BufRead, Read, Write};

use crate::net::{http, Body, Request, Response};

/// Why a request couldn't be read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReadError {
    /// The peer went away, or was silent too long.
    Closed,
    /// The head or the body is over its limit.
    TooLarge,
    /// Not HTTP/1.x as we read it.
    Bad,
}

impl From<io::Error> for ReadError {
    fn from(_: io::Error) -> ReadError {
        ReadError::Closed
    }
}

/// A parsed request line and headers.
pub(crate) struct Head {
    pub method: String,
    pub target: String,
    /// The `1` of `HTTP/1.1`.
    pub minor: u8,
    pub headers: Vec<(String, Vec<u8>)>,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&[u8]> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_slice())
    }

    fn header_has(&self, name: &str, token: &str) -> bool {
        self.headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case(name)).any(|(_, v)| {
            String::from_utf8_lossy(v).split(',').any(|t| t.trim().eq_ignore_ascii_case(token))
        })
    }

    /// Whether the connection stays open after this request.
    pub fn keep_alive(&self) -> bool {
        if self.minor == 0 {
            self.header_has("connection", "keep-alive")
        } else {
            !self.header_has("connection", "close")
        }
    }
}

const MAX_HEADERS: usize = 64;

/// The next request's head; `None` when the peer closed between requests.
pub(crate) fn read_head(r: &mut impl BufRead, max: usize) -> Result<Option<Head>, ReadError> {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let avail = r.fill_buf()?;
        if avail.is_empty() {
            return if buf.is_empty() { Ok(None) } else { Err(ReadError::Closed) };
        }
        let from = buf.len().saturating_sub(3);
        let n = avail.len();
        buf.extend_from_slice(avail);
        if let Some(at) = find(&buf[from..], b"\r\n\r\n") {
            let end = from + at + 4;
            r.consume(n - (buf.len() - end));
            buf.truncate(end);
            break;
        }
        r.consume(n);
        if buf.len() > max {
            return Err(ReadError::TooLarge);
        }
    }
    if buf.len() > max {
        return Err(ReadError::TooLarge);
    }
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut req = httparse::Request::new(&mut headers);
    match req.parse(&buf) {
        Ok(httparse::Status::Complete(_)) => {}
        Err(httparse::Error::TooManyHeaders) => return Err(ReadError::TooLarge),
        _ => return Err(ReadError::Bad),
    }
    Ok(Some(Head {
        method: req.method.unwrap_or_default().to_string(),
        target: req.path.unwrap_or_default().to_string(),
        minor: req.version.unwrap_or(1),
        headers: req.headers.iter().map(|h| (h.name.to_string(), h.value.to_vec())).collect(),
    }))
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The body that follows `head`, at most `max` bytes. Answers
/// `Expect: 100-continue` first.
pub(crate) fn read_body(head: &Head, r: &mut impl BufRead, w: &mut impl Write, max: usize) -> Result<Vec<u8>, ReadError> {
    let chunked = head.header_has("transfer-encoding", "chunked");
    let length = match head.header("content-length") {
        Some(v) => Some(std::str::from_utf8(v).ok().and_then(|s| s.trim().parse::<u64>().ok()).ok_or(ReadError::Bad)?),
        None => None,
    };
    if chunked && length.is_some() {
        return Err(ReadError::Bad);
    }
    if length.is_some_and(|n| n > max as u64) {
        return Err(ReadError::TooLarge);
    }
    if (chunked || length.is_some_and(|n| n > 0)) && head.header_has("expect", "100-continue") {
        w.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
        w.flush()?;
    }
    if chunked {
        return read_chunked(r, max);
    }
    let n = length.unwrap_or(0) as usize;
    let mut body = vec![0; n];
    r.read_exact(&mut body)?;
    Ok(body)
}

fn read_line(r: &mut impl BufRead, max: usize) -> Result<String, ReadError> {
    let mut line = Vec::new();
    r.take(max as u64 + 2).read_until(b'\n', &mut line)?;
    if !line.ends_with(b"\n") {
        return Err(if line.len() > max { ReadError::TooLarge } else { ReadError::Closed });
    }
    String::from_utf8(line).map(|l| l.trim_end().to_string()).map_err(|_| ReadError::Bad)
}

fn read_chunked(r: &mut impl BufRead, max: usize) -> Result<Vec<u8>, ReadError> {
    let mut body = Vec::new();
    loop {
        let line = read_line(r, 1024)?;
        let size = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size, 16).map_err(|_| ReadError::Bad)?;
        if size == 0 {
            // Trailers, up to the empty line.
            while !read_line(r, 8 * 1024)?.is_empty() {}
            return Ok(body);
        }
        if body.len() + size > max {
            return Err(ReadError::TooLarge);
        }
        let at = body.len();
        body.resize(at + size, 0);
        r.read_exact(&mut body[at..])?;
        if !read_line(r, 0)?.is_empty() {
            return Err(ReadError::Bad);
        }
    }
}

/// The `http` request for a head and its body.
pub(crate) fn request(head: &Head, body: Vec<u8>) -> Result<Request, ReadError> {
    let mut builder = http::Request::builder()
        .method(head.method.as_str())
        .uri(head.target.as_str())
        .version(if head.minor == 0 { http::Version::HTTP_10 } else { http::Version::HTTP_11 });
    for (name, value) in &head.headers {
        builder = builder.header(name.as_str(), value.as_slice());
    }
    let body = if body.is_empty() { Body::Empty } else { Body::Bytes(body) };
    builder.body(body).map_err(|_| ReadError::Bad)
}

/// Headers the connection, not the handler, decides.
const HOP: &[&str] = &["connection", "content-length", "transfer-encoding", "keep-alive", "upgrade"];

/// Write `res`. A streaming body is sent chunked as it is read; an error
/// reading it ends the connection mid-stream (the client sees it cut).
pub(crate) fn write_response(w: &mut impl Write, res: Response, keep_alive: bool, head_only: bool) -> io::Result<()> {
    let (parts, body) = res.into_parts();
    let mut out = Vec::with_capacity(256);
    let reason = parts.status.canonical_reason().unwrap_or("");
    write!(out, "HTTP/1.1 {} {reason}\r\n", parts.status.as_u16())?;
    for (name, value) in &parts.headers {
        if HOP.contains(&name.as_str()) {
            continue;
        }
        out.extend_from_slice(name.as_str().as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(value.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    if !keep_alive {
        out.extend_from_slice(b"Connection: close\r\n");
    }
    let no_body = parts.status.is_informational() || parts.status == http::StatusCode::NO_CONTENT
        || parts.status == http::StatusCode::NOT_MODIFIED;
    match body {
        Body::Stream(mut reader) if !head_only && !no_body => {
            out.extend_from_slice(b"Transfer-Encoding: chunked\r\n\r\n");
            w.write_all(&out)?;
            w.flush()?;
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                write!(w, "{n:x}\r\n")?;
                w.write_all(&buf[..n])?;
                w.write_all(b"\r\n")?;
                w.flush()?;
            }
            w.write_all(b"0\r\n\r\n")?;
        }
        Body::Stream(_) => {
            out.extend_from_slice(b"\r\n");
            w.write_all(&out)?;
        }
        Body::Bytes(bytes) => {
            if !no_body {
                write!(out, "Content-Length: {}\r\n", bytes.len())?;
            }
            out.extend_from_slice(b"\r\n");
            if !head_only && !no_body {
                out.extend_from_slice(&bytes);
            }
            w.write_all(&out)?;
        }
        Body::Empty => {
            if !no_body {
                out.extend_from_slice(b"Content-Length: 0\r\n");
            }
            out.extend_from_slice(b"\r\n");
            w.write_all(&out)?;
        }
    }
    w.flush()
}

/// A bodiless answer from the listener itself (limits), closing the
/// connection.
pub(crate) fn status_only(code: u16, message: &str) -> Response {
    let mut res = Response::new(Body::from(message.to_string()));
    *res.status_mut() = http::StatusCode::from_u16(code).unwrap_or(http::StatusCode::BAD_REQUEST);
    res.headers_mut().insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("text/plain; charset=utf-8"));
    res
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::*;

    fn parse(raw: &str, max_body: usize) -> Result<(Head, Vec<u8>), ReadError> {
        let mut r = BufReader::new(Cursor::new(raw.as_bytes().to_vec()));
        let head = read_head(&mut r, 1024)?.ok_or(ReadError::Closed)?;
        let body = read_body(&head, &mut r, &mut Vec::new(), max_body)?;
        Ok((head, body))
    }

    #[test]
    fn reads_heads_and_bodies() {
        let (head, body) = parse("POST /mcp HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\nabcEXTRA", 10).unwrap();
        assert_eq!((head.method.as_str(), head.target.as_str(), head.minor), ("POST", "/mcp", 1));
        assert_eq!(body, b"abc");
        assert!(head.keep_alive());

        let raw = "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3;x=y\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        let (head, body) = parse(raw, 10).unwrap();
        assert_eq!(body, b"abcde");
        assert!(!head.keep_alive());

        assert!(!parse("GET / HTTP/1.0\r\n\r\n", 10).unwrap().0.keep_alive());
    }

    #[test]
    fn refuses_what_is_too_large_or_not_http() {
        assert_eq!(parse("POST / HTTP/1.1\r\nContent-Length: 11\r\n\r\n", 10).err(), Some(ReadError::TooLarge));
        let raw = "POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n8\r\n12345678\r\n8\r\n12345678\r\n0\r\n\r\n";
        assert_eq!(parse(raw, 10).err(), Some(ReadError::TooLarge));
        assert_eq!(parse(&format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(2000)), 10).err(), Some(ReadError::TooLarge));
        assert_eq!(parse("HELLO\r\n\r\n", 10).err(), Some(ReadError::Bad));
        assert_eq!(parse("POST / HTTP/1.1\r\nContent-Length: x\r\n\r\n", 10).err(), Some(ReadError::Bad));
        let mut empty = BufReader::new(Cursor::new(Vec::new()));
        assert!(read_head(&mut empty, 1024).unwrap().is_none());
    }

    #[test]
    fn writes_lengths_and_chunks() {
        let mut out = Vec::new();
        let mut res = Response::new(Body::from("hi"));
        res.headers_mut().insert("content-length", http::HeaderValue::from_static("99"));
        write_response(&mut out, res, false, false).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nhi");

        let (tx, body) = Body::channel();
        tx.send("data: 1\n\n");
        tx.send("data: 2\n\n");
        drop(tx);
        let mut out = Vec::new();
        write_response(&mut out, Response::new(body), true, false).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n9\r\ndata: 1\n\n\r\n9\r\ndata: 2\n\n\r\n0\r\n\r\n"
        );
    }
}
