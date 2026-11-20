//! Extensions in the app (docs/spec/extensions.md, draft): the folders of
//! `<data>/extensions/` ([`pitwall_core::extensions`]), read when Pitwall
//! starts and again from Settings → General → Extensions ("Look again").
//! Nothing in them is run: an extension is its manifest. One that is on
//! (`ui.json` `extensions.<id>.enabled`, off until the user turns it on)
//! gets a section of the sidebar of its own, shaped like a project's.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::{App, Global};
use pitwall_core::extensions::{discover, Found, Manifest};

/// The extensions found, and the folder they were looked for in.
#[derive(Clone, Default)]
pub struct Installed {
    pub dir: Option<PathBuf>,
    pub found: Rc<Vec<Found>>,
}

impl Global for Installed {}

/// The icon an extension gets when it names none, or one this Pitwall
/// doesn't have.
pub const DEFAULT_ICON: &str = "extension";

/// The sidebar group key of extension `id` (folded or not: `ui.json`
/// `collapsed`). Never a project's: those are folders or `m:`/`a:` keys.
pub fn group_key(id: &str) -> String {
    format!("ext:{id}")
}

/// Look in `root`'s extensions folder (`None`: no data folder, none).
pub fn init(root: Option<PathBuf>, cx: &mut App) {
    let dir = root.map(|r| pitwall_core::paths::Paths::new(r).extensions_dir());
    let found = dir.as_deref().map(discover).unwrap_or_default();
    cx.set_global(Installed { dir, found: Rc::new(found) });
}

/// Look in the extensions folder again (Settings → "Look again").
pub fn rescan(cx: &mut App) {
    let dir = cx.try_global::<Installed>().and_then(|i| i.dir.clone());
    let found = dir.as_deref().map(discover).unwrap_or_default();
    cx.set_global(Installed { dir, found: Rc::new(found) });
}

/// Every extension folder found, usable or not.
pub fn found(cx: &App) -> Rc<Vec<Found>> {
    cx.try_global::<Installed>().map(|i| i.found.clone()).unwrap_or_default()
}

/// The folder extensions are looked for in.
pub fn dir(cx: &App) -> Option<PathBuf> {
    cx.try_global::<Installed>().and_then(|i| i.dir.clone())
}

/// The usable extensions that are on in `ui`, in folder order.
pub fn enabled(found: &[Found], ui: &crate::ui_state::UiState) -> Vec<Manifest> {
    found.iter().filter_map(|f| f.manifest.as_ref().ok()).filter(|m| ui.extension_on(&m.id)).cloned().collect()
}

/// The icon to draw for `m`.
pub fn icon_of(m: &Manifest) -> &str {
    m.icon.as_deref().filter(|i| crate::kit::has_icon(i)).unwrap_or(DEFAULT_ICON)
}

/// Tests: these extensions found (no folder read).
#[cfg(test)]
pub fn set_found(found: Vec<Found>, cx: &mut App) {
    cx.set_global(Installed { dir: None, found: Rc::new(found) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_state::UiState;

    fn ext(id: &str) -> Found {
        Found { folder: id.into(), manifest: Ok(Manifest { id: id.into(), name: id.to_uppercase(), icon: None, description: None }) }
    }

    /// Off until turned on; a broken one is never on; the switch is kept
    /// in `ui.json` and an old file without it loads.
    #[test]
    fn extensions_are_off_until_turned_on() {
        let found = vec![ext("alpha"), Found { folder: "bad".into(), manifest: Err("no".into()) }, ext("beta")];
        let ui = UiState::default();
        assert!(enabled(&found, &ui).is_empty());
        let ui = ui.set_extension("beta", true).set_extension("bad", true);
        let on: Vec<String> = enabled(&found, &ui).into_iter().map(|m| m.id).collect();
        assert_eq!(on, ["beta"]);
        let json = serde_json::to_value(&ui).unwrap();
        assert_eq!(json["extensions"]["beta"], serde_json::json!({"enabled": true}));
        let back = UiState::sanitize(json);
        assert!(back.extension_on("beta") && !back.extension_on("alpha"));
        // Off again: the entry goes, and an empty map isn't written.
        let ui = back.set_extension("beta", false).set_extension("bad", false);
        assert!(serde_json::to_value(&ui).unwrap().get("extensions").is_none());
        // Keys of later versions stay.
        let raw = serde_json::json!({"v": 1, "extensions": {"beta": {"enabled": true, "later": 7}}});
        let ui = UiState::sanitize(raw).set_extension("beta", false);
        assert_eq!(serde_json::to_value(&ui).unwrap()["extensions"]["beta"], serde_json::json!({"enabled": false, "later": 7}));
        // A value of the wrong type: off.
        assert!(!UiState::sanitize(serde_json::json!({"v": 1, "extensions": {"beta": {"enabled": "yes"}}})).extension_on("beta"));
    }

    #[test]
    fn an_unknown_icon_gets_the_extension_icon() {
        let mut m = Manifest { id: "a".into(), name: "A".into(), icon: Some("inbox".into()), description: None };
        assert_eq!(icon_of(&m), "inbox");
        m.icon = Some("no-such-icon".into());
        assert_eq!(icon_of(&m), DEFAULT_ICON);
        m.icon = None;
        assert_eq!(icon_of(&m), DEFAULT_ICON);
        assert!(crate::kit::has_icon(DEFAULT_ICON));
        assert!(!group_key("a").starts_with('/'));
    }
}
