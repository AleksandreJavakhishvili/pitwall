//! The local socket and Pitwall's data folder, per OS (architecture.md §9
//! decision 7): Unix sockets and `~/.pitwall` on macOS/Linux; named pipes
//! (`pitwall_proto::pipe`) and `%APPDATA%\Pitwall` on Windows.

use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
pub type Conn = std::os::unix::net::UnixStream;

#[cfg(unix)]
pub fn connect(path: &Path) -> io::Result<Conn> {
    std::os::unix::net::UnixStream::connect(path)
}

#[cfg(windows)]
pub type Conn = interprocess::os::windows::named_pipe::DuplexPipeStream<interprocess::os::windows::named_pipe::pipe_mode::Bytes>;

#[cfg(windows)]
pub fn connect(path: &Path) -> io::Result<Conn> {
    let name = pitwall_proto::pipe::pipe_name(&path.to_string_lossy());
    Conn::connect_by_path_with_wait_mode(name.as_str(), interprocess::ConnectWaitMode::Timeout(std::time::Duration::from_secs(3)))
}

/// `%APPDATA%\Pitwall`, or `$PITWALL_HOME` (pitwall-core's `Paths::default_root`).
#[cfg(windows)]
pub fn data_dir() -> PathBuf {
    match std::env::var_os("PITWALL_HOME") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => match std::env::var_os("APPDATA").filter(|a| !a.is_empty()) {
            Some(a) => PathBuf::from(a).join("Pitwall"),
            None => std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default().join("AppData").join("Roaming").join("Pitwall"),
        },
    }
}

/// `~/.pitwall`, or `$PITWALL_HOME` (pitwall-core's `Paths::default_root`).
#[cfg(not(windows))]
pub fn data_dir() -> PathBuf {
    match std::env::var_os("PITWALL_HOME") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/")).join(".pitwall"),
    }
}
