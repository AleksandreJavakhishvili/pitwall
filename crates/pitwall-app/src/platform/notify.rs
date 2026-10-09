//! System notifications for "an agent needs you" (Tauri:
//! `src-tauri/src/attention.rs` with `tauri-plugin-notification`, which
//! sends them with `notify-rust`; so does this).
//!
//! - macOS: Notification Center through the app's bundle id; an unbundled
//!   dev build borrows Terminal's, as the Tauri plugin does in dev.
//! - Windows: a toast; the AppUserModelID is set only for an installed app
//!   (not from `target/debug` or `target/release`), as the plugin does.
//! - Linux: `org.freedesktop.Notifications` over D-Bus.
//!
//! Shown only when the user isn't looking at Pitwall, and never from a
//! bench instance (`PITWALL_BENCH=1`, scripts/bench.sh).

/// The bundle id of the installed app (Windows AppUserModelID).
pub const APP_ID: &str = super::BUNDLE_ID;

/// The notification text for an attention item (`attention::body`).
pub fn body(reason: &str, detail: Option<&str>) -> String {
    match (reason, detail) {
        ("blocked", Some(d)) => format!("Needs you — {d}"),
        ("blocked", None) => "Needs you".to_string(),
        (_, Some(d)) => format!("Done — {d}"),
        _ => "Finished its turn".to_string(),
    }
}

/// Bench instances stay out of the user's way.
pub fn bench() -> bool {
    std::env::var_os("PITWALL_BENCH").is_some_and(|v| v == "1")
}

/// Whether to notify: not while Pitwall's window has the user's attention.
pub fn wanted(window_focused: bool, bench: bool) -> bool {
    !window_focused && !bench
}

/// Once, at start: which app the notifications come from.
pub fn init() {
    #[cfg(target_os = "macos")]
    {
        let id = super::macos::bundle_id().unwrap_or_else(|| "com.apple.Terminal".into());
        let _ = notify_rust::set_application(&id);
    }
}

/// Show a notification, off the main thread (delivery can block briefly).
pub fn send(title: String, body: String) {
    let spawned = std::thread::Builder::new()
        .name("pitwall-notify".into())
        .spawn(move || {
            let mut n = notify_rust::Notification::new();
            n.summary(&title).body(&body).auto_icon();
            #[cfg(windows)]
            if installed() {
                n.app_id(APP_ID);
            }
            if let Err(e) = n.show() {
                eprintln!("pitwall: could not show a notification: {e}");
            }
        });
    if let Err(e) = spawned {
        eprintln!("pitwall: could not show a notification: {e}");
    }
}

/// Windows: running from an install, not a cargo build folder.
#[cfg(windows)]
fn installed() -> bool {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.to_path_buf()));
    !dir.is_some_and(|d| d.ends_with("target\\debug") || d.ends_with("target\\release"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_text() {
        assert_eq!(
            body("blocked", Some("Permission: Edit")),
            "Needs you — Permission: Edit"
        );
        assert_eq!(body("blocked", None), "Needs you");
        assert_eq!(body("done", Some("x")), "Done — x");
        assert_eq!(body("done", None), "Finished its turn");
    }

    #[test]
    fn quiet_while_looking_or_benchmarking() {
        assert!(wanted(false, false));
        assert!(!wanted(true, false));
        assert!(!wanted(false, true));
    }
}
