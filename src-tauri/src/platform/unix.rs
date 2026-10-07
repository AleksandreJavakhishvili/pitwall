//! macOS and Linux: the Dock (or launcher) badge; no tray icon.

use tauri::{AppHandle, Manager};

/// No tray here (`HostInfo::tray` is false): the Dock reopens the window on
/// macOS, and closing the window quits on Linux.
pub fn setup_tray(_app: &AppHandle) -> tauri::Result<()> {
    Ok(())
}

/// The Dock badge (`HostInfo::badge` = `Badge::Dock`).
pub fn set_badge(app: &AppHandle, blocked: usize) {
    if let Some(w) = app.get_webview_window("main") {
        let count = (blocked > 0).then_some(blocked as i64);
        let _ = w.set_badge_count(count);
    }
}
