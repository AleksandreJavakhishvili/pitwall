//! HTTP as collaboration sees it: [`http`]'s request and response types
//! with a [`Body`] that is either bytes or a blocking stream (for SSE), and
//! the [`HttpClient`] a backend hands out for one device. Blocking, no async
//! runtime (architecture.md §1): a backend serves each request on a thread
//! of its own and streams a response by reading its body.

use std::io::{self, Read};
use std::sync::mpsc;
use std::time::Duration;

use crate::error::{PwError, Result};

pub type Request = http::Request<Body>;
pub type Response = http::Response<Body>;

/// A request or response body.
#[derive(Default)]
pub enum Body {
    #[default]
    Empty,
    Bytes(Vec<u8>),
    /// Read until EOF; written to the wire as it comes (chunked), so an SSE
    /// stream is one event per write.
    Stream(Box<dyn Read + Send>),
}

impl Body {
    /// A JSON body (callers set `Content-Type`).
    pub fn json<T: serde::Serialize + ?Sized>(value: &T) -> Body {
        Body::Bytes(serde_json::to_vec(value).unwrap_or_default())
    }

    /// A streaming body fed through the returned writer; dropping the
    /// writer ends it.
    pub fn channel() -> (BodyWriter, Body) {
        let (tx, rx) = mpsc::channel();
        (BodyWriter(tx), Body::Stream(Box::new(ChannelReader { rx, chunk: Vec::new(), at: 0 })))
    }

    pub fn is_stream(&self) -> bool {
        matches!(self, Body::Stream(_))
    }

    /// All of it, refusing more than `limit` bytes (collaboration reads
    /// request bodies with a 2 MiB limit, §8.1).
    pub fn read_all(self, limit: usize) -> Result<Vec<u8>> {
        let bytes = match self {
            Body::Empty => Vec::new(),
            Body::Bytes(b) => b,
            Body::Stream(r) => {
                let mut out = Vec::new();
                r.take(limit as u64 + 1).read_to_end(&mut out)?;
                out
            }
        };
        if bytes.len() > limit {
            return Err(PwError::other(format!("body larger than {limit} bytes")));
        }
        Ok(bytes)
    }

    /// The body as a reader, whatever it is.
    pub fn into_reader(self) -> Box<dyn Read + Send> {
        match self {
            Body::Empty => Box::new(io::empty()),
            Body::Bytes(b) => Box::new(io::Cursor::new(b)),
            Body::Stream(r) => r,
        }
    }
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Body::Empty => f.write_str("Body::Empty"),
            Body::Bytes(b) => write!(f, "Body::Bytes({} bytes)", b.len()),
            Body::Stream(_) => f.write_str("Body::Stream"),
        }
    }
}

impl From<Vec<u8>> for Body {
    fn from(bytes: Vec<u8>) -> Body {
        Body::Bytes(bytes)
    }
}

impl From<String> for Body {
    fn from(text: String) -> Body {
        Body::Bytes(text.into_bytes())
    }
}

impl From<&str> for Body {
    fn from(text: &str) -> Body {
        Body::Bytes(text.as_bytes().to_vec())
    }
}

/// The writing end of [`Body::channel`].
#[derive(Clone)]
pub struct BodyWriter(mpsc::Sender<Vec<u8>>);

impl BodyWriter {
    /// Queue one chunk. `false`: the reader is gone (the peer hung up), so
    /// the writer can stop.
    pub fn send(&self, chunk: impl Into<Vec<u8>>) -> bool {
        self.0.send(chunk.into()).is_ok()
    }
}

struct ChannelReader {
    rx: mpsc::Receiver<Vec<u8>>,
    chunk: Vec<u8>,
    at: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.at >= self.chunk.len() {
            match self.rx.recv() {
                Ok(chunk) => {
                    self.chunk = chunk;
                    self.at = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.chunk.len() - self.at);
        buf[..n].copy_from_slice(&self.chunk[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// How long a client waits. Put one in a request's extensions
/// (`req.extensions_mut().insert(Timeouts { … })`); without one, clients use
/// [`Timeouts::default`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// Reaching the device (discovery probes use 2 s, §3).
    pub connect: Duration,
    /// Silence on an open response before giving up; `None`: wait (SSE).
    pub idle: Option<Duration>,
}

impl Default for Timeouts {
    fn default() -> Timeouts {
        Timeouts { connect: Duration::from_secs(10), idle: Some(Duration::from_secs(60)) }
    }
}

/// Outbound HTTP to one device, from [`Network::client`](super::Network::client):
/// the backend picks the route and attaches this device's credentials.
/// The request URI's path and query are used; scheme and host, if any, are
/// the backend's business. A refused or unreachable device is an
/// [`Unreachable`](crate::error::ErrorCode::Unreachable) error; an HTTP
/// error status is an `Ok` response.
pub trait HttpClient: Send + Sync {
    fn send(&self, req: Request) -> Result<Response>;
}
