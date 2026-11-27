//! Extensions in the app (docs/spec/extensions.md): the folders of
//! `<data>/extensions/` ([`pitwall_core::extensions`]), read when Pitwall
//! starts and again from Settings → Extensions ("Look again").
//! One that is on (`ui.json` `extensions.<id>.enabled`, off until the user
//! turns it on) has its adapters run ([`hub`], one per connection:
//! [`connections`]) while it is on; its threads show in its inbox tile
//! (`main_screen::inbox_tile`), and an agent's open asks as a badge on the
//! agent's row ([`asks_for`]).
//!
//! **Development folders** (docs/spec/extensions.md, "Dev mode"): a folder
//! loaded from Settings ("Load from folder…") is recorded in `ui.json`
//! (`extensions.<id>.devPath`) and used where it is, never copied; it
//! takes the place of an installed copy with its id while it is listed
//! ([`load_dev`], [`reload_dev`], [`forget_dev`]).

pub mod actions;
pub mod adapter;
pub mod agent_setup;
pub mod calls;
pub mod catalog;
pub mod connections;
pub mod hub;
pub mod https;
pub mod index;
pub mod safe_update;
pub mod service;
pub mod stale;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Global};
use pitwall_core::extensions::{discover, read_dev, Found, Manifest};

use crate::platform::secrets::SecretStore;

/// The extensions found, the folder they were looked for in, and the
/// folder of those the app ships ([`catalog`]).
#[derive(Clone, Default)]
pub struct Installed {
    pub dir: Option<PathBuf>,
    pub found: Rc<Vec<Found>>,
    pub bundle: Option<PathBuf>,
    /// The development folders `found` was read with, by id.
    pub dev: Rc<BTreeMap<String, PathBuf>>,
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
pub fn start(root: Option<&std::path::Path>, engine: Option<pitwall_core::engine::Shared>, server: Option<Arc<pitwall_daemon::Server>>, cx: &mut App) {
    cx.set_global(Secrets { store: crate::platform::secrets::system(), scope: scope_of(root) });
    if let Some(e) = engine {
        cx.set_global(Engine(e));
    }
    if let Some(s) = server {
        calls::set_api(s, cx);
    }
    connections::migrate(cx);
    follow(cx);
}

/// The engine agents are launched by (their extension variables).
struct Engine(pitwall_core::engine::Shared);

impl Global for Engine {}

/// Tests: the engine agents are launched by.
#[cfg(test)]
pub fn set_engine(e: pitwall_core::engine::Shared, cx: &mut App) {
    cx.set_global(Engine(e));
}

/// The engine agents are launched by, when there is one.
pub(crate) fn engine(cx: &App) -> Option<pitwall_core::engine::Shared> {
    cx.try_global::<Engine>().map(|e| e.0.clone())
}

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
        // A development folder added or taken out (here or in ui.json by
        // hand): read the folders again first.
        let dev = dev_paths(&crate::ui_state::get(cx));
        if cx.try_global::<Installed>().is_some_and(|i| *i.dev != dev) {
            rescan(cx);
        }
        calls::sync(cx);
        h.update(cx, |h, cx| h.reconcile(cx));
        // Services start and stop with their extensions.
        let s = service::services(cx);
        s.update(cx, |s, cx| s.reconcile(cx));
        if let Some(e) = cx.try_global::<Engine>().map(|e| e.0.clone()) {
            e.set_extension_env(env_sources(cx));
            calls::sync_queue(&e, cx);
        }
        // Extensions' skills for agents, for the projects given them.
        crate::tiles::agents::sync_skills(cx);
        // Which setup agents start under now: the ones before are stale.
        stale::sync(cx);
        // A tool's account added, an extension turned on, or start: the
        // setup the user confirmed covers the accounts it doesn't yet.
        actions::auto_setup_if_changed(cx);
    };
    // A new ask: the notification an agent needing the user gets.
    let watched = hub::hub(cx);
    cx.subscribe(&watched, |_, e: &hub::HubEvent, cx| {
        let name_of = |ext: &str, cx: &App| manifest_of(ext, cx).map(|m| m.name).unwrap_or_else(|| ext.to_string());
        match e {
            hub::HubEvent::NewAsk { ext, thread } => {
                let name = name_of(ext, cx);
                let who = thread.from.as_ref().map_or(name.clone(), |f| f.agent.clone());
                crate::platform::notify_attention(who, format!("Asks you — {}", thread.title), cx);
            }
            // An extension's own notification, labelled with its name: the
            // system's while Pitwall isn't in front (never from a bench
            // instance), and a toast in Pitwall, which opens what it names.
            hub::HubEvent::Notify { ext, notify } => {
                let title = format!("{}: {}", name_of(ext, cx), notify.title);
                crate::platform::notify_attention(title.clone(), notify.body.clone(), cx);
                crate::windows::extension_toast(ext, title, notify.body.clone(), notify.open.clone(), cx);
            }
            hub::HubEvent::CommandDone { ext, title, ok, message } => {
                if *ok && message.is_empty() {
                    return;
                }
                let head = if *ok { format!("{}: {title}", name_of(ext, cx)) } else { format!("{}: “{title}” didn't work", name_of(ext, cx)) };
                crate::windows::extension_toast(ext, head, message.clone(), None, cx);
            }
        }
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
        move |_, cx| {
            // ui.json's copy for an older Pitwall follows the carried-over
            // connection.
            connections::mirror(cx);
            c(cx)
        }
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

/// Extension `id`'s manifest, if it is found and readable.
pub fn manifest_of(id: &str, cx: &App) -> Option<Manifest> {
    found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == id).cloned()
}

/// The extensions that are on and whose adapters run now: what their
/// views, commands and badges come from.
pub fn running(cx: &App) -> Vec<Manifest> {
    let Some(hub) = hub::try_hub(cx) else { return vec![] };
    let hub = hub.read(cx);
    enabled(&found(cx), &crate::ui_state::get(cx)).into_iter().filter(|m| hub.get(&m.id).is_some_and(|s| s.run == hub::Run::Running)).collect()
}

/// A badge or note an extension put on a sidebar row.
#[derive(Debug, Clone, PartialEq)]
pub struct RowMark {
    pub ext: String,
    pub name: String,
    pub badge: pitwall_core::extensions::view::RowBadge,
}

/// The badges and notes on agent `a`'s row: each running extension's
/// that names it (by Pitwall's id, or by its `matchAgents`), then its
/// tiles'.
pub fn marks_for_agent(a: &pitwall_proto::AgentView, cx: &App) -> Vec<RowMark> {
    let mut out = adapter_marks_for_agent(a, cx);
    out.extend(crate::tiles::group::marks_for_agent(a, cx));
    out
}

fn adapter_marks_for_agent(a: &pitwall_proto::AgentView, cx: &App) -> Vec<RowMark> {
    use pitwall_core::extensions::view::Target;
    let Some(hub) = hub::try_hub(cx) else { return vec![] };
    let list = crate::accounts::list(cx);
    let mut out = Vec::new();
    for m in running(cx) {
        let Some(st) = hub.read(cx).get(&m.id) else { continue };
        for (conn, b) in &st.badges {
            let mine = match &b.target {
                Target::Agent(id) => *id == a.id,
                Target::From(f) => is_agents(&m, a, &hub::thread_id(conn, "badge"), f, list),
                Target::Project(_) => false,
            };
            if mine {
                out.push(RowMark { ext: m.id.clone(), name: m.name.clone(), badge: b.clone() });
            }
        }
    }
    out
}

/// The badges and notes on project `path`'s row, adapters' then tiles'.
pub fn marks_for_project(path: &str, cx: &App) -> Vec<RowMark> {
    use pitwall_core::extensions::view::Target;
    let mut out = Vec::new();
    if let Some(hub) = hub::try_hub(cx) {
        for m in running(cx) {
            let Some(st) = hub.read(cx).get(&m.id) else { continue };
            out.extend(st.badges.iter().filter(|(_, b)| matches!(&b.target, Target::Project(p) if p == path)).map(|(_, b)| RowMark { ext: m.id.clone(), name: m.name.clone(), badge: b.clone() }));
        }
    }
    out.extend(crate::tiles::group::marks_for_project(path, cx));
    out
}

/// The extensions that are on and have an adapter: those with an inbox.
pub fn inboxes(cx: &App) -> Vec<Manifest> {
    enabled(&found(cx), &crate::ui_state::get(cx)).into_iter().filter(Manifest::has_inbox).collect()
}

/// Pitwall quits: every adapter is killed.
pub fn quit(cx: &mut App) {
    if let Some(h) = hub::try_hub(cx) {
        h.update(cx, |h, _| h.quit());
    }
    if let Some(s) = service::try_services(cx) {
        s.update(cx, |s, _| s.quit());
    }
}

/// The icon an extension gets when it names none, or one this Pitwall
/// doesn't have.
pub const DEFAULT_ICON: &str = "extension";

/// Look in `root`'s extensions folder (`None`: no data folder, none), and
/// the development folders in `ui.json`.
pub fn init(root: Option<PathBuf>, cx: &mut App) {
    let dir = root.map(|r| pitwall_core::paths::Paths::new(r).extensions_dir());
    let dev = dev_paths(&crate::ui_state::get(cx));
    let found = find_all(dir.as_deref(), &dev);
    cx.set_global(Installed { dir, found: Rc::new(found), bundle: catalog::locate(), dev: Rc::new(dev) });
}

/// Look in the extensions folder and the development folders again
/// (Settings → "Look again", Reload).
pub fn rescan(cx: &mut App) {
    let mut i = cx.try_global::<Installed>().cloned().unwrap_or_default();
    let dev = dev_paths(&crate::ui_state::get(cx));
    i.found = Rc::new(find_all(i.dir.as_deref(), &dev));
    i.dev = Rc::new(dev);
    cx.set_global(i);
}

/// The development folders `ui` lists, by id.
pub fn dev_paths(ui: &crate::ui_state::UiState) -> BTreeMap<String, PathBuf> {
    ui.extensions.iter().filter_map(|(id, e)| Some((id.clone(), PathBuf::from(e.dev_path.as_ref()?)))).collect()
}

/// The extensions in `dir`, each development folder of `dev` in place of
/// an installed one with its id, by folder (id) order.
pub fn find_all(dir: Option<&std::path::Path>, dev: &BTreeMap<String, PathBuf>) -> Vec<Found> {
    let mut found: Vec<Found> = dir.map(discover).unwrap_or_default().into_iter().filter(|f| !dev.contains_key(f.key())).collect();
    found.extend(dev.iter().map(|(id, path)| read_dev(path, id)));
    found.sort_by(|a, b| a.folder.cmp(&b.folder));
    found
}

/// Extension `id`'s development folder, when it is loaded from one.
pub fn dev_folder(id: &str, cx: &App) -> Option<PathBuf> {
    cx.try_global::<Installed>().and_then(|i| i.dev.get(id).cloned())
}

/// The folder extension `id` runs in: its development folder, else its
/// folder in the extensions folder.
pub fn folder_of(id: &str, cx: &App) -> Option<PathBuf> {
    dev_folder(id, cx).or_else(|| dir(cx).map(|d| d.join(id)))
}

/// Load the extension in `folder` in place, as a development folder: its
/// id, or why not. It stays off (an installed copy with its id that was on
/// is turned off too: nothing runs until the user turns it on).
pub fn load_dev(folder: &std::path::Path, cx: &mut App) -> Result<String, String> {
    let folder = folder.canonicalize().map_err(|_| format!("{} isn't there", folder.display()))?;
    if !folder.is_dir() {
        return Err(format!("{} isn't a folder", folder.display()));
    }
    let m = pitwall_core::extensions::read_in_place(&folder).map_err(|why| format!("It can't be used: {why}."))?;
    if let Some(other) = dev_folder(&m.id, cx).filter(|p| *p != folder) {
        return Err(format!("“{}” is already loaded from {}: remove that one from the list first.", m.id, pitwall_core::paths::tildify(&other.to_string_lossy())));
    }
    let path = folder.to_string_lossy().into_owned();
    crate::ui_state::update(cx, |ui| {
        *ui = std::mem::take(ui).with_extension(&m.id, |e| {
            e.dev_path = Some(path);
            e.enabled = false;
        })
    });
    rescan(cx);
    Ok(m.id)
}

/// `pitwall extension dev` (`extension.devReload`): the extension in
/// `folder` loaded as a development folder and turned on with the
/// permissions turning on grants by default, or (loaded and on already)
/// reloaded in place. Its id, and whether it was loaded now.
pub fn load_dev_or_reload(folder: &std::path::Path, cx: &mut App) -> Result<(String, bool), String> {
    let folder = folder.canonicalize().map_err(|_| format!("{} isn't there", folder.display()))?;
    let m = pitwall_core::extensions::read_in_place(&folder).map_err(|why| format!("It can't be used: {why}."))?;
    let loaded = dev_folder(&m.id, cx).is_some_and(|p| p == folder);
    if loaded && crate::ui_state::get(cx).extension_on(&m.id) {
        reload_dev(&m.id, cx);
        return Ok((m.id, false));
    }
    let id = if loaded { m.id.clone() } else { load_dev(&folder, cx)? };
    let m = manifest_of(&id, cx).ok_or_else(|| "It can't be read after loading it.".to_string())?;
    let kept = crate::ui_state::get(cx).extension(&id).grants;
    let picks = crate::settings::extensions::initial_picks(&m, &kept);
    crate::settings::extensions::turn_on_with(&m, &picks, cx);
    Ok((id, true))
}

/// Read development extension `id`'s manifest again and, when it is on,
/// start its adapters again.
pub fn reload_dev(id: &str, cx: &mut App) {
    rescan(cx);
    let h = hub::hub(cx);
    h.update(cx, |h, cx| h.restart(id, cx));
    // Its tiles: the running component replaced in place, with its state.
    if let Some(t) = crate::tiles::try_tiles(cx) {
        t.update(cx, |t, cx| t.reload(id, cx));
    }
}

/// Take development extension `id` off the list: turned off and
/// forgotten as a development folder. Its folder is never touched; an
/// installed copy with its id shows again.
pub fn forget_dev(id: &str, cx: &mut App) {
    crate::ui_state::update(cx, |ui| {
        *ui = std::mem::take(ui).with_extension(id, |e| {
            e.dev_path = None;
            e.enabled = false;
        })
    });
    rescan(cx);
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

/// Where web view `view` of extension `m` opens: its manifest's address
/// (a `{setting:KEY}` in it filled from the extension's settings), else
/// the one its adapter last gave; checked again here (https, or http on
/// this computer). Why not, in plain words.
pub fn web_view_address(m: &Manifest, view: &str, cx: &App) -> Result<String, String> {
    let w = m.contributes.web_view(view).ok_or_else(|| format!("{} has no web view “{view}”.", m.name))?;
    let url = match &w.url {
        Some(u) => match u.strip_prefix("{setting:").and_then(|r| r.split_once('}')) {
            Some((key, rest)) => {
                let base = connections::chosen(&m.id, None, cx).and_then(|c| c.values.get(key).cloned()).filter(|v| !v.trim().is_empty()).ok_or_else(|| format!("Set {}'s “{key}” first (Settings → Extensions).", m.name))?;
                format!("{}{rest}", base.trim_end_matches('/'))
            }
            None => u.clone(),
        },
        None => match hub::try_hub(cx).and_then(|h| h.read(cx).get(&m.id).and_then(|st| st.web_urls.get(view).cloned())) {
            Some(r) => r?,
            None => return Err(format!("{} hasn't said where “{}” is yet (is it running?).", m.name, w.title)),
        },
    };
    pitwall_core::extensions::view::web_view_url(&url)
}

/// The icon to draw for `m`.
pub fn icon_of(m: &Manifest) -> &str {
    m.icon.as_deref().filter(|i| crate::kit::has_icon(i)).unwrap_or(DEFAULT_ICON)
}

/// Tests: these extensions found (no folder read).
#[cfg(test)]
pub fn set_found(found: Vec<Found>, cx: &mut App) {
    let bundle = cx.try_global::<Installed>().and_then(|i| i.bundle.clone());
    cx.set_global(Installed { dir: None, found: Rc::new(found), bundle, dev: Rc::default() });
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

    /// Every extension in the repo passes `pitwall extension check` and
    /// `test` (`pitwall_core::extensions::check`): its manifest and files,
    /// and what its `test/pitwall.json` expects (its actions' exact command
    /// lines, the variables Settings shows, its adapter's run and its
    /// agreement with a fake server; a run is skipped without Node). Keeps
    /// the checks for a real extension out of the crate's source.
    #[test]
    fn each_shipped_extension_passes_its_checks() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions");
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path()).filter(|p| p.join(pitwall_core::extensions::MANIFEST).is_file()).collect();
        dirs.sort();
        assert!(!dirs.is_empty(), "no extensions in {}", root.display());
        let path = pitwall_core::shell::spawn_path();
        for dir in dirs {
            let r = pitwall_core::extensions::check::check(&dir, path.as_deref());
            assert!(r.passed(), "{}", r.text());
            let r = pitwall_core::extensions::check::test(&dir, path.as_deref());
            assert!(r.passed(), "{}", r.text());
        }
    }

    /// A development folder takes the place of an installed copy with its
    /// id, is read where it is, and goes back to the installed one when it
    /// is taken off the list; a folder whose id changed says so.
    #[test]
    fn a_development_folder_is_used_in_place() {
        let t = pitwall_core::testing::TempDir::new("ext-dev");
        let put = |dir: &std::path::Path, id: &str, name: &str| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(pitwall_core::extensions::MANIFEST), format!(r#"{{"id": "{id}", "name": "{name}"}}"#)).unwrap();
        };
        let installed = t.path().join("data/extensions");
        put(&installed.join("inbox"), "inbox", "Installed");
        put(&installed.join("other"), "other", "Other");
        let work = t.path().join("code/my-inbox");
        put(&work, "inbox", "Mine");
        let names = |dev: &BTreeMap<String, PathBuf>| find_all(Some(&installed), dev).iter().map(|f| f.manifest.as_ref().map(|m| m.name.clone()).unwrap_or_else(|e| e.clone())).collect::<Vec<_>>();
        assert_eq!(names(&BTreeMap::new()), ["Installed", "Other"]);
        let dev = BTreeMap::from([("inbox".to_string(), work.clone())]);
        assert_eq!(names(&dev), ["Mine", "Other"]);
        put(&work, "renamed", "Mine");
        assert!(names(&dev)[0].contains("its id is now “renamed”"), "{:?}", names(&dev));
        let ui = UiState::default().with_extension("inbox", |e| e.dev_path = Some(work.to_string_lossy().into_owned()));
        assert_eq!(dev_paths(&ui), dev);
        assert_eq!(serde_json::to_value(&ui).unwrap()["extensions"]["inbox"]["devPath"], serde_json::json!(work.to_string_lossy()));
        let back = UiState::sanitize(serde_json::to_value(&ui).unwrap());
        assert_eq!(dev_paths(&back), dev, "kept in ui.json");
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
