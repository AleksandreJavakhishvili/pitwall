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
//!   pipes, Toolhelp32 process facts;
//! - `seatbelt.rs` (macOS): the shared sessions' sandbox
//!   (`sandbox::SessionSandbox`, docs/spec/collaboration/sessions.md §6.3).
//!   Other OSes answer `Unsupported` until their backends land (Linux C3,
//!   Windows C5).

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

#[cfg(target_os = "macos")]
mod seatbelt;

/// This OS's shared-session sandbox (`sandbox::session_sandbox`).
#[cfg(target_os = "macos")]
pub(crate) fn session_sandbox() -> Box<dyn crate::sandbox::SessionSandbox> {
    Box::new(seatbelt::Seatbelt::new())
}

/// No backend here yet: this machine can ask but not host.
#[cfg(not(target_os = "macos"))]
pub(crate) fn session_sandbox() -> Box<dyn crate::sandbox::SessionSandbox> {
    #[cfg(target_os = "linux")]
    let why = "hosting shared sessions on Linux comes with the bubblewrap sandbox (phase C3)";
    #[cfg(windows)]
    let why = "hosting shared sessions on Windows comes with the WSL2 route (phase C5)";
    #[cfg(not(any(target_os = "linux", windows)))]
    let why = "hosting shared sessions isn't supported on this system";
    Box::new(NoSandbox { why })
}

/// The backend of an OS that can't host shared sessions (yet).
#[cfg(not(target_os = "macos"))]
struct NoSandbox {
    why: &'static str,
}

#[cfg(not(target_os = "macos"))]
impl crate::sandbox::SessionSandbox for NoSandbox {
    fn name(&self) -> &str {
        "none"
    }

    fn support(&self) -> crate::sandbox::Support {
        crate::sandbox::Support::Unsupported { why: self.why.to_string() }
    }

    fn wrap(&self, _: &crate::sandbox::SandboxPolicy, _: &[String]) -> crate::error::Result<crate::sandbox::Wrapped> {
        Err(crate::error::PwError::unsupported(self.why))
    }
}
