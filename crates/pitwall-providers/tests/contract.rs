//! Every provider passes pitwall-core's provider contract
//! (`testing::contract`, architecture.md §7): here the local provider, with
//! a real `pitwall-hold` holder, and core's `FakeProvider` (so the suite
//! itself is checked against a provider without processes).
//!
//! Safety: only `/bin/sh` runs (the contract's shell kind), in a temp git
//! repo; holder sockets and the hook socket live in a temp dir, never in
//! `~/.pitwall`. Processes are only ever ended by their exact pid.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};

use pitwall_core::exec::LocalExec;
use pitwall_core::provider::{MachineId, Provider, ProviderCaps};
use pitwall_core::testing::contract::{self, ContractHarness};
use pitwall_core::testing::{FakeProvider, TempDir};
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
    let caps = ProviderCaps { create: true, survives_detach: true, capture: true, ..Default::default() };
    contract::run(&fake(Some(caps), true));
}
