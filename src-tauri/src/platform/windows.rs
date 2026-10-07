//! Windows: the tray icon (`HostInfo::tray`) and the taskbar overlay badge
//! (`Badge::Taskbar`). The File menu is `menu::build_file_menu`.

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use super::badge;
use crate::menu::{QUIT_ID, QUIT_STOP_ID, SETTINGS_ID, SHOW_ID};

const TRAY_ID: &str = "pitwall-tray";

/// The tray icon: closing the window only hides it (agents keep running),
/// so this is the way back, like the Dock on macOS.
pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, SHOW_ID, "Open Pitwall", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, SETTINGS_ID, "Settings…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, "Quit Pitwall", true, None::<&str>)?;
    let quit_stop = MenuItem::with_id(app, QUIT_STOP_ID, "Quit and Stop Agents", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &settings, &sep, &quit, &quit_stop])?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID).tooltip("Pitwall").menu(&menu).show_menu_on_left_click(false).on_tray_icon_event(
        |tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                crate::show_main(tray.app_handle());
            }
        },
    );
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// The blocked count on the taskbar button (overlay icon) and in the tray
/// tooltip.
pub fn set_badge(app: &AppHandle, blocked: usize) {
    if let Some(w) = app.get_webview_window("main") {
        let icon = (blocked > 0).then(|| Image::new_owned(badge::rgba(blocked), badge::SIZE, badge::SIZE));
        let _ = w.set_overlay_icon(icon);
    }
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let tip = match blocked {
            0 => "Pitwall".to_string(),
            1 => "Pitwall — 1 agent needs you".to_string(),
            n => format!("Pitwall — {n} agents need you"),
        };
        let _ = tray.set_tooltip(Some(tip));
    }
}
