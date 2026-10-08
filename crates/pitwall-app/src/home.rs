//! Which data folder the app runs on, and whether it may (docs/spec/gpui/README.md
//! "Data folder").
//!
//! Everything Pitwall owns — `state.json`, the hook and CLI sockets, the
//! agents' terminal holders, `ui.json` and `windows.json` — lives under one
//! folder ([`Paths`]):
//!
//! - The platform default (`~/.pitwall` on macOS, `$XDG_DATA_HOME/pitwall` on
//!   Linux, `%APPDATA%\Pitwall` on Windows): the folder the Tauri app (up to
//!   v0.1.x) used, so its agents, holders, spaces and settings carry over.
//! - `$PITWALL_HOME` overrides it (development, tests, a second copy).
//!
//! Two apps on one folder would fight over it: binding the hook socket
//! replaces the other app's, and both would drive the same holders. So the
//! app refuses when another Pitwall (a second copy of this one, or an older
//! Pitwall still running) answers on that folder's CLI socket. A refusal
//! still opens the window, with the reason in it; no engine starts.

use std::path::{Path, PathBuf};

use pitwall_core::paths::{tildify, Paths, HOME_ENV};

/// Why the app won't host an engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Another Pitwall answers on this folder's CLI socket.
    InUse { root: PathBuf },
}

impl Refusal {
    pub fn title(&self) -> &'static str {
        match self {
            Refusal::InUse { .. } => "Another Pitwall is using this data folder",
        }
    }

    pub fn explanation(&self) -> Vec<String> {
        match self {
            Refusal::InUse { root } => vec![
                format!("A running Pitwall answers on {}.", tildify(&root.to_string_lossy())),
                "Running two on one folder would take over its sockets and its agents' terminals.".into(),
                format!("Quit the other one, or start this one with {HOME_ENV} set to a separate folder."),
            ],
        }
    }
}

/// What the environment says (read once at start; a value, so tests don't
/// touch the process environment).
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// `$PITWALL_HOME`, when set and not empty.
    pub home: Option<PathBuf>,
}

impl Env {
    pub fn current() -> Env {
        Env {
            home: std::env::var_os(HOME_ENV)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        }
    }
}

/// The data folder to host the engine on, or why not. `default_root` is the
/// platform default; `in_use` asks whether a Pitwall answers on a CLI socket.
pub fn choose(
    env: &Env,
    default_root: PathBuf,
    in_use: impl Fn(&Path) -> bool,
) -> Result<Paths, Refusal> {
    let root = env.home.clone().unwrap_or(default_root);
    let paths = Paths::new(root);
    if in_use(&paths.cli_socket()) {
        return Err(Refusal::InUse {
            root: paths.root().to_path_buf(),
        });
    }
    Ok(paths)
}

/// A Pitwall answers on `socket` (a Unix socket, or a named pipe on Windows).
/// Connecting is harmless: a server that gets no hello drops the connection.
pub fn answers(socket: &Path) -> bool {
    pitwall_client::connect_raw(socket).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(home: Option<&str>) -> Env {
        Env {
            home: home.map(PathBuf::from),
        }
    }

    const DEFAULT: &str = "/data/default";

    #[test]
    fn a_separate_home_is_used() {
        let got = choose(&env(Some("/tmp/pw-preview")), DEFAULT.into(), |_| false).unwrap();
        assert_eq!(got.root(), Path::new("/tmp/pw-preview"));
    }

    #[test]
    fn the_default_folder_is_used_without_an_override() {
        let got = choose(&env(None), DEFAULT.into(), |_| false).unwrap();
        assert_eq!(got.root(), Path::new(DEFAULT));
    }

    #[test]
    fn a_folder_another_pitwall_serves_is_refused() {
        let busy = Paths::new(DEFAULT).cli_socket();
        let got = choose(&env(None), DEFAULT.into(), |s| s == busy);
        assert_eq!(
            got,
            Err(Refusal::InUse {
                root: DEFAULT.into()
            })
        );
        // The same check guards a separate folder (a second copy on it).
        let got = choose(&env(Some("/tmp/pw-2")), DEFAULT.into(), |_| true);
        assert_eq!(
            got,
            Err(Refusal::InUse {
                root: "/tmp/pw-2".into()
            })
        );
    }

    #[test]
    fn nothing_listening_means_not_in_use() {
        let dir = std::env::temp_dir().join(format!("pw-app-home-{}", std::process::id()));
        assert!(!answers(&Paths::new(&dir).cli_socket()));
    }

    #[test]
    fn refusals_explain_themselves() {
        let r = Refusal::InUse {
            root: DEFAULT.into(),
        };
        assert!(!r.title().is_empty());
        assert!(r.explanation().iter().any(|l| l.contains(HOME_ENV)), "{r:?}");
    }
}
