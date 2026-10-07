//! Everything OS-specific in the holder, behind one narrow interface. The
//! protocol, the server and the client are OS-neutral and only call what is
//! listed here. `unix.rs` (macOS, Linux) and `windows.rs` implement the same
//! items:
//!
//! - **Endpoint** — where a holder listens. Pitwall names it by a path
//!   (`~/.pitwall/run/hold/<agentId>.sock`); Unix uses that socket file,
//!   Windows a named pipe derived from it. `listen(path)` / `connect(path)`
//!   give a `Listener` (`accept`) / `Stream` (with the [`Conn`] methods),
//!   reachable only by the current user. `prepare_endpoint`,
//!   `secure_endpoint`, `remove_stale_endpoint`, `endpoint_token` and
//!   `remove_endpoint` cover the file bookkeeping that Unix needs and named
//!   pipes don't (no-ops there).
//! - **Detaching** — `detach()` splits the started process into a short-lived
//!   launcher and the long-lived holder, which must survive its parent and
//!   not receive its parent's terminal or session signals (Unix: fork +
//!   setsid; Windows: re-spawn itself detached). The two sides talk over a
//!   one-shot readiness pipe. `hide_console` keeps a launcher started from a
//!   GUI app windowless (Windows).
//! - **PTY** — `spawn_pty(spec)` starts the program in a new pseudo-terminal
//!   (Unix: forkpty; Windows: ConPTY) and returns byte reader / writer plus a
//!   `PtyControl` (resize, hang up, kill, wait).
//! - **Processes** — `alive(pid)` and `force_kill(pid)`, by exact pid only.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

/// What to run in the PTY.
pub struct PtySpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
}

pub enum Role {
    /// The process that was started: read the holder's one-line report.
    Launcher(Box<dyn Read>),
    /// The detached holder: write the report, then serve.
    Holder(Box<dyn Write>),
}

/// What a connection can do besides `Read` / `Write`.
pub trait Conn: Sized {
    /// `None` blocks forever. A read that times out fails with `WouldBlock`
    /// or `TimedOut`.
    fn set_recv_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    fn set_send_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    /// Separate halves for a reader thread and a writer thread.
    fn split(self) -> (RecvHalf, SendHalf);
}

#[cfg_attr(not(windows), allow(dead_code))]
mod pipe_name;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(any(unix, windows)))]
compile_error!("pitwall-hold has no platform module for this OS yet: add platform/<os>.rs (see platform/mod.rs)");
