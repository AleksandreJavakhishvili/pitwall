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
    server: Option<Handle>,
}

impl Host {
    /// Open the engine on `paths` and start its background work and the CLI
    /// socket. Call only after `home::choose` said the folder is ours.
    pub fn start(paths: Paths, bridge: Bridge) -> Host {
        let holder = holder_bin()
            .map(|b| crate::packaging::stable_sidecar(b, &paths.root().join("bin")));
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
        let cfg = Config {
            socket: paths.cli_socket(),
            version: env!("CARGO_PKG_VERSION").into(),
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
}
