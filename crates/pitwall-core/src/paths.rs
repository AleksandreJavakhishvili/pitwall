//! Where things are. [`Paths`] is everything Pitwall owns (`~/.pitwall` by
//! default); it is a value handed to the engine, so tests run in a temp dir.
//! The user's home folder (for `~` in what the user types and sees) is a
//! separate, read-only concern: [`home`], [`tildify`], [`expand_tilde`].

use std::path::{Path, PathBuf};

use crate::platform;

/// Pitwall's own files, all under one root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn new(root: impl Into<PathBuf>) -> Paths {
        Paths { root: root.into() }
    }

    /// The platform's data dir for Pitwall (`~/.pitwall` on macOS).
    pub fn default_root() -> PathBuf {
        platform::data_dir()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.json")
    }

    pub fn projects_file(&self) -> PathBuf {
        self.root.join("projects.json")
    }

    /// The hook relay posts here (`PITWALL_SOCKET` in agents' environment).
    pub fn hook_socket(&self) -> PathBuf {
        self.root.join("run").join("pitwall.sock")
    }

    /// Agents' terminal holders: `<agentId>.sock` each (pitwall-hold).
    pub fn hold_dir(&self) -> PathBuf {
        self.root.join("run").join("hold")
    }

    pub fn hook_script(&self) -> PathBuf {
        self.root.join("bin").join("pitwall-hook")
    }

    pub fn user_agents_dir(&self) -> PathBuf {
        self.root.join("agents")
    }

    /// Where versions before 0.1 made agent worktrees. Pitwall no longer creates
    /// any; agents still working in one keep their stored path.
    pub fn legacy_worktrees_dir(&self) -> PathBuf {
        self.root.join("worktrees")
    }
}

/// The user's home folder.
pub fn home() -> PathBuf {
    platform::home_dir()
}

/// Show paths under the home directory as `~/...`.
pub fn tildify(path: &str) -> String {
    let home = home();
    let home = home.to_string_lossy();
    match path.strip_prefix(home.as_ref()) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// `~` / `~/x` as typed by the user → under the home folder.
pub fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => home().join(rest.trim_start_matches('/')),
        _ => PathBuf::from(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_lives_under_the_root() {
        let p = Paths::new("/tmp/pw-root");
        for f in [p.state_file(), p.projects_file(), p.hook_socket(), p.hold_dir(), p.hook_script(), p.user_agents_dir()] {
            assert!(f.starts_with("/tmp/pw-root"), "{}", f.display());
        }
        assert_eq!(p.root(), Path::new("/tmp/pw-root"));
    }

    #[test]
    fn tilde_expansion() {
        assert_eq!(expand_tilde("~/x"), home().join("x"));
        assert_eq!(expand_tilde("~"), home());
        assert_eq!(expand_tilde("/a/~b"), PathBuf::from("/a/~b"));
        assert_eq!(tildify(&home().join("p").to_string_lossy()), "~/p");
        assert_eq!(tildify("/elsewhere"), "/elsewhere");
    }
}
