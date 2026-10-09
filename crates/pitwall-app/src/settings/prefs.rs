//! The settings Settings edits, as a small copy of their `ui.json` fields
//! (`theme`, `look`, `reduceMotion`, `density`, `fontSize`,
//! `hideElsewhere`). The file itself is `crate::ui_state`.

use crate::theme::{clamp_font, Density, Look, ThemePref};
use crate::ui_state::UiState;

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
        UiPrefs::of(&UiState::default())
    }
}

impl UiPrefs {
    /// Read from the state, the font clamped as Settings shows it.
    pub fn of(s: &UiState) -> UiPrefs {
        UiPrefs {
            theme: s.theme,
            look: s.look,
            reduce_motion: s.reduce_motion,
            density: s.density,
            font_size: clamp_font(s.font_size.round() as i32),
            hide_elsewhere: s.hide_elsewhere,
        }
    }

    /// Write into the state, keeping everything else.
    pub fn write(&self, s: &mut UiState) {
        s.theme = self.theme;
        s.look = self.look;
        s.reduce_motion = self.reduce_motion;
        s.density = self.density;
        if clamp_font(s.font_size.round() as i32) != self.font_size {
            s.font_size = self.font_size as f64;
        }
        s.hide_elsewhere = self.hide_elsewhere;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::DEFAULT_FONT;
    use serde_json::json;

    #[test]
    fn reads_the_tauri_apps_keys_and_sanitizes() {
        let blob = json!({
            "v": 1, "theme": "light", "look": "glass", "reduceMotion": true,
            "density": "dense", "fontSize": 15, "hideElsewhere": true, "spaces": []
        });
        let p = UiPrefs::of(&UiState::sanitize(blob));
        assert_eq!(p.theme, ThemePref::Light);
        assert_eq!(p.look, Look::Glass);
        assert!(p.reduce_motion && p.hide_elsewhere);
        assert_eq!(p.density, Density::Dense);
        assert_eq!(p.font_size, 15);

        let junk = json!({ "v": 1, "theme": "purple", "look": 3, "reduceMotion": "yes", "density": "huge", "fontSize": 99 });
        let s = UiState::sanitize(junk);
        let p = UiPrefs::of(&s);
        assert_eq!(p.theme, ThemePref::System);
        assert_eq!(p.look, Look::Flat);
        assert!(!p.reduce_motion);
        assert_eq!(p.density, Density::Compact);
        assert_eq!(p.font_size, 22, "clamped like clampFont");
        assert_eq!(s.spaces.len(), 1, "a bad setting doesn't reset the file");
        assert_eq!(UiPrefs::default().font_size, DEFAULT_FONT);
    }

    #[test]
    fn writing_keeps_the_rest_of_the_state() {
        let mut s = UiState::sanitize(json!({ "v": 1, "wall": ["main"], "fontSize": 13.5 }));
        let mut p = UiPrefs::of(&s);
        p.theme = ThemePref::Light;
        p.write(&mut s);
        assert_eq!(s.wall, vec!["main".to_string()]);
        assert_eq!(s.font_size, 13.5, "an unchanged font keeps its exact value");
        p.font_size = 16;
        p.write(&mut s);
        assert_eq!(s.font_size, 16.0);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["theme"], "light");
        assert_eq!(v["reduceMotion"], false);
    }
}
