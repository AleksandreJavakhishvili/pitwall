//! Unix (macOS and Linux): the home folder, executable bits and local
//! sockets. Where the two differ (data folders, process facts, privacy) see
//! `macos.rs` and `linux.rs`.

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
