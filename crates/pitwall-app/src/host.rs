//! Hosting `pitwall-core`'s Engine in this process, as `src-tauri/src/lib.rs`
//! `setup` does: the same providers (local holders, agw), the same file store,
//! the `pitwall` CLI's socket server with its approvals, and the hook relay
//! (started by `Engine::start`). Only the event sink differs: a channel into
//! GPUI ([`Bridge`]) instead of webview events.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pitwall_core::clock::SystemClock;
use pitwall_core::paths::Paths;
use pitwall_core::store::FileStore;
use pitwall_core::{Deps, Engine, Shared};
use pitwall_daemon::{Approvals, Config, Handle, ProcessIdentity};
use pitwall_providers::agw::{AgwConfig, AgwProvider};
use pitwall_providers::local::{LocalConfig, LocalProvider};

use crate::bridge::{AppEvent, Bridge};

/// How long a CLI request waits for the user before it is denied (as in
/// `src-tauri/src/server.rs`).
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(120);

/// The running engine and what serves it.
pub struct Host {
    pub engine: Shared,
    pub approvals: Arc<Approvals>,
    /// `pitwall settings` (settings::backend).
    pub settings: Arc<crate::settings::backend::AppSettings>,
    /// `pitwall space` (cli_spaces).
    pub workspace: Arc<crate::cli_spaces::AppWorkspace>,
    server: Option<Handle>,
}

impl Host {
    /// Open the engine on `paths` and start its background work and the CLI
    /// socket. Call only after `home::choose` said the folder is ours.
    pub fn start(paths: Paths, bridge: Bridge) -> Host {
        Host::start_with_holder(paths, bridge, holder_bin())
    }

    /// [`Host::start`] with a given holder binary (tests).
    fn start_with_holder(paths: Paths, bridge: Bridge, holder: Result<PathBuf, String>) -> Host {
        let holder = holder.map(|b| crate::packaging::stable_sidecar(b, &paths.root().join("bin")));
        let local = LocalProvider::new(LocalConfig {
            hold_dir: paths.hold_dir(),
            holder_bin: holder.clone(),
            hook_socket: paths.hook_socket(),
            cli_socket: paths.cli_socket(),
        });
        let agw = AgwProvider::new(AgwConfig::new(paths.hold_dir(), holder));
        let engine = Engine::open(Deps {
            store: Arc::new(FileStore::new(paths.state_file())),
            paths: paths.clone(),
            events: Arc::new(bridge.clone()),
            clock: Arc::new(SystemClock::new()),
            providers: vec![Arc::new(local), Arc::new(agw)],
        });
        engine.start();

        let approvals = Approvals::new(APPROVAL_TIMEOUT);
        approvals.on_change(move |list| bridge.send(AppEvent::Approvals(list)));
        let identify = ProcessIdentity::new(engine.clone());
        let settings = crate::settings::backend::AppSettings::new(Some(engine.clone()));
        let workspace = crate::cli_spaces::AppWorkspace::new();
        let cfg = Config {
            socket: paths.cli_socket(),
            version: env!("CARGO_PKG_VERSION").into(),
            settings: Some(settings.clone()),
            workspace: Some(workspace.clone()),
        };
        let server = match pitwall_daemon::serve(engine.clone(), approvals.clone(), identify, cfg) {
            Ok(h) => Some(h),
            Err(e) => {
                eprintln!("pitwall: the command-line socket is off: {e}");
                None
            }
        };
        Host {
            engine,
            approvals,
            settings,
            workspace,
            server,
        }
    }

    /// Quitting: save, and close the CLI socket. Agents keep running in
    /// their holders (as with the Tauri app's ⌘Q).
    pub fn shutdown(&mut self) {
        let _ = self.engine.save();
        if let Some(server) = self.server.take() {
            server.stop();
        }
    }
}

/// The `pitwall-hold` terminal holder: `$PITWALL_HOLD_BIN`, next to this
/// executable (a bundle, or `target/<profile>/` after a workspace build), or
/// where `build.rs` built it. Same order as `src-tauri/src/holder.rs`.
pub fn holder_bin() -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf));
    find_holder(
        std::env::var_os("PITWALL_HOLD_BIN").map(PathBuf::from),
        exe_dir,
        option_env!("PITWALL_HOLD_BUILT"),
    )
}

fn find_holder(
    env: Option<PathBuf>,
    exe_dir: Option<PathBuf>,
    built: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(p) = env {
        return Ok(p);
    }
    let name = format!("pitwall-hold{}", std::env::consts::EXE_SUFFIX);
    let mut candidates = Vec::new();
    if let Some(dir) = exe_dir {
        candidates.push(dir.join(&name));
        if let Some(up) = dir.parent() {
            candidates.push(up.join(&name));
        }
    }
    candidates.extend(built.map(PathBuf::from));
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| "the terminal holder (pitwall-hold) is missing next to the app".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_holder_override_wins() {
        let got = find_holder(Some("/opt/x/pitwall-hold".into()), None, None).unwrap();
        assert_eq!(got, PathBuf::from("/opt/x/pitwall-hold"));
    }

    #[test]
    fn the_holder_is_found_next_to_the_app_or_where_it_was_built() {
        let dir = std::env::temp_dir().join(format!("pw-app-holder-{}", std::process::id()));
        let bin = dir.join(format!("pitwall-hold{}", std::env::consts::EXE_SUFFIX));
        std::fs::create_dir_all(dir.join("deps")).unwrap();
        std::fs::write(&bin, "").unwrap();
        assert_eq!(find_holder(None, Some(dir.clone()), None).unwrap(), bin);
        assert_eq!(
            find_holder(None, Some(dir.join("deps")), None).unwrap(),
            bin,
            "tests run from deps/"
        );
        assert_eq!(find_holder(None, None, bin.to_str()).unwrap(), bin);
        assert!(find_holder(None, Some(dir.join("a/b")), None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real engine comes up on a temp folder, serves the CLI socket and
    /// reports through the bridge; shutting down removes the socket.
    #[test]
    fn hosts_the_engine_in_a_temp_folder() {
        let dir = std::env::temp_dir().join(format!("pw-app-host-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = Paths::new(&dir);
        let (bridge, _rx) = Bridge::new();
        let mut host = Host::start(paths.clone(), bridge);
        assert!(host.engine.views().is_empty());
        assert!(
            crate::home::answers(&paths.cli_socket()),
            "the CLI socket is served"
        );
        host.shutdown();
        assert!(
            !crate::home::answers(&paths.cli_socket()),
            "and closed on shutdown"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The holder binary for the tests below: `PITWALL_HOLD_BIN`, else built
    /// once into its own target dir (as pitwall-providers' contract tests do).
    #[cfg(unix)]
    fn test_holder() -> PathBuf {
        use std::sync::OnceLock;
        static BIN: OnceLock<PathBuf> = OnceLock::new();
        BIN.get_or_init(|| {
            if let Some(p) = std::env::var_os("PITWALL_HOLD_BIN") {
                return PathBuf::from(p);
            }
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let target = root.join("target").join("hold-tests");
            let mut cmd = std::process::Command::new(env!("CARGO"));
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

    /// Everything the agent's terminal printed so far (the holder replays
    /// its buffer on attach), waiting up to 20 s for `want`.
    #[cfg(unix)]
    fn output_until(engine: &pitwall_core::Shared, id: &str, want: &str) -> String {
        use pitwall_core::engine::input;
        use std::sync::Mutex;
        let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
        let sink = seen.clone();
        let sub = input::attach_output(
            engine,
            id,
            Box::new(move |b: &[u8]| {
                sink.lock().unwrap().extend_from_slice(b);
                true
            }),
        )
        .expect("attach to the agent");
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let text = || String::from_utf8_lossy(&seen.lock().unwrap()).into_owned();
        while !text().contains(want) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        input::detach_output(engine, id, sub);
        text()
    }

    const REATTACH_DIR: &str = "PITWALL_APP_TEST_REATTACH_DIR";

    /// First half of `a_second_app_reattaches_the_first_apps_agents`, run by
    /// it in a child process (the app that started the agent, e.g. the
    /// Tauri Pitwall up to v0.1.x): start an agent, see it answer, quit.
    #[cfg(unix)]
    #[test]
    #[ignore = "run in its own process by a_second_app_reattaches_the_first_apps_agents"]
    fn first_app_starts_an_agent() {
        use pitwall_core::engine::{input, lifecycle};
        use pitwall_core::model::CreateAgentRequest;
        let Some(dir) = std::env::var_os(REATTACH_DIR).map(PathBuf::from) else {
            return;
        };
        let project = dir.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let (bridge, _rx) = Bridge::new();
        let mut host = Host::start_with_holder(Paths::new(&dir), bridge, Ok(test_holder()));
        let view = lifecycle::create(
            &host.engine,
            CreateAgentRequest {
                name: "demo".into(),
                kind: "shell".into(),
                project_path: project.to_string_lossy().into_owned(),
                ..Default::default()
            },
        )
        .expect("start a shell agent");
        input::write_input(&host.engine, &view.id, "echo first-$((40+2))\r").unwrap();
        let out = output_until(&host.engine, &view.id, "first-42");
        assert!(out.contains("first-42"), "the agent answered:\n{out}");
        host.shutdown();
        std::fs::write(dir.join("agent-id"), &view.id).unwrap();
        // The process exits here, as the app does on quit: the agent's
        // terminal lives on in its pitwall-hold process.
    }

    /// Holder continuity across apps: agents a previous Pitwall process
    /// started (in its holders, under the same data folder) are re-attached
    /// by the next one, with their output, and keep taking input. This is
    /// how the GPUI app picks up the Tauri app's agents at the switch: same
    /// folder layout (`run/hold/`), same holder binary and protocol.
    #[cfg(unix)]
    #[test]
    fn a_second_app_reattaches_the_first_apps_agents() {
        use pitwall_core::engine::{input, lifecycle};
        // Short: holder sockets live under it (104-byte Unix socket limit).
        let dir = PathBuf::from(format!("/tmp/pwra-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let holder = test_holder();
        let first = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "host::tests::first_app_starts_an_agent", "--ignored", "--nocapture", "--test-threads=1"])
            .env(REATTACH_DIR, &dir)
            .env("PITWALL_HOLD_BIN", &holder)
            // A plain shell for the made-up agent, not the user's login shell.
            .env("SHELL", "/bin/sh")
            .status()
            .expect("run the first app");
        assert!(first.success(), "the first app started its agent");
        let id = std::fs::read_to_string(dir.join("agent-id")).unwrap();

        let (bridge, _rx) = Bridge::new();
        let mut host = Host::start_with_holder(Paths::new(&dir), bridge, Ok(holder));
        let view = host.engine.views().into_iter().find(|v| v.id == id);
        let running = view.as_ref().is_some_and(|v| v.running);
        let mut out = String::new();
        if running {
            out = output_until(&host.engine, &id, "first-42");
            input::write_input(&host.engine, &id, "echo second-$((50+8))\r").unwrap();
            out.push_str(&output_until(&host.engine, &id, "second-58"));
        }
        // Clean up before asserting: end the agent and its holder.
        let _ = lifecycle::remove(&host.engine, &id, false);
        host.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(running, "re-attached and running: {view:?}");
        assert!(out.contains("first-42"), "the first app's output came back:\n{out}");
        assert!(out.contains("second-58"), "and the agent takes input:\n{out}");
    }
}
