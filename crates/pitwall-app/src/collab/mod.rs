//! Collaboration in the app (docs/spec/collaboration/ui.md §11): People
//! (⌘⇧P), the Ask panel (⌘⇧A), the Share dialog, the sidebar's "Shared"
//! group with per-caller rows and Usage, the shared session's strip and
//! Messages, and Settings → Collaboration and Networks.
//!
//! Everything goes through the daemon's [`CollabService`] as Pitwall's
//! own window (the owner UI may do everything, §8.5), on a background
//! thread; [`CollabState`] keeps what was read last and notifies its
//! observers only when something changed. Read-only shares (C1a): the
//! "Can change code" answers are shown but can't be picked yet, and there
//! is no Apply or patch UI.

pub mod ask;
pub mod messages;
pub mod people;
pub mod settings_page;
pub mod share_dialog;
pub mod sidebar;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyElement, AnyView, AnyWindowHandle, App, AppContext, Context, Entity, Global, IntoElement, Window};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use pitwall_daemon::{Caller, CollabService};
use pitwall_proto::api::collab::{CollabSettingsView, NoticeKind, OwnerNotice, PersonView, ShareView};
use pitwall_proto::collab::{AskView, DirectoryEntry, SharedSessionView, TeamCard};
use pitwall_proto::{method, AgentView, ErrorBody};

/// How often the lists are read again (in-process, cheap).
pub const POLL: Duration = Duration::from_secs(3);

/// The hosted collaboration service (`None`: not hosted, e.g. refused).
#[derive(Clone)]
pub struct CollabHost(pub Option<Arc<CollabService>>);

impl Global for CollabHost {}

/// Pitwall's own window, as the daemon names it.
pub fn ui_caller() -> Caller {
    Caller { pid: None, agent: None, process: Some("pitwall".into()), ui: true }
}

/// Everything the screens show, as read last.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub shares: Vec<ShareView>,
    /// Per share slug.
    pub sessions: BTreeMap<String, Vec<SharedSessionView>>,
    pub teams: Vec<TeamCard>,
    pub people: Vec<PersonView>,
    pub asks: Vec<AskView>,
    pub settings: Option<CollabSettingsView>,
    /// What the owner should hear about (needs you, no answer, the day's
    /// agent time, the agent's limit), oldest first.
    pub notices: Vec<OwnerNotice>,
}

impl Snapshot {
    /// Shared agents I may use, from everyone's entries (the Directory).
    pub fn directory(&self) -> Vec<DirectoryEntry> {
        let mut all: Vec<DirectoryEntry> = self.people.iter().flat_map(|p| p.shares.iter().cloned()).collect();
        all.sort_by(|a, b| (a.owner.as_str(), a.slug.as_str()).cmp(&(b.owner.as_str(), b.slug.as_str())));
        all.dedup_by(|a, b| a.owner == b.owner && a.slug == b.slug);
        all
    }

    /// The share of agent `id`.
    pub fn share_of(&self, agent_id: &str) -> Option<&ShareView> {
        self.shares.iter().find(|s| s.settings.agent == agent_id)
    }

    /// The share and session an engine agent is (a shared session's own agent).
    pub fn session_of(&self, agent_id: &str) -> Option<(&ShareView, &SharedSessionView)> {
        self.sessions.iter().find_map(|(slug, list)| {
            let s = list.iter().find(|s| s.agent_id == agent_id)?;
            let share = self.shares.iter().find(|v| v.settings.slug == *slug)?;
            Some((share, s))
        })
    }
}

/// The app's collaboration state (one per app).
pub struct CollabState {
    pub data: Snapshot,
    pub loaded: bool,
    /// Sidebar share headings folded (by slug).
    pub collapsed: BTreeSet<String>,
    /// The Share dialog or Usage, over one window.
    pub dialog: Option<(AnyWindowHandle, AnyView)>,
    /// The Ask panel's chosen agent (`alice/api-expert`).
    pub ask_target: Option<String>,
    /// The last notice told as a system notification.
    pub notified: Option<u64>,
}

impl CollabState {
    fn new() -> CollabState {
        CollabState { data: Snapshot::default(), loaded: false, collapsed: BTreeSet::new(), dialog: None, ask_target: None, notified: None }
    }

    /// New data: notify only when it changed. Notices that are new since
    /// the last look become system notifications (not needs-you: the
    /// engine tells that one for every blocked agent), never the ones
    /// already there when Pitwall started.
    pub fn set(&mut self, data: Snapshot, cx: &mut Context<Self>) {
        let last = data.notices.last().map(|n| n.id);
        if let Some(seen) = self.notified {
            for n in data.notices.iter().filter(|n| n.id > seen && n.kind != NoticeKind::NeedsYou) {
                crate::platform::notify_user(n.share.clone(), n.text.clone(), cx);
            }
        }
        if last.is_some() || self.notified.is_none() {
            self.notified = Some(last.unwrap_or(0).max(self.notified.unwrap_or(0)));
        }
        self.loaded = true;
        if data != self.data {
            self.data = data;
            cx.notify();
        }
    }
}

pub struct Model(pub Entity<CollabState>);

impl Global for Model {}

/// The state, made on first use (also without a service: tests, refused).
pub fn model_or_new(cx: &mut App) -> Entity<CollabState> {
    if let Some(m) = cx.try_global::<Model>() {
        return m.0.clone();
    }
    let e = cx.new(|_| CollabState::new());
    cx.set_global(Model(e.clone()));
    e
}

pub fn model(cx: &App) -> Option<Entity<CollabState>> {
    cx.try_global::<Model>().map(|m| m.0.clone())
}

pub fn service(cx: &App) -> Option<Arc<CollabService>> {
    cx.try_global::<CollabHost>().and_then(|h| h.0.clone())
}

/// Collaboration is hosted here (the People and Ask buttons show).
pub fn hosted(cx: &App) -> bool {
    service(cx).is_some()
}

/// What was read last (empty before the first read).
pub fn snapshot(cx: &App) -> Snapshot {
    model(cx).map(|m| m.read(cx).data.clone()).unwrap_or_default()
}

pub fn share_of(agent_id: &str, cx: &App) -> Option<ShareView> {
    model(cx).and_then(|m| m.read(cx).data.share_of(agent_id).cloned())
}

/// The agent menu offers Share… for it: a coding agent on this Mac (not a
/// terminal, not the Race Engineer, not itself a shared session).
pub fn can_share(a: &AgentView, cx: &App) -> bool {
    hosted(cx)
        && !a.terminal
        && !a.engineer
        && a.machine.provider == pitwall_core::provider::ProviderId::LOCAL
        && model(cx).is_none_or(|m| m.read(cx).data.session_of(&a.id).is_none())
}

/// Read everything once (blocking; off the UI thread).
pub fn fetch(svc: &CollabService) -> Snapshot {
    let ui = ui_caller();
    fn get<T: DeserializeOwned + Default>(svc: &CollabService, ui: &Caller, m: &str, p: Value) -> T {
        svc.call(ui, m, p).and_then(|r| r.ok()).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }
    let shares: Vec<ShareView> = get(svc, &ui, method::SHARE_LIST, Value::Null);
    let sessions = shares
        .iter()
        .map(|s| (s.settings.slug.clone(), get(svc, &ui, method::SHARE_SESSIONS, json!({"share": s.settings.slug}))))
        .collect();
    let settings: Option<CollabSettingsView> = svc.call(&ui, method::COLLAB_SETTINGS_GET, Value::Null).and_then(|r| r.ok()).and_then(|v| serde_json::from_value(v).ok());
    Snapshot {
        shares,
        sessions,
        teams: get(svc, &ui, method::TEAM_LIST, Value::Null),
        people: get(svc, &ui, method::PEOPLE_LIST, Value::Null),
        asks: get(svc, &ui, method::ASK_LIST, Value::Null),
        settings,
        notices: get(svc, &ui, method::SHARE_NOTICES, Value::Null),
    }
}

/// Shared sessions that need the owner beyond what the engine counts as
/// blocked: a turn whose reply couldn't be read, results held for Send.
/// They flag amber and count in the badge like a blocked agent.
pub fn extra_attention(cx: &App) -> usize {
    let Some(m) = model(cx) else { return 0 };
    m.read(cx).data.sessions.values().flatten().filter(|s| s.state != pitwall_proto::collab::SessionState::Blocked && (s.flagged || s.held > 0)).count()
}

/// Read everything again now (after a change).
pub fn refresh(cx: &mut App) {
    let (Some(svc), Some(m)) = (service(cx), model(cx)) else { return };
    let task = cx.background_executor().spawn(async move { fetch(&svc) });
    cx.spawn(async move |cx| {
        let data = task.await;
        let _ = m.update(cx, |s, cx| s.set(data, cx));
    })
    .detach();
}

/// Call `method` as Pitwall's window off the UI thread; `done` gets the
/// answer on the UI thread, then everything is read again.
pub fn call<T: DeserializeOwned + Send + 'static>(
    method: &'static str,
    params: Value,
    cx: &mut App,
    done: impl FnOnce(Result<T, ErrorBody>, &mut App) + 'static,
) {
    let Some(svc) = service(cx) else {
        done(Err(ErrorBody::new(pitwall_proto::code::UNSUPPORTED, "Collaboration isn't running in this Pitwall")), cx);
        return;
    };
    let task = cx.background_executor().spawn(async move {
        let r = svc.call(&ui_caller(), method, params).unwrap_or_else(|| Err(ErrorBody::new(pitwall_proto::code::UNKNOWN_METHOD, method)));
        r.and_then(|v| serde_json::from_value::<T>(v).map_err(|e| ErrorBody::new(pitwall_proto::code::OTHER, e.to_string())))
    });
    cx.spawn(async move |cx| {
        let r = task.await;
        let _ = cx.update(|cx| {
            done(r, cx);
            refresh(cx);
        });
    })
    .detach();
}

/// Once at start: the service, the routes, the panel section, the palette.
pub fn init(svc: Option<Arc<CollabService>>, cx: &mut App) {
    cx.set_global(CollabHost(svc.clone()));
    let m = model_or_new(cx);
    use crate::main_screen::{register_route, Route};
    register_route(cx, Route::People, |ctx, window, cx| cx.new(|cx| people::PeopleView::new(ctx.screen.clone(), window, cx)).into());
    register_route(cx, Route::Ask, |ctx, window, cx| cx.new(|cx| ask::AskPanel::new(ctx.screen.clone(), window, cx)).into());
    messages::register(cx);
    crate::palette::registry::register(cx, palette_commands);
    if let Some(svc) = svc {
        // Read now, then every few seconds while a window shows.
        cx.spawn(async move |cx| loop {
            let s = svc.clone();
            let data = cx.background_executor().spawn(async move { fetch(&s) }).await;
            if m.update(cx, |st, cx| st.set(data, cx)).is_err() {
                break;
            }
            cx.background_executor().timer(POLL).await;
            crate::platform::visible::until_any_visible(cx).await;
        })
        .detach();
    }
}

fn palette_commands(pc: &crate::palette::registry::PaletteContext, cx: &App) -> Vec<crate::palette::registry::Command> {
    use crate::main_screen::Route;
    use crate::palette::registry::{rank, Command};
    if !hosted(cx) {
        return vec![];
    }
    let route = |r: Route| {
        move |window: &mut Window, cx: &mut App| {
            let _ = window;
            open_route(r, cx);
        }
    };
    let mut out = vec![
        Command::new("collab-people", "People: teams and shared agents", "people teams directory shared agents collaboration", route(Route::People))
            .icon("people")
            .keys("⌘⇧P")
            .rank(rank::VIEWS + 10),
        Command::new("collab-ask", "Ask a shared agent", "ask shared agent teammate question", route(Route::Ask)).icon("ask").keys("⌘⇧A").rank(rank::VIEWS + 11),
    ];
    if let Some(a) = pc.selected().filter(|a| can_share(a, cx)) {
        let id = a.id.clone();
        let label = if share_of(&a.id, cx).is_some() { format!("Sharing {}…", a.name) } else { format!("Share {}…", a.name) };
        out.push(
            Command::new("collab-share", label, format!("share expose {} teammates", a.name), move |window, cx| share_dialog::open(&id, window, cx))
                .icon("share")
                .rank(rank::VIEWS + 12),
        );
    }
    out
}

/// Show `route` in the active (or main) window.
pub fn open_route(route: crate::main_screen::Route, cx: &mut App) {
    cx.defer(move |cx| {
        let Some(target) = crate::windows::active_or_main(cx) else { return };
        let _ = target.update(cx, |view, _, cx| {
            if let Some(s) = view.screen.clone() {
                s.update(cx, |s, cx| s.set_route(route, cx));
            }
        });
    });
}

/// Show a dialog (the Share dialog, Usage) over `window`.
pub fn show_dialog(view: AnyView, window: &mut Window, cx: &mut App) {
    let handle = window.window_handle();
    let m = model_or_new(cx);
    m.update(cx, |s, cx| {
        s.dialog = Some((handle, view));
        cx.notify();
    });
    window.refresh();
}

pub fn close_dialog(cx: &mut App) {
    if let Some(m) = model(cx) {
        m.update(cx, |s, cx| {
            if s.dialog.take().is_some() {
                cx.notify();
            }
        });
    }
    cx.refresh_windows();
}

/// The open dialog, when it belongs to this window (`ui::MainView` draws it).
pub fn overlay(window: &Window, cx: &App) -> Option<AnyElement> {
    let (h, v) = model(cx)?.read(cx).dialog.clone()?;
    (h.window_id() == window.window_handle().window_id()).then(|| v.into_any_element())
}

/// An agent kind's name from the kind catalog (`Claude Code`), else its id.
pub fn kind_name(kind: &str, cx: &App) -> String {
    cx.try_global::<crate::main_screen::EngineHandle>()
        .and_then(|e| e.0.kinds().kinds().into_iter().find(|k| k.id == kind).map(|k| k.name))
        .unwrap_or_else(|| kind.to_string())
}

/// A person's short name (`bob@example.com` → `bob`).
pub fn short(login: &str) -> &str {
    login.split('@').next().unwrap_or(login)
}
