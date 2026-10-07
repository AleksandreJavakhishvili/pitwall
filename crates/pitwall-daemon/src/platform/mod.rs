//! OS-specific pieces behind narrow functions (architecture.md §9 decision
//! 7): the listening socket, the peer's pid, the process table. Unix
//! sockets on macOS/Linux; per-user named pipes on Windows.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(any(unix, windows)))]
compile_error!("pitwall-daemon: no platform module for this OS (see platform/mod.rs)");
