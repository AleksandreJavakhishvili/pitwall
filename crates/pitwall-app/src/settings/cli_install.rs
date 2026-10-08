//! Settings → "Command-line tool" (Tauri: `src-tauri/src/cli_install.rs`,
//! moved here unchanged in behaviour): a symlink named `pitwall` to the
//! `pitwall-cli` the app ships, in `~/.local/bin` or `/usr/local/bin`. On
//! Windows (no symlinks without privileges) a copy, `pitwall.exe`, in
//! `%LOCALAPPDATA%\Pitwall\bin` with a marker file saying it is Pitwall's.
//! The UI asks first; nothing that isn't Pitwall's own is ever replaced.
//!
//! platform.md puts this under `platform/cli_install.rs`; move it there when
//! that module exists. The AppImage copy-out (`stable_sidecar`) comes with
//! packaging (phase 8).

use std::path::{Path, PathBuf};

/// What the link is called.
pub const LINK_NAME: &str = "pitwall";
const BIN_NAME: &str = "pitwall-cli";

/// The shipped CLI: next to the app executable (bundle, or `target/<profile>/`
/// after a workspace build), or one level up (tests run from `deps/`).
pub fn cli_bin() -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf));
    find_cli(exe_dir)
        .ok_or_else(|| "the command-line tool (pitwall-cli) is missing next to the app".into())
}

fn find_cli(exe_dir: Option<PathBuf>) -> Option<PathBuf> {
    let name = format!("{BIN_NAME}{}", std::env::consts::EXE_SUFFIX);
    let mut candidates = Vec::new();
    if let Some(dir) = exe_dir {
        candidates.push(dir.join(&name));
        if let Some(up) = dir.parent() {
            candidates.push(up.join(&name));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliDir {
    pub path: String,
    /// On `$PATH` as the app sees it (a Dock app's PATH may be shorter than
    /// the user's shell's).
    pub on_path: bool,
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliStatus {
    /// The shipped tool (`None`: missing from this build).
    pub bin: Option<String>,
    /// An existing `pitwall` link to it.
    pub installed: Option<String>,
    /// Where it can be installed, best first.
    pub dirs: Vec<CliDir>,
}

/// `~/.local/bin`, then `/usr/local/bin`.
#[cfg(not(windows))]
pub fn candidate_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".local").join("bin"),
        PathBuf::from("/usr/local/bin"),
    ]
}

/// `%LOCALAPPDATA%\Pitwall\bin`.
#[cfg(windows)]
pub fn candidate_dirs(home: &Path) -> Vec<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData").join("Local"));
    vec![local.join("Pitwall").join("bin")]
}

/// The installed command: `pitwall` (`pitwall.exe` on Windows).
fn link_path(dir: &Path) -> PathBuf {
    dir.join(format!("{LINK_NAME}{}", std::env::consts::EXE_SUFFIX))
}

/// Marks a Windows copy as Pitwall's own.
#[cfg_attr(not(windows), allow(dead_code))]
fn marker(dir: &Path) -> PathBuf {
    dir.join("pitwall-cli.installed")
}

/// A `pitwall` link in `dir` that points at a Pitwall CLI (Windows: our copy).
fn our_link(dir: &Path) -> Option<PathBuf> {
    let link = link_path(dir);
    if cfg!(windows) {
        return (link.is_file() && marker(dir).is_file()).then_some(link);
    }
    let target = std::fs::read_link(&link).ok()?;
    target
        .file_name()?
        .to_string_lossy()
        .starts_with(BIN_NAME)
        .then_some(link)
}

pub fn status(bin: Option<&Path>, dirs: &[PathBuf], path_env: &str) -> CliStatus {
    let on_path: Vec<PathBuf> = std::env::split_paths(path_env)
        .filter(|p| !p.as_os_str().is_empty())
        .collect();
    CliStatus {
        bin: bin.map(|b| b.to_string_lossy().into_owned()),
        installed: dirs
            .iter()
            .find_map(|d| our_link(d))
            .map(|l| l.to_string_lossy().into_owned()),
        dirs: dirs
            .iter()
            .map(|d| CliDir {
                path: d.to_string_lossy().into_owned(),
                on_path: on_path.iter().any(|p| p == d),
                exists: d.is_dir(),
            })
            .collect(),
    }
}

/// The status as Settings shows it (`cli_status`).
pub fn current() -> CliStatus {
    let bin = cli_bin().ok();
    let dirs = candidate_dirs(&pitwall_core::paths::home());
    let path = pitwall_core::shell::spawn_path()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    status(bin.as_deref(), &dirs, &path)
}

/// Install into `dir`, one of [`current`]'s dirs (`install_cli`).
pub fn install_into(dir: &str) -> Result<CliStatus, String> {
    let dirs = candidate_dirs(&pitwall_core::paths::home());
    if !dirs.iter().any(|d| d.to_string_lossy() == dir) {
        return Err(format!(
            "{dir} is not a place Pitwall installs the command-line tool"
        ));
    }
    install(&cli_bin()?, Path::new(dir))?;
    Ok(current())
}

/// Link `dir/pitwall` → `bin`. Creates `dir` if missing; replaces only a
/// `pitwall` link that is already Pitwall's.
pub fn install(bin: &Path, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let link = link_path(dir);
    if std::fs::symlink_metadata(&link).is_ok() {
        if our_link(dir).is_none() {
            return Err(format!(
                "{} already exists and isn't Pitwall's; not replacing it",
                link.display()
            ));
        }
        std::fs::remove_file(&link)
            .map_err(|e| format!("can't replace {}: {e}", link.display()))?;
    }
    symlink(bin, &link).map_err(|e| format!("can't create {}: {e}", link.display()))?;
    #[cfg(windows)]
    std::fs::write(marker(dir), bin.to_string_lossy().as_bytes())
        .map_err(|e| format!("can't create {}: {e}", marker(dir).display()))?;
    Ok(link)
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Windows: a copy (symlinks need Developer Mode or admin rights).
#[cfg(not(unix))]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::fs::copy(target, link).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pw-app-cli-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    #[cfg(unix)]
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
        // Again (the app moved): Pitwall's own link is replaced.
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
        std::fs::write(link_path(&dir), "someone else's").unwrap();
        let e = install(Path::new("/x/pitwall-cli"), &dir).unwrap_err();
        assert!(e.contains("isn't Pitwall's"), "{e}");
        assert_eq!(
            std::fs::read_to_string(link_path(&dir)).unwrap(),
            "someone else's"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_offered_dirs_are_accepted() {
        let e = install_into("/somewhere/else").unwrap_err();
        assert!(e.contains("not a place"), "{e}");
    }

    #[test]
    fn the_cli_is_found_next_to_the_app() {
        let root = temp("find");
        let name = format!("{BIN_NAME}{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(root.join(&name), "").unwrap();
        assert_eq!(find_cli(Some(root.clone())), Some(root.join(&name)));
        assert_eq!(find_cli(Some(root.join("deps"))), Some(root.join(&name)));
        assert_eq!(find_cli(Some(root.join("a/b"))), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
