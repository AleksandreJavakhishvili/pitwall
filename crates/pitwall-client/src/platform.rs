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

/// `~/.pitwall`, or `$PITWALL_HOME` (pitwall-core's `Paths::default_root`
/// on macOS/Linux).
pub fn data_dir() -> PathBuf {
    match std::env::var_os("PITWALL_HOME") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")).join(".pitwall"),
    }
}
