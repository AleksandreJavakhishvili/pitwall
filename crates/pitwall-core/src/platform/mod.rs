//! OS-specific pieces behind narrow functions (architecture.md §9 decision 7):
//! the rest of the core has no `cfg` and no `std::os::*`. Each OS module
//! provides the same functions:
//!
//! - `unix.rs` (macOS and Linux): home folder, executable bits, local sockets;
//! - `macos.rs` + `macos_procs.rs`: `~/.pitwall`, Application Support, Full
//!   Disk Access, `ps` / `lsof`;
//! - `linux.rs`: XDG folders, no folder privacy, `/proc` (`procfs.rs`,
//!   `xdg.rs` — both pure enough to be tested on any Unix).
//!
//! A Windows port adds `windows.rs` with the same functions.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::*;

#[cfg(all(unix, not(target_os = "linux")))]
mod macos;
#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) use macos::*;
#[cfg(all(unix, not(target_os = "linux")))]
mod macos_procs;
#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) use macos_procs::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::*;
#[cfg(any(target_os = "linux", all(unix, test)))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod procfs;
#[cfg(any(target_os = "linux", all(unix, test)))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod xdg;
