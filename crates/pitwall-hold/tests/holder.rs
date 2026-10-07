//! End-to-end tests against the real `pitwall-hold` binary. Only harmless
//! programs (sh, python3 on 127.0.0.1), sockets in a temp dir, and cleanup by
//! exact pid.

use std::ffi::OsString;
use std::io::Write;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use interprocess::local_socket::{prelude::*, GenericFilePath, Stream};
use pitwall_hold::client::{self, Conn, Launch};
use pitwall_hold::{proto, Msg};

const BIN: &str = env!("CARGO_BIN_EXE_pitwall-hold");

/// A temp socket dir plus the holder started in it; cleans up on drop.
struct Held {
    dir: PathBuf,
    socket: PathBuf,
    holder: u32,
    child: u32,
}

fn temp_dir(tag: &str) -> PathBuf {
    // Short: Unix socket paths are limited to ~104 bytes on macOS.
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pwh-{}-{tag}{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn start(tag: &str, program: &str, args: &[&str], grace_ms: u64) -> Held {
    let dir = temp_dir(tag);
    let socket = dir.join("agent.sock");
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    let mut cmd = client::command(
        Path::new(BIN),
        &Launch {
            socket: &socket,
            cols: 100,
            rows: 30,
            cwd: Some(&dir),
            grace: Some(Duration::from_millis(grace_ms)),
            program: program.as_ref(),
            args: &args,
        },
    );
    cmd.env("PW_TEST_MARK", "marked");
    let (holder, child) = client::launch(&mut cmd).expect("holder starts");
    Held { dir, socket, holder, child }
}

impl Held {
    fn connect(&self) -> Conn {
        client::connect(&self.socket, Duration::from_secs(5)).expect("connect")
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        if let Ok(mut c) = client::connect(&self.socket, Duration::from_secs(1)) {
            let _ = c.send(&proto::shutdown(200));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while client::alive(self.holder) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        if client::alive(self.holder) {
            client::force_kill(self.child);
            client::force_kill(self.holder);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Read frames until `done(collected_output, last_msg)` or the timeout.
fn read_until(c: &mut Conn, secs: u64, mut done: impl FnMut(&str, &Msg) -> bool) -> (String, Vec<Msg>) {
    c.set_recv_timeout(Some(Duration::from_millis(200))).unwrap();
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut text = String::new();
    let mut msgs = Vec::new();
    while Instant::now() < deadline {
        match c.recv() {
            Ok(Some(m)) => {
                if let Msg::Output(b) | Msg::Replay(b) = &m {
                    text.push_str(&String::from_utf8_lossy(b));
                }
                let stop = done(&text, &m);
                msgs.push(m);
                if stop {
                    break;
                }
            }
            Ok(None) => break,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => panic!("recv: {e}"),
        }
    }
    (text, msgs)
}

fn wait_gone(h: &Held, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if !client::alive(h.holder) && !h.socket.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn input_resize_status_and_env() {
    let h = start(
        "io",
        "/bin/sh",
        &["-c", "echo ready $PW_TEST_MARK $(pwd -P); while IFS= read -r l; do echo \"got:$l\"; [ \"$l\" = size ] && stty size; done"],
        500,
    );
    let mut c = h.connect();
    assert_eq!(c.info.child_pid, h.child);
    assert_eq!(c.info.holder_pid, h.holder);
    assert_eq!((c.info.cols, c.info.rows), (100, 30));
    c.send(&proto::attach(true)).unwrap();
    let dir = std::fs::canonicalize(&h.dir).unwrap();
    let want = format!("ready marked {}", dir.display());
    let (text, _) = read_until(&mut c, 10, |t, _| t.contains(&want));
    assert!(text.contains(&want), "env + cwd reach the child: {text:?}");

    c.send(&proto::input(b"hello\n")).unwrap();
    let (text, _) = read_until(&mut c, 10, |t, _| t.contains("got:hello"));
    assert!(text.contains("got:hello"), "{text:?}");

    c.send(&proto::resize(90, 33)).unwrap();
    c.send(&proto::input(b"size\n")).unwrap();
    let (text, _) = read_until(&mut c, 10, |t, _| t.contains("33 90"));
    assert!(text.contains("33 90"), "{text:?}");
    let info = c.status().unwrap();
    assert_eq!((info.cols, info.rows, info.exit), (90, 33, None));

    // Unknown frames get an ERROR; the connection stays usable.
    c.send(&proto::encode(0x7f, b"")).unwrap();
    let (_, msgs) = read_until(&mut c, 5, |_, m| matches!(m, Msg::Error(_)));
    assert!(msgs.iter().any(|m| matches!(m, Msg::Error(_))));
    assert_eq!(c.status().unwrap().child_pid, h.child);
}

#[test]
fn second_holder_on_a_live_socket_is_refused() {
    let h = start("dup", "/bin/sh", &["-c", "sleep 30"], 200);
    let args = [OsString::from("-c"), OsString::from("echo nope")];
    let mut cmd = client::command(
        Path::new(BIN),
        &Launch { socket: &h.socket, cols: 80, rows: 24, cwd: None, grace: None, program: "/bin/sh".as_ref(), args: &args },
    );
    let err = client::launch(&mut cmd).unwrap_err();
    assert!(err.to_string().contains("already uses"), "{err}");
    assert_eq!(h.connect().info.child_pid, h.child, "the original is untouched");
}

#[test]
fn other_protocol_versions_get_welcome_then_close() {
    let h = start("ver", "/bin/sh", &["-c", "sleep 30"], 200);
    let raw = || {
        let s = Stream::connect(h.socket.as_path().to_fs_name::<GenericFilePath>().unwrap()).unwrap();
        s.set_recv_timeout(Some(Duration::from_secs(5))).unwrap();
        s
    };
    let mut s = raw();
    s.write_all(&proto::encode(proto::HELLO, &99u16.to_be_bytes())).unwrap();
    let (ty, body) = proto::read_frame(&mut s).unwrap().unwrap();
    match Msg::decode(ty, body).unwrap() {
        Msg::Welcome { version, info } => {
            assert_eq!(version, proto::PROTOCOL_VERSION);
            assert_eq!(info.unwrap().child_pid, h.child);
        }
        m => panic!("expected WELCOME, got {m:?}"),
    }
    assert!(proto::read_frame(&mut s).unwrap().is_none(), "closed after WELCOME");

    // Anything before HELLO is an error.
    let mut s = raw();
    s.write_all(&proto::status()).unwrap();
    let (ty, _) = proto::read_frame(&mut s).unwrap().unwrap();
    assert_eq!(ty, proto::ERROR);
}

#[test]
fn holder_exits_after_its_child_with_a_readable_final_status() {
    let h = start("exit", "/bin/sh", &["-c", "sleep 0.3; echo bye; exit 3"], 1500);
    let mut c = h.connect();
    c.send(&proto::attach(true)).unwrap();
    let (text, msgs) = read_until(&mut c, 10, |_, m| matches!(m, Msg::Exit(_)));
    assert!(text.contains("bye"), "output before exit: {text:?}");
    assert_eq!(msgs.last(), Some(&Msg::Exit(3)));

    // Within the grace period a late client still reads the status…
    let late = h.connect();
    assert_eq!(late.info.exit, Some(3));
    drop(late);
    drop(c);
    // …then the holder removes its socket and exits.
    assert!(wait_gone(&h, 10), "holder {} still running", h.holder);
    assert!(!client::alive(h.child));
}

#[test]
fn shutdown_hangs_up_then_kills() {
    // Ignores SIGHUP, so the holder has to escalate after the grace period.
    let h = start("stop", "/bin/sh", &["-c", "trap '' HUP; echo armed; while :; do sleep 0.1; done"], 3000);
    let mut c = h.connect();
    c.send(&proto::attach(true)).unwrap();
    read_until(&mut c, 10, |t, _| t.contains("armed"));
    let t0 = Instant::now();
    c.send(&proto::shutdown(300)).unwrap();
    let (_, msgs) = read_until(&mut c, 10, |_, m| matches!(m, Msg::Exit(_)));
    assert_eq!(msgs.last(), Some(&Msg::Exit(128 + 9)));
    assert!(t0.elapsed() >= Duration::from_millis(250));
    // After SHUTDOWN it doesn't linger for the whole grace period.
    assert!(wait_gone(&h, 2), "holder {} still running", h.holder);
}

/// architecture.md §9 contract: the client side goes away and comes back while
/// a fake agent holds a listening port → port still open, output continues,
/// no respawn.
#[test]
fn contract_terminal_survives_client_restart() {
    let script = r#"
import os, socket, sys, time
s = socket.socket(); s.bind(("127.0.0.1", 0)); s.listen(8); s.setblocking(False)
print("started pid=%d port=%d" % (os.getpid(), s.getsockname()[1]), flush=True)
i = 0
while True:
    try:
        c, _ = s.accept(); c.sendall(b"hi\n"); c.close()
    except BlockingIOError:
        pass
    i += 1
    print("tick %d" % i, flush=True)
    time.sleep(0.1)
"#;
    let h = start("contract", "python3", &["-u", "-c", script], 500);

    // Detached: the holder is not our child (re-parented) and not in our session.
    let ppid = std::process::Command::new("ps").args(["-o", "ppid=", "-p", &h.holder.to_string()]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&ppid.stdout).trim(), "1", "holder is re-parented");

    let mut c = h.connect();
    c.send(&proto::attach(true)).unwrap();
    let (text, _) = read_until(&mut c, 15, |t, _| t.contains("tick 3"));
    let port: u16 = text
        .split("port=")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(|| panic!("no port in {text:?}"));
    let last_tick = |t: &str| t.rsplit("tick ").next().and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
    let before = last_tick(&text);
    assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());

    // The client side goes away entirely (the app quits / is rebuilt).
    drop(c);
    std::thread::sleep(Duration::from_millis(800));
    assert!(TcpStream::connect(("127.0.0.1", port)).is_ok(), "port still open with no client");

    // A new client comes back: same process, history replayed, output continues.
    let mut c = h.connect();
    assert_eq!(c.info.child_pid, h.child, "no respawn");
    assert_eq!(c.info.exit, None);
    c.send(&proto::attach(true)).unwrap();
    let (text, msgs) = read_until(&mut c, 15, |t, m| matches!(m, Msg::Output(_)) && last_tick(t) > before + 8);
    assert_eq!(text.matches("started pid=").count(), 1, "started once: {text:?}");
    assert!(text.contains(&format!("started pid={} port={port}", h.child)), "the child is the agent itself");
    assert!(matches!(msgs.first(), Some(Msg::Replay(_))), "history first");
    assert!(last_tick(&text) > before + 8, "output kept going: {before} → {}", last_tick(&text));
    assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());

    // Only an explicit request ends it.
    c.send(&proto::shutdown(500)).unwrap();
    let (_, msgs) = read_until(&mut c, 10, |_, m| matches!(m, Msg::Exit(_)));
    assert!(matches!(msgs.last(), Some(Msg::Exit(_))));
    assert!(wait_gone(&h, 5));
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err(), "port closed with the agent");
}
