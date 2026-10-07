//! macOS: `~/.pitwall`, `~/Library/Application Support`, the Full Disk
//! Access probe, and process facts from `ps` / `lsof` (also used on other
//! BSD-style Unixes, which ship both tools).

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use super::unix::{data_dir_or, home_dir};
use crate::exec::local_stdout;

const PROCESS_TIMEOUT: Duration = Duration::from_secs(5);

/// How the UI names this machine (`host::HostInfo`).
pub const MACHINE_LABEL: &str = "This Mac";
/// ⌘ shortcuts: terminals never see ⌘.
pub const SHORTCUTS: crate::host::Shortcuts = crate::host::Shortcuts::Meta;
/// The Dock brings hidden windows back and shows a badge.
pub const HAS_DOCK: bool = true;
pub const HAS_TRAY: bool = false;
pub const MENU_BAR: crate::host::MenuBar = crate::host::MenuBar::App;
pub const BADGE: crate::host::Badge = crate::host::Badge::Dock;

/// Everything Pitwall owns lives under ~/.pitwall, or under `$PITWALL_HOME`
/// when set.
pub fn data_dir() -> PathBuf {
    data_dir_or(|| home_dir().join(".pitwall"))
}

/// Per-user application data of other apps (editors' "recently opened").
pub fn app_support_dir() -> PathBuf {
    home_dir().join("Library/Application Support")
}

/// The shell agents start in when `$SHELL` is unset (the macOS default).
pub fn default_shell() -> String {
    "/bin/zsh".into()
}

// ---------------------------------------------------------------- privacy (macOS TCC)

/// Opens items that only Full Disk Access unlocks, read-only, without reading
/// them (`permissions::classify` maps the results). These locations have no
/// consent prompt: without the grant macOS fails the open with EPERM, so
/// checking never makes macOS ask. Desktop/Documents/Downloads and other
/// apps' containers are deliberately absent — touching those *does* prompt.
/// Empty where there is no such permission (not macOS).
pub fn full_disk_access_probes() -> Vec<io::Result<()>> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let home = home_dir();
    vec![
        // The per-user privacy database: present on every Mac.
        std::fs::File::open(home.join("Library/Application Support/com.apple.TCC/TCC.db")).map(drop),
        // Safari's data folder, in case the database moves.
        std::fs::read_dir(home.join("Library/Safari")).map(drop),
    ]
}

/// Opens a system URL (a System Settings pane) with the default handler.
pub fn open_system_url(url: &str) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("privacy settings are a macOS feature".into());
    }
    let status = std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .map_err(|e| format!("could not open System Settings: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("could not open System Settings".into())
    }
}

// ---------------------------------------------------------------- processes

/// `ps -axww -o pid=,ppid=,args=` output (parsed by `procs::parse_table`).
pub fn process_table() -> Option<String> {
    local_stdout(&["/bin/ps", "-axww", "-o", "pid=,ppid=,args="], PROCESS_TIMEOUT)
}

/// `(pid, cwd)` of each process in `pids` that lsof could see.
pub fn process_cwds(pids: &[u32], timeout: Duration) -> Option<Vec<(u32, String)>> {
    let pids: Vec<String> = pids.iter().map(u32::to_string).collect();
    let pids = pids.join(",");
    local_stdout(&["/usr/sbin/lsof", "-a", "-d", "cwd", "-p", &pids, "-Fn"], timeout).map(|out| parse_lsof_cwd(&out))
}

/// `(pid, cwd)` pairs from `lsof -a -d cwd -p <pids> -Fn`.
fn parse_lsof_cwd(out: &str) -> Vec<(u32, String)> {
    let mut res = Vec::new();
    let mut pid: Option<u32> = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix('p') {
            pid = p.trim().parse().ok();
        } else if let Some(n) = line.strip_prefix('n') {
            if let Some(p) = pid.take() {
                res.push((p, n.to_string()));
            }
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsof_output() {
        let got = parse_lsof_cwd(include_str!("fixtures/lsof_cwd.txt"));
        assert_eq!(
            got,
            vec![
                (41207, "/Users/dev/code/orders-api".to_string()),
                (52318, "/Users/dev/My Projects/web app".to_string()),
            ]
        );
    }
}
