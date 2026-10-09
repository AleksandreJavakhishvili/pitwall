//! Benchmark hook for `scripts/bench.sh` (docs/spec/perf.md), as the Tauri
//! app's `src-tauri/src/bench.rs` + `src/lib/bench.ts`. Inert unless the app
//! runs with `PITWALL_BENCH=1` (the bench script sets it only together with
//! an isolated `PITWALL_HOME`).
//!
//! - Commands: the script writes `<n> <command>` to `$PITWALL_HOME/bench-cmd`;
//!   each new line goes to the main window: `wall on|off`, `review on|off`,
//!   `visit-all`, `palette on|off`, `settings on|off`.
//! - Readiness: once the main window has drawn with its agents loaded, the
//!   time since start goes to `$PITWALL_HOME/bench-ready` (cold start).
//! - Unobtrusive: the app never activates (macOS: accessory, no Dock icon)
//!   and its window sits off-screen; `PITWALL_BENCH_VISIBLE=1` keeps it where
//!   it is, floating above other windows (never occluded) without focus.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{App, AsyncApp, WindowHandle};

use crate::ui::MainView;

pub fn enabled() -> bool {
    std::env::var_os("PITWALL_BENCH").is_some_and(|v| v == "1")
}

/// `PITWALL_BENCH_VISIBLE=1`: on screen, above the others.
pub fn visible() -> bool {
    enabled() && std::env::var_os("PITWALL_BENCH_VISIBLE").is_some_and(|v| v == "1")
}

/// Where a bench window goes: far off the left edge of any display.
pub const OFF_SCREEN: (f32, f32) = (-20_000., 0.);

/// One line of `bench-cmd` without its counter (`"3 wall on"` → `"wall on"`).
pub fn command(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    Some(
        line.split_once(' ')
            .map(|(_, c)| c)
            .unwrap_or(line)
            .to_string(),
    )
}

/// Before any window opens: never become the active app.
pub fn prepare() {
    #[cfg(target_os = "macos")]
    if enabled() {
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            NSApplication::sharedApplication(mtm)
                .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        }
    }
}

/// Watch `bench-cmd` and write `bench-ready` (call once the main window is
/// open).
pub fn start(main: WindowHandle<MainView>, root: PathBuf, started: Instant, cx: &mut App) {
    if !enabled() {
        return;
    }
    cx.spawn(async move |cx: &mut AsyncApp| {
        // Ready: the window has drawn with the agents loaded.
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            let loaded = main
                .read_with(cx, |v, cx| {
                    v.screen.is_some()
                        && cx
                            .windows()
                            .iter()
                            .any(|w| w.window_id() == main.window_id())
                })
                .unwrap_or(false);
            if loaded {
                let ms = started.elapsed().as_millis();
                let _ = std::fs::write(root.join("bench-ready"), format!("{ms}\n"));
                break;
            }
        }
        let file = root.join("bench-cmd");
        let mut last = String::new();
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let text = text.trim().to_string();
            if text.is_empty() || text == last {
                continue;
            }
            last = text.clone();
            let Some(cmd) = command(&text) else { continue };
            if main
                .update(cx, |v, window, cx| v.bench(&cmd, window, cx))
                .is_err()
            {
                break;
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_drop_their_counter() {
        assert_eq!(command("3 wall on\n").as_deref(), Some("wall on"));
        assert_eq!(command("visit-all").as_deref(), Some("visit-all"));
        assert_eq!(command("  "), None);
    }
}
