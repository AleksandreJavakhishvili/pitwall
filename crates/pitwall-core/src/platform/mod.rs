//! OS-specific pieces behind narrow functions (architecture.md §9 decision 7):
//! the rest of the core has no `cfg` and no `std::os::*`. Each OS provides
//! the same functions and `host::HostInfo` constants:
//!
//! - `unix.rs` (macOS and Linux): home folder, executable bits, the login
//!   shell, the hook relay script, local sockets (Unix socket files);
//! - `macos.rs` + `macos_procs.rs`: `~/.pitwall`, Application Support, Full
//!   Disk Access, `ps` / `lsof`;
//! - `linux.rs`: XDG folders, no folder privacy, `/proc` (`procfs.rs`,
//!   `xdg.rs` — both pure enough to be tested on any Unix);
//! - `windows.rs` + `windows_procs.rs`: `%APPDATA%\Pitwall`, PowerShell,
//!   the registry's PATH, the `pitwall-hook` relay binary, per-user named
//!   pipes, Toolhelp32 process facts.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::*;
#[cfg(unix)]
pub use unix::{LocalListener, LocalStream};

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
