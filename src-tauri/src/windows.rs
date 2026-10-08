//! Extra windows (one per moved-out space), their bounds, and the UI-owned
//! state blob shared by all windows.
//!
//! Files: `~/.pitwall/ui.json` is the UI's opaque blob; `~/.pitwall/windows.json`
//! is backend-owned (window label, space and bounds) so windows come back
//! where they were.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};

use pitwall_core::paths::Paths;

pub const MAIN: &str = "main";
const PREFIX: &str = "pitwall-";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowEntry {
    pub label: String,
    #[serde(default)]
    pub space_id: Option<String>,
    #[serde(default)]
    pub bounds: Option<Bounds>,
}

#[derive(Default, Serialize, Deserialize)]
struct WindowsFile {
    windows: Vec<WindowEntry>,
}

struct Tracker {
    entries: Vec<WindowEntry>,
    dirty_since: Option<Instant>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn tracker() -> &'static Mutex<Tracker> {
    static T: OnceLock<Mutex<Tracker>> = OnceLock::new();
    T.get_or_init(|| {
        let entries = std::fs::read_to_string(windows_file())
            .ok()
            .and_then(|s| serde_json::from_str::<WindowsFile>(&s).ok())
            .map(|f| f.windows)
            .unwrap_or_default();
        Mutex::new(Tracker { entries, dirty_since: None })
    })
}

static QUITTING: AtomicBool = AtomicBool::new(false);

fn windows_file() -> std::path::PathBuf {
    Paths::default_root().join("windows.json")
}

fn ui_file() -> std::path::PathBuf {
    Paths::default_root().join("ui.json")
}

fn write_atomic(path: &std::path::Path, contents: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, contents).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

// ------------------------------------------------------------ UI blob

fn ui_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

pub fn get_ui_state() -> Value {
    let _g = lock(ui_lock());
    std::fs::read_to_string(ui_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

pub fn set_ui_state(app: &AppHandle, state: Value, source_window: &str) -> Result<(), String> {
    {
        let _g = lock(ui_lock());
        let text = serde_json::to_string(&state).map_err(|e| e.to_string())?;
        write_atomic(&ui_file(), &text)?;
    }
    let _ = app.emit("ui-state-changed", json!({ "state": state, "sourceWindow": source_window }));
    Ok(())
}

// ------------------------------------------------------------ windows

/// Percent-encode for a query value.
pub fn encode_query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn label_number(label: &str) -> Option<u32> {
    label.strip_prefix(PREFIX)?.parse().ok()
}

fn next_label(app: &AppHandle) -> String {
    let open = app.webview_windows().into_keys().filter_map(|l| label_number(&l));
    let saved: Vec<u32> = lock(tracker()).entries.iter().filter_map(|e| label_number(&e.label)).collect();
    let n = open.chain(saved).max().unwrap_or(0) + 1;
    format!("{PREFIX}{n}")
}

fn bounds_visible(app: &AppHandle, b: &Bounds) -> bool {
    let Ok(monitors) = app.available_monitors() else { return false };
    monitors.iter().any(|m| {
        let (p, s) = (m.position(), m.size());
        let cx = b.x + 40;
        let cy = b.y + 20;
        cx >= p.x && cy >= p.y && cx < p.x + s.width as i32 && cy < p.y + s.height as i32
    })
}

fn apply_bounds(app: &AppHandle, window: &tauri::WebviewWindow, bounds: Option<Bounds>) {
    let Some(b) = bounds else { return };
    if b.width < 200 || b.height < 150 {
        return;
    }
    let _ = window.set_size(PhysicalSize::new(b.width, b.height));
    if bounds_visible(app, &b) {
        let _ = window.set_position(PhysicalPosition::new(b.x, b.y));
    }
}

fn build(app: &AppHandle, label: &str, space_id: &str, bounds: Option<Bounds>) -> Result<(), String> {
    let url = format!("index.html?space={}", encode_query(space_id));
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .title("Pitwall")
        .inner_size(1400.0, 900.0)
        .min_inner_size(640.0, 480.0)
        // In-webview HTML5 drag & drop (agents between tiles) needs Tauri's
        // native file-drop handler off, as for `main` in tauri.conf.json.
        .disable_drag_drop_handler()
        // Glass on Windows: Mica only shows behind a transparent window
        // (platform/glass.rs; main's is in tauri.windows.conf.json).
        .transparent(crate::platform::glass::TRANSPARENT_WINDOWS)
        .build()
        .map_err(|e| format!("could not open window: {e}"))?;
    apply_bounds(app, &window, bounds);
    Ok(())
}

pub fn open_window(app: &AppHandle, space_id: &str) -> Result<String, String> {
    let label = next_label(app);
    build(app, &label, space_id, None)?;
    let mut t = lock(tracker());
    t.entries.retain(|e| e.label != label);
    t.entries.push(WindowEntry {
        label: label.clone(),
        space_id: Some(space_id.to_string()),
        bounds: None,
    });
    t.dirty_since = Some(Instant::now());
    Ok(label)
}

pub fn focus_window(app: &AppHandle, label: &str) -> Result<(), String> {
    let w = app.get_webview_window(label).ok_or_else(|| format!("no window {label}"))?;
    let _ = w.show();
    let _ = w.unminimize();
    let _ = w.set_focus();
    Ok(())
}

pub fn list_windows(app: &AppHandle) -> Vec<String> {
    let mut labels: Vec<String> = app.webview_windows().into_keys().collect();
    labels.sort_by_key(|l| (l != MAIN, label_number(l), l.clone()));
    labels
}

/// Restore main's bounds and reopen saved secondary windows. Call in setup.
pub fn restore(app: &AppHandle) {
    let entries = lock(tracker()).entries.clone();
    for e in entries {
        if e.label == MAIN {
            if let Some(w) = app.get_webview_window(MAIN) {
                apply_bounds(app, &w, e.bounds);
            }
        } else if let (Some(space), Some(_)) = (&e.space_id, label_number(&e.label)) {
            if let Err(err) = build(app, &e.label, space, e.bounds) {
                eprintln!("pitwall: {err}");
            }
        }
    }
    start_saver();
}

pub fn on_bounds(label: &str, position: Option<PhysicalPosition<i32>>, size: Option<PhysicalSize<u32>>) {
    if QUITTING.load(Ordering::Relaxed) {
        return;
    }
    let mut t = lock(tracker());
    let idx = match t.entries.iter().position(|e| e.label == label) {
        Some(i) => i,
        None if label == MAIN => {
            t.entries.push(WindowEntry { label: MAIN.into(), space_id: None, bounds: None });
            t.entries.len() - 1
        }
        None => return,
    };
    let b = t.entries[idx].bounds.get_or_insert(Bounds { x: 0, y: 0, width: 0, height: 0 });
    if let Some(p) = position {
        (b.x, b.y) = (p.x, p.y);
    }
    if let Some(s) = size {
        if s.width == 0 || s.height == 0 {
            return;
        }
        (b.width, b.height) = (s.width, s.height);
    }
    t.dirty_since = Some(Instant::now());
}

/// A secondary window was closed by the user: forget it.
pub fn on_closed(app: &AppHandle, label: &str) {
    if QUITTING.load(Ordering::Relaxed) || label == MAIN {
        return;
    }
    crate::platform::glass::forget(label);
    {
        let mut t = lock(tracker());
        t.entries.retain(|e| e.label != label);
        t.dirty_since = Some(Instant::now());
    }
    let _ = app.emit("window-closed", json!({ "label": label }));
}

pub fn quitting() {
    QUITTING.store(true, Ordering::Relaxed);
    flush();
}

fn flush() {
    let entries = {
        let mut t = lock(tracker());
        if t.dirty_since.take().is_none() {
            return;
        }
        t.entries.clone()
    };
    if let Ok(text) = serde_json::to_string_pretty(&WindowsFile { windows: entries }) {
        if let Err(e) = write_atomic(&windows_file(), &text) {
            eprintln!("pitwall: could not save windows: {e}");
        }
    }
}

/// Debounced saver: writes windows.json 500ms after the last change.
fn start_saver() {
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_millis(250));
        let settled = lock(tracker())
            .dirty_since
            .is_some_and(|t| t.elapsed() >= Duration::from_millis(500));
        if settled {
            flush();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_query_values() {
        assert_eq!(encode_query("abc-1_2.~"), "abc-1_2.~");
        assert_eq!(encode_query("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
    }

    #[test]
    fn label_numbers() {
        assert_eq!(label_number("pitwall-3"), Some(3));
        assert_eq!(label_number("main"), None);
        assert_eq!(label_number("pitwall-x"), None);
    }

    #[test]
    fn windows_file_roundtrip() {
        let f = WindowsFile {
            windows: vec![WindowEntry {
                label: "pitwall-1".into(),
                space_id: Some("s".into()),
                bounds: Some(Bounds { x: -5, y: 10, width: 800, height: 600 }),
            }],
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("\"spaceId\":\"s\""));
        let back: WindowsFile = serde_json::from_str(&s).unwrap();
        assert_eq!(back.windows, f.windows);
    }
}
