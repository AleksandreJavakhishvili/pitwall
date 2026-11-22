//! Extensions in the app (docs/spec/extensions.md): the folders of
//! `<data>/extensions/` ([`pitwall_core::extensions`]), read when Pitwall
//! starts and again from Settings → General → Extensions ("Look again").
//! One that is on (`ui.json` `extensions.<id>.enabled`, off until the user
//! turns it on) gets a section of the sidebar of its own, shaped like a
//! project's, and its adapter is run ([`hub`]) while it is on.

pub mod actions;
pub mod adapter;
pub mod hub;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Global};
use pitwall_core::extensions::{discover, Found, Manifest};

use crate::platform::secrets::SecretStore;

/// The extensions found, and the folder they were looked for in.
#[derive(Clone, Default)]
pub struct Installed {
    pub dir: Option<PathBuf>,
    pub found: Rc<Vec<Found>>,
}

impl Global for Installed {}

/// Where extensions' secrets are kept, and the data folder they belong to
/// (part of each item's account name, so a test instance's secrets never
/// mix with the real one's).
#[derive(Clone)]
struct Secrets {
    store: Arc<dyn SecretStore>,
    scope: String,
}

impl Global for Secrets {}

/// The secure store extensions' secrets are in (in memory when none was
/// set: tests).
pub fn secret_store(cx: &mut App) -> Arc<dyn SecretStore> {
    secrets(cx).store
}

fn secrets(cx: &mut App) -> Secrets {
    if let Some(s) = cx.try_global::<Secrets>() {
        return s.clone();
    }
    let s = Secrets { store: Arc::new(crate::platform::secrets::Memory::default()), scope: String::new() };
    cx.set_global(s.clone());
    s
}

/// The account name extension `id`'s secret `key` is kept under:
/// `<id>.<key>`, then the data folder's hash when it isn't the default
/// one.
pub fn account(id: &str, key: &str, cx: &mut App) -> String {
    let scope = secrets(cx).scope;
    if scope.is_empty() {
        format!("{id}.{key}")
    } else {
        format!("{id}.{key}@{scope}")
    }
}

/// A short, stable name for data folder `root` ("" for `~/.pitwall`).
fn scope_of(root: Option<&std::path::Path>) -> String {
    let Some(root) = root else { return "none".into() };
    if pitwall_core::paths::Paths::default_root() == root {
        return String::new();
    }
    // FNV-1a: stable across builds (DefaultHasher isn't promised to be).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in root.to_string_lossy().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Keep `secret` as extension `id`'s `key` (off the UI thread), record that
/// it is set, and start the adapter again. `done` gets the error, if any.
pub fn set_secret(id: &str, key: &str, secret: String, cx: &mut App, done: impl FnOnce(Option<String>, &mut App) + 'static) {
    change_secret(id, key, Some(secret), cx, done)
}

/// Forget extension `id`'s secret `key`.
pub fn remove_secret(id: &str, key: &str, cx: &mut App, done: impl FnOnce(Option<String>, &mut App) + 'static) {
    change_secret(id, key, None, cx, done)
}

fn change_secret(id: &str, key: &str, secret: Option<String>, cx: &mut App, done: impl FnOnce(Option<String>, &mut App) + 'static) {
    let store = secret_store(cx);
    let acct = account(id, key, cx);
    let set = secret.is_some();
    let job = cx.background_executor().spawn(async move {
        match secret {
            Some(s) => store.set(&acct, &s),
            None => store.delete(&acct),
        }
    });
    let (id, key) = (id.to_string(), key.to_string());
    cx.spawn(async move |cx| {
        let res = job.await;
        cx.update(|cx| {
            let err = res.err();
            if err.is_none() {
                crate::ui_state::update(cx, |ui| {
                    *ui = std::mem::take(ui).with_extension(&id, |e| {
                        e.secrets_set.retain(|k| *k != key);
                        if set {
                            e.secrets_set.push(key.clone());
                            e.secrets_set.sort();
                        }
                    });
                });
                let h = hub::hub(cx);
                h.update(cx, |h, cx| h.secrets_changed(&id, cx));
            }
            done(err, cx);
        });
    })
    .detach();
}

/// Run adapters from now on: the system's secure store for data folder
/// `root`, and a hub that follows the switches and settings.
pub fn start(root: Option<&std::path::Path>, engine: Option<pitwall_core::engine::Shared>, cx: &mut App) {
    cx.set_global(Secrets { store: crate::platform::secrets::system(), scope: scope_of(root) });
    if let Some(e) = engine {
        cx.set_global(Engine(e));
    }
    follow(cx);
}

/// The engine agents are launched by (their extension variables).
struct Engine(pitwall_core::engine::Shared);

impl Global for Engine {}

/// The extensions that are on, with their plain settings: what launches
/// put in agents' environment.
pub fn env_sources(cx: &App) -> Vec<(Manifest, std::collections::BTreeMap<String, String>)> {
    let ui = crate::ui_state::get(cx);
    enabled(&found(cx), &ui).into_iter().filter(|m| !m.agent_env.is_empty()).map(|m| {
        let values = ui.extension(&m.id).values;
        (m, values)
    }).collect()
}

/// The hub follows `ui.json` and the extensions found; so do the
/// variables launched agents get.
pub fn follow(cx: &mut App) {
    let h = hub::hub(cx);
    let ui = crate::ui_state::store(cx);
    let changed = move |cx: &mut App| {
        h.update(cx, |h, cx| h.reconcile(cx));
        if let Some(e) = cx.try_global::<Engine>().map(|e| e.0.clone()) {
            e.set_extension_env(env_sources(cx));
        }
    };
    let c = std::rc::Rc::new(changed);
    cx.observe(&ui, {
        let c = c.clone();
        move |_, cx| c(cx)
    })
    .detach();
    cx.observe_global::<Installed>({
        let c = c.clone();
        move |cx| c(cx)
    })
    .detach();
    c(cx);
}

/// Pitwall quits: every adapter is killed.
pub fn quit(cx: &mut App) {
    if let Some(h) = hub::try_hub(cx) {
        h.update(cx, |h, _| h.quit());
    }
}

/// The icon an extension gets when it names none, or one this Pitwall
/// doesn't have.
pub const DEFAULT_ICON: &str = "extension";

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
        Found { folder: id.into(), manifest: Ok(Manifest::named(id, &id.to_uppercase())) }
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
        let mut m = Manifest { icon: Some("inbox".into()), ..Manifest::named("a", "A") };
        assert_eq!(icon_of(&m), "inbox");
        m.icon = Some("no-such-icon".into());
        assert_eq!(icon_of(&m), DEFAULT_ICON);
        m.icon = None;
        assert_eq!(icon_of(&m), DEFAULT_ICON);
        assert!(crate::kit::has_icon(DEFAULT_ICON));
    }
}
