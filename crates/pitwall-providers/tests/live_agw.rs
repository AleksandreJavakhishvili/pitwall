//! One live check of the agw provider against a real agw session you name
//! for it (a Claude Code session, stopped before and after). Nothing else is
//! touched. The VM and session come only from the environment:
//!
//! `PITWALL_AGW_LIVE_VM=<vm> PITWALL_AGW_LIVE=<session> cargo test -p pitwall-providers --test live_agw -- --ignored --nocapture`
//!
//! Steps: describe (must be stopped) → `agw session start <session>` → attach
//! (holder running `agw session attach <session>`) → screen status → type
//! "pitwall live test" WITHOUT Enter (an agent harness: nothing is ever
//! submitted), see it in the stream and the ssh capture, clear it (Ctrl-U,
//! then backspaces if needed) → restart through the provider (stop, start,
//! attach) → close the attachment and `remove` (the session keeps running)
//! → `agw session stop <session>`. A guard stops it if anything fails after
//! it was started. Processes are only ended through their holder (exact pid).

// The fake agw (and agw itself) are shell scripts / Unix tools.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pitwall_core::clock::SystemClock;
use pitwall_core::kind::custom_kind;
use pitwall_core::provider::{LaunchIntent, LaunchSpec, Locator, MachineId, NativeState, Provider, TermSize};
use pitwall_core::term::TermHost;
use pitwall_providers::agw::{AgwConfig, AgwProvider};

const TEXT: &str = "pitwall live test";

fn holder() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target").join("hold-tests");
    let mut cmd = Command::new(env!("CARGO"));
    cmd.current_dir(&root).args(["build", "-q", "-p", "pitwall-hold", "--bin", "pitwall-hold", "--target-dir"]).arg(&target);
    for key in ["RUSTC_WORKSPACE_WRAPPER", "RUSTC_WRAPPER", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET"] {
        cmd.env_remove(key);
    }
    assert!(cmd.status().expect("run cargo").success());
    target.join("debug").join("pitwall-hold")
}

fn wait(what: &str, secs: u64, f: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if f() {
            eprintln!("  ok: {what}");
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    eprintln!("  NOT within {secs}s: {what}");
    false
}

/// The visible screen's input line(s): the text after the last prompt mark.
fn screen(h: &TermHost) -> String {
    h.screen_text().0
}

/// Stops the session when dropped (only armed once this test started it).
struct StopGuard<'a>(&'a AgwProvider, &'a Locator, bool);
impl Drop for StopGuard<'_> {
    fn drop(&mut self) {
        if self.2 {
            eprintln!("guard: stopping {}", self.1);
            let _ = self.0.stop(self.1);
        }
    }
}

#[test]
#[ignore = "live: starts, types into (no Enter) and stops the agw session named by PITWALL_AGW_LIVE"]
fn live_agw_session() {
    let (Ok(vm), Ok(session)) = (std::env::var("PITWALL_AGW_LIVE_VM"), std::env::var("PITWALL_AGW_LIVE")) else {
        eprintln!("set PITWALL_AGW_LIVE_VM=<vm> and PITWALL_AGW_LIVE=<session> to run");
        return;
    };
    let run = std::env::temp_dir().join(format!("pwlive-{}", std::process::id()));
    let p = AgwProvider::new(AgwConfig::new(run.join("hold"), Ok(holder())));
    let loc = Locator::new(p.id(), &MachineId::new(&vm), &session);
    let size = TermSize::new(120, 40);
    let clock = Arc::new(SystemClock::new());

    // 1. Describe: it must be a stopped Claude Code session.
    let out = Command::new("agw").args(["session", "describe", &session, "--output", "json"]).output().expect("agw");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("describe JSON");
    let s = &v["data"]["session"];
    eprintln!("1. describe: status={} harness={} vm={}", s["status"], s["harness_integration"], s["vm_name"]);
    assert_eq!(s["vm_name"], vm.as_str());
    assert_eq!(s["status"], "stopped", "only run from a stopped session");
    let agent_harness = s["harness_integration"] != "shell";
    assert!(agent_harness, "this check is written for an agent harness (no Enter)");
    assert_eq!(p.state(&loc).unwrap(), NativeState::Stopped);

    // 2. Start through the provider.
    let kind = custom_kind("unused");
    let launch = LaunchSpec { agent: "live", kind: &kind, cwd: "", intent: LaunchIntent::Fresh, worktree: None, size, hooks: None };
    let mut guard = StopGuard(&p, &loc, false);
    let started = p.start(&loc, &launch).expect("agw session start");
    guard.2 = true;
    assert!(started.term.is_none());
    assert!(wait("state Running", 60, || p.state(&loc).ok() == Some(NativeState::Running)));
    eprintln!("2. started");

    // 3. Attach; Claude's input prompt shows up.
    let host = TermHost::new(p.attach(&loc, size).expect("attach"), size, clock.clone());
    let ready = wait("Claude's prompt on the attached screen", 60, || {
        let t = screen(&host);
        t.contains('❯') && !t.to_ascii_lowercase().contains("trust")
    });
    eprintln!("3. attached; {} bytes; screen tail:", host.history().len());
    for l in screen(&host).lines().filter(|l| !l.trim().is_empty()).rev().take(5) {
        eprintln!("   | {l}");
    }

    // 4. Screen status.
    let (text, title) = host.screen_text();
    let d = pitwall_detect::detect("claude", &text, title.as_deref());
    eprintln!("4. screen detection: {:?} {:?}", d.state, d.detail);

    // 5. Type without Enter, see it, clear it.
    if ready {
        host.write(TEXT.as_bytes()).unwrap();
        let in_stream = wait("typed text in the stream", 15, || screen(&host).contains(TEXT));
        let cap = p.capture(&loc).expect("capture");
        let in_capture = cap.contains(TEXT);
        eprintln!("5. typed (no Enter): stream={in_stream} capture={in_capture}");
        host.write(b"\x15").unwrap(); // Ctrl-U
        if !wait("cleared by Ctrl-U", 5, || !screen(&host).contains(TEXT)) {
            host.write(&[0x7f; TEXT.len()]).unwrap();
            wait("cleared by backspaces", 5, || !screen(&host).contains(TEXT));
        }
        let cleared = !screen(&host).contains(TEXT) && !p.capture(&loc).unwrap_or_default().contains(TEXT);
        eprintln!("   cleared (stream and capture): {cleared}");
        assert!(in_stream && in_capture && cleared);
    } else {
        eprintln!("5. SKIPPED typing: no plain input prompt on screen");
    }

    // 6. Restart through the provider: stop, start, attach.
    p.stop(&loc).expect("stop");
    assert!(wait("attachment ends on stop", 30, || host.ended()));
    assert!(wait("state Stopped", 30, || p.state(&loc).ok() == Some(NativeState::Stopped)));
    host.close(Duration::from_millis(500));
    p.start(&loc, &launch).expect("start again");
    assert!(wait("state Running again", 60, || p.state(&loc).ok() == Some(NativeState::Running)));
    let again = TermHost::new(p.attach(&loc, size).expect("attach again"), size, clock);
    assert!(wait("output after restart", 60, || screen(&again).contains('❯')));
    eprintln!("6. restarted through the provider");

    // 7. Detach and remove in Pitwall: the session keeps running.
    again.close(Duration::from_millis(1500));
    p.remove(&loc).expect("remove");
    std::thread::sleep(Duration::from_secs(2));
    let st = p.state(&loc).unwrap();
    eprintln!("7. after detach + remove: {st:?}");
    assert_eq!(st, NativeState::Running, "detaching/removing never stops the agw session");

    // 8. Leave it stopped, as found.
    guard.2 = false;
    p.stop(&loc).expect("final stop");
    assert!(wait("final state Stopped", 60, || p.state(&loc).ok() == Some(NativeState::Stopped)));
    let _ = std::fs::remove_dir_all(&run);
    eprintln!("8. stopped");
}
