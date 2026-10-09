//! Unix (macOS and Linux): the home folder, executable bits and local
//! sockets. Where the two differ (data folders, process facts, privacy) see
//! `macos.rs` and `linux.rs`.

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Socket files (`host::HostInfo`).
pub const LOCAL_SOCKETS: crate::host::LocalSockets = crate::host::LocalSockets::Unix;

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `$PITWALL_HOME` when set (isolated test instances and benchmarks,
/// scripts/bench.sh); otherwise the platform's default.
pub(super) fn data_dir_or(default: impl FnOnce() -> PathBuf) -> PathBuf {
    match std::env::var_os(crate::paths::HOME_ENV) {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => default(),
    }
}

/// A file anyone may execute.
pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

pub fn make_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

/// Files `path` may name as a program (Unix: exactly that one).
pub fn executable_candidates(path: &Path) -> Vec<PathBuf> {
    vec![path.to_path_buf()]
}

/// `std::fs::canonicalize`.
pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// Nothing to hide: Unix programs get no window of their own.
pub fn hide_console(_cmd: &mut std::process::Command) {}

/// `link` runs `target` (a program): a symlink, replaced when it points
/// elsewhere. The Race Engineer's `pitwall` (crate::engineer).
pub fn link_program(target: &Path, link: &Path) -> io::Result<()> {
    if std::fs::read_link(link).ok().as_deref() == Some(target) {
        return Ok(());
    }
    let _ = std::fs::remove_file(link);
    std::os::unix::fs::symlink(target, link)
}

/// A symlink at `link` to `target` (tests).
#[cfg(test)]
pub fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

// ---------------------------------------------------------------- login shell

/// `$SHELL`, else the OS's default shell (`macos.rs` / `linux.rs`).
pub fn default_login_shell() -> String {
    std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(super::default_shell)
}

/// `program` on PATH, for a login shell that isn't POSIX (`$PITWALL_SHELL`
/// set to pwsh); POSIX shells resolve with `command -v` (`shell::which`).
pub fn which(program: &str) -> Option<String> {
    let path = crate::shell::spawn_path().map(std::ffi::OsString::from).or_else(|| std::env::var_os("PATH"))?;
    std::env::split_paths(&path)
        .map(|d| d.join(program))
        .find(|p| is_executable(p))
        .map(|p| p.to_string_lossy().into_owned())
}

/// PATH without asking a shell: none on Unix (the login shell is asked,
/// `shell::login_env_path`).
pub fn system_path() -> Option<String> {
    None
}

/// Where package managers put programs (Homebrew on Apple silicon, Homebrew
/// on Intel and other `/usr/local` installs, pipx/uv, cargo): added to PATH
/// when the login shell can't be asked.
pub fn common_bin_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".local/bin"),
        home.join(".cargo/bin"),
    ]
}

// ---------------------------------------------------------------- hook relay

/// Write the relay script (`script`) at `dest`, executable.
pub fn install_hook_relay(dest: &Path, script: &str) -> Result<(), String> {
    if std::fs::read_to_string(dest).ok().as_deref() != Some(script) {
        std::fs::write(dest, script).map_err(|e| e.to_string())?;
    }
    make_executable(dest).map_err(|e| e.to_string())
}

/// `sh '<dest>'`.
pub fn hook_relay_command(dest: &Path) -> String {
    format!("sh {}", crate::shell::quote(&dest.to_string_lossy()))
}

// ---------------------------------------------------------------- local sockets

/// A listening local socket (a Unix socket file).
pub struct LocalListener(UnixListener);

impl LocalListener {
    /// Bind at `path`, replacing a stale socket file.
    pub fn bind(path: &Path) -> io::Result<LocalListener> {
        let _ = std::fs::remove_file(path);
        UnixListener::bind(path).map(LocalListener)
    }

    pub fn incoming(&self) -> impl Iterator<Item = LocalStream> + '_ {
        self.0.incoming().flatten().map(LocalStream)
    }
}

pub struct LocalStream(UnixStream);

impl LocalStream {
    pub fn set_timeouts(&self, d: Duration) {
        let _ = self.0.set_read_timeout(Some(d));
        let _ = self.0.set_write_timeout(Some(d));
    }

    /// Two connected ends (tests).
    #[cfg(test)]
    pub fn pair() -> io::Result<(LocalStream, LocalStream)> {
        let (a, b) = UnixStream::pair()?;
        Ok((LocalStream(a), LocalStream(b)))
    }
}

impl Read for LocalStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for LocalStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_sockets_carry_bytes_both_ways() {
        let (mut a, mut b) = LocalStream::pair().unwrap();
        a.write_all(b"ping").unwrap();
        let mut buf = [0u8; 4];
        b.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ping");
    }
}
