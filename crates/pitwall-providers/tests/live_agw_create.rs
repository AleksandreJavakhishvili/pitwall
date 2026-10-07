//! Live check of creating an agw session from Pitwall (step 8 slice c).
//! NOT run by default, and only when you ask for it: it creates and then
//! deletes a real session on the VM. The VM and workspace come only from the
//! environment:
//!
//! `PITWALL_AGW_LIVE_CREATE=<vm> PITWALL_AGW_LIVE_WORKSPACE=<workspace> cargo test -p pitwall-providers --test live_agw_create -- --ignored --nocapture`
//!
//! Steps: `pitwall-test` must not exist on the VM → the provider's form
//! for it (read-only listings; the workspace must be offered) →
//! `create` = `agw --non-interactive session create --vm <vm>
//! --workspace <workspace> --template <form default> --admin pitwall-test` →
//! attach (holder running `agw session attach pitwall-test`) → output
//! arrives and the screen has something on it (nothing is typed) → close the
//! attachment and `remove` (Pitwall only detaches; the session still runs)
//! → cleanup: `agw --non-interactive session delete pitwall-test --yes`
//! (admin mode in an existing workspace: no workspace or agent user is made,
//! so nothing else needs deleting) → gone from `agw session list`. A guard
//! runs the same delete if anything fails after the create.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pitwall_core::clock::SystemClock;
use pitwall_core::kind::custom_kind;
use pitwall_core::provider::{CreateSpec, LaunchIntent, LaunchSpec, MachineId, NativeState, Provider, TermSize};
use pitwall_core::term::TermHost;
use pitwall_providers::agw::{AgwConfig, AgwProvider};

const SESSION: &str = "pitwall-test";

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

/// The cleanup: agw's own delete of exactly this session.
fn delete_session() -> bool {
    eprintln!("cleanup: agw --non-interactive session delete {SESSION} --yes");
    Command::new("agw").args(["--non-interactive", "session", "delete", SESSION, "--yes"]).status().is_ok_and(|s| s.success())
}

/// Deletes `pitwall-test` when dropped (armed once this test created it).
struct DeleteGuard(bool);
impl Drop for DeleteGuard {
    fn drop(&mut self) {
        if self.0 {
            delete_session();
        }
    }
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

#[test]
#[ignore = "live: creates and deletes the agw session `pitwall-test` on the VM named by PITWALL_AGW_LIVE_CREATE"]
fn live_agw_create_attach_and_clean_up() {
    let (Ok(vm_name), Ok(workspace)) = (std::env::var("PITWALL_AGW_LIVE_CREATE"), std::env::var("PITWALL_AGW_LIVE_WORKSPACE")) else {
        eprintln!("set PITWALL_AGW_LIVE_CREATE=<vm> and PITWALL_AGW_LIVE_WORKSPACE=<workspace> to run");
        return;
    };
    let run = std::env::temp_dir().join(format!("pwlivec-{}", std::process::id()));
    let p = AgwProvider::new(AgwConfig::new(run.join("hold"), Ok(holder())));
    let vm = MachineId::new(&vm_name);
    assert!(
        !p.sessions(&vm_name, false).expect("agw session list").iter().any(|s| s.name == SESSION),
        "{SESSION} already exists on {vm_name}: not touching it"
    );

    let form = p.create_form(&vm).expect("form");
    eprintln!("form error: {:?}", form.error);
    assert!(!form.folder);
    let ws = form.field("workspace").expect("workspace field");
    assert!(ws.choices.iter().any(|c| c.value == workspace), "workspace {workspace} is offered");
    let chosen: BTreeMap<String, String> = [("workspace", workspace.as_str()), ("runAs", "admin")].into_iter().map(|(k, v)| (k.into(), v.into())).collect();
    let options = form.values(&chosen).expect("options fit the form");
    let summary = form.summarize(SESSION, &options);
    eprintln!("{}", summary.text);
    assert!(summary.creates.is_empty(), "nothing but the session is made");

    let kind = custom_kind("unused");
    let size = TermSize::new(120, 40);
    let spec = CreateSpec {
        machine: &vm,
        name: SESSION,
        options: &options,
        launch: LaunchSpec { agent: "live-create", kind: &kind, cwd: "", intent: LaunchIntent::Fresh, worktree: None, size, hooks: None },
    };
    let mut guard = DeleteGuard(true);
    let started = p.create(&spec).expect("agw session create");
    eprintln!("created {} kind={:?} cwd={:?}", started.locator, started.kind, started.cwd);
    let loc = started.locator.clone();
    assert_eq!(p.state(&loc).expect("state"), NativeState::Running, "create also starts it");

    let host = TermHost::new(p.attach(&loc, size).expect("attach"), size, Arc::new(SystemClock::new()));
    let got_output = wait("output on the attachment", 30, || host.history().len() > 100);
    let screen = host.screen_text().0;
    let lines = screen.lines().filter(|l| !l.trim().is_empty()).count();
    eprintln!("screen: {lines} non-empty lines; capture: {:?}", p.capture(&loc).map(|s| s.lines().filter(|l| !l.trim().is_empty()).count()));
    host.close(Duration::from_millis(1500));
    p.remove(&loc).expect("remove only detaches");
    let still = p.state(&loc).expect("state after remove");

    guard.0 = false;
    let deleted = delete_session();
    let gone = !p.sessions(&vm_name, false).expect("agw session list").iter().any(|s| s.name == SESSION);
    let _ = std::fs::remove_dir_all(&run);
    assert!(got_output && lines > 0, "the new session's screen arrived");
    assert_eq!(still, NativeState::Running, "removing it from Pitwall left it running");
    assert!(deleted && gone, "cleaned up: {SESSION} deleted on {vm_name}");
}
