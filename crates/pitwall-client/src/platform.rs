//! The local socket and Pitwall's data folder, per OS (architecture.md §9
//! decision 7). Unix sockets today; Windows (named pipes) comes with the
//! Windows port.

use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
pub type Conn = std::os::unix::net::UnixStream;

#[cfg(unix)]
pub fn connect(path: &Path) -> io::Result<Conn> {
    std::os::unix::net::UnixStream::connect(path)
}

#[cfg(not(unix))]
pub type Conn = std::net::TcpStream;

#[cfg(not(unix))]
pub fn connect(_path: &Path) -> io::Result<Conn> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "Pitwall's socket isn't available on this platform yet"))
}

/// `$PITWALL_HOME`, else the platform default — the same folder as
/// pitwall-core's `Paths::default_root`: `~/.pitwall` on macOS,
/// `$XDG_DATA_HOME/pitwall` (default `~/.local/share/pitwall`) on Linux.
pub fn data_dir() -> PathBuf {
    match std::env::var_os("PITWALL_HOME") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => default_data_dir(),
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(not(target_os = "linux"))]
fn default_data_dir() -> PathBuf {
    home().join(".pitwall")
}

#[cfg(target_os = "linux")]
fn default_data_dir() -> PathBuf {
    match std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        Some(p) if p.is_absolute() => p.join("pitwall"),
        _ => home().join(".local/share/pitwall"),
    }
}
