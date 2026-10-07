use std::collections::HashMap;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process::Command;

use crate::identity::Proc;

pub type Listener = UnixListener;
pub type Stream = UnixStream;

/// Listen at `path`: its folder only for this user (0700), the socket too
/// (0600). A stale socket is replaced; a live one (another Pitwall) is an
/// error.
pub fn bind(path: &Path) -> io::Result<Listener> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        if std::fs::metadata(dir)?.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err(io::Error::new(io::ErrorKind::AddrInUse, format!("another Pitwall is listening at {}", path.display())));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Wake a blocked `accept` (shutting down).
pub fn poke(path: &Path) {
    let _ = UnixStream::connect(path);
}

/// The pid of the process at the other end.
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub fn peer_pid(s: &Stream) -> Option<u32> {
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: a valid socket fd and an out-parameter of the size we pass.
    let rc = unsafe {
        libc::getsockopt(s.as_raw_fd(), libc::SOL_LOCAL, libc::LOCAL_PEERPID, (&mut pid as *mut libc::pid_t).cast(), &mut len)
    };
    (rc == 0 && pid > 0).then_some(pid as u32)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn peer_pid(s: &Stream) -> Option<u32> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: a valid socket fd and an out-parameter of the size we pass.
    let rc = unsafe {
        libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len)
    };
    (rc == 0 && cred.pid > 0).then_some(cred.pid as u32)
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux", target_os = "android")))]
pub fn peer_pid(_s: &Stream) -> Option<u32> {
    None
}

/// Every process's parent and program name (`ps`), or `None` if `ps` failed.
pub fn process_table() -> Option<HashMap<u32, Proc>> {
    let out = Command::new("ps").args(["-axo", "pid=,ppid=,comm="]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(parse_ps(&String::from_utf8_lossy(&out.stdout)))
}

pub fn parse_ps(text: &str) -> HashMap<u32, Proc> {
    text.lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let pid = it.next()?.parse().ok()?;
            let ppid = it.next()?.parse().ok()?;
            let comm = it.collect::<Vec<_>>().join(" ");
            let name = Path::new(&comm).file_name().map_or(comm.clone(), |n| n.to_string_lossy().into_owned());
            Some((pid, Proc { ppid, name }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ps() {
        let t = parse_ps("    1     0 /sbin/launchd\n  412     1 /Applications/Pitwall.app/Contents/MacOS/pitwall-hold\nbad line\n 413 412 -zsh\n");
        assert_eq!(t[&412], Proc { ppid: 1, name: "pitwall-hold".into() });
        assert_eq!(t[&413].name, "-zsh");
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn a_socket_knows_its_peer() {
        let (a, _b) = UnixStream::pair().unwrap();
        assert_eq!(peer_pid(&a), Some(std::process::id()));
    }
}
