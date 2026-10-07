//! Client side: start a holder, connect to one, talk to it. OS-neutral.

use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::platform::{self, RecvHalf, SendHalf, Stream};
use crate::proto::{self, Info, Msg, PROTOCOL_VERSION};

/// Reading half of a connection (after `Conn::split`).
pub type Reader = RecvHalf;
/// Writing half of a connection (after `Conn::split`).
pub type Writer = SendHalf;

/// Where the holder of `agent_id` listens: `<dir>/<agent_id>.sock`. On Unix
/// this is the socket file; Windows maps it to a named pipe.
pub fn socket_path(dir: &Path, agent_id: &str) -> PathBuf {
    dir.join(format!("{agent_id}.sock"))
}

/// Remove a socket left behind by a holder that is gone (connect said
/// `ConnectionRefused`).
pub fn remove_stale(path: &Path) {
    platform::remove_stale_endpoint(path);
}

pub use platform::{alive, force_kill};

/// A raw connection to whatever listens at `path`, without the handshake
/// (tests and diagnostics). `Read` / `Write` work on it and on `&RawStream`.
pub type RawStream = Stream;
pub use platform::Conn as RawConn;

pub fn connect_raw(path: &Path) -> io::Result<RawStream> {
    platform::connect(path)
}

/// A connection that has completed the HELLO / WELCOME handshake.
pub struct Conn {
    stream: Stream,
    pub version: u16,
    /// As of the handshake.
    pub info: Info,
}

/// Connect and handshake. Errors: `NotFound` (no holder there),
/// `ConnectionRefused` (stale socket, holder gone), `Unsupported` (the holder
/// speaks another protocol version), or IO errors / timeouts for a peer that
/// doesn't answer within `timeout`.
pub fn connect(path: &Path, timeout: Duration) -> io::Result<Conn> {
    let stream = platform::connect(path)?;
    stream.set_recv_timeout(Some(timeout))?;
    stream.set_send_timeout(Some(timeout))?;
    (&stream).write_all(&proto::hello())?;
    let (ty, body) = proto::read_frame(&mut &stream)?.ok_or(io::ErrorKind::UnexpectedEof)?;
    let (version, info) = match Msg::decode(ty, body)? {
        Msg::Welcome { version, info } => (version, info),
        Msg::Error(e) => return Err(io::Error::other(format!("holder: {e}"))),
        other => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("expected WELCOME, got {other:?}"))),
    };
    if version != PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("holder speaks protocol {version}, this client {PROTOCOL_VERSION}"),
        ));
    }
    let info = info.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "WELCOME without info"))?;
    stream.set_recv_timeout(None)?;
    stream.set_send_timeout(None)?;
    Ok(Conn { stream, version, info })
}

impl Conn {
    /// `None` blocks forever (the default after the handshake).
    pub fn set_recv_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.stream.set_recv_timeout(timeout)
    }

    /// Send one already-encoded frame (see `proto::{attach, input, …}`).
    pub fn send(&mut self, frame: &[u8]) -> io::Result<()> {
        (&self.stream).write_all(frame)
    }

    /// Next frame; `Ok(None)` when the holder closed the connection.
    pub fn recv(&mut self) -> io::Result<Option<Msg>> {
        match proto::read_frame(&mut &self.stream)? {
            Some((ty, body)) => Msg::decode(ty, body).map(Some),
            None => Ok(None),
        }
    }

    /// Ask for INFO, skipping any output frames that arrive first.
    pub fn status(&mut self) -> io::Result<Info> {
        self.send(&proto::status())?;
        loop {
            match self.recv()? {
                Some(Msg::Info(i)) => return Ok(i),
                Some(_) => {}
                None => return Err(io::ErrorKind::UnexpectedEof.into()),
            }
        }
    }

    /// Separate halves for a reader thread and a writer thread.
    pub fn split(self) -> (Reader, Writer) {
        self.stream.split()
    }
}

/// How to start a holder. The child gets the environment of the `Command`
/// this builds, so set or remove variables on it before `launch`.
pub struct Launch<'a> {
    pub socket: &'a Path,
    pub cols: u16,
    pub rows: u16,
    pub cwd: Option<&'a Path>,
    pub grace: Option<Duration>,
    pub program: &'a OsStr,
    pub args: &'a [OsString],
}

pub fn command(bin: &Path, l: &Launch) -> Command {
    let mut cmd = Command::new(bin);
    cmd.arg("--socket").arg(l.socket);
    cmd.arg("--cols").arg(l.cols.to_string());
    cmd.arg("--rows").arg(l.rows.to_string());
    if let Some(cwd) = l.cwd {
        cmd.arg("--cwd").arg(cwd);
    }
    if let Some(g) = l.grace {
        cmd.arg("--grace-ms").arg(g.as_millis().to_string());
    }
    cmd.arg("--").arg(l.program).args(l.args);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    platform::hide_console(&mut cmd);
    cmd
}

/// Run the launcher; returns (holder pid, child pid) once the child runs.
pub fn launch(cmd: &mut Command) -> io::Result<(u32, u32)> {
    let out = cmd.output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if out.status.success() {
        let mut it = stdout.trim().strip_prefix("ready ").unwrap_or("").split_whitespace();
        if let (Some(Ok(h)), Some(Ok(c))) = (it.next().map(str::parse), it.next().map(str::parse)) {
            return Ok((h, c));
        }
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let msg = err.trim().trim_start_matches("pitwall-hold: ");
    Err(io::Error::other(if msg.is_empty() { format!("holder failed to start ({})", out.status) } else { msg.to_string() }))
}
