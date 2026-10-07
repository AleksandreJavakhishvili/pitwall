//! Telling the user something needs them: notification and badge.

use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use pitwall_core::model::Attention;

fn window_focused(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
        .unwrap_or(false)
}

/// The notification text for an attention item.
pub fn body(item: &Attention) -> String {
    match (item.reason, &item.detail) {
        ("blocked", Some(d)) => format!("Needs you — {d}"),
        ("blocked", None) => "Needs you".to_string(),
        (_, Some(d)) => format!("Done — {d}"),
        _ => "Finished its turn".to_string(),
    }
}

/// A system notification, unless the user is looking at Pitwall already.
pub fn notify(app: &AppHandle, item: &Attention) {
    // Bench instances (scripts/bench.sh) stay out of the user's way.
    if window_focused(app) || crate::bench::enabled() {
        return;
    }
    let _ = app.notification().builder().title(&item.name).body(body(item)).show();
}

/// Dock badge on macOS; taskbar overlay and tray tooltip on Windows.
pub fn set_badge(app: &AppHandle, blocked: usize) {
    crate::platform::set_badge(app, blocked);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(reason: &'static str, detail: Option<&str>) -> Attention {
        Attention { agent_id: "a".into(), name: "n".into(), reason, detail: detail.map(Into::into) }
    }

    #[test]
    fn notification_text() {
        assert_eq!(body(&item("blocked", Some("Permission: Edit"))), "Needs you — Permission: Edit");
        assert_eq!(body(&item("blocked", None)), "Needs you");
        assert_eq!(body(&item("done", Some("x"))), "Done — x");
        assert_eq!(body(&item("done", None)), "Finished its turn");
    }
}
