//! The settings kept in `ui.json`, in the Tauri app's keys and values
//! (`src/state/workspace.ts` `UiState`), so they carry over when the release
//! switches: `theme`, `look`, `reduceMotion`, `density`, `fontSize`,
//! `hideElsewhere`.
//!
//! `ui.json` holds much more (spaces, layout, Wall, tile fonts). This module
//! only touches its own keys: every write re-reads the file and merges, so
//! whatever else is in it stays. At merge, these fields move into the typed
//! `UiState` entity (docs/spec/gpui/README.md "State model").

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Map, Value};

use super::appearance::{clamp_font, Density, Inputs, Look, ThemePref, DEFAULT_FONT};

pub const FILE: &str = "ui.json";

/// The settings Settings edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiPrefs {
    pub theme: ThemePref,
    pub look: Look,
    pub reduce_motion: bool,
    pub density: Density,
    /// Base terminal font, px (9–22).
    pub font_size: u8,
    /// Settings → "Show agents running elsewhere" is the inverse.
    pub hide_elsewhere: bool,
}

impl Default for UiPrefs {
    fn default() -> Self {
        UiPrefs {
            theme: ThemePref::System,
            look: Look::Flat,
            reduce_motion: false,
            density: Density::Compact,
            font_size: DEFAULT_FONT,
            hide_elsewhere: false,
        }
    }
}

fn parse<T: serde::de::DeserializeOwned>(v: Option<&Value>) -> Option<T> {
    serde_json::from_value(v?.clone()).ok()
}

impl UiPrefs {
    /// From the blob, each field sanitized as `workspace.ts` `sanitize` does.
    pub fn from_json(v: &Value) -> UiPrefs {
        let d = UiPrefs::default();
        UiPrefs {
            theme: parse(v.get("theme")).unwrap_or(d.theme),
            look: parse(v.get("look")).unwrap_or(d.look),
            reduce_motion: v.get("reduceMotion") == Some(&Value::Bool(true)),
            density: parse(v.get("density")).unwrap_or(d.density),
            font_size: v
                .get("fontSize")
                .and_then(Value::as_f64)
                .map(|n| clamp_font(n.round() as i32))
                .unwrap_or(d.font_size),
            hide_elsewhere: v.get("hideElsewhere") == Some(&Value::Bool(true)),
        }
    }

    /// Write these keys into `blob`, keeping every other key.
    pub fn merge_into(&self, blob: &mut Value) {
        if !blob.is_object() {
            *blob = Value::Object(Map::new());
        }
        let o = blob.as_object_mut().expect("an object");
        o.entry("v").or_insert(Value::from(1));
        o.insert("theme".into(), to(&self.theme));
        o.insert("look".into(), to(&self.look));
        o.insert("reduceMotion".into(), Value::Bool(self.reduce_motion));
        o.insert("density".into(), to(&self.density));
        o.insert("fontSize".into(), Value::from(self.font_size));
        o.insert("hideElsewhere".into(), Value::Bool(self.hide_elsewhere));
    }

    pub fn inputs(&self) -> Inputs {
        Inputs {
            pref: self.theme,
            look: self.look,
            reduce_motion: self.reduce_motion,
            density: self.density,
            font_size: self.font_size,
        }
    }
}

fn to<T: serde::Serialize>(x: &T) -> Value {
    serde_json::to_value(x).unwrap_or(Value::Null)
}

pub fn file(root: &Path) -> PathBuf {
    root.join(FILE)
}

/// One writer at a time in this process (the Tauri app locks the same way).
fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: Mutex<()> = Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
}

fn read_blob(root: &Path) -> Value {
    std::fs::read_to_string(file(root))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

pub fn load(root: &Path) -> UiPrefs {
    let _g = lock();
    UiPrefs::from_json(&read_blob(root))
}

/// Merge `prefs` into the file on disk, atomically (temp file + rename).
pub fn save(root: &Path, prefs: &UiPrefs) -> Result<(), String> {
    let _g = lock();
    let mut blob = read_blob(root);
    prefs.merge_into(&mut blob);
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let path = file(root);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string(&blob).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_tauri_apps_keys_and_sanitizes() {
        let blob = json!({
            "v": 1, "theme": "light", "look": "glass", "reduceMotion": true,
            "density": "dense", "fontSize": 15, "hideElsewhere": true, "spaces": []
        });
        let p = UiPrefs::from_json(&blob);
        assert_eq!(p.theme, ThemePref::Light);
        assert_eq!(p.look, Look::Glass);
        assert!(p.reduce_motion && p.hide_elsewhere);
        assert_eq!(p.density, Density::Dense);
        assert_eq!(p.font_size, 15);

        let junk = json!({ "theme": "purple", "look": 3, "reduceMotion": "yes", "density": "huge", "fontSize": 99 });
        let p = UiPrefs::from_json(&junk);
        assert_eq!(p.theme, ThemePref::System);
        assert_eq!(p.look, Look::Flat);
        assert!(!p.reduce_motion);
        assert_eq!(p.density, Density::Compact);
        assert_eq!(p.font_size, 22, "clamped like clampFont");
        assert_eq!(UiPrefs::from_json(&Value::Null), UiPrefs::default());
    }

    #[test]
    fn writing_keeps_the_rest_of_the_blob() {
        let dir = std::env::temp_dir().join(format!("pw-app-prefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let other = json!({ "v": 1, "spaces": [{ "id": "s1", "name": "All" }], "wall": ["main"], "theme": "dark" });
        std::fs::write(file(&dir), other.to_string()).unwrap();
        let mut p = load(&dir);
        assert_eq!(p.theme, ThemePref::Dark);
        p.theme = ThemePref::Light;
        p.look = Look::Glass;
        p.font_size = 14;
        save(&dir, &p).unwrap();
        let back: Value =
            serde_json::from_str(&std::fs::read_to_string(file(&dir)).unwrap()).unwrap();
        assert_eq!(back["spaces"][0]["name"], "All");
        assert_eq!(back["wall"][0], "main");
        assert_eq!(back["theme"], "light");
        assert_eq!(back["look"], "glass");
        assert_eq!(back["fontSize"], 14);
        assert_eq!(back["reduceMotion"], false);
        assert_eq!(load(&dir), p);
        // A missing file is created with the version.
        let fresh = dir.join("fresh");
        save(&fresh, &UiPrefs::default()).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(file(&fresh)).unwrap()).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["density"], "compact");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
