//! Agents that started before an extension changed
//! (docs/extensions/updates.md, "Running agents"): each agent keeps the
//! setup of every extension it started with (`AgentRecord::ext_setup`).
//! The engine is told what each extension that is on would give an agent
//! now ([`sync`]); a running agent that started under another carries the
//! extension in `AgentView::stale_for`, shown as a chip on its row and its
//! pane, and listed on the extension's card with Restart.
//!
//! What Pitwall does with them is a setting (`extensions.restartAfterUpdate`,
//! and per extension on its card, [`mode`]): `ask` (nothing by itself),
//! `whenIdle` or `never`. With `whenIdle` an agent is restarted only once
//! its own status has been idle or done (waiting for the user) for
//! [`DWELL`], never while it works or waits on an approval, one at a time,
//! and only one that can continue its conversation; a notice says so and
//! the extension's activity log keeps it.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{App, AppContext, Entity, Global};
use pitwall_core::extensions::update::{self as core, Restart, Setup};
use pitwall_proto::{AgentView, Status};

/// How long an agent waits for the user before `whenIdle` restarts it.
pub const DWELL: Duration = Duration::from_secs(30);
/// How often `whenIdle` looks.
pub const TICK: Duration = Duration::from_secs(2);
/// Where the global setting is kept in `ui.json`.
pub const SETTING: &str = "extensions.restartAfterUpdate";

/// The extensions that are on and affect agents, with the steps their
/// setup actions last ran with.
pub fn sources(cx: &App) -> Vec<Setup> {
    let ui = crate::ui_state::get(cx);
    super::enabled(&super::found(cx), &ui)
        .into_iter()
        .filter(|m| !m.agent_env.is_empty() || m.actions.iter().any(|a| a.setup))
        .map(|m| Setup { steps: ui.extension(&m.id).action_steps, manifest: m })
        .collect()
}

/// What the engine was last told, and the accounts then.
#[derive(Default)]
struct Synced(Option<(Vec<Setup>, Option<pitwall_core::accounts::Accounts>)>);

impl Global for Synced {}

/// Tell the engine the setup agents start under now (when it or the
/// accounts changed); with `whenIdle` on somewhere, start watching.
pub fn sync(cx: &mut App) {
    let list = sources(cx);
    let accounts = crate::accounts::list(cx).cloned();
    let now = Some((list.clone(), accounts));
    if cx.default_global::<Synced>().0 != now {
        cx.default_global::<Synced>().0 = now;
        if let Some(e) = super::engine(cx) {
            e.set_extension_setup(list);
        }
    }
    if any_when_idle(cx) {
        watch(cx);
    }
}

/// The global setting.
pub fn global_mode(cx: &App) -> Restart {
    let ui = crate::ui_state::get(cx);
    pitwall_proto::settings::SETTINGS
        .iter()
        .find(|s| s.key == SETTING)
        .and_then(|def| crate::settings::data::value(&ui, def))
        .and_then(|v| v.as_str().and_then(Restart::parse))
        .unwrap_or_default()
}

/// Set the global setting.
pub fn set_global_mode(r: Restart, cx: &mut App) {
    let Some(def) = pitwall_proto::settings::SETTINGS.iter().find(|s| s.key == SETTING) else { return };
    crate::ui_state::update(cx, |u| *u = crate::settings::data::with(u, def, &serde_json::Value::String(r.id().into())));
}

/// What Pitwall does with extension `ext`'s agents after an update: its
/// own choice, else the global one.
pub fn mode(ext: &str, cx: &App) -> Restart {
    crate::ui_state::get(cx).extension(ext).restart_after_update.as_deref().and_then(Restart::parse).unwrap_or_else(|| global_mode(cx))
}

/// Extension `ext`'s own choice (`None`: the global one).
pub fn set_mode(ext: &str, r: Option<Restart>, cx: &mut App) {
    crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension(ext, |e| e.restart_after_update = r.map(|r| r.id().to_string())));
}

fn any_when_idle(cx: &App) -> bool {
    let ui = crate::ui_state::get(cx);
    super::enabled(&super::found(cx), &ui).iter().any(|m| mode(&m.id, cx) == Restart::WhenIdle)
}

/// This Pitwall's agents.
fn agents(cx: &App) -> Vec<AgentView> {
    cx.try_global::<crate::agents::StoreHandle>().map(|h| h.0.read(cx).local().cloned().collect()).unwrap_or_default()
}

/// The running agents that use extension `ext`'s previous setup.
pub fn stale_agents(ext: &str, cx: &App) -> Vec<AgentView> {
    agents(cx).into_iter().filter(|a| a.stale_for.iter().any(|e| e == ext)).collect()
}

/// Waiting for the user, by its own status: idle, or done with its turn.
pub fn waiting(a: &AgentView) -> bool {
    matches!(a.status, Status::Idle | Status::Done)
}

/// It can be restarted continuing its conversation.
pub fn resumable(a: &AgentView) -> bool {
    a.running && a.caps.restart && a.caps.resume
}

/// What an agent's chip says: its text, its tooltip, and whether it is a
/// warning (an extension whose agents stop working until restarted).
pub fn chip(a: &AgentView, cx: &App) -> Option<(String, String, bool)> {
    if a.stale_for.is_empty() {
        return None;
    }
    let mut names = Vec::new();
    let mut warn = false;
    for ext in &a.stale_for {
        match super::manifest_of(ext, cx) {
            Some(m) => {
                warn |= m.update.agents == core::Agents::Restart;
                names.push(m.name);
            }
            None => names.push(ext.clone()),
        }
    }
    let names = names.join(", ");
    let tip = if warn {
        format!("Started before {names} was updated: it doesn't work with {names} until restarted")
    } else {
        format!("Started before {names} was updated: it keeps the old version until restarted")
    };
    Some(("old setup".into(), tip, warn))
}

/// Tests: what restarts an agent instead of the engine.
#[derive(Clone)]
pub struct RestartHook(pub Rc<RestartFn>);

/// Restarts the agent with this id.
pub type RestartFn = dyn Fn(&str, &mut App);

impl Global for RestartHook {}

/// Restarts asked for, one at a time, and how long each candidate has
/// been waiting for the user.
#[derive(Default)]
pub struct Restarter {
    idle_since: HashMap<String, Instant>,
    queue: Vec<(String, Vec<String>)>,
    busy: Option<String>,
    /// Restarts that failed (agent, with the setup it had): not tried by
    /// itself again.
    gave_up: HashSet<String>,
    watching: bool,
}

struct RestarterHandle(Entity<Restarter>);

impl Global for RestarterHandle {}

fn restarter(cx: &mut App) -> Entity<Restarter> {
    if let Some(h) = cx.try_global::<RestarterHandle>() {
        return h.0.clone();
    }
    let e = cx.new(|_| Restarter::default());
    cx.set_global(RestarterHandle(e.clone()));
    e
}

/// Restart agent `id` now (Restart on the card or its chip) to pick up
/// `exts`; one at a time, after any already asked for.
pub fn restart_now(id: &str, exts: Vec<String>, cx: &mut App) {
    let r = restarter(cx);
    r.update(cx, |r, _| {
        if r.busy.as_deref() != Some(id) && !r.queue.iter().any(|(q, _)| q == id) {
            r.queue.push((id.to_string(), exts));
        }
    });
    next(cx);
}

/// "Restart all idle" on extension `ext`'s card: each of its stale agents
/// waiting for the user now.
pub fn restart_idle(ext: &str, cx: &mut App) -> usize {
    let list: Vec<AgentView> = stale_agents(ext, cx).into_iter().filter(|a| waiting(a) && a.caps.restart).collect();
    for a in &list {
        restart_now(&a.id, vec![ext.to_string()], cx);
    }
    list.len()
}

/// Start the next restart asked for, unless one runs.
fn next(cx: &mut App) {
    let r = restarter(cx);
    let job = r.update(cx, |r, _| {
        if r.busy.is_some() || r.queue.is_empty() {
            return None;
        }
        let j = r.queue.remove(0);
        r.busy = Some(j.0.clone());
        Some(j)
    });
    let Some((id, exts)) = job else { return };
    let name = agents(cx).into_iter().find(|a| a.id == id).map(|a| a.name).unwrap_or_else(|| id.clone());
    let agent = id.clone();
    let finish = move |res: Result<(), String>, cx: &mut App| {
        let r = restarter(cx);
        r.update(cx, |r, _| {
            r.busy = None;
            r.idle_since.remove(&agent);
            if res.is_err() {
                r.gave_up.insert(agent.clone());
            }
        });
        let ext_names: Vec<String> = exts.iter().map(|e| super::manifest_of(e, cx).map(|m| m.name).unwrap_or_else(|| e.clone())).collect();
        let names = ext_names.join(", ");
        for e in &exts {
            log(e, &name, res.as_ref().err().cloned(), cx);
        }
        let first = exts.first().cloned().unwrap_or_default();
        match &res {
            Ok(()) => crate::windows::extension_toast(&first, format!("Restarted {name} to pick up {names}"), "It continues its conversation.".into(), None, cx),
            Err(why) => crate::windows::extension_toast(&first, format!("Couldn't restart {name}"), why.clone(), None, cx),
        }
        cx.defer(next);
    };
    if let Some(hook) = cx.try_global::<RestartHook>().cloned() {
        (hook.0)(&id, cx);
        cx.defer(move |cx| finish(Ok(()), cx));
        return;
    }
    let Some(engine) = super::engine(cx) else {
        cx.defer(move |cx| finish(Err("Pitwall's engine isn't running.".into()), cx));
        return;
    };
    let id2 = id.clone();
    let task = cx.background_executor().spawn(async move { pitwall_core::engine::lifecycle::restart(&engine, &id2, None) });
    cx.spawn(async move |cx| {
        let res = task.await;
        cx.update(|cx| match res {
            Ok(view) => {
                crate::agents::patch(view, cx);
                finish(Ok(()), cx);
            }
            Err(e) => finish(Err(e), cx),
        });
    })
    .detach();
}

/// A line in extension `ext`'s activity log about restarting agent `name`.
fn log(ext: &str, name: &str, failed: Option<String>, cx: &mut App) {
    use pitwall_core::extensions::activity::{Entry, Outcome, Via};
    let outcome = match failed {
        None => Outcome::Ok,
        Some(message) => Outcome::Failed { code: "failed".into(), message },
    };
    let log = super::calls::activity(ext, cx);
    log.lock().unwrap_or_else(|e| e.into_inner()).push(Entry::now(Via::Pitwall, "agent restarted for the update", outcome).with_detail(Some(name.to_string())));
}

/// Look every [`TICK`] while some extension restarts when idle.
fn watch(cx: &mut App) {
    let r = restarter(cx);
    if r.read(cx).watching {
        return;
    }
    r.update(cx, |r, _| r.watching = true);
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(TICK).await;
        let go_on = cx.update(|cx| {
            if !any_when_idle(cx) {
                restarter(cx).update(cx, |r, _| {
                    r.watching = false;
                    r.idle_since.clear();
                });
                return false;
            }
            tick(cx);
            true
        });
        if !go_on {
            break;
        }
    })
    .detach();
}

/// `whenIdle`: each stale agent of an extension that restarts when idle,
/// once it has waited for the user for [`DWELL`]; one at a time; never
/// one that works, waits on an approval or can't continue its
/// conversation.
pub fn tick(cx: &mut App) {
    let now = cx.background_executor().now();
    let list = agents(cx);
    let r = restarter(cx);
    let mut pick = None;
    let mut keep: HashSet<String> = HashSet::new();
    for a in &list {
        let exts: Vec<String> = a.stale_for.iter().filter(|e| mode(e, cx) == Restart::WhenIdle).cloned().collect();
        if exts.is_empty() || !resumable(a) || !waiting(a) || r.read(cx).gave_up.contains(&a.id) {
            continue;
        }
        keep.insert(a.id.clone());
        let since = r.update(cx, |r, _| *r.idle_since.entry(a.id.clone()).or_insert(now));
        if pick.is_none() && now.duration_since(since) >= DWELL {
            pick = Some((a.id.clone(), exts));
        }
    }
    let busy = r.update(cx, |r, _| {
        r.idle_since.retain(|id, _| keep.contains(id));
        r.busy.is_some() || !r.queue.is_empty()
    });
    if let (Some((id, exts)), false) = (pick, busy) {
        restart_now(&id, exts, cx);
    }
}

/// An agent's chip, when it uses an extension's previous setup: with its
/// tooltip, found as `<prefix>-<agent id>` (the sidebar's row, the pane's
/// header).
pub fn chip_view(prefix: &str, a: &AgentView, t: &crate::theme::Theme, cx: &App) -> Option<gpui::AnyElement> {
    use gpui::{IntoElement, InteractiveElement, ParentElement, StatefulInteractiveElement, Styled};
    let (text, tip, warn) = chip(a, cx)?;
    let tone = if warn { crate::kit::Tone::Warn } else { crate::kit::Tone::Subtle };
    let id = format!("{prefix}-{}", a.id);
    let sel = id.clone();
    Some(
        gpui::div()
            .id(gpui::SharedString::from(id))
            .debug_selector(move || sel)
            .flex_none()
            .tooltip(crate::kit::tooltip(tip))
            .child(crate::kit::chip_tone(text, tone, t))
            .into_any_element(),
    )
}
