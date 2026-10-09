//! `ui.json` edited outside Pitwall applies live: the file is checked
//! every [`POLL`], read once it has settled for [`SETTLE`], checked against
//! the settings registry and applied. The app's own writes are recognised
//! and skipped; a value the registry doesn't allow keeps the last good one
//! and is reported in a toast, and a file that isn't valid JSON changes
//! nothing.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use gpui::App;
use serde_json::Value;

use crate::main_screen::workspace;

pub const POLL: Duration = Duration::from_millis(500);
pub const SETTLE: Duration = Duration::from_millis(200);

/// What changes when a file changes: its modification time and size.
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// Watch the data folder's `ui.json` (nothing without a data folder).
pub fn start(cx: &mut App) {
    let store = crate::ui_state::store(cx);
    let Some(root) = store.read(cx).root().cloned() else {
        return;
    };
    let written = store.read(cx).written.clone();
    watch(workspace::ui_file(&root), written, cx);
}

fn watch(path: PathBuf, written: std::sync::Arc<std::sync::Mutex<Option<String>>>, cx: &mut App) {
    let mut seen = stamp(&path);
    cx.spawn(async move |cx| {
        let timer = |d| cx.background_executor().timer(d);
        loop {
            timer(POLL).await;
            let mut now = stamp(&path);
            if now == seen {
                continue;
            }
            // Wait for the writer to finish.
            loop {
                timer(SETTLE).await;
                let again = stamp(&path);
                if again == now {
                    break;
                }
                now = again;
            }
            seen = now;
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let ours = written.lock().map(|w| w.as_deref() == Some(text.as_str())).unwrap_or(false);
            if ours {
                continue;
            }
            if cx.update(|cx| external(&text, cx)).is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Apply `ui.json` as edited outside Pitwall; returns what was reported.
pub fn external(text: &str, cx: &mut App) -> Vec<String> {
    let current = crate::ui_state::get(cx);
    let problems = match serde_json::from_str::<Value>(text) {
        Err(e) => vec![format!("ui.json isn't valid JSON ({e}); the settings stay as they are.")],
        Ok(raw) => match super::data::from_file(raw, &current) {
            Err(e) => vec![format!("ui.json wasn't applied: {e}.")],
            Ok((state, problems)) => {
                if state != current {
                    crate::ui_state::update(cx, |s| *s = state);
                }
                problems
                    .into_iter()
                    .map(|p| format!("{p}; kept the last good value."))
                    .collect()
            }
        },
    };
    if !problems.is_empty() {
        report(&problems.join("\n"), cx);
    }
    problems
}

/// A toast in a main window (stderr when there is none).
fn report(text: &str, cx: &mut App) {
    eprintln!("pitwall: {text}");
    let screen = cx
        .windows()
        .into_iter()
        .filter_map(|w| w.downcast::<crate::ui::MainView>())
        .find_map(|w| w.read(cx).ok().and_then(|v| v.screen.clone()));
    if let Some(screen) = screen {
        let text = text.to_string();
        screen.update(cx, |s, cx| s.failed("apply ui.json", text, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn edits_apply_live_and_bad_values_keep_the_last_good_one(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("pw-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ui.json");
        std::fs::write(&file, r#"{"v":1,"theme":"dark"}"#).unwrap();
        let root = dir.clone();
        cx.update(|cx| {
            crate::ui_state::init(Some(root), cx);
            start(cx);
        });
        let wait = |cx: &mut TestAppContext| {
            for _ in 0..4 {
                cx.executor().advance_clock(POLL + SETTLE);
                cx.run_until_parked();
            }
        };
        // Someone edits the file: applied.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&file, r#"{"v":1,"theme":"light","density":"dense"}"#).unwrap();
        wait(cx);
        cx.update(|cx| {
            let s = crate::ui_state::get(cx);
            assert_eq!(s.theme, crate::theme::ThemePref::Light);
            assert_eq!(s.density, crate::theme::Density::Dense);
        });
        // A bad value: the rest applies, it keeps the last good one, and
        // the corrected file is written back.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&file, r#"{"v":1,"theme":"purple","density":"comfortable"}"#).unwrap();
        wait(cx);
        cx.update(|cx| {
            let s = crate::ui_state::get(cx);
            assert_eq!(s.theme, crate::theme::ThemePref::Light);
            assert_eq!(s.density, crate::theme::Density::Comfortable);
        });
        let back: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(back["theme"], "light");
        // Broken JSON changes nothing.
        cx.update(|cx| {
            let p = external("{nope", cx);
            assert!(p[0].contains("isn't valid JSON"), "{p:?}");
            assert_eq!(crate::ui_state::get(cx).density, crate::theme::Density::Comfortable);
            let p = external(r#"{"v":1,"fontSize":30}"#, cx);
            assert!(p[0].contains("fontSize") && p[0].contains("kept the last good value"), "{p:?}");
            assert!(external(r#"{"v":1,"fontSize":20}"#, cx).is_empty());
            assert_eq!(crate::theme::appearance(cx).font_size, 20);
        });
        let _ = std::fs::remove_dir_all(&dir);
    }
}
