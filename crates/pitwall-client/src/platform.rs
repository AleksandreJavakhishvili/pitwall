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

/// `$PITWALL_HOME`, else the platform default — the same folder as
/// pitwall-core's `Paths::default_root`: `~/.pitwall` on macOS,
/// `$XDG_DATA_HOME/pitwall` (default `~/.local/share/pitwall`) on Linux,
/// `%APPDATA%\Pitwall` on Windows.
pub fn data_dir() -> PathBuf {
    match std::env::var_os("PITWALL_HOME") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => default_data_dir(),
    }
}

#[cfg(unix)]
fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(all(unix, not(target_os = "linux")))]
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

#[cfg(windows)]
fn default_data_dir() -> PathBuf {
    match std::env::var_os("APPDATA").filter(|a| !a.is_empty()) {
        Some(a) => PathBuf::from(a).join("Pitwall"),
        None => std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default().join("AppData").join("Roaming").join("Pitwall"),
    }
}
