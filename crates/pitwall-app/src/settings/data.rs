//! Settings as data: the registry (`pitwall_proto::settings`) applied to
//! the app's `ui.json` state ([`crate::ui_state`]). The Settings pages, the
//! CLI's `settings.*` ([`super::backend`]) and external edits of the file
//! ([`super::watch`]) all change settings through here.

use gpui::App;
use serde_json::{Map, Value};

use pitwall_proto::settings::{Page, Setting, PAGE_KEY, SETTINGS};

use crate::ui_state::UiState;

/// The state as `ui.json` holds it.
pub fn file_value(state: &UiState) -> Value {
    serde_json::to_value(state).unwrap_or(Value::Null)
}

fn object(state: &UiState) -> Map<String, Value> {
    match file_value(state) {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// A `ui.json` setting's value in `state` (the default when unusable, as
/// the app reads it). `None` for settings kept outside `ui.json`.
pub fn value(state: &UiState, def: &Setting) -> Option<Value> {
    def.read_file(&file_value(state))
        .map(|r| r.unwrap_or_else(|_| def.default_value()))
}

/// `state` with `def` set to `v` (an allowed value).
pub fn with(state: &UiState, def: &Setting, v: &Value) -> UiState {
    let mut obj = object(state);
    def.write_file(&mut obj, v);
    UiState::sanitize(Value::Object(obj))
}

/// The file `raw` as the new state: values the registry doesn't allow keep
/// `current`'s (the last good one), each reported. A file that isn't
/// Pitwall's settings at all is refused.
pub fn from_file(raw: Value, current: &UiState) -> Result<(UiState, Vec<String>), String> {
    let Value::Object(mut obj) = raw else {
        return Err("it isn't a JSON object".into());
    };
    if obj.get("v").and_then(Value::as_u64) != Some(1) {
        return Err("\"v\" must be 1".into());
    }
    let now = file_value(current);
    let mut problems = Vec::new();
    for def in SETTINGS {
        if let Some(Err(e)) = def.read_file(&Value::Object(obj.clone())) {
            problems.push(e);
            match def.read_file(&now) {
                Some(Ok(good)) => def.write_file(&mut obj, &good),
                _ => {
                    if let Some(k) = def.file_key() {
                        obj.remove(k);
                    }
                }
            }
        }
    }
    Ok((UiState::sanitize(Value::Object(obj)), problems))
}

/// The Settings page shown last (`settingsPage`).
pub fn page(cx: &App) -> Page {
    page_of(&crate::ui_state::get(cx))
}

pub fn page_of(state: &UiState) -> Page {
    state
        .rest
        .get(PAGE_KEY)
        .and_then(Value::as_str)
        .and_then(Page::parse)
        .unwrap_or(Page::General)
}

/// Show `page` next (remembered in `ui.json`).
pub fn set_page(cx: &mut App, page: Page) {
    crate::ui_state::update(cx, |s| {
        s.rest
            .insert(PAGE_KEY.into(), Value::String(page.id().into()));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_proto::settings::find;
    use serde_json::json;

    #[test]
    fn registry_values_go_to_the_typed_state() {
        let s = UiState::default();
        let theme = find("appearance.theme").unwrap();
        assert_eq!(value(&s, theme), Some(json!("system")));
        let s = with(&s, theme, &json!("light"));
        assert_eq!(s.theme, crate::theme::ThemePref::Light);
        let s = with(&s, find("general.showElsewhere").unwrap(), &json!(false));
        assert!(s.hide_elsewhere);
        let s = with(&s, find("appearance.terminalFontSize").unwrap(), &json!(16));
        assert_eq!(s.font_size, 16.0);
        assert_eq!(value(&s, find("agents.codexHooks").unwrap()), None);
        assert_eq!(s.spaces.len(), 1, "the rest is kept");
    }

    #[test]
    fn a_bad_value_in_the_file_keeps_the_last_good_one() {
        let current = with(&UiState::default(), find("appearance.theme").unwrap(), &json!("dark"));
        let raw = json!({"v": 1, "theme": "purple", "density": "dense", "fontSize": 99, "wall": ["main"]});
        let (s, problems) = from_file(raw, &current).unwrap();
        assert_eq!(s.theme, crate::theme::ThemePref::Dark, "kept");
        assert_eq!(s.density, crate::theme::Density::Dense, "applied");
        assert_eq!(s.font_size, 13.0, "kept");
        assert_eq!(s.wall, vec!["main".to_string()]);
        assert_eq!(problems.len(), 2);
        assert!(problems[0].contains("\"theme\"") && problems[0].contains("purple"), "{problems:?}");
        assert!(from_file(json!([1]), &current).is_err());
        assert!(from_file(json!({"theme": "dark"}), &current).is_err());
    }

    #[test]
    fn the_page_is_remembered_in_the_file() {
        let mut s = UiState::default();
        assert_eq!(page_of(&s), Page::General);
        s.rest.insert(PAGE_KEY.into(), json!("folderAccess"));
        assert_eq!(page_of(&s), Page::FolderAccess);
        s.rest.insert(PAGE_KEY.into(), json!("nope"));
        assert_eq!(page_of(&s), Page::General);
    }
}
