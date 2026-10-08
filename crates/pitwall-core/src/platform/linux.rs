//! Linux: XDG folders and process facts from `/proc`.
//!
//! - Pitwall's data lives in `$XDG_DATA_HOME/pitwall` (default
//!   `~/.local/share/pitwall`), or `$PITWALL_HOME` when set.
//! - Other apps' settings (VS Code's and Cursor's "recently opened") are under
//!   `$XDG_CONFIG_HOME` (default `~/.config`): `Code/User/globalStorage/…`,
//!   the same layout as macOS's Application Support.
//! - No per-app folder privacy (macOS TCC): no probes, no settings pane.
//! - Processes: `/proc/<pid>/{stat,cmdline,cwd}` (see `procfs.rs`), no `ps`
//!   or `lsof` subprocesses.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::procfs::ProcFs;
use super::unix::{data_dir_or, home_dir};
use super::xdg;

/// How the UI names this machine (`host::HostInfo`).
pub const MACHINE_LABEL: &str = "This computer";
/// Ctrl+Shift shortcuts: Ctrl alone belongs to the terminal.
pub const SHORTCUTS: crate::host::Shortcuts = crate::host::Shortcuts::CtrlShift;
/// No Dock (or tray) to bring a hidden window back: closing the main window
/// quits.
pub const HAS_DOCK: bool = false;
pub const HAS_TRAY: bool = false;
/// No native menu: GTK menu bars take F10 and Ctrl accelerators.
pub const MENU_BAR: crate::host::MenuBar = crate::host::MenuBar::None;
/// The launcher's count where the desktop supports one (Unity API).
pub const BADGE: crate::host::Badge = crate::host::Badge::Dock;

fn proc() -> ProcFs {
    ProcFs::new("/proc")
}

pub fn data_dir() -> PathBuf {
    data_dir_or(|| xdg::data_home(&home_dir()).join("pitwall"))
}

/// Per-user settings of other apps (editors' "recently opened").
pub fn app_support_dir() -> PathBuf {
    xdg::config_home(&home_dir())
}

/// The shell agents start in when `$SHELL` is unset.
pub fn default_shell() -> String {
    ["/bin/bash", "/usr/bin/bash"]
        .into_iter()
        .find(|p| Path::new(p).is_file())
        .unwrap_or("/bin/sh")
        .into()
}

/// Linux has no per-app folder permission to check.
pub fn full_disk_access_probes() -> Vec<io::Result<()>> {
    Vec::new()
}

pub fn open_system_url(_url: &str) -> Result<(), String> {
    Err("privacy settings are a macOS feature".into())
}

/// `pid ppid args` per process (parsed by `procs::parse_table`).
pub fn process_table() -> Option<String> {
    proc().table()
}

/// `(pid, cwd)` of each process in `pids` whose folder is readable. File
/// reads only, so the timeout (for `lsof` elsewhere) never applies.
pub fn process_cwds(pids: &[u32], _timeout: Duration) -> Option<Vec<(u32, String)>> {
    Some(proc().cwds(pids))
}

/// `pid tpgid` per pid (parsed by `procs::parse_pid_numbers`).
pub fn terminal_groups(pids: &[u32]) -> Option<String> {
    Some(proc().terminal_groups(pids))
}

/// `pid args` per pid (parsed by `procs::parse_pid_args`).
pub fn process_args(pids: &[u32]) -> Option<String> {
    Some(proc().process_args(pids))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procs::{parse_pid_args, parse_pid_numbers, parse_table};

    #[test]
    fn reads_this_machines_processes() {
        // A harmless process of our own; killed by its exact pid below.
        let mut child = std::process::Command::new("/bin/sleep").arg("30").spawn().expect("spawn sleep");
        let pid = child.id();
        let me = std::process::id();
        // Right after the spawn the child may still be in exec, with no
        // command line yet: give it a moment.
        let mut args = parse_pid_args(&process_args(&[pid, me]).unwrap());
        for _ in 0..100 {
            if args.get(&pid).map(String::as_str) == Some("/bin/sleep 30") {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
            args = parse_pid_args(&process_args(&[pid, me]).unwrap());
        }
        let groups = parse_pid_numbers(&terminal_groups(&[pid]).unwrap());
        let table = parse_table(&process_table().unwrap());
        let cwds = process_cwds(&[me], Duration::from_secs(1)).unwrap();
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(args.get(&pid).map(String::as_str), Some("/bin/sleep 30"));
        assert!(args.contains_key(&me));
        assert!(groups.contains_key(&pid));
        assert!(table.iter().any(|r| r.pid == pid && r.ppid == me));
        let here = std::env::current_dir().unwrap();
        assert_eq!(cwds, vec![(me, here.to_string_lossy().into_owned())]);
        assert_eq!(process_args(&[]).as_deref(), Some(""));
    }

    #[test]
    fn data_lives_in_xdg_data_home() {
        if std::env::var_os(crate::paths::HOME_ENV).is_none() {
            assert!(data_dir().ends_with("pitwall"));
        }
    }
}
