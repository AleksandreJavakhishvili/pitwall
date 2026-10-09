//! Linux: the launcher badge through the Unity LauncherEntry D-Bus API
//! (`com.canonical.Unity.LauncherEntry.Update`), which GNOME's Dash to Dock,
//! KDE Plasma's task manager and others show; best effort, as Tauri does.
//! No tray and no attention request (gpui exposes neither; the
//! notification is the signal there).

use std::collections::HashMap;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};

use zbus::zvariant::Value;

/// The desktop entry the badge belongs to: the .deb/AppImage launcher,
/// which cargo-packager names after the main binary
/// (`usr/share/applications/pitwall.desktop`). Keep in step with
/// packaging and with [`super::WINDOW_APP_ID`].
pub const DESKTOP_ID: &str = "application://pitwall.desktop";

/// The badge's properties for a count.
fn props(count: usize) -> HashMap<&'static str, Value<'static>> {
    HashMap::from([
        ("count", Value::from(count as i64)),
        ("count-visible", Value::from(count > 0)),
    ])
}

/// Counts go to one worker thread that owns the session-bus connection
/// (connecting can block; the UI thread never waits on D-Bus).
fn worker() -> &'static Mutex<Sender<usize>> {
    static TX: OnceLock<Mutex<Sender<usize>>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = channel::<usize>();
        let _ = std::thread::Builder::new()
            .name("pitwall-badge".into())
            .spawn(move || {
                let Ok(conn) = zbus::blocking::Connection::session() else {
                    return;
                };
                for count in rx {
                    let _ = conn.emit_signal(
                        None::<&str>,
                        "/",
                        "com.canonical.Unity.LauncherEntry",
                        "Update",
                        &(DESKTOP_ID, props(count)),
                    );
                }
            });
        Mutex::new(tx)
    })
}

pub fn set_badge(count: usize) {
    if let Ok(tx) = worker().lock() {
        let _ = tx.send(count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_badge_hides_at_zero() {
        assert_eq!(props(0)["count-visible"], Value::from(false));
        assert_eq!(props(4)["count"], Value::from(4i64));
    }

    #[test]
    fn the_window_and_the_badge_name_the_same_launcher() {
        assert_eq!(
            DESKTOP_ID,
            format!("application://{}.desktop", super::super::WINDOW_APP_ID)
        );
    }
}
