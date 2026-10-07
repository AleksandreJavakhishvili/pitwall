//! This machine, for the local provider (`pitwall-providers`) and the app:
//! process facts the platform layer reads (architecture.md §9 decision 7),
//! and what the desktop offers ([`HostInfo`]). Nothing in the engine calls
//! these for an agent; it asks the agent's provider.

use std::time::Duration;

use serde::Serialize;

use crate::paths::{tildify, Paths};
use crate::platform;

/// The current folder of each of `pids` that could be read (`lsof` on
/// macOS, `/proc` on Linux);
/// `None` when the lookup itself failed.
pub fn process_cwds(pids: &[u32], timeout: Duration) -> Option<Vec<(u32, String)>> {
    platform::process_cwds(pids, timeout)
}

/// The current folder of process `pid`.
pub fn process_cwd(pid: u32, timeout: Duration) -> Option<String> {
    process_cwds(&[pid], timeout)?.into_iter().find(|(p, _)| *p == pid).map(|(_, cwd)| cwd)
}

/// How the UI names this machine ("This Mac").
pub fn machine_label() -> &'static str {
    platform::MACHINE_LABEL
}

/// The modifier of Pitwall's own shortcuts.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Shortcuts {
    /// ⌘ (macOS): terminals never use it, so ⌘K etc. are free.
    Meta,
    /// Ctrl+Shift (Linux): Ctrl alone belongs to the terminal (Ctrl+C, Ctrl+R,
    /// Ctrl+W …), as in other terminal apps there; ⌘⇧ shortcuts become
    /// Ctrl+Shift+Alt.
    CtrlShift,
}

/// What this desktop offers, as capabilities: the UI (and the app shell)
/// decide by these, never by which OS it is (architecture.md §3).
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    /// How the UI names this machine.
    pub machine_label: String,
    pub shortcuts: Shortcuts,
    /// Pitwall's data folder as shown to the user (`~/…`).
    pub data_dir: String,
    /// A Dock icon brings hidden windows back (and shows a badge), so
    /// closing the main window can just hide it. Without one, closing quits
    /// the app (agents keep running in their holders).
    pub dock: bool,
}

impl HostInfo {
    pub fn current(paths: &Paths) -> HostInfo {
        HostInfo {
            machine_label: machine_label().into(),
            shortcuts: platform::SHORTCUTS,
            data_dir: tildify(&paths.root().to_string_lossy()),
            dock: platform::HAS_DOCK,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_for_the_ui() {
        let info = HostInfo { machine_label: "This Mac".into(), shortcuts: Shortcuts::CtrlShift, data_dir: "~/.pitwall".into(), dock: true };
        let v = serde_json::to_value(&info).unwrap();
        assert_eq!(v["machineLabel"], "This Mac");
        assert_eq!(v["shortcuts"], "ctrlShift");
        assert_eq!(v["dataDir"], "~/.pitwall");
        assert_eq!(v["dock"], true);
        let here = HostInfo::current(&Paths::new(crate::paths::home().join("pw")));
        assert_eq!(here.data_dir, "~/pw");
        assert!(!here.machine_label.is_empty());
    }
}
