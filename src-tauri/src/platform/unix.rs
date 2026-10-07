//! macOS (and Linux until it has its own): today's behaviour.

use tauri::menu::Menu;
use tauri::{AppHandle, Manager, Runtime};

pub fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    crate::menu::build(app)
}

/// Nothing extra: the Dock reopens the window.
pub fn setup(_app: &AppHandle) -> tauri::Result<()> {
    Ok(())
}

/// The Dock badge.
pub fn set_badge(app: &AppHandle, blocked: usize) {
    if let Some(w) = app.get_webview_window("main") {
        let count = (blocked > 0).then_some(blocked as i64);
        let _ = w.set_badge_count(count);
    }
}
