//! Every provider passes pitwall-core's provider contract
//! (`testing::contract`, architecture.md §7): here the local provider, with
//! a real `pitwall-hold` holder; the agw provider against a fake `agw`
//! (`tests/fake_agw/`: shell scripts, one fake VM, sessions as files);
//! and core's `FakeProvider` (so the suite itself is checked against a
//! provider without processes).
//!
//! Safety: only `/bin/sh` runs (the contract's shell kind, the fake agw), in
//! temp folders; holder sockets and the hook socket live in a temp dir, never
//! in `~/.pitwall`. The real agw is never run here except by the `#[ignore]`d
//! read-only check at the end. Processes are only ever ended by their exact
//! pid.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

use pitwall_core::exec::LocalExec;
use pitwall_core::provider::{Locator, MachineId, Provider, ProviderCaps};
use pitwall_core::testing::contract::{self, ContractHarness};
use pitwall_core::testing::{FakeProvider, TempDir};
use pitwall_providers::agw::{AgwConfig, AgwProvider};
use pitwall_providers::local::{LocalConfig, LocalProvider};

/// The holder binary: `PITWALL_HOLD_BIN`, else built once for these tests
/// into its own target dir (so it never waits on the outer build's lock).
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
        target.join("debug").join(format!("pitwall-hold{}", std::env::consts::EXE_SUFFIX))
    })
    .clone()
}

/// A scratch git repository with one commit.
fn repo(dir: &Path) -> String {
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("git")
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q"]);
    std::fs::write(dir.join("README"), "contract\n").unwrap();
    git(&["add", "."]);
    git(&["-c", "user.name=pw", "-c", "user.email=pw@example.invalid", "commit", "-q", "-m", "init"]);
    dir.to_string_lossy().into_owned()
}

struct Local {
    provider: Arc<LocalProvider>,
    workspace: String,
    run: PathBuf,
    _dir: TempDir,
}

impl Drop for Local {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.run);
    }
}

impl Local {
    fn new() -> Local {
        let dir = TempDir::new("contract-local");
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        // Short socket paths: Unix sockets have a small path limit.
        let run = std::env::temp_dir().join(format!("pwc-{}", &uuid_ish()));
        std::fs::create_dir_all(&run).unwrap();
        let provider = Arc::new(LocalProvider::new(LocalConfig {
            hold_dir: run.join("hold"),
            holder_bin: Ok(holder()),
            hook_socket: run.join("hook.sock"),
            cli_socket: run.join("cli.sock"),
        }));
        Local { provider, workspace: repo(&ws), run, _dir: dir }
    }
}

fn uuid_ish() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos();
    format!("{}-{n:x}", std::process::id())
}

impl ContractHarness for Local {
    fn provider(&self) -> Arc<dyn Provider> {
        self.provider.clone()
    }
    fn machine(&self) -> MachineId {
        MachineId::new(MachineId::THIS_MAC)
    }
    fn workspace(&self) -> String {
        self.workspace.clone()
    }
}

#[test]
fn local_provider_passes_the_contract() {
    contract::run(&Local::new());
}

/// An agent started on this Mac can find Pitwall: its id, the hook socket
/// and the CLI's socket are in its environment (the `pitwall` CLI run
/// inside it connects there).
#[test]
fn local_agents_know_how_to_reach_pitwall() {
    use pitwall_core::clock::SystemClock;
    use pitwall_core::provider::{CreateSpec, LaunchIntent, LaunchSpec, TermSize};
    use pitwall_core::term::TermHost;
    let l = Local::new();
    let kind = contract::shell_kind();
    let size = TermSize::new(100, 30);
    let started = l
        .provider
        .create(&CreateSpec {
            machine: &MachineId::new(MachineId::THIS_MAC),
            name: "env",
            options: &Default::default(),
            launch: LaunchSpec { agent: "env-agent", kind: &kind, cwd: &l.workspace, intent: LaunchIntent::Fresh, worktree: None, size, hooks: None },
        })
        .unwrap();
    let loc = started.locator.clone();
    let host = TermHost::new(started.term.expect("a local terminal"), size, Arc::new(SystemClock::new()));
    host.write(b"echo \"pw[$PITWALL_ENV|$PITWALL_AGENT_ID|$PITWALL_CLI_SOCKET]\"\r").unwrap();
    let want = format!("pw[1|env-agent|{}]", l.run.join("cli.sock").display());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !String::from_utf8_lossy(&host.history()).contains(&want) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = String::from_utf8_lossy(&host.history()).into_owned();
    host.close(std::time::Duration::from_millis(500));
    let _ = l.provider.remove(&loc);
    assert!(out.contains(&want), "no {want:?} in:\n{out}");
}

struct Fake {
    provider: Arc<FakeProvider>,
    workspace: String,
    _dir: TempDir,
}

impl ContractHarness for Fake {
    fn provider(&self) -> Arc<dyn Provider> {
        self.provider.clone()
    }
    fn machine(&self) -> MachineId {
        self.provider.machine_id().clone()
    }
    fn workspace(&self) -> String {
        self.workspace.clone()
    }
}

fn fake(caps: Option<ProviderCaps>, remote: bool) -> Fake {
    let dir = TempDir::new("contract-fake");
    let workspace = repo(dir.path());
    let provider = FakeProvider::new(Arc::new(LocalExec));
    if let Some(c) = caps {
        provider.set_caps(c);
    }
    provider.set_remote(remote);
    Fake { provider, workspace, _dir: dir }
}

#[test]
fn fake_provider_passes_the_contract() {
    contract::run(&fake(None, false));
}

/// tmux-like: terminals come from `attach`, EOF is a dropped attachment,
/// and fewer capabilities (each must answer `Unsupported`).
#[test]
fn a_remote_like_provider_with_fewer_caps_passes_too() {
    let caps = ProviderCaps { create: true, start: true, survives_detach: true, capture: true, ..Default::default() };
    contract::run(&fake(Some(caps), true));
}

// ------------------------------------------------------------------ agw

/// The agw provider against the fake agw in `tests/fake_agw/`: the contract
/// creates its session in workspace "work" (`agw session create`); one
/// running session ("adopt-me") is there to adopt.
struct Agw {
    provider: Arc<AgwProvider>,
    workspace: String,
    run: PathBuf,
    _dir: TempDir,
}

impl Drop for Agw {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.run);
    }
}

fn install_fake(src: &str, to: &Path, dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let body = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_agw").join(src)).unwrap();
    std::fs::write(to, body.replace("__DIR__", &dir.to_string_lossy())).unwrap();
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o755)).unwrap();
}

impl Agw {
    fn new() -> Agw {
        let dir = TempDir::new("contract-agw");
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let workspace = repo(&ws);
        let state = dir.path().join("agw");
        std::fs::create_dir_all(state.join("bin")).unwrap();
        install_fake("agw", &state.join("bin/agw"), &state);
        install_fake("ssh", &state.join("bin/ssh"), &state);
        install_fake("tmux", &state.join("bin/tmux"), &state);
        std::fs::write(state.join("adopt-me.state"), "running\n").unwrap();
        std::fs::write(state.join("adopt-me.ws"), &workspace).unwrap();
        std::fs::write(state.join("work.ws"), &workspace).unwrap();
        let run = std::env::temp_dir().join(format!("pwa-{}", uuid_ish()));
        std::fs::create_dir_all(&run).unwrap();
        let mut cfg = AgwConfig::new(run.join("hold"), Ok(holder()));
        cfg.agw = Some(state.join("bin/agw"));
        cfg.ssh = state.join("bin/ssh");
        Agw { provider: Arc::new(AgwProvider::new(cfg)), workspace, run, _dir: dir }
    }
}

impl ContractHarness for Agw {
    fn provider(&self) -> Arc<dyn Provider> {
        self.provider.clone()
    }
    fn machine(&self) -> MachineId {
        MachineId::new("fakevm")
    }
    fn workspace(&self) -> String {
        self.workspace.clone()
    }
    fn existing(&self) -> Option<Locator> {
        Some(Locator::new(self.provider.id(), &self.machine(), "adopt-me"))
    }
    fn create_options(&self) -> std::collections::BTreeMap<String, String> {
        [("workspace", "work"), ("runAs", "admin"), ("template", "contract")].into_iter().map(|(k, v)| (k.into(), v.into())).collect()
    }
}

#[test]
fn agw_provider_passes_the_contract_against_a_fake_agw() {
    contract::run(&Agw::new());
}

/// A provider that adopts sessions instead of creating them, without
/// processes: the suite's adopt path itself.
#[test]
fn an_adopting_fake_provider_passes_too() {
    let f = fake(Some(ProviderCaps { start: true, attach_existing: true, survives_detach: true, capture: true, ..Default::default() }), true);
    f.provider.add_session("adopt-me", "contract-sh", &f.workspace);
    struct Adopting(Fake);
    impl ContractHarness for Adopting {
        fn provider(&self) -> Arc<dyn Provider> {
            self.0.provider()
        }
        fn machine(&self) -> MachineId {
            self.0.machine()
        }
        fn workspace(&self) -> String {
            self.0.workspace()
        }
        fn existing(&self) -> Option<Locator> {
            Some(Locator::new(self.0.provider.id(), self.0.provider.machine_id(), "adopt-me"))
        }
    }
    contract::run(&Adopting(f));
}

/// Read-only check against the real agw on this Mac: lists VMs and
/// sessions, reads the state and screen (`tmux capture-pane` over agw's ssh
/// alias) of every running session. Never attaches, types, starts or stops.
/// `cargo test -p pitwall-providers --test contract real_agw -- --ignored --nocapture`
#[test]
#[ignore = "needs the real agw and its VMs (read-only)"]
fn real_agw_read_only_listing_and_capture() {
    let p = AgwProvider::new(AgwConfig::new(std::env::temp_dir().join("pw-agw-real-unused"), Err("no attach here".into())));
    eprintln!("agw {:?}", p.version());
    for m in p.machines().expect("agw vm list") {
        eprintln!("machine {} ({:?})", m.id, m.detail);
        for d in p.discover(&m.id).expect("agw session list") {
            eprintln!("  {} kind={} state={:?} cwd={:?} user={:?}", d.locator, d.kind, d.state, d.cwd, d.user);
            if d.state == Some(pitwall_core::provider::NativeState::Running) {
                assert_eq!(p.state(&d.locator).unwrap(), pitwall_core::provider::NativeState::Running);
                let screen = p.capture(&d.locator).expect("capture over ssh");
                let lines: Vec<_> = screen.lines().filter(|l| !l.trim().is_empty()).collect();
                eprintln!("    screen: {} non-empty lines, last: {:?}", lines.len(), lines.last());
                assert!(!lines.is_empty(), "a running session has something on screen");
            }
        }
    }
}

/// Read-only check of the attach path against one real session you
/// name: `agw session attach` in a holder at the session's current size
/// (so the window isn't resized), wait for tmux to draw the screen, then
/// detach. Nothing is typed. `PITWALL_AGW_ATTACH=<vm>/<session>
/// PITWALL_AGW_SIZE=<cols>x<rows> cargo test -p pitwall-providers --test
/// contract real_agw_attach -- --ignored --nocapture`
#[test]
#[ignore = "attaches to a real agw session (read-only; needs PITWALL_AGW_ATTACH)"]
fn real_agw_attach_receives_output_without_typing() {
    use pitwall_core::provider::TermSize;
    let Some(target) = std::env::var("PITWALL_AGW_ATTACH").ok() else { return };
    let (vm, name) = target.split_once('/').expect("<vm>/<session>");
    let (c, r) = std::env::var("PITWALL_AGW_SIZE").ok().and_then(|s| {
        let (c, r) = s.split_once('x')?;
        Some((c.parse().ok()?, r.parse().ok()?))
    }).unwrap_or((120, 40));
    let run = std::env::temp_dir().join(format!("pwr-{}", uuid_ish()));
    let p = AgwProvider::new(AgwConfig::new(run.join("hold"), Ok(holder())));
    let loc = Locator::new(p.id(), &MachineId::new(vm), name);
    let size = TermSize::new(c, r);
    let host = pitwall_core::term::TermHost::new(p.attach(&loc, size).expect("attach"), size, Arc::new(pitwall_core::clock::SystemClock::new()));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while host.history().len() < 200 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let got = host.history().len();
    let (screen, title) = host.screen_text();
    host.close(std::time::Duration::from_millis(1500));
    let _ = std::fs::remove_dir_all(&run);
    eprintln!("received {got} bytes; title {title:?}; screen tail:");
    for l in screen.lines().filter(|l| !l.trim().is_empty()).rev().take(4) {
        eprintln!("  {l}");
    }
    assert!(got >= 200, "tmux redraws the screen on attach");
}
