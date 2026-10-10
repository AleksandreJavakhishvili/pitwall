//! One app per data folder, and a second launch focuses the first
//! (docs/spec/gpui/platform.md "Single instance").
//!
//! `home::choose` already refuses a second app on one folder (the CLI socket
//! answers). Before showing that refusal, the second launch asks the running
//! app to show its main window over a small "focus" endpoint in the data
//! folder's `run/` and exits when it answers. The endpoint is a Unix socket
//! (`run/app.sock`), or on Windows a loopback TCP port written with a random
//! token to `run/app.port` (std has no named pipes). A Tauri Pitwall has no
//! endpoint, so the refusal window still explains that case.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::channel::mpsc::unbounded;
use futures::StreamExt;
use gpui::{App, AsyncApp};

use crate::menu::ShowMain;

const ASK: &str = "focus";
const OK: &str = "ok";
const TIMEOUT: Duration = Duration::from_secs(2);

#[cfg(unix)]
fn endpoint(root: &Path) -> PathBuf {
    root.join("run").join("app.sock")
}

#[cfg(windows)]
fn endpoint(root: &Path) -> PathBuf {
    root.join("run").join("app.port")
}

/// The second launch: ask the app running on `root` to come forward.
/// Whether it answered.
pub fn focus_running(root: &Path) -> bool {
    ask(&endpoint(root)).is_ok()
}

fn exchange<S: std::io::Read + Write>(mut s: S, line: &str) -> std::io::Result<()> {
    s.write_all(format!("{line}\n").as_bytes())?;
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply)?;
    if reply.trim() == OK {
        Ok(())
    } else {
        Err(std::io::Error::other("no answer"))
    }
}

#[cfg(unix)]
fn ask(path: &Path) -> std::io::Result<()> {
    let s = std::os::unix::net::UnixStream::connect(path)?;
    s.set_read_timeout(Some(TIMEOUT))?;
    exchange(s, ASK)
}

#[cfg(windows)]
fn ask(path: &Path) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let (port, token) = text
        .trim()
        .split_once(' ')
        .ok_or_else(|| std::io::Error::other("bad port file"))?;
    let port: u16 = port.parse().map_err(std::io::Error::other)?;
    let s = std::net::TcpStream::connect_timeout(
        &(std::net::Ipv4Addr::LOCALHOST, port).into(),
        TIMEOUT,
    )?;
    s.set_read_timeout(Some(TIMEOUT))?;
    exchange(s, &format!("{ASK} {token}"))
}

/// Answer one request: `on_focus` runs if it asked to focus, before the
/// reply, so the asker's answer means the focus was taken.
fn answer<S: std::io::Read + Write>(s: S, want: &str, on_focus: impl FnOnce()) -> bool {
    let mut line = String::new();
    let mut reader = BufReader::new(s);
    if reader.read_line(&mut line).is_err() || line.trim() != want {
        return false;
    }
    on_focus();
    let _ = reader.get_mut().write_all(format!("{OK}\n").as_bytes());
    true
}

/// Serve the focus endpoint for the app running on `root`: a request shows
/// the main window. Removed when the app quits.
pub fn serve(root: &Path, cx: &mut App) {
    let path = endpoint(root);
    let (tx, mut rx) = unbounded::<()>();
    match listen(&path, move || {
        let _ = tx.unbounded_send(());
    }) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("pitwall: a second launch can't focus this one: {e}");
            return;
        }
    }
    cx.spawn(async move |cx: &mut AsyncApp| {
        while rx.next().await.is_some() {
            let _ = cx.update(|cx| {
                cx.activate(true);
                cx.dispatch_action(&ShowMain);
            });
        }
    })
    .detach();
    cx.on_app_quit(move |_| {
        let _ = std::fs::remove_file(&path);
        async {}
    })
    .detach();
}

#[cfg(unix)]
fn listen(path: &Path, on_focus: impl Fn() + Send + 'static) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Ours to replace: `home::choose` found no other app on this folder.
    let _ = std::fs::remove_file(path);
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    std::thread::Builder::new()
        .name("pitwall-focus".into())
        .spawn(move || {
            for conn in listener.incoming().flatten() {
                let _ = conn.set_read_timeout(Some(TIMEOUT));
                answer(conn, ASK, &on_focus);
            }
        })?;
    Ok(())
}

#[cfg(windows)]
fn listen(path: &Path, on_focus: impl Fn() + Send + 'static) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let token = uuid_like();
    std::fs::write(path, format!("{port} {token}"))?;
    let want = format!("{ASK} {token}");
    std::thread::Builder::new()
        .name("pitwall-focus".into())
        .spawn(move || {
            for conn in listener.incoming().flatten() {
                let _ = conn.set_read_timeout(Some(TIMEOUT));
                answer(conn, &want, &on_focus);
            }
        })?;
    Ok(())
}

/// A token only a reader of the port file knows.
#[cfg(windows)]
fn uuid_like() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    h.write_u32(std::process::id());
    let a = h.finish();
    let b = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    format!("{a:016x}{b:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn a_second_launch_reaches_the_first() {
        let root = std::env::temp_dir().join(format!("pw-focus-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(!focus_running(&root), "nobody serves yet");

        let asked = Arc::new(AtomicUsize::new(0));
        let seen = asked.clone();
        listen(&endpoint(&root), move || {
            seen.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        assert!(focus_running(&root));
        assert!(focus_running(&root));
        assert_eq!(asked.load(Ordering::SeqCst), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_the_focus_line_is_answered() {
        let (mut a, b) = socket_pair();
        a.write_all(b"something else\n").unwrap();
        assert!(!answer(b, ASK, || unreachable!()));
    }

    #[cfg(unix)]
    fn socket_pair() -> (
        std::os::unix::net::UnixStream,
        std::os::unix::net::UnixStream,
    ) {
        std::os::unix::net::UnixStream::pair().unwrap()
    }

    #[cfg(windows)]
    fn socket_pair() -> (std::net::TcpStream, std::net::TcpStream) {
        let l = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let a = std::net::TcpStream::connect(l.local_addr().unwrap()).unwrap();
        (a, l.accept().unwrap().0)
    }
}
