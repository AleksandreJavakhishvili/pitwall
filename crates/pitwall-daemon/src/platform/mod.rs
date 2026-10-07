//! OS-specific pieces behind narrow functions (architecture.md §9 decision
//! 7): the listening socket, the peer's pid, the process table. Unix
//! sockets on macOS/Linux; Windows (named pipes) comes with that port.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(not(unix))]
compile_error!("pitwall-daemon: only Unix platforms are supported so far (named pipes come with the Windows port)");
