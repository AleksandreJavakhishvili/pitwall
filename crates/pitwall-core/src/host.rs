//! This machine, for the local provider (`pitwall-providers`) and the app:
//! process facts the platform layer reads (architecture.md §9 decision 7),
//! and what the desktop offers ([`HostInfo`]). Nothing in the engine calls
//! these for an agent; it asks the agent's provider.

use std::time::Duration;

use serde::Serialize;

use crate::paths::{tildify, Paths};
use crate::platform;

/// The current folder of each of `pids` that could be read (`lsof` on
/// macOS, `/proc` on Linux, Toolhelp32 + the PEB on Windows);
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
    /// Ctrl+Shift (Linux, Windows): Ctrl alone belongs to the terminal
    /// (Ctrl+C, Ctrl+R, Ctrl+W …), as in other terminal apps there; ⌘⇧
    /// shortcuts become Ctrl+Shift+Alt, and terminals copy and paste with
    /// Ctrl+Shift+C / Ctrl+Shift+V.
    CtrlShift,
}

/// The window's native menu.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MenuBar {
    /// The app menu (macOS): Tauri's default menu plus Settings… ⌘, and
    /// "Quit and Stop Agents"; its Edit items use ⌘, which terminals never see.
    App,
    /// A File menu only (Windows): Settings… Ctrl+Shift+,, Close, Quit, Quit
    /// and Stop Agents. No Edit menu: Ctrl+C / Ctrl+V belong to terminals.
    File,
    /// No native menu (Linux: GTK menu bars take F10 and Ctrl accelerators
    /// from terminals); Settings and quitting are in the command palette.
    None,
}

/// Where the "N agents need you" count shows outside the window.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Badge {
    /// A number on the Dock icon (macOS; Unity-style launchers on Linux).
    Dock,
    /// An overlay icon on the taskbar button plus the tray icon's tooltip
    /// (Windows).
    Taskbar,
}

/// What this machine's local sockets are (the CLI, hook relay and holders
/// connect to them by a path under the data folder either way).
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LocalSockets {
    /// Socket files under `<data>/run` (macOS, Linux).
    Unix,
    /// Per-user named pipes, `\\.\pipe\pitwall-…`, derived from those paths
    /// (Windows; `pitwall_proto::pipe::pipe_name`).
    NamedPipe,
}

/// The window material the desktop can draw behind the app (Settings →
/// Appearance → Look: Glass). The compositor draws it, so it costs the app
/// almost nothing; without one the UI paints its own backdrop (CSS).
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WindowGlass {
    /// macOS: an `NSVisualEffectView` behind the webview (vibrancy).
    Vibrancy,
    /// Windows 11: the Mica backdrop.
    Mica,
    /// Nothing native (Linux, Windows 10): the UI's own backdrop.
    None,
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
    /// A Dock icon brings hidden windows back.
    pub dock: bool,
    /// A tray (notification area) icon brings hidden windows back and has
    /// Open / Settings / Quit items.
    pub tray: bool,
    pub menu: MenuBar,
    pub badge: Badge,
    pub local_sockets: LocalSockets,
    pub glass: WindowGlass,
}

/// `PITWALL_GLASS=vibrancy|mica|lite` forces a Glass tier (development:
/// seeing "Glass lite" on a Mac). A tier the OS can't draw just shows the
/// webview's own backdrop.
fn glass_override(v: Option<&str>) -> Option<WindowGlass> {
    match v? {
        "vibrancy" => Some(WindowGlass::Vibrancy),
        "mica" => Some(WindowGlass::Mica),
        "lite" | "none" => Some(WindowGlass::None),
        _ => None,
    }
}

impl HostInfo {
    pub fn current(paths: &Paths) -> HostInfo {
        HostInfo {
            machine_label: machine_label().into(),
            shortcuts: platform::SHORTCUTS,
            data_dir: tildify(&paths.root().to_string_lossy()),
            dock: platform::HAS_DOCK,
            tray: platform::HAS_TRAY,
            menu: platform::MENU_BAR,
            badge: platform::BADGE,
            local_sockets: platform::LOCAL_SOCKETS,
            glass: glass_override(std::env::var("PITWALL_GLASS").ok().as_deref()).unwrap_or_else(platform::window_glass),
        }
    }

    /// Closing the main window can just hide it when the Dock or the tray
    /// brings it back. Otherwise closing quits the app (agents keep running
    /// in their holders).
    pub fn hides_on_close(&self) -> bool {
        self.dock || self.tray
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_for_the_ui() {
        let info = HostInfo {
            machine_label: "This Mac".into(),
            shortcuts: Shortcuts::CtrlShift,
            data_dir: "~/.pitwall".into(),
            dock: true,
            tray: false,
            menu: MenuBar::File,
            badge: Badge::Taskbar,
            local_sockets: LocalSockets::NamedPipe,
            glass: WindowGlass::Mica,
        };
        let v = serde_json::to_value(&info).unwrap();
        assert_eq!(v["machineLabel"], "This Mac");
        assert_eq!(v["shortcuts"], "ctrlShift");
        assert_eq!(v["dataDir"], "~/.pitwall");
        assert_eq!(v["dock"], true);
        assert_eq!(v["tray"], false);
        assert_eq!(v["menu"], "file");
        assert_eq!(v["badge"], "taskbar");
        assert_eq!(v["localSockets"], "namedPipe");
        assert_eq!(v["glass"], "mica");
        assert!(info.hides_on_close());
        assert!(!HostInfo { dock: false, ..info }.hides_on_close());
        let here = HostInfo::current(&Paths::new(crate::paths::home().join("pw")));
        assert!(here.data_dir.starts_with('~') && here.data_dir.ends_with("pw"), "{}", here.data_dir);
        assert!(!here.machine_label.is_empty());
    }

    #[test]
    fn glass_can_be_forced_for_development() {
        assert_eq!(glass_override(Some("lite")), Some(WindowGlass::None));
        assert_eq!(glass_override(Some("mica")), Some(WindowGlass::Mica));
        assert_eq!(glass_override(Some("vibrancy")), Some(WindowGlass::Vibrancy));
        assert_eq!(glass_override(Some("frosted")), None);
        assert_eq!(glass_override(None), None);
    }
}
