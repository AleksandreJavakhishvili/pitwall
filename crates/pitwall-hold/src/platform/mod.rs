//! Everything OS-specific in the holder, behind one narrow interface. The
//! protocol, the server and the client are OS-neutral and only call what is
//! listed here. Today: `unix.rs` (macOS, also Linux). A Windows port adds
//! `windows.rs` with the same items:
//!
//! - **Endpoint** — where a holder listens. `endpoint_name(path)` maps the
//!   Pitwall path (`~/.pitwall/run/hold/<agentId>.sock`) to an `interprocess`
//!   local-socket name (Unix: that socket file; Windows: a named pipe derived
//!   from it). `prepare_endpoint`, `secure_endpoint`, `remove_stale_endpoint`,
//!   `endpoint_token` and `remove_endpoint` cover the file bookkeeping that
//!   Unix needs and named pipes don't (they can be no-ops there).
//! - **Detaching** — `detach()` splits the started process into a short-lived
//!   launcher and the long-lived holder, which must survive its parent and
//!   not receive its parent's terminal or session signals (Unix: fork +
//!   setsid; Windows: re-spawn itself detached, then the holder side is
//!   recognised in `main`). The two sides talk over a one-shot readiness pipe.
//! - **PTY** — `spawn_pty(spec)` starts the program in a new pseudo-terminal
//!   (Unix: forkpty; Windows: ConPTY) and returns byte reader / writer plus a
//!   `PtyControl` (resize, hang up, kill, wait).
//! - **Processes** — `alive(pid)` and `force_kill(pid)`, by exact pid only.

use std::ffi::OsString;
use std::path::PathBuf;

/// What to run in the PTY.
pub struct PtySpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
}

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(not(unix))]
compile_error!("pitwall-hold has no platform module for this OS yet: add platform/<os>.rs (see platform/mod.rs)");
