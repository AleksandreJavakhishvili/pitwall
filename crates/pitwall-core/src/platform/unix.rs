//! Unix (macOS) implementations.

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::shell::run_with_timeout;

const PROCESS_TIMEOUT: Duration = Duration::from_secs(5);

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Everything Pitwall owns lives under ~/.pitwall.
pub fn data_dir() -> PathBuf {
    home_dir().join(".pitwall")
}

/// Per-user application data of other apps (editors' "recently opened").
pub fn app_support_dir() -> PathBuf {
    home_dir().join("Library/Application Support")
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

// ---------------------------------------------------------------- processes

/// `ps -axww -o pid=,ppid=,args=` output (parsed by `scan::parse_ps`).
pub fn process_table() -> Option<String> {
    let mut ps = Command::new("/bin/ps");
    ps.args(["-axww", "-o", "pid=,ppid=,args="]);
    run_with_timeout(ps, PROCESS_TIMEOUT)
}

/// `(pid, cwd)` of each process in `pids` that lsof could see.
pub fn process_cwds(pids: &[u32], timeout: Duration) -> Option<Vec<(u32, String)>> {
    let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
    let mut lsof = Command::new("/usr/sbin/lsof");
    lsof.args(["-a", "-d", "cwd", "-p", &pids.join(","), "-Fn"]);
    run_with_timeout(lsof, timeout).map(|out| parse_lsof_cwd(&out))
}

/// `(pid, cwd)` pairs from `lsof -a -d cwd -p <pids> -Fn`.
fn parse_lsof_cwd(out: &str) -> Vec<(u32, String)> {
    let mut res = Vec::new();
    let mut pid: Option<u32> = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix('p') {
            pid = p.trim().parse().ok();
        } else if let Some(n) = line.strip_prefix('n') {
            if let Some(p) = pid.take() {
                res.push((p, n.to_string()));
            }
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsof_output() {
        let got = parse_lsof_cwd(include_str!("fixtures/lsof_cwd.txt"));
        assert_eq!(
            got,
            vec![
                (29306, "/Users/dev/code/orders-api".to_string()),
                (83806, "/Users/dev/My Projects/web app".to_string()),
            ]
        );
    }

    #[test]
    fn local_sockets_carry_bytes_both_ways() {
        let (mut a, mut b) = LocalStream::pair().unwrap();
        a.write_all(b"ping").unwrap();
        let mut buf = [0u8; 4];
        b.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ping");
    }
}
