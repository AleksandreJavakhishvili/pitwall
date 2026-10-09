//! Long pastes and terminal modes through a real `pitwall-hold` holder:
//! the engine's terminal (`TermHost`) → holder → PTY → a program.
//!
//! Safety: the programs are `/bin/sh` and this test binary itself (as a slow
//! reader), in a temp dir; holders are stopped through their socket, else
//! killed by exact pid.
#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use pitwall_core::clock::SystemClock;
use pitwall_core::provider::TermSize;
use pitwall_core::term::{paste_bytes, TermHost};
use pitwall_providers::hold::{self, HoldTerm, Program};

/// The holder binary: `PITWALL_HOLD_BIN`, else built once for these tests
/// (as `tests/contract.rs` does).
fn holder() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        if let Some(p) = std::env::var_os("PITWALL_HOLD_BIN") {
            return PathBuf::from(p);
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let target = root.join("target").join("hold-tests");
        let mut cmd = Command::new(env!("CARGO"));
        cmd.current_dir(&root)
            .args(["build", "-q", "-p", "pitwall-hold", "--bin", "pitwall-hold", "--target-dir"])
            .arg(&target);
        for key in ["RUSTC_WORKSPACE_WRAPPER", "RUSTC_WRAPPER", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET"] {
            cmd.env_remove(key);
        }
        assert!(cmd.status().expect("run cargo").success(), "building pitwall-hold failed");
        target.join("debug").join("pitwall-hold")
    })
    .clone()
}

/// A program in a holder, in a temp dir; stopped and removed on drop.
struct Held {
    dir: PathBuf,
    socket: PathBuf,
}

impl Held {
    fn start(tag: &str, program: &str, args: &[String], env: &[(&str, &std::ffi::OsStr)]) -> Held {
        // Short: Unix socket paths are limited to ~104 bytes on macOS.
        let dir = std::env::temp_dir().join(format!("pwlp-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("a.sock");
        let p = Program { program, args, cwd: Some(&dir), env };
        hold::run(&holder(), &socket, TermSize::new(100, 30), p).expect("holder starts");
        Held { dir, socket }
    }

    fn attach(&self) -> Arc<TermHost> {
        let term = HoldTerm::connect(&self.socket, true).expect("connect");
        TermHost::new(Box::new(term), TermSize::new(100, 30), Arc::new(SystemClock::new()))
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        // Hangs up, then kills the program (and the holder) by exact pid.
        let _ = hold::shutdown(&self.socket, Duration::from_millis(200));
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn wait_until(secs: u64, f: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !f() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    f()
}

/// A long, multi-line, non-ASCII text (Georgian, accents, emoji).
fn long_text(len: usize) -> String {
    let mut s = String::new();
    let mut i = 0;
    while s.len() < len {
        s.push_str(&format!("{i:06} ქართული ტექსტი — résumé ✓ 🙂 'q' \"dq\" $HOME `t`\n"));
        i += 1;
    }
    s
}

const OUT: &str = "PW_SLOW_READER_OUT";
const WANT: &str = "PW_SLOW_READER_WANT";

/// Not a test of its own: the slow reader the paste test runs in the holder
/// (this binary, started again with the variables below). Reads its raw
/// terminal in small pieces with a pause after each, like a busy TUI, and
/// keeps every byte.
#[test]
fn slow_reader() {
    let (Some(out), Some(want)) = (std::env::var_os(OUT), std::env::var(WANT).ok()) else {
        return;
    };
    let want: usize = want.parse().unwrap();
    let mut file = std::fs::File::create(out).unwrap();
    let mut stdin = std::io::stdin().lock();
    let mut buf = [0u8; 512];
    let mut got = 0;
    while got < want {
        let n = stdin.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).unwrap();
        got += n;
        std::thread::sleep(Duration::from_micros(500));
    }
}

#[test]
fn a_long_paste_reaches_a_slow_reader_whole_and_in_order() {
    let text = long_text(1024 * 1024);
    let want = [paste_bytes(&text), b"\r".to_vec(), b"typed".to_vec()].concat();
    let dir = std::env::temp_dir().join(format!("pwlp-{}-out", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("got.bin");
    let exe = std::env::current_exe().unwrap();
    // Raw: every byte as it is (no line editing, no echo), as a TUI reads.
    let script = "stty raw -echo && printf 'ready\\r\\n' && exec \"$0\" --exact slow_reader --nocapture --test-threads=1 -q >/dev/null";
    let args = vec!["-c".to_string(), script.to_string(), exe.to_string_lossy().into_owned()];
    let want_len = want.len().to_string();
    let env = [(OUT, out.as_os_str()), (WANT, std::ffi::OsStr::new(&want_len))];
    let held = Held::start("paste", "/bin/sh", &args, &env);
    let host = held.attach();
    assert!(wait_until(10, || host.screen_text().0.contains("ready")), "the reader starts");

    // As Next up / `queue send` do: the paste, Enter; then someone types.
    let t0 = Instant::now();
    host.send_text(text);
    host.write(b"typed").unwrap();
    assert!(t0.elapsed() < Duration::from_millis(250), "queued without waiting for the reader: {:?}", t0.elapsed());

    let size = || std::fs::metadata(&out).map(|m| m.len() as usize).unwrap_or(0);
    assert!(wait_until(120, || size() >= want.len()), "{} of {} bytes arrived", size(), want.len());
    let got = std::fs::read(&out).unwrap();
    assert!(got == want, "the bytes differ (got {}, want {})", got.len(), want.len());
    drop(held);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bracketed_paste_survives_more_output_than_the_ring_holds() {
    // Like Claude Code: bracketed paste on once at the start, then lots of
    // output (more than the holder's and the engine's 1 MiB rings).
    let script = "printf '\\033[?2004h\\033[?25l'; i=0; while [ $i -lt 12000 ]; do \
                  echo \"output line $i ..........................................................................................\"; \
                  i=$((i+1)); done; echo DONE; exec sleep 60";
    let args = vec!["-c".to_string(), script.to_string()];
    let held = Held::start("modes", "/bin/sh", &args, &[]);
    let first = held.attach();
    assert!(wait_until(30, || first.screen_text().0.contains("DONE")), "the output ends");
    let history = first.history();
    assert!(history.len() > 1024 * 1024);
    assert!(history.starts_with(b"\x1b[?25l\x1b[?2004h"), "the engine's ring keeps the modes");
    drop(first);

    // A terminal that attaches now (the app restarted): the holder's replay
    // starts with the modes, though the sequences left its ring long ago.
    let second = held.attach();
    let history = second.history();
    assert!(history.starts_with(b"\x1b[?25l\x1b[?2004h"), "replay starts with {:?}", String::from_utf8_lossy(&history[..40]));
    assert!(!history.windows(14).any(|w| w == b"output line 0 "), "the start is no longer in the ring");
}
