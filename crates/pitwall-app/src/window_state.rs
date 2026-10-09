//! The main window's size and place, remembered across runs (Tauri:
//! `windows.json`, `src-tauri/src/windows.rs`). Kept in its own file,
//! `<data>/app-windows.json`, in logical pixels: the Tauri file stores
//! physical pixels per webview label, and both apps may share a folder after
//! the switch-over (phase 9 folds the two together).

use std::path::{Path, PathBuf};

use gpui::{point, px, size, Bounds, Pixels, WindowBounds};
use serde::{Deserialize, Serialize};

/// The first window, before anything was saved.
pub const DEFAULT_SIZE: (f32, f32) = (1400., 900.);
/// Below this the sidebar and a terminal no longer fit (tauri.conf.json's
/// `minWidth`/`minHeight`).
pub const MIN_SIZE: (f32, f32) = (640., 480.);

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Windowed,
    Maximized,
    Fullscreen,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub state: State,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    main: Option<Saved>,
}

pub fn file(root: &Path) -> PathBuf {
    root.join("app-windows.json")
}

impl Saved {
    pub fn from_bounds(b: WindowBounds) -> Saved {
        let (state, r) = match b {
            WindowBounds::Windowed(r) => (State::Windowed, r),
            WindowBounds::Maximized(r) => (State::Maximized, r),
            WindowBounds::Fullscreen(r) => (State::Fullscreen, r),
        };
        Saved {
            state,
            x: r.origin.x.into(),
            y: r.origin.y.into(),
            width: r.size.width.into(),
            height: r.size.height.into(),
        }
    }

    /// As window bounds, never smaller than [`MIN_SIZE`].
    pub fn to_bounds(self) -> WindowBounds {
        let r = Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(
                px(self.width.max(MIN_SIZE.0)),
                px(self.height.max(MIN_SIZE.1)),
            ),
        };
        match self.state {
            State::Windowed => WindowBounds::Windowed(r),
            State::Maximized => WindowBounds::Maximized(r),
            State::Fullscreen => WindowBounds::Fullscreen(r),
        }
    }

    /// Still (at least partly) on one of `displays`: a display that was
    /// unplugged must not leave the window out of reach.
    pub fn visible_on(&self, displays: &[Bounds<Pixels>]) -> bool {
        let r = Bounds {
            origin: point(px(self.x), px(self.y)),
            size: size(px(self.width), px(self.height)),
        };
        displays.iter().any(|d| d.intersects(&r))
    }
}

pub fn load(root: &Path) -> Option<Saved> {
    let text = std::fs::read_to_string(file(root)).ok()?;
    serde_json::from_str::<File>(&text).ok()?.main
}

pub fn save(root: &Path, saved: Saved) -> Result<(), String> {
    let path = file(root);
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let text =
        serde_json::to_string_pretty(&File { main: Some(saved) }).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// Bounds waiting for their debounced save, written at once on quit
/// (Tauri saves `windows.json` on exit too).
static PENDING: std::sync::Mutex<Option<(std::path::PathBuf, Saved)>> = std::sync::Mutex::new(None);

/// Remember `saved` as the next write for `root`.
pub fn pend(root: &Path, saved: Saved) {
    if let Ok(mut p) = PENDING.lock() {
        *p = Some((root.to_path_buf(), saved));
    }
}

/// Write the pending bounds now, if any (the debounce or quitting).
pub fn flush() {
    let next = PENDING.lock().ok().and_then(|mut p| p.take());
    if let Some((root, saved)) = next {
        if let Err(e) = save(&root, saved) {
            eprintln!("pitwall: could not save the window's bounds: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds {
            origin: point(px(x), px(y)),
            size: size(px(w), px(h)),
        }
    }

    #[test]
    fn bounds_survive_a_round_trip_on_disk() {
        let dir = std::env::temp_dir().join(format!("pw-app-win-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir), None);
        let saved = Saved::from_bounds(WindowBounds::Maximized(rect(10., 20., 1000., 700.)));
        save(&dir, saved).unwrap();
        assert_eq!(load(&dir), Some(saved));
        assert_eq!(
            load(&dir).unwrap().to_bounds(),
            WindowBounds::Maximized(rect(10., 20., 1000., 700.))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restored_windows_keep_the_minimum_size() {
        let tiny = Saved {
            state: State::Windowed,
            x: 0.,
            y: 0.,
            width: 100.,
            height: 50.,
        };
        assert_eq!(
            tiny.to_bounds(),
            WindowBounds::Windowed(rect(0., 0., MIN_SIZE.0, MIN_SIZE.1))
        );
    }

    #[test]
    fn a_window_on_a_missing_display_is_not_restored_there() {
        let displays = [rect(0., 0., 1440., 900.)];
        let on = Saved {
            state: State::Windowed,
            x: 100.,
            y: 100.,
            width: 800.,
            height: 600.,
        };
        let off = Saved { x: 5000., ..on };
        assert!(on.visible_on(&displays));
        assert!(!off.visible_on(&displays));
    }
}
