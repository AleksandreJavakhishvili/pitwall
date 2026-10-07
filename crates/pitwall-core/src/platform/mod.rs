//! OS-specific pieces behind narrow functions (architecture.md §9 decision 7):
//! the rest of the core has no `cfg` and no `std::os::*`. `unix.rs` +
//! `unix_procs.rs` (macOS, Linux) and `windows.rs` + `windows_procs.rs`
//! implement the same functions.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::*;
#[cfg(unix)]
pub use unix::{LocalListener, LocalStream};
#[cfg(unix)]
mod unix_procs;
#[cfg(unix)]
pub(crate) use unix_procs::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;
#[cfg(windows)]
pub use windows::{LocalListener, LocalStream};
#[cfg(windows)]
mod windows_procs;
#[cfg(windows)]
pub(crate) use windows_procs::*;

/// Named pipes for local sockets on Windows (pure; tested everywhere).
#[cfg_attr(not(windows), allow(dead_code))]
mod pipe_name;
