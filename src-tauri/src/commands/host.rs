//! What this desktop offers (`pitwall_core::host::HostInfo`): the UI's
//! shortcut modifier, how it names this machine, the data folder.

use pitwall_core::host::HostInfo;
use pitwall_core::paths::Paths;

#[tauri::command]
pub fn host_info() -> HostInfo {
    HostInfo::current(&Paths::new(Paths::default_root()))
}

/// Quit Pitwall (command palette; the native menu's Quit items where there
/// is one). Agents keep running in their holders unless `stopAgents`.
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle, stop_agents: bool) {
    crate::menu::quit(&app, stop_agents);
}

/// Settings → Appearance → Look: the native window material for the calling
/// window (each window asks for itself, so moved-out spaces get it too).
#[tauri::command]
pub fn set_window_glass(window: tauri::WebviewWindow, on: bool) -> crate::platform::glass::GlassState {
    let glass = HostInfo::current(&Paths::new(Paths::default_root())).glass;
    crate::platform::glass::set(&window, glass, on)
}
