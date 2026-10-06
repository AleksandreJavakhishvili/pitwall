//! OS-specific pieces behind narrow functions (architecture.md §9 decision 7):
//! the rest of the core has no `cfg` and no `std::os::*`. Today only Unix
//! (macOS) is implemented; a Windows port adds `windows.rs` with the same
//! functions.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::*;
#[cfg(unix)]
mod unix_procs;
#[cfg(unix)]
pub(crate) use unix_procs::*;
