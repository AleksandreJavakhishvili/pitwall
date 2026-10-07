//! The provider contract (architecture.md §7): what every [`Provider`] must
//! do, checked the same way for each. A provider's tests implement
//! [`ContractHarness`] and call [`run`]. Terminals are exercised through the
//! core's [`TermHost`], exactly as the engine uses them.
//!
//! Checked: machines and kinds; `discover` is read-only; create (or, for a
//! provider that adopts sessions instead, attach to the harness's existing
//! session, which `discover` lists) → output arrives, input echoes, resize reaches the process (`stty size`); a second
//! subscriber gets the replay; a client re-attach (Pitwall restarting) finds
//! the same process with its history and input still working; `exec` (and
//! `exec_at` the agent) runs git in the workspace, files round-trip, `temp_dir` is writable;
//! `process_cwd`, `capture`, `attach` and `start(Resume)` answer
//! `Unsupported` exactly when their capability is false; stop → `Stopped`
//! (or `Gone`) with EOF as `eof_is_exit` declares; start again (`caps.start`)
//! and an agent that exits by itself reports its exit code; resume continues
//! the conversation; remove.
//!
//! Only harmless commands run (an interactive POSIX shell), in the
//! harness's scratch workspace.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::clock::{Clock, SystemClock};
use crate::error::ErrorCode;
use crate::exec::{self, Cmd};
use crate::kind::{AgentKind, KindCatalog};
use crate::provider::{
    CreateSpec, LaunchIntent, LaunchSpec, Locator, MachineId, NativeState, Provider, Started, TermSize,
};
use crate::term::TermHost;

/// What a provider's test supplies.
pub trait ContractHarness {
    fn provider(&self) -> Arc<dyn Provider>;
    fn machine(&self) -> MachineId;
    /// A scratch folder on the machine: a git repository with one commit.
    fn workspace(&self) -> String;
    /// How long output may take to show up.
    fn patience(&self) -> Duration {
        Duration::from_secs(20)
    }
    /// For a provider that adopts sessions instead of creating them
    /// (`caps.create` false, `caps.attach_existing`): a running session on
    /// `machine()` that runs [`shell_kind`]'s shell in `workspace()`. The
    /// suite types into it, stops, starts and removes it.
    fn existing(&self) -> Option<Locator> {
        None
    }
    /// For a provider whose machines have their own create form (a
    /// platform, `caps.create`): the options that make a session running the
    /// contract's shell in `workspace()`. Empty for Pitwall's folder form.
    fn create_options(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
}

/// A kind that runs an interactive POSIX shell; resuming prints
/// `resumed:<id>` first. Nothing else is ever started by the contract.
pub fn shell_kind() -> AgentKind {
    AgentKind {
        id: "contract-sh".into(),
        name: "Contract shell".into(),
        command: "/bin/sh".into(),
        new_args: vec!["-i".into()],
        resume_args: vec!["-c".into(), "echo resumed:{session_id}; exec /bin/sh -i".into()],
        assign_session_id: false,
        hooks: crate::kind::HookMode::None,
        rulesync_target: None,
        worktree_args: vec![],
        worktree_dirs: vec![],
        process_names: vec![],
        aliases: vec![],
    }
}

fn clock() -> Arc<dyn Clock> {
    Arc::new(SystemClock::new())
}

fn text(h: &TermHost) -> String {
    String::from_utf8_lossy(&h.history()).into_owned()
}

/// Wait until `host`'s output contains `needle`.
fn wait_for(h: &TermHost, needle: &str, patience: Duration, what: &str) -> String {
    let deadline = Instant::now() + patience;
    while !text(h).contains(needle) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let t = text(h);
    assert!(t.contains(needle), "contract: {what}: no {needle:?} in output:\n{t}");
    t
}

fn wait_until(patience: Duration, what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + patience;
    while !f() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(f(), "contract: {what}");
}

fn unsupported<T>(r: crate::error::Result<T>, what: &str) {
    match r {
        Err(e) if e.is(ErrorCode::Unsupported) => {}
        Err(e) => panic!("contract: {what} is not a capability, so it must answer Unsupported, got {:?}: {e}", e.code),
        Ok(_) => panic!("contract: {what} is not a capability, so it must answer Unsupported, but it worked"),
    }
}

/// The live terminal of a started agent (from `start`, else `attach`).
fn host_of(p: &dyn Provider, started: Started, size: TermSize) -> Arc<TermHost> {
    let io = match started.term {
        Some(t) => t,
        None => p.attach(&started.locator, size).expect("contract: attach after start"),
    };
    TermHost::new(io, size, clock())
}

fn launch<'a>(agent: &'a str, kind: &'a AgentKind, cwd: &'a str, intent: LaunchIntent, size: TermSize) -> LaunchSpec<'a> {
    LaunchSpec { agent, kind, cwd, intent, worktree: None, size, hooks: None }
}

/// Run the whole contract against `h`'s provider. Panics on the first
/// broken promise, naming it.
pub fn run(h: &dyn ContractHarness) {
    let p = h.provider();
    let p = &*p;
    let caps = p.caps();
    let m = h.machine();
    let ws = h.workspace();
    let patience = h.patience();
    let kind = shell_kind();
    let tag = &uuid::Uuid::new_v4().to_string()[..8];
    let agent = format!("contract-{tag}");

    // Machines, kinds, discover.
    let machines = p.machines().expect("contract: machines");
    assert!(machines.iter().any(|x| x.id == m), "contract: machines() lists the harness machine");
    let catalog = KindCatalog::new(std::path::PathBuf::from("/nonexistent/pitwall-contract-agents"));
    let kinds = p.kinds(&m, &catalog).expect("contract: kinds");
    assert!(!kinds.is_empty(), "contract: kinds() lists the catalog's kinds");
    let before = p.discover(&m).expect("contract: discover");
    assert_eq!(before, p.discover(&m).expect("contract: discover again"), "contract: discover is read-only");

    // Create, or adopt the harness's session.
    let size = TermSize::new(100, 30);
    let options = h.create_options();
    let form = p.create_form(&m).expect("contract: create_form");
    if caps.create {
        assert_eq!(form.folder, !caps.platform_create, "contract: caps.platform_create says whether the form has a folder");
    }
    if form.folder {
        assert!(options.is_empty(), "contract: a folder form takes no options");
    } else {
        form.values(&options).expect("contract: the harness's options fit the provider's form");
    }
    let spec = CreateSpec {
        machine: &m,
        name: &agent,
        options: &options,
        launch: launch(&agent, &kind, &ws, LaunchIntent::Fresh, size),
    };
    let (loc, host) = if caps.create {
        let started = p.create(&spec).expect("contract: create");
        let loc: Locator = started.locator.clone();
        assert!(!started.resumed, "contract: a fresh start is not a resume");
        (loc, host_of(p, started, size))
    } else {
        unsupported(p.create(&spec), "create");
        if !caps.attach_existing {
            return;
        }
        let loc = h.existing().expect("contract: a provider that adopts sessions needs ContractHarness::existing");
        let listed = before.iter().find(|d| d.locator == loc).expect("contract: discover lists the existing session");
        assert_eq!(listed.state, Some(NativeState::Running), "contract: discover says it runs");
        (loc.clone(), TermHost::new(p.attach(&loc, size).expect("contract: attach to an existing session"), size, clock()))
    };
    assert_eq!(loc.provider, *p.id(), "contract: the locator names its provider");
    assert_eq!(loc.machine, m, "contract: the locator names its machine");
    assert_eq!(p.state(&loc).expect("contract: state"), NativeState::Running, "contract: created → Running");

    // Input echoes; output arrives.
    host.write(format!("echo pw-{tag}\r").as_bytes()).expect("contract: write");
    wait_for(&host, &format!("\npw-{tag}"), patience, "echo output");

    // Resize reaches the process.
    assert!(host.resize(90, 33));
    std::thread::sleep(Duration::from_millis(100));
    host.write(b"stty size\r").expect("contract: write");
    wait_for(&host, "33 90", patience, "stty size after resize");

    // A second subscriber gets the replay.
    let got = Arc::new(Mutex::new(Vec::new()));
    let g = got.clone();
    let sub = host.attach(Box::new(move |b: &[u8]| {
        g.lock().unwrap().extend_from_slice(b);
        true
    }));
    let replay = String::from_utf8_lossy(&got.lock().unwrap()).into_owned();
    assert!(replay.contains(&format!("\npw-{tag}")), "contract: a new subscriber gets the replay:\n{replay}");
    host.detach(sub);

    // Where it works; its screen.
    let pid = host.pid();
    match caps.process_cwd {
        true => {
            let x = p.exec(&m).expect("contract: exec");
            let want = x.real_path(&ws).unwrap_or(ws.clone());
            let deadline = Instant::now() + patience;
            let mut cwd = None;
            while Instant::now() < deadline {
                cwd = p.process_cwd(&loc, pid).expect("contract: process_cwd");
                if cwd.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            let cwd = cwd.expect("contract: process_cwd finds the running agent");
            assert_eq!(x.real_path(&cwd).unwrap_or(cwd), want, "contract: process_cwd is the workspace");
        }
        false => unsupported(p.process_cwd(&loc, pid), "process_cwd"),
    }
    match caps.capture {
        true => assert!(p.capture(&loc).expect("contract: capture").contains(&format!("pw-{tag}"))),
        false => unsupported(p.capture(&loc), "capture"),
    }

    // Pitwall goes away and comes back: the same process, its history, input.
    if caps.survives_detach {
        let again = TermHost::new(p.attach(&loc, size).expect("contract: re-attach"), size, clock());
        assert_eq!(again.pid(), pid, "contract: re-attach finds the same process (no respawn)");
        let seen = text(&again);
        assert_eq!(seen.matches(&format!("\npw-{tag}")).count(), 1, "contract: history replayed once on re-attach:\n{seen}");
        again.write(format!("echo again-{tag}\r").as_bytes()).expect("contract: write after re-attach");
        wait_for(&again, &format!("\nagain-{tag}"), patience, "input after re-attach");
        wait_for(&host, &format!("\nagain-{tag}"), patience, "the first client sees it too");
    } else if !caps.attach_existing {
        unsupported(p.attach(&loc, size), "attach");
    }

    // Commands and files where the agent works.
    if caps.exec {
        let x = p.exec(&m).expect("contract: exec");
        let out = x.run(&Cmd::new(&["git", "status", "--porcelain"]).cwd(&ws)).expect("contract: git status runs");
        assert!(out.ok(), "contract: git status in the workspace: {}", out.stderr_text());
        // Where this agent works, as the user it runs as.
        let at = p.exec_at(&loc).expect("contract: exec_at");
        let out = at.run(&Cmd::new(&["git", "status", "--porcelain"]).cwd(&ws)).expect("contract: git status runs as the agent");
        assert!(out.ok(), "contract: git status as the agent: {}", out.stderr_text());
        let tmp = x.temp_dir().expect("contract: temp_dir");
        let f = exec::join(&tmp, &format!("pitwall-contract-{tag}.txt"));
        x.write_file(&f, b"round trip").expect("contract: temp_dir is writable");
        assert_eq!(x.read_file(&f, 100).expect("contract: read_file").as_deref(), Some(&b"round trip"[..]));
        x.remove_file(&f).expect("contract: remove_file");
        assert_eq!(x.read_file(&f, 100).expect("contract: read_file"), None);
    } else {
        unsupported(p.exec(&m).map(|_| ()), "exec");
    }

    // Stop.
    if host.eof_is_exit() {
        host.close(Duration::from_millis(500));
    }
    p.stop(&loc).expect("contract: stop");
    wait_until(patience, "stop ends the terminal", || host.ended());
    assert_ne!(p.state(&loc).expect("contract: state"), NativeState::Running, "contract: stopped → not Running");
    if host.eof_is_exit() {
        assert!(host.exit().is_some(), "contract: a stopped local agent reports its exit");
    }
    let after_stop = p.attach(&loc, size);
    assert!(after_stop.is_err(), "contract: nothing to attach to after stop");

    // Started again, it exits by itself with its code.
    let fresh = launch(&agent, &kind, &ws, LaunchIntent::Fresh, size);
    if caps.start {
        let restarted = p.start(&loc, &fresh).expect("contract: start");
        assert_eq!(restarted.locator, loc, "contract: start keeps the locator");
        let host = host_of(p, restarted, size);
        host.write(format!("echo up-{tag}\r").as_bytes()).expect("contract: write");
        wait_for(&host, &format!("\nup-{tag}"), patience, "restarted agent echoes");
        host.write(b"exit 3\r").expect("contract: write");
        wait_until(patience, "an exiting agent ends its terminal", || host.ended());
        if host.eof_is_exit() {
            wait_until(patience, "exit code 3 is reported", || host.exit().and_then(|e| e.code) == Some(3));
        }
        wait_until(patience, "an exited agent is not Running", || p.state(&loc).map(|s| s != NativeState::Running).unwrap_or(false));
    } else {
        unsupported(p.start(&loc, &fresh).map(|_| ()), "start");
    }

    // Resume continues the conversation, exactly when the provider can.
    let resume = launch(&agent, &kind, &ws, LaunchIntent::Resume(format!("conv-{tag}")), size);
    if caps.resume {
        let s = p.start(&loc, &resume).expect("contract: start(Resume)");
        assert!(s.resumed, "contract: start(Resume) with a resumable kind resumes");
        assert_eq!(s.conversation_id.as_deref(), Some(&*format!("conv-{tag}")));
        let host = host_of(p, s, size);
        wait_for(&host, &format!("resumed:conv-{tag}"), patience, "the kind's resume args ran");
        host.close(Duration::from_millis(500));
        p.stop(&loc).expect("contract: stop");
        wait_until(patience, "stop ends the resumed agent", || host.ended());
    } else {
        unsupported(p.start(&loc, &resume).map(|_| ()), "start(Resume)");
    }

    // Remove.
    p.remove(&loc).expect("contract: remove");
    assert_ne!(p.state(&loc).expect("contract: state"), NativeState::Running, "contract: removed → not Running");
}
