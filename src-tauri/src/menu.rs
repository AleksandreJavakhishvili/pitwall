//! Native menus, chosen by `HostInfo::menu` (one capability, no OS checks
//! here):
//!
//! - `MenuBar::App` (macOS): Tauri's default menu (standard Edit items so
//!   copy/paste/select-all keep working in inputs and terminals, ⌘Q quit,
//!   Window menu) plus "Pitwall → Settings… ⌘,".
//! - `MenuBar::File` (Windows): a File menu only — Settings… Ctrl+Shift+,,
//!   Close Window, Quit. No Edit menu: its accelerators (Ctrl+C, Ctrl+V, …)
//!   would be taken from the terminals; the webview copies and pastes by
//!   itself.
//! - `MenuBar::None` (Linux): GTK menu bars take F10 and Ctrl accelerators
//!   from terminals. Settings is Ctrl+Shift+, in the UI, and quitting (with
//!   or without stopping agents) is in the command palette.
//!
//! Settings asks the focused window's UI to open Settings via the
//! `open-settings` event. "Quit and Stop Agents" is the one way to end every
//! agent at once; plain Quit leaves them running in their holders. The tray
//! (`platform`) shares these ids and [`on_event`].

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use pitwall_core::engine::lifecycle;
use pitwall_core::host::{HostInfo, MenuBar};
use pitwall_core::Shared;

use crate::windows;

pub const SETTINGS_ID: &str = "pitwall-settings";
pub const QUIT_STOP_ID: &str = "pitwall-quit-stop";
/// The tray's "Open Pitwall" (show the main window).
pub const SHOW_ID: &str = "pitwall-show";
/// Quit the UI only (agents keep running, as with ⌘Q).
pub const QUIT_ID: &str = "pitwall-quit";
/// Event the UI listens to (payload: none).
pub const OPEN_SETTINGS: &str = "open-settings";

/// Builds a window's native menu.
pub type BuildMenu<R> = fn(&AppHandle<R>) -> tauri::Result<Menu<R>>;

/// The menu builder for this desktop, if it has a native menu.
pub fn for_host<R: Runtime>(host: &HostInfo) -> Option<BuildMenu<R>> {
    match host.menu {
        MenuBar::App => Some(build_app_menu::<R>),
        MenuBar::File => Some(build_file_menu::<R>),
        MenuBar::None => None,
    }
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

/// The window's menu bar with File only (module docs).
pub fn build_file_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let settings = MenuItem::with_id(app, SETTINGS_ID, "Settings…", true, Some("Ctrl+Shift+Comma"))?;
    let quit_stop = MenuItem::with_id(app, QUIT_STOP_ID, "Quit and Stop Agents", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit Pitwall", true, None::<&str>)?;
    let close = PredefinedMenuItem::close_window(app, Some("Close Window"))?;
    let sep = || PredefinedMenuItem::separator(app);
    let file = Submenu::with_items(app, "File", true, &[&settings, &sep()?, &close, &sep()?, &quit, &quit_stop])?;
    Menu::with_items(app, &[&file])
}

/// The app menu (module docs).
pub fn build_app_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
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
    if event.id() == SHOW_ID {
        if let Some(w) = app.get_webview_window(windows::MAIN) {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
        return;
    }
    if event.id() == QUIT_ID {
        quit(app, false);
        return;
    }
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
