//! WASI stdout and stderr into the tile's Log (runtime.md §3.1): bytes the
//! guest writes are kept in a buffer, and the worker turns complete lines
//! into Log lines after each call. Writes past [`MAX_STDIO_BYTES`] between
//! two calls are dropped (never an error, so `println!` can't panic).

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use bytes::Bytes;
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::p2::{OutputStream, Pollable, StreamResult};

use crate::limits::{MAX_LOG_LINE, MAX_STDIO_BYTES};

/// One output stream's buffer, shared by every handle the guest opens.
#[derive(Clone, Default)]
pub(crate) struct LogPipe(Arc<Mutex<Vec<u8>>>);

impl LogPipe {
    fn push(&self, bytes: &[u8]) {
        let mut buf = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let room = MAX_STDIO_BYTES.saturating_sub(buf.len());
        buf.extend_from_slice(&bytes[..bytes.len().min(room)]);
    }

    /// The complete lines written so far (and, with `all`, the unfinished
    /// last one too), each cut to [`MAX_LOG_LINE`] bytes.
    pub fn take_lines(&self, all: bool) -> Vec<String> {
        let mut buf = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let end = if all || buf.len() >= MAX_STDIO_BYTES { buf.len() } else { buf.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1) };
        let taken: Vec<u8> = buf.drain(..end).collect();
        String::from_utf8_lossy(&taken)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| crate::host::cut(l, MAX_LOG_LINE))
            .collect()
    }
}

impl IsTerminal for LogPipe {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for LogPipe {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }

    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

#[wasmtime_wasi::async_trait]
impl OutputStream for LogPipe {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        self.push(&bytes);
        Ok(())
    }

    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }

    fn check_write(&mut self) -> StreamResult<usize> {
        // Always room: what doesn't fit is dropped in `push`.
        Ok(MAX_STDIO_BYTES)
    }
}

#[wasmtime_wasi::async_trait]
impl Pollable for LogPipe {
    async fn ready(&mut self) {}
}

impl tokio::io::AsyncWrite for LogPipe {
    fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        self.push(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

