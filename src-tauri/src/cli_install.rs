//! "Install command-line tool" (Settings), like VS Code's `code`: a symlink
//! named `pitwall` to the `pitwall-cli` the app ships (a Tauri `externalBin`
//! next to the app executable), in `~/.local/bin` or `/usr/local/bin`. The
//! UI asks the user first; nothing that isn't Pitwall's own link is ever
//! replaced.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// What the link is called.
pub const LINK_NAME: &str = "pitwall";
const BIN_NAME: &str = "pitwall-cli";

/// Where the CLI ships: next to the app executable (bundle and dev), one
/// level up (tests), or where build.rs built it.
pub fn cli_bin() -> Result<PathBuf, String> {
    let name = format!("{BIN_NAME}{}", std::env::consts::EXE_SUFFIX);
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join(&name));
        if let Some(up) = dir.parent() {
            candidates.push(up.join(&name));
        }
    }
    if let Some(built) = option_env!("PITWALL_CLI_BUILT") {
        candidates.push(PathBuf::from(built));
    }
    candidates.into_iter().find(|p| p.is_file()).ok_or_else(|| "the command-line tool (pitwall-cli) is missing next to the app".into())
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CliDir {
    pub path: String,
    /// On `$PATH` as the app sees it (a Dock app's PATH may be shorter
    /// than the user's shell's).
    pub on_path: bool,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CliStatus {
    /// The shipped tool (`None`: missing from this build).
    pub bin: Option<String>,
    /// An existing `pitwall` link to it.
    pub installed: Option<String>,
    /// Where it can be installed, best first.
    pub dirs: Vec<CliDir>,
}

/// `~/.local/bin`, then `/usr/local/bin`.
pub fn candidate_dirs(home: &Path) -> Vec<PathBuf> {
    vec![home.join(".local").join("bin"), PathBuf::from("/usr/local/bin")]
}

/// A `pitwall` link in `dir` that points at a Pitwall CLI.
fn our_link(dir: &Path) -> Option<PathBuf> {
    let link = dir.join(LINK_NAME);
    let target = std::fs::read_link(&link).ok()?;
    (target.file_name()?.to_string_lossy().starts_with(BIN_NAME)).then_some(link)
}

pub fn status(bin: Option<&Path>, dirs: &[PathBuf], path_env: &str) -> CliStatus {
    let on_path: Vec<&Path> = path_env.split(':').filter(|p| !p.is_empty()).map(Path::new).collect();
    CliStatus {
        bin: bin.map(|b| b.to_string_lossy().into_owned()),
        installed: dirs.iter().find_map(|d| our_link(d)).map(|l| l.to_string_lossy().into_owned()),
        dirs: dirs
            .iter()
            .map(|d| CliDir { path: d.to_string_lossy().into_owned(), on_path: on_path.contains(&d.as_path()), exists: d.is_dir() })
            .collect(),
    }
}

/// Link `dir/pitwall` → `bin`. Creates `dir` if missing; replaces only a
/// `pitwall` link that is already Pitwall's.
pub fn install(bin: &Path, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let link = dir.join(LINK_NAME);
    if std::fs::symlink_metadata(&link).is_ok() {
        if our_link(dir).is_none() {
            return Err(format!("{} already exists and isn't Pitwall's; not replacing it", link.display()));
        }
        std::fs::remove_file(&link).map_err(|e| format!("can't replace {}: {e}", link.display()))?;
    }
    symlink(bin, &link).map_err(|e| format!("can't create {}: {e}", link.display()))?;
    Ok(link)
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_target: &Path, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "not on this platform yet"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pw-cli-install-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn installs_a_link_and_reports_it() {
        let root = temp("ok");
        let bin = root.join("app").join("pitwall-cli");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        let dirs = candidate_dirs(&root.join("home"));
        let before = status(Some(&bin), &dirs[..1], "/usr/bin");
        assert_eq!(before.installed, None);
        assert!(!before.dirs[0].exists && !before.dirs[0].on_path);
        let link = install(&bin, &dirs[0]).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), bin);
        let path_env = format!("/usr/bin:{}", dirs[0].display());
        let after = status(Some(&bin), &dirs[..1], &path_env);
        assert_eq!(after.installed.as_deref(), Some(link.to_str().unwrap()));
        assert!(after.dirs[0].on_path && after.dirs[0].exists);
        // Again (e.g. the app moved): Pitwall's own link is replaced.
        let moved = root.join("moved").join("pitwall-cli");
        std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
        std::fs::write(&moved, "").unwrap();
        install(&moved, &dirs[0]).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), moved);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn never_replaces_someone_elses_pitwall() {
        let root = temp("other");
        let dir = root.join("bin");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pitwall"), "someone else's").unwrap();
        let e = install(Path::new("/x/pitwall-cli"), &dir).unwrap_err();
        assert!(e.contains("isn't Pitwall's"), "{e}");
        assert_eq!(std::fs::read_to_string(dir.join("pitwall")).unwrap(), "someone else's");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_shipped_cli_is_found() {
        assert!(cli_bin().expect("pitwall-cli next to the app or built by build.rs").is_file());
    }
}
