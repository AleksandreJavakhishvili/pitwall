//! Extensions in the app (docs/spec/extensions.md): the folders of
//! `<data>/extensions/` ([`pitwall_core::extensions`]), read when Pitwall
//! starts and again from Settings → Extensions ("Look again").
//! One that is on (`ui.json` `extensions.<id>.enabled`, off until the user
//! turns it on) has its adapters run ([`hub`], one per connection:
//! [`connections`]) while it is on; its threads show in its inbox tile
//! (`main_screen::inbox_tile`), and an agent's open asks as a badge on the
//! agent's row ([`asks_for`]).

pub mod actions;
pub mod adapter;
pub mod catalog;
pub mod connections;
pub mod hub;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Global};
use pitwall_core::extensions::{discover, Found, Manifest};

use crate::platform::secrets::SecretStore;

/// The extensions found, the folder they were looked for in, and the
/// folder of those the app ships ([`catalog`]).
#[derive(Clone, Default)]
pub struct Installed {
    pub dir: Option<PathBuf>,
    pub found: Rc<Vec<Found>>,
    pub bundle: Option<PathBuf>,
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

/// The account name a secret `key` is kept under for secrets name `id` (a
/// connection's id, or the extension's id for one from before
/// connections: `Account::secrets_name`): `<id>.<key>`, then the data
/// folder's hash when it isn't the default one.
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

/// Keep `secret` as `key` of the connection Settings shows for extension
/// `id` (made when it has none), off the UI thread; record that it is set,
/// and start its adapter again. `done` gets the error, if any.
pub fn set_secret(id: &str, key: &str, secret: String, cx: &mut App, done: impl FnOnce(Option<String>, &mut App) + 'static) {
    connections::set_settings_secret(id, key, Some(secret), cx, done)
}

/// Forget `key` of the connection Settings shows for extension `id`.
pub fn remove_secret(id: &str, key: &str, cx: &mut App, done: impl FnOnce(Option<String>, &mut App) + 'static) {
    connections::set_settings_secret(id, key, None, cx, done)
}

/// Run adapters from now on: the system's secure store for data folder
/// `root`, and a hub that follows the switches and settings.
pub fn start(root: Option<&std::path::Path>, engine: Option<pitwall_core::engine::Shared>, cx: &mut App) {
    cx.set_global(Secrets { store: crate::platform::secrets::system(), scope: scope_of(root) });
    if let Some(e) = engine {
        cx.set_global(Engine(e));
    }
    connections::migrate(cx);
    follow(cx);
}

/// The engine agents are launched by (their extension variables).
struct Engine(pitwall_core::engine::Shared);

impl Global for Engine {}

/// The extensions that are on and set variables on agents: launches fill
/// them from each agent's connection (`Engine::extension_env`).
pub fn env_sources(cx: &App) -> Vec<Manifest> {
    let ui = crate::ui_state::get(cx);
    enabled(&found(cx), &ui).into_iter().filter(|m| !m.agent_env.is_empty()).collect()
}

/// The hub follows `ui.json`, the extensions found and their connections
/// (the accounts); so do the variables launched agents get.
pub fn follow(cx: &mut App) {
    let h = hub::hub(cx);
    let ui = crate::ui_state::store(cx);
    let accounts = crate::accounts::model(cx);
    let changed = move |cx: &mut App| {
        h.update(cx, |h, cx| h.reconcile(cx));
        if let Some(e) = cx.try_global::<Engine>().map(|e| e.0.clone()) {
            e.set_extension_env(env_sources(cx));
        }
    };
    // A new ask: the notification an agent needing the user gets.
    let watched = hub::hub(cx);
    cx.subscribe(&watched, |_, e: &hub::HubEvent, cx| {
        let hub::HubEvent::NewAsk { ext, thread } = e;
        let name = found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == *ext).map(|m| m.name.clone()).unwrap_or_default();
        let who = thread.from.as_ref().map_or(name.clone(), |f| f.agent.clone());
        crate::platform::notify_attention(who, format!("Asks you — {}", thread.title), cx);
    })
    .detach();
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
    cx.observe(&accounts, {
        let c = c.clone();
        move |_, cx| c(cx)
    })
    .detach();
    c(cx);
}

/// Whether thread `thread` (its id as views know it), from `from`, is
/// agent `a`'s, by `m`'s `matchAgents`: the names must match, and a
/// thread of a connection only matches agents whose project's connection
/// for the extension it is now (docs/spec/profiles.md); `list`: the
/// accounts.
pub fn is_agents(m: &Manifest, a: &pitwall_proto::AgentView, thread: &str, from: &pitwall_core::extensions::protocol::From, list: Option<&pitwall_core::accounts::Accounts>) -> bool {
    let Some(rule) = &m.match_agents else { return false };
    let (conn, _) = hub::split_thread(m, thread);
    if !conn.is_empty() {
        let mine = list.and_then(|l| l.connection_for(&m.id, &a.cwd, &a.project)).map(|c| c.id.as_str());
        if mine != Some(conn) {
            return false;
        }
    }
    let f = pitwall_core::extensions::Facts { project: &a.project, agent: &a.name, agent_id: &a.id, settings: None };
    let expand = |t: &str| pitwall_core::extensions::expand(t, &f);
    expand(&rule.agent).as_deref() == Some(from.agent.as_str()) && expand(&rule.session).as_deref() == Some(from.session.as_str())
}

/// The Pitwall agent thread `thread`, from `from`, is from.
pub fn agent_of<'a>(m: &Manifest, agents: &'a [pitwall_proto::AgentView], thread: &str, from: &pitwall_core::extensions::protocol::From, cx: &App) -> Option<&'a pitwall_proto::AgentView> {
    let list = crate::accounts::list(cx);
    agents.iter().find(|a| is_agents(m, a, thread, from, list))
}

/// An agent's open asks in an extension: how many, whether one blocks it,
/// and the thread with the oldest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAsks {
    pub ext: String,
    pub name: String,
    pub thread: String,
    pub asks: u32,
    pub blocking: bool,
}

/// Agent `a`'s open asks in the first extension that is on and has some.
pub fn asks_for(a: &pitwall_proto::AgentView, cx: &App) -> Option<AgentAsks> {
    let hub = hub::try_hub(cx)?;
    let hub = hub.read(cx);
    let ui = crate::ui_state::get(cx);
    let list = crate::accounts::list(cx);
    enabled(&found(cx), &ui).into_iter().find_map(|m| {
        let st = hub.get(&m.id)?;
        if st.run != hub::Run::Running {
            return None;
        }
        let mine: Vec<_> = st.threads.iter().filter(|t| t.open_asks > 0 && t.from.as_ref().is_some_and(|f| is_agents(&m, a, &t.id, f, list))).collect();
        let oldest = mine.iter().min_by(|x, y| x.last_activity.cmp(&y.last_activity))?;
        Some(AgentAsks { ext: m.id.clone(), name: m.name.clone(), thread: oldest.id.clone(), asks: mine.iter().map(|t| t.open_asks).sum(), blocking: mine.iter().any(|t| t.blocking) })
    })
}

/// The extensions that are on and have an adapter: those with an inbox.
pub fn inboxes(cx: &App) -> Vec<Manifest> {
    enabled(&found(cx), &crate::ui_state::get(cx)).into_iter().filter(|m| m.adapter.is_some()).collect()
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
    cx.set_global(Installed { dir, found: Rc::new(found), bundle: catalog::locate() });
}

/// Look in the extensions folder again (Settings → "Look again").
pub fn rescan(cx: &mut App) {
    let mut i = cx.try_global::<Installed>().cloned().unwrap_or_default();
    i.found = Rc::new(i.dir.as_deref().map(discover).unwrap_or_default());
    cx.set_global(i);
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
    let bundle = cx.try_global::<Installed>().and_then(|i| i.bundle.clone());
    cx.set_global(Installed { dir: None, found: Rc::new(found), bundle });
}

/// Tests: every extension in the repo's `extensions/` folder, its
/// manifest, and what its `test/pitwall.json` says Pitwall's tests expect
/// of it (each test reads its part; an extension without one is only
/// parsed and planned). Keeps the checks for a real extension out of the
/// crate's source.
#[cfg(test)]
pub fn shipped_all() -> Vec<(PathBuf, Manifest, serde_json::Value)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("extension.json").is_file()).collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no extensions in {}", root.display());
    dirs.into_iter()
        .map(|dir| {
            let text = std::fs::read_to_string(dir.join("extension.json")).unwrap();
            let m = pitwall_core::extensions::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
            let fixture = std::fs::read_to_string(dir.join("test/pitwall.json"))
                .map(|t| serde_json::from_str(&t).unwrap_or_else(|e| panic!("{}/test/pitwall.json: {e}", dir.display())))
                .unwrap_or(serde_json::Value::Null);
            (dir, m, fixture)
        })
        .collect()
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
