//! Native app menu: Tauri's default menu (standard Edit items so copy/paste/
//! select-all keep working in inputs and terminals, ⌘Q quit, Window menu)
//! plus "Pitwall → Settings… ⌘," which asks the focused window's UI to open
//! Settings via the `open-settings` event. "Quit and Stop Agents" is the one
//! way to end every agent at once; plain ⌘Q leaves them running in their
//! holders.
//!
//! Only where app shortcuts use ⌘ (`HostInfo::shortcuts`): elsewhere the
//! default menu's accelerators (Ctrl+C, Ctrl+W, Ctrl+Q, F10 for the menu bar)
//! would take keys from the terminals. There Settings is Ctrl+Shift+, and
//! quitting (with or without stopping agents) is in the command palette.

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use pitwall_core::engine::lifecycle;
use pitwall_core::host::{HostInfo, Shortcuts};
use pitwall_core::Shared;

/// Whether this desktop gets the native menu (module docs).
pub fn wanted(host: &HostInfo) -> bool {
    host.shortcuts == Shortcuts::Meta
}

/// Quit the app; with `stop_agents`, end every agent first (otherwise they
/// keep running in their holders).
pub fn quit<R: Runtime>(app: &AppHandle<R>, stop_agents: bool) {
    let app = app.clone();
    std::thread::spawn(move || {
        if stop_agents {
            if let Some(core) = app.try_state::<Shared>() {
                lifecycle::stop_all(core.inner());
            }
        }
        app.exit(0);
    });
}

use crate::windows;

const SETTINGS_ID: &str = "pitwall-settings";
const QUIT_STOP_ID: &str = "pitwall-quit-stop";
/// Event the UI listens to (payload: none).
pub const OPEN_SETTINGS: &str = "open-settings";

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let menu = Menu::default(app)?;
    let settings = MenuItem::with_id(app, SETTINGS_ID, "Settings…", true, Some("CmdOrCtrl+,"))?;
    let quit_stop = MenuItem::with_id(app, QUIT_STOP_ID, "Quit and Stop Agents", true, None::<&str>)?;
    // macOS: the first submenu is the app menu (About, —, Services, …).
    // Put Settings right after About, the usual place.
    if let Some(app_menu) = menu.items()?.first().and_then(|i| i.as_submenu()) {
        if cfg!(target_os = "macos") {
            app_menu.insert_items(&[&settings, &PredefinedMenuItem::separator(app)?], 2)?;
        } else {
            app_menu.prepend(&settings)?;
        }
        app_menu.append(&quit_stop)?;
    }
    Ok(menu)
}

pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    if event.id() == QUIT_STOP_ID {
        quit(app, true);
        return;
    }
    if event.id() != SETTINGS_ID {
        return;
    }
    // Open it in the window the user is looking at; fall back to main.
    let windows = app.webview_windows();
    let target = windows
        .values()
        .find(|w| w.is_focused().unwrap_or(false))
        .or_else(|| windows.get(windows::MAIN));
    let Some(w) = target else { return };
    let _ = w.show();
    let _ = w.unminimize();
    let _ = w.set_focus();
    if let Err(e) = app.emit_to(w.label(), OPEN_SETTINGS, ()) {
        eprintln!("pitwall: could not ask the UI to open Settings: {e}");
    }
}
