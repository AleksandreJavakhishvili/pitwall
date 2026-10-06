//! Telling the user something needs them: event, notification, Dock badge.

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::model::Attention;

fn window_focused(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
        .unwrap_or(false)
}

pub fn raise(app: &AppHandle, items: Vec<Attention>) {
    if items.is_empty() {
        return;
    }
    let focused = window_focused(app);
    for item in items {
        let _ = app.emit("attention", &item);
        if focused {
            continue;
        }
        let body = match (item.reason, &item.detail) {
            ("blocked", Some(d)) => format!("Needs you — {d}"),
            ("blocked", None) => "Needs you".to_string(),
            (_, Some(d)) => format!("Done — {d}"),
            _ => "Finished its turn".to_string(),
        };
        let _ = app
            .notification()
            .builder()
            .title(&item.name)
            .body(body)
            .show();
    }
}

pub fn set_badge(app: &AppHandle, blocked: usize) {
    if let Some(w) = app.get_webview_window("main") {
        let count = (blocked > 0).then_some(blocked as i64);
        let _ = w.set_badge_count(count);
    }
}
