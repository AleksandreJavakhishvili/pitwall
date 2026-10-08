//! Benchmark hook for `scripts/bench.sh` (docs/spec/perf.md). Inert unless
//! the app runs with `PITWALL_BENCH=1` — and the bench script only ever sets
//! that together with an isolated `PITWALL_HOME`.
//!
//! - Commands: the script writes `<n> <command>` to `$PITWALL_HOME/bench-cmd`;
//!   each new line is sent to the main window as a `bench` event (the UI side
//!   is `src/lib/bench.ts`: wall on/off, review on/off, visit-all, …).
//! - Readiness: when the UI has rendered with its agents loaded it emits
//!   `bench-ready`; the time since the process started goes to
//!   `$PITWALL_HOME/bench-ready` (cold start to first window).
//! - Unobtrusive: the instance never activates (accessory app: no Dock icon,
//!   no focus stealing) and its windows sit off-screen. They are still shown
//!   and rendered, but macOS may treat them as occluded, so WebKit can drop
//!   GPU tiles sooner than for a window you look at (docs/spec/perf.md).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Listener, Manager, PhysicalPosition};

use pitwall_core::paths::Paths;

pub fn enabled() -> bool {
    std::env::var_os("PITWALL_BENCH").is_some_and(|v| v == "1")
}

/// Where bench windows go: far off the left edge of any display.
const OFF_SCREEN: PhysicalPosition<i32> = PhysicalPosition { x: -20_000, y: 0 };

/// Before the windows appear: never become the active app.
pub fn prepare(app: &mut tauri::App) {
    if enabled() {
        #[cfg(target_os = "macos")]
        app.set_activation_policy(tauri::ActivationPolicy::Accessory);
        let _ = app;
    }
}

pub fn start(app: &AppHandle, started: Instant) {
    if !enabled() {
        return;
    }
    // `PITWALL_BENCH_VISIBLE=1` (bench.py --visible): stay where the config
    // put the window, on screen and above other windows (never occluded, so
    // WebKit keeps its GPU tiles), so the compositor and WebKit's GPU process
    // work as for a window you look at (window material included).
    let visible = std::env::var_os("PITWALL_BENCH_VISIBLE").is_some_and(|v| v == "1");
    for w in app.webview_windows().values() {
        let _ = if visible { w.set_always_on_top(true) } else { w.set_position(OFF_SCREEN) };
    }
    let root: PathBuf = Paths::default_root();
    let ready_file = root.join("bench-ready");
    app.listen("bench-ready", move |_| {
        let ms = started.elapsed().as_millis();
        let _ = std::fs::write(&ready_file, format!("{ms}\n"));
    });
    let app = app.clone();
    let cmd_file = root.join("bench-cmd");
    std::thread::Builder::new()
        .name("bench".into())
        .spawn(move || {
            let mut last = String::new();
            loop {
                std::thread::sleep(Duration::from_millis(200));
                let Ok(text) = std::fs::read_to_string(&cmd_file) else { continue };
                let text = text.trim().to_string();
                if text.is_empty() || text == last {
                    continue;
                }
                last = text.clone();
                // "<n> <command>": the counter only makes repeats distinct.
                let command = text.split_once(' ').map(|(_, c)| c).unwrap_or(&text).to_string();
                let _ = app.emit_to("main", "bench", command);
            }
        })
        .expect("spawn bench thread");
}
