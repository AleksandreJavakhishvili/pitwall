//! Wire format (see the crate docs for the table). Pure: no IO except the
//! blocking `read_frame` / `write_frame` helpers over `Read` / `Write`.

use std::io::{self, Read, Write};

pub const PROTOCOL_VERSION: u16 = 1;
/// Largest `len` accepted in a frame header.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

// Client → holder.
pub const HELLO: u8 = 0x01;
pub const ATTACH: u8 = 0x02;
pub const INPUT: u8 = 0x03;
pub const RESIZE: u8 = 0x04;
pub const STATUS: u8 = 0x05;
pub const SHUTDOWN: u8 = 0x06;

// Holder → client.
pub const WELCOME: u8 = 0x81;
pub const OUTPUT: u8 = 0x82;
pub const REPLAY: u8 = 0x83;
pub const EXIT: u8 = 0x84;
pub const INFO: u8 = 0x85;
pub const ERROR: u8 = 0x86;

/// What a holder reports about itself (WELCOME and INFO).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub holder_pid: u32,
    pub child_pid: u32,
    pub cols: u16,
    pub rows: u16,
    /// `Some(code)` once the child has exited (128 + signal if killed).
    pub exit: Option<i32>,
}

impl Info {
    const LEN: usize = 4 + 4 + 2 + 2 + 1 + 4;

    pub fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.holder_pid.to_be_bytes());
        out.extend_from_slice(&self.child_pid.to_be_bytes());
        out.extend_from_slice(&self.cols.to_be_bytes());
        out.extend_from_slice(&self.rows.to_be_bytes());
        out.push(self.exit.is_some() as u8);
        out.extend_from_slice(&self.exit.unwrap_or(0).to_be_bytes());
    }

    pub fn decode(b: &[u8]) -> Option<Info> {
        if b.len() < Self::LEN {
            return None;
        }
        Some(Info {
            holder_pid: u32::from_be_bytes(b[0..4].try_into().ok()?),
            child_pid: u32::from_be_bytes(b[4..8].try_into().ok()?),
            cols: u16::from_be_bytes(b[8..10].try_into().ok()?),
            rows: u16::from_be_bytes(b[10..12].try_into().ok()?),
            exit: (b[12] != 0).then(|| i32::from_be_bytes(b[13..17].try_into().unwrap())),
        })
    }
}

/// A decoded holder → client frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    Welcome { version: u16, info: Option<Info> },
    Output(Vec<u8>),
    Replay(Vec<u8>),
    Exit(i32),
    Info(Info),
    Error(String),
    /// A frame type this client doesn't know (additive changes).
    Unknown(u8),
}

impl Msg {
    pub fn decode(ty: u8, body: Vec<u8>) -> io::Result<Msg> {
        let short = || io::Error::new(io::ErrorKind::InvalidData, format!("short frame 0x{ty:02x}"));
        Ok(match ty {
            WELCOME => {
                let version = u16::from_be_bytes(body.get(0..2).ok_or_else(short)?.try_into().unwrap());
                Msg::Welcome { version, info: Info::decode(&body[2..]) }
            }
            OUTPUT => Msg::Output(body),
            REPLAY => Msg::Replay(body),
            EXIT => Msg::Exit(i32::from_be_bytes(body.get(0..4).ok_or_else(short)?.try_into().unwrap())),
            INFO => Msg::Info(Info::decode(&body).ok_or_else(short)?),
            ERROR => Msg::Error(String::from_utf8_lossy(&body).into_owned()),
            other => Msg::Unknown(other),
        })
    }
}

/// Append one frame to `out`.
pub fn frame(out: &mut Vec<u8>, ty: u8, body: &[u8]) {
    let len = u32::try_from(body.len() + 1).expect("frame too large");
    out.extend_from_slice(&len.to_be_bytes());
    out.push(ty);
    out.extend_from_slice(body);
}

pub fn encode(ty: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 5);
    frame(&mut out, ty, body);
    out
}

pub fn hello() -> Vec<u8> {
    encode(HELLO, &PROTOCOL_VERSION.to_be_bytes())
}
pub fn attach(replay: bool) -> Vec<u8> {
    encode(ATTACH, &[replay as u8])
}
pub fn input(bytes: &[u8]) -> Vec<u8> {
    encode(INPUT, bytes)
}
pub fn resize(cols: u16, rows: u16) -> Vec<u8> {
    let mut b = cols.to_be_bytes().to_vec();
    b.extend_from_slice(&rows.to_be_bytes());
    encode(RESIZE, &b)
}
pub fn status() -> Vec<u8> {
    encode(STATUS, &[])
}
pub fn shutdown(grace_ms: u32) -> Vec<u8> {
    encode(SHUTDOWN, &grace_ms.to_be_bytes())
}

pub fn welcome(info: &Info) -> Vec<u8> {
    let mut b = PROTOCOL_VERSION.to_be_bytes().to_vec();
    info.encode(&mut b);
    encode(WELCOME, &b)
}

/// Take one complete frame off the front of `buf`, if there is one.
pub fn take_frame(buf: &mut Vec<u8>) -> io::Result<Option<(u8, Vec<u8>)>> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_be_bytes(buf[0..4].try_into().unwrap()) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("bad frame length {len}")));
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let ty = buf[4];
    let body = buf[5..4 + len].to_vec();
    buf.drain(..4 + len);
    Ok(Some((ty, body)))
}

/// Blocking read of one frame; `Ok(None)` on a clean EOF between frames.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut head = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut head[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let len = u32::from_be_bytes(head) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("bad frame length {len}")));
    }
    let mut rest = vec![0u8; len];
    r.read_exact(&mut rest)?;
    let ty = rest[0];
    rest.remove(0);
    Ok(Some((ty, rest)))
}

pub fn write_frame(w: &mut impl Write, ty: u8, body: &[u8]) -> io::Result<()> {
    w.write_all(&encode(ty, body))?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_whole_and_in_pieces() {
        let mut wire = Vec::new();
        wire.extend(hello());
        wire.extend(input(b"ls\r"));
        wire.extend(resize(120, 40));
        wire.extend(shutdown(1500));

        let mut r = &wire[..];
        assert_eq!(read_frame(&mut r).unwrap(), Some((HELLO, vec![0, 1])));
        assert_eq!(read_frame(&mut r).unwrap(), Some((INPUT, b"ls\r".to_vec())));
        assert_eq!(read_frame(&mut r).unwrap(), Some((RESIZE, vec![0, 120, 0, 40])));
        assert_eq!(read_frame(&mut r).unwrap(), Some((SHUTDOWN, 1500u32.to_be_bytes().to_vec())));
        assert_eq!(read_frame(&mut r).unwrap(), None);

        // Byte by byte through the incremental parser.
        let mut buf = Vec::new();
        let mut got = Vec::new();
        for b in &wire {
            buf.push(*b);
            while let Some(f) = take_frame(&mut buf).unwrap() {
                got.push(f.0);
            }
        }
        assert_eq!(got, [HELLO, INPUT, RESIZE, SHUTDOWN]);
        assert!(buf.is_empty());
    }

    #[test]
    fn rejects_bad_lengths() {
        let mut buf = vec![0, 0, 0, 0, 1];
        assert!(take_frame(&mut buf).is_err());
        let mut buf = ((MAX_FRAME + 1) as u32).to_be_bytes().to_vec();
        assert!(take_frame(&mut buf).is_err());
        let mut r: &[u8] = &[0, 0, 0, 5, OUTPUT, b'a'];
        assert!(read_frame(&mut r).is_err(), "truncated body");
    }

    #[test]
    fn welcome_and_info_decode() {
        let info = Info { holder_pid: 10, child_pid: 11, cols: 80, rows: 24, exit: None };
        let (ty, body) = read_frame(&mut &welcome(&info)[..]).unwrap().unwrap();
        assert_eq!(Msg::decode(ty, body).unwrap(), Msg::Welcome { version: PROTOCOL_VERSION, info: Some(info) });

        let done = Info { exit: Some(137), ..info };
        let mut b = Vec::new();
        done.encode(&mut b);
        b.extend_from_slice(b"future fields");
        assert_eq!(Msg::decode(INFO, b).unwrap(), Msg::Info(done));
        assert_eq!(Msg::decode(EXIT, (-1i32).to_be_bytes().to_vec()).unwrap(), Msg::Exit(-1));
        assert_eq!(Msg::decode(0x99, vec![]).unwrap(), Msg::Unknown(0x99));
        assert!(Msg::decode(INFO, vec![1, 2]).is_err());
    }

    #[test]
    fn hello_layout_is_frozen() {
        // len=3, type=HELLO, version=1. Never change this.
        assert_eq!(hello(), [0, 0, 0, 3, 0x01, 0, 1]);
    }
}
