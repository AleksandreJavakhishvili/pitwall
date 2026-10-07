//! Framing (architecture.md §4): one connection carries everything.
//!
//! ```text
//! frame  := len:u32be  type:u8  body        (len counts type + body)
//! type 0 := JSON message (UTF-8)
//! type 1 := terminal data: stream:u32be  bytes…   (both directions)
//! ```

use std::io::{self, Read, Write};

use serde::Serialize;

pub const JSON: u8 = 0;
pub const TERM: u8 = 1;
/// Larger frames are a protocol error (a broken or hostile peer).
pub const MAX_FRAME: u32 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// A JSON message, still undecoded.
    Json(Vec<u8>),
    /// Terminal bytes on a stream opened by `term.attach`.
    Term { stream: u32, data: Vec<u8> },
    /// A frame type this side doesn't know (ignored: additive changes).
    Unknown(u8),
}

fn frame(ty: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + body.len());
    out.extend_from_slice(&(body.len() as u32 + 1).to_be_bytes());
    out.push(ty);
    out.extend_from_slice(body);
    out
}

/// A JSON message frame.
pub fn json<T: Serialize>(msg: &T) -> Vec<u8> {
    frame(JSON, &serde_json::to_vec(msg).expect("wire types serialize"))
}

/// A terminal data frame.
pub fn term(stream: u32, data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(&stream.to_be_bytes());
    body.extend_from_slice(data);
    frame(TERM, &body)
}

pub fn write_json<T: Serialize>(w: &mut impl Write, msg: &T) -> io::Result<()> {
    w.write_all(&json(msg))?;
    w.flush()
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// The next frame; `None` at a clean end of stream (between frames).
pub fn read(r: &mut impl Read) -> io::Result<Option<Frame>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len);
    if len == 0 || len > MAX_FRAME {
        return Err(bad("bad frame length"));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    let body = buf.split_off(1);
    Ok(Some(match buf[0] {
        JSON => Frame::Json(body),
        TERM => {
            if body.len() < 4 {
                return Err(bad("short terminal frame"));
            }
            let stream = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
            Frame::Term { stream, data: body[4..].to_vec() }
        }
        other => Frame::Unknown(other),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let mut wire = json(&serde_json::json!({"id": 1}));
        wire.extend(term(3, b"\x1b[0mhi"));
        wire.extend(frame(9, b"future"));
        let mut r = &wire[..];
        assert_eq!(read(&mut r).unwrap(), Some(Frame::Json(br#"{"id":1}"#.to_vec())));
        assert_eq!(read(&mut r).unwrap(), Some(Frame::Term { stream: 3, data: b"\x1b[0mhi".to_vec() }));
        assert_eq!(read(&mut r).unwrap(), Some(Frame::Unknown(9)));
        assert_eq!(read(&mut r).unwrap(), None);
    }

    #[test]
    fn the_length_prefix_is_big_endian_and_counts_the_type() {
        assert_eq!(json(&1), vec![0, 0, 0, 2, JSON, b'1']);
    }

    #[test]
    fn broken_frames_are_errors() {
        assert!(read(&mut &[0u8, 0, 0, 0][..]).is_err(), "zero length");
        assert!(read(&mut &[0xffu8, 0, 0, 0][..]).is_err(), "too long");
        assert!(read(&mut &[0u8, 0, 0, 3, TERM, 0, 0][..]).is_err(), "short terminal frame");
        assert!(read(&mut &[0u8, 0, 0, 5, JSON, b'{'][..]).is_err(), "truncated body");
    }
}
