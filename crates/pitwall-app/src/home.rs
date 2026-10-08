//! Which data folder this app runs on, and whether it may (docs/spec/gpui/README.md
//! "Coexistence with the Tauri app").
//!
//! Everything Pitwall owns — `state.json`, the hook and CLI sockets, the
//! agents' terminal holders — lives under one folder ([`Paths`]). Two apps on
//! one folder would fight over it: binding the hook socket replaces the other
//! app's, and both would drive the same holders. So while the GPUI app grows
//! next to the Tauri app:
//!
//! - It runs on `$PITWALL_HOME` when that is set (a separate folder: its own
//!   state, sockets and holders; the user's agents are never touched).
//! - Without `$PITWALL_HOME` it refuses, unless `PITWALL_APP_SHARE_HOME=1`
//!   says to use the default folder (the switch-over, phase 9).
//! - Either way it refuses when another Pitwall (the Tauri app, or a second
//!   copy of this one) answers on that folder's CLI socket.
//!
//! A refusal still opens the window, with the reason in it; no engine starts.

use std::path::{Path, PathBuf};

use pitwall_core::paths::{tildify, Paths, HOME_ENV};

/// Opt in to the default data folder (shared with the Tauri app's).
pub const SHARE_ENV: &str = "PITWALL_APP_SHARE_HOME";

/// Why the app won't host an engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No `$PITWALL_HOME` and no opt-in to the default folder.
    NoSeparateHome { default_root: PathBuf },
    /// Another Pitwall answers on this folder's CLI socket.
    InUse { root: PathBuf },
}

impl Refusal {
    pub fn title(&self) -> &'static str {
        match self {
            Refusal::NoSeparateHome { .. } => "Pitwall (GPUI preview) needs its own data folder",
            Refusal::InUse { .. } => "Another Pitwall is using this data folder",
        }
    }

    pub fn explanation(&self) -> Vec<String> {
        match self {
            Refusal::NoSeparateHome { default_root } => vec![
                format!(
                    "This preview would share {} with the Pitwall app, its sockets and its agents' terminals.",
                    tildify(&default_root.to_string_lossy())
                ),
                format!("Start it with {HOME_ENV} set to a separate folder, e.g. {HOME_ENV}=/tmp/pitwall-preview."),
                format!("{SHARE_ENV}=1 uses the default folder instead (only when no other Pitwall runs)."),
            ],
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
    /// `PITWALL_APP_SHARE_HOME=1`.
    pub share: bool,
}

impl Env {
    pub fn current() -> Env {
        Env {
            home: std::env::var_os(HOME_ENV)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            share: std::env::var_os(SHARE_ENV).is_some_and(|v| v == "1"),
        }
    }
}

/// The data folder to host the engine on, or why not. `default_root` is the
/// platform default (what the Tauri app uses); `in_use` asks whether a
/// Pitwall answers on a CLI socket.
pub fn choose(
    env: &Env,
    default_root: PathBuf,
    in_use: impl Fn(&Path) -> bool,
) -> Result<Paths, Refusal> {
    let root = match (&env.home, env.share) {
        (Some(home), _) => home.clone(),
        (None, true) => default_root,
        (None, false) => return Err(Refusal::NoSeparateHome { default_root }),
    };
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

    fn env(home: Option<&str>, share: bool) -> Env {
        Env {
            home: home.map(PathBuf::from),
            share,
        }
    }

    const DEFAULT: &str = "/data/default";

    #[test]
    fn a_separate_home_is_used() {
        let got = choose(&env(Some("/tmp/pw-preview"), false), DEFAULT.into(), |_| {
            false
        })
        .unwrap();
        assert_eq!(got.root(), Path::new("/tmp/pw-preview"));
    }

    #[test]
    fn the_default_folder_needs_an_opt_in() {
        let got = choose(&env(None, false), DEFAULT.into(), |_| false);
        assert_eq!(
            got,
            Err(Refusal::NoSeparateHome {
                default_root: DEFAULT.into()
            })
        );
        let got = choose(&env(None, true), DEFAULT.into(), |_| false).unwrap();
        assert_eq!(got.root(), Path::new(DEFAULT));
    }

    #[test]
    fn a_folder_another_pitwall_serves_is_refused() {
        let busy = Paths::new(DEFAULT).cli_socket();
        let got = choose(&env(None, true), DEFAULT.into(), |s| s == busy);
        assert_eq!(
            got,
            Err(Refusal::InUse {
                root: DEFAULT.into()
            })
        );
        // The same check guards a separate folder (a second preview on it).
        let got = choose(&env(Some("/tmp/pw-2"), false), DEFAULT.into(), |_| true);
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
        for r in [
            Refusal::NoSeparateHome {
                default_root: DEFAULT.into(),
            },
            Refusal::InUse {
                root: DEFAULT.into(),
            },
        ] {
            assert!(!r.title().is_empty());
            assert!(
                r.explanation().iter().any(|l| l.contains(HOME_ENV)),
                "{r:?}"
            );
        }
    }
}
