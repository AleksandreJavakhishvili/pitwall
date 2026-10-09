//! A terminal fed by a real `pitwall-hold` holder (started here, in a temp
//! dir, running a harmless `/bin/sh` script; stopped by exact pid).
//!
//! Needs the holder binary: `cargo build -p pitwall-hold`, then
//! `PITWALL_HOLD_BIN=<target>/debug/pitwall-hold cargo test -p pitwall-term-view --features holder --test holder -- --ignored`.
#![cfg(all(unix, feature = "holder"))]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pitwall_hold::client::{self, Launch};
use pitwall_hold::proto;
use pitwall_term_view::holder::HolderStream;
use pitwall_term_view::{TermSize, Terminal, TerminalConfig};

fn wait_for(t: &Terminal, text: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if t.screen_text().contains(text) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
#[ignore = "needs PITWALL_HOLD_BIN (a built pitwall-hold)"]
fn a_holder_feeds_the_terminal_and_takes_input_and_resizes() {
    let bin = PathBuf::from(std::env::var("PITWALL_HOLD_BIN").expect("PITWALL_HOLD_BIN"));
    // Short path: Unix socket paths are limited to ~104 bytes on macOS.
    let dir = std::env::temp_dir().join(format!("pwtv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("t.sock");
    let script = "printf 'ready\\n'; read line; printf 'got:%s\\n' \"$line\"; stty size; read _";
    let args: Vec<OsString> = vec!["-c".into(), script.into()];
    let mut cmd = client::command(
        &bin,
        &Launch {
            socket: &socket,
            cols: 80,
            rows: 24,
            cwd: Some(&dir),
            grace: Some(Duration::from_millis(200)),
            program: "/bin/sh".as_ref(),
            args: &args,
        },
    );
    let (holder, child) = client::launch(&mut cmd).expect("holder starts");

    let result = std::panic::catch_unwind(|| {
        let stream = HolderStream::connect(Path::new(&socket), true).expect("attach");
        let t = Terminal::new(stream, TermSize::new(80, 24), TerminalConfig::default());
        assert!(wait_for(&t, "ready"), "{}", t.screen_text());
        t.resize(TermSize::new(90, 20));
        t.write(b"hi there\r");
        assert!(wait_for(&t, "got:hi there"), "{}", t.screen_text());
        assert!(wait_for(&t, "20 90"), "{}", t.screen_text());
    });

    // Stop the holder this test started.
    if let Ok(mut c) = client::connect(&socket, Duration::from_secs(1)) {
        let _ = c.send(&proto::shutdown(200));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while client::alive(holder) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    if client::alive(holder) {
        client::force_kill(child);
        client::force_kill(holder);
    }
    let _ = std::fs::remove_dir_all(&dir);
    result.unwrap();
}
