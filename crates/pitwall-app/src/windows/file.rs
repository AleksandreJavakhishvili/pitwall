//! `windows.json`: each window's label, the space it was opened for and its
//! bounds, in the Tauri app's format (`src-tauri/src/windows.rs`) so the
//! file carries over at the switch. Bounds are physical pixels (Tauri's
//! `Moved` / `Resized` events): the outer position and the inner size.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The first window's label; extra windows are `pitwall-N`.
pub const MAIN: &str = "main";
const PREFIX: &str = "pitwall-";

/// Saved sizes below this are ignored (`apply_bounds`).
pub const MIN_SAVED: (u32, u32) = (200, 150);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Bounds {
    /// Big enough to be worth restoring.
    pub fn usable(&self) -> bool {
        self.width >= MIN_SAVED.0 && self.height >= MIN_SAVED.1
    }
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

#[derive(Debug, Default, Serialize, Deserialize)]
struct WindowsFile {
    windows: Vec<WindowEntry>,
}

pub fn path(root: &Path) -> PathBuf {
    root.join("windows.json")
}

/// The saved windows (none when the file is missing or unreadable).
pub fn load(root: &Path) -> Vec<WindowEntry> {
    std::fs::read_to_string(path(root))
        .ok()
        .and_then(|s| serde_json::from_str::<WindowsFile>(&s).ok())
        .map(|f| f.windows)
        .unwrap_or_default()
}

/// Write the file atomically (a temp file renamed over it), pretty-printed
/// as Tauri writes it.
pub fn save(root: &Path, windows: &[WindowEntry]) -> Result<(), String> {
    let file = WindowsFile {
        windows: windows.to_vec(),
    };
    let text = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let target = path(root);
    let tmp = target.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())
}

/// `pitwall-3` → 3; `main` and anything else → `None`.
pub fn label_number(label: &str) -> Option<u32> {
    label.strip_prefix(PREFIX)?.parse().ok()
}

/// The next free label after every one in use or saved.
pub fn next_label<'a>(taken: impl IntoIterator<Item = &'a str>) -> String {
    let n = taken
        .into_iter()
        .filter_map(label_number)
        .max()
        .unwrap_or(0)
        + 1;
    format!("{PREFIX}{n}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tauri_file_reads_and_writes_the_same() {
        // As src-tauri writes it (camelCase, physical pixels, main first).
        let tauri = r#"{
  "windows": [
    { "label": "main", "spaceId": null, "bounds": { "x": 10, "y": 20, "width": 2800, "height": 1800 } },
    { "label": "pitwall-2", "spaceId": "space-a1", "bounds": { "x": -1600, "y": 40, "width": 1200, "height": 900 } },
    { "label": "pitwall-3", "spaceId": "space-b2" }
  ]
}"#;
        let dir = std::env::temp_dir().join(format!("pw-windows-file-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(path(&dir), tauri).unwrap();
        let w = load(&dir);
        assert_eq!(w.len(), 3);
        assert_eq!(w[1].space_id.as_deref(), Some("space-a1"));
        assert_eq!(w[1].bounds.unwrap().x, -1600);
        assert_eq!(w[2].bounds, None);
        save(&dir, &w).unwrap();
        let text = std::fs::read_to_string(path(&dir)).unwrap();
        assert!(text.contains("\"spaceId\": \"space-a1\""), "{text}");
        assert_eq!(load(&dir), w);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_or_broken_file_is_no_windows() {
        let dir = std::env::temp_dir().join(format!("pw-windows-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load(&dir).is_empty());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(path(&dir), "{ not json").unwrap();
        assert!(load(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn labels_count_up_past_every_one_taken() {
        assert_eq!(label_number("pitwall-3"), Some(3));
        assert_eq!(label_number("main"), None);
        assert_eq!(label_number("pitwall-x"), None);
        assert_eq!(next_label([MAIN]), "pitwall-1");
        assert_eq!(next_label(["main", "pitwall-4", "pitwall-2"]), "pitwall-5");
    }

    #[test]
    fn tiny_bounds_are_not_restored() {
        let b = Bounds {
            x: 0,
            y: 0,
            width: 199,
            height: 600,
        };
        assert!(!b.usable());
        assert!(Bounds { width: 800, ..b }.usable());
    }
}
