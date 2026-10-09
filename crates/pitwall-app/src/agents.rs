//! The UI's model of the engine: an [`AgentStore`] entity fed by the bridge
//! (Tauri: `src/lib/useAgents.ts` listening to `agents-changed`), and the
//! sidebar's grouping and ordering (a port of `src/lib/status.ts` and
//! `src/lib/groups.ts`).

use gpui::{App, AsyncApp, Context, Entity, EventEmitter};

use futures::channel::mpsc::UnboundedReceiver;
use futures::StreamExt;

use pitwall_core::events::Event;
use pitwall_proto::{AgentView, ApprovalView, MachineView, Status};

use crate::bridge::AppEvent;

/// What the UI knows about the engine, kept current by [`AgentStore::listen`].
#[derive(Default)]
pub struct AgentStore {
    pub agents: Vec<AgentView>,
    pub approvals: Vec<ApprovalView>,
    /// Agents needing the user (the Dock badge's number).
    pub blocked: usize,
    /// Engine events applied so far (the shell shows it: proof of life).
    pub updates: u64,
}

/// Emitted for engine events views react to beyond re-rendering.
#[derive(Debug, Clone)]
pub enum StoreEvent {
    /// An agent turned blocked or done (later: notifications, toasts).
    Attention {
        agent_id: String,
        name: String,
        reason: &'static str,
        /// What it waits for or did ("Permission: Edit"), when known.
        detail: Option<String>,
    },
    /// A scan step started or finished (onboarding's checklist).
    ScanProgress(pitwall_core::onboarding::scan::ScanProgress),
    /// The project list changed.
    ProjectsChanged(Vec<pitwall_core::onboarding::project_list::Project>),
}

impl EventEmitter<StoreEvent> for AgentStore {}

impl AgentStore {
    /// A store seeded with the engine's current agents.
    pub fn new(agents: Vec<AgentView>) -> AgentStore {
        let blocked = agents
            .iter()
            .filter(|a| a.status == Status::Blocked)
            .count();
        AgentStore {
            agents,
            blocked,
            ..Default::default()
        }
    }

    /// Apply one bridge event; whether anything shown changed.
    pub fn apply(&mut self, event: AppEvent, cx: &mut Context<Self>) -> bool {
        self.updates += 1;
        match event {
            // The same list again (only something not shown changed):
            // nothing to redraw.
            AppEvent::Engine(Event::AgentsChanged(views)) if views == self.agents => return false,
            AppEvent::Engine(Event::AgentsChanged(views)) => self.agents = views,
            AppEvent::Engine(Event::BlockedCount(n)) => self.blocked = n,
            AppEvent::Engine(Event::Attention(a)) => {
                cx.emit(StoreEvent::Attention {
                    agent_id: a.agent_id,
                    name: a.name,
                    reason: a.reason,
                    detail: a.detail,
                });
                return false;
            }
            AppEvent::Approvals(list) => self.approvals = list,
            // Onboarding listens to these (settings::onboarding).
            AppEvent::Engine(Event::ScanProgress(p)) => {
                cx.emit(StoreEvent::ScanProgress(p));
                return false;
            }
            AppEvent::Engine(Event::ProjectsChanged(list)) => {
                cx.emit(StoreEvent::ProjectsChanged(list));
                return false;
            }
        }
        true
    }

    /// Drain the bridge into `store` on the main thread, for as long as the
    /// app runs.
    pub fn listen(store: &Entity<AgentStore>, mut rx: UnboundedReceiver<AppEvent>, cx: &mut App) {
        let store = store.downgrade();
        cx.spawn(async move |cx: &mut AsyncApp| {
            while let Some(event) = rx.next().await {
                let alive = store.update(cx, |s, cx| {
                    if s.apply(event, cx) {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    pub fn groups(&self) -> Vec<ProjectGroup> {
        group_by_project(&self.agents)
    }

    pub fn agent(&self, id: &str) -> Option<&AgentView> {
        self.agents.iter().find(|a| a.id == id)
    }
}

/// Sidebar order: blocked → done → working → idle → others.
pub fn status_order(s: Status) -> u8 {
    match s {
        Status::Blocked => 0,
        Status::Done => 1,
        Status::Working => 2,
        Status::Idle => 3,
        Status::Unknown => 4,
        Status::Exited => 5,
        Status::Stopped => 6,
    }
}

/// How a status is said (`STATUS_WORD`).
pub fn status_word(s: Status) -> &'static str {
    match s {
        Status::Blocked => "needs you",
        Status::Done => "done",
        Status::Working => "working",
        Status::Idle => "idle",
        Status::Unknown => "unknown",
        Status::Exited => "exited",
        Status::Stopped => "stopped",
    }
}

/// The status flag (`STATUS_GLYPH`; working is drawn as a dot).
pub fn status_glyph(s: Status) -> &'static str {
    match s {
        Status::Blocked => "▲",
        Status::Done => "⚑",
        Status::Working => "●",
        Status::Idle => "●",
        Status::Unknown => "?",
        Status::Exited | Status::Stopped => "■",
    }
}

/// One project's agents on one machine.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectGroup {
    pub key: String,
    pub project: String,
    pub display: String,
    pub machine: Option<MachineView>,
    pub can_create: bool,
    /// Sorted by [`status_order`], then creation time.
    pub agents: Vec<AgentView>,
    pub blocked: usize,
}

fn machine_key(m: &MachineView) -> String {
    format!("{}:{}", m.provider, m.id)
}

/// Group agents by machine and project: groups on machines where agents can
/// be started first, then other machines by label; within one, by display
/// name (`groupByProject`).
pub fn group_by_project(agents: &[AgentView]) -> Vec<ProjectGroup> {
    let mut groups: Vec<ProjectGroup> = Vec::new();
    for a in agents {
        let can_create = a.machine.can_create;
        let key = if can_create {
            a.project.clone()
        } else {
            format!("{}|{}", machine_key(&a.machine), a.project)
        };
        match groups.iter_mut().find(|g| g.key == key) {
            Some(g) => g.agents.push(a.clone()),
            None => groups.push(ProjectGroup {
                key,
                project: a.project.clone(),
                display: if a.project_display.is_empty() {
                    a.project.clone()
                } else {
                    a.project_display.clone()
                },
                machine: Some(a.machine.clone()),
                can_create,
                agents: vec![a.clone()],
                blocked: 0,
            }),
        }
    }
    for g in &mut groups {
        g.agents
            .sort_by_key(|a| (status_order(a.status), a.created_at));
        g.blocked = g
            .agents
            .iter()
            .filter(|a| a.status == Status::Blocked)
            .count();
    }
    let label = |g: &ProjectGroup| {
        if g.can_create {
            String::new()
        } else {
            g.machine
                .as_ref()
                .map(|m| m.label.clone())
                .unwrap_or_default()
        }
    };
    groups.sort_by(|a, b| {
        b.can_create
            .cmp(&a.can_create)
            .then_with(|| label(a).cmp(&label(b)))
            .then_with(|| a.display.cmp(&b.display))
    });
    groups
}

/// The app's one store, for code that has no handle to it.
pub struct StoreHandle(pub Entity<AgentStore>);

impl gpui::Global for StoreHandle {}

/// An `AgentView` a command returned goes in at once (the engine's
/// `agents-changed` may lag; React `useAgents` patches it in).
pub fn patch(view: AgentView, cx: &mut gpui::App) {
    let Some(store) = cx.try_global::<StoreHandle>().map(|h| h.0.clone()) else {
        return;
    };
    store.update(cx, |s, cx| {
        if let Some(a) = s.agents.iter_mut().find(|a| a.id == view.id) {
            if *a != view {
                *a = view;
                cx.notify();
            }
        }
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gpui::AppContext;
    use serde_json::json;

    /// A made-up agent (every field AgentView has).
    pub fn agent(
        id: &str,
        project: &str,
        status: &str,
        created_at: u64,
        can_create: bool,
    ) -> AgentView {
        serde_json::from_value(json!({
            "id": id, "name": id, "kind": "shell", "kindName": "Terminal", "terminal": true,
            "sessionId": null, "cwd": project, "cwdDisplay": project,
            "project": project, "projectDisplay": project.trim_start_matches("/work/"),
            "branch": null, "worktree": false, "worktreePending": false, "location": "local",
            "machine": { "provider": if can_create { "local" } else { "agw" }, "id": "m1",
                         "label": if can_create { "This Mac" } else { "vm-1" }, "canCreate": can_create },
            "agentInTerminal": false, "restartAs": null, "status": status, "statusSource": "activity",
            "statusDetail": null, "running": true, "cols": 80, "rows": 24, "added": 0, "removed": 0,
            "filesChanged": 0, "queue": [], "autoSend": false, "lastSent": null, "lastSentAt": null,
            "createdAt": created_at, "currentTaskId": null,
            "caps": serde_json::to_value(pitwall_proto::AgentCaps::default()).unwrap()
        }))
        .expect("a complete AgentView")
    }

    #[test]
    fn groups_by_project_with_needy_agents_first() {
        let agents = vec![
            agent("b-idle", "/work/beta", "idle", 1, true),
            agent("a-work", "/work/alpha", "working", 2, true),
            agent("b-blocked", "/work/beta", "blocked", 3, true),
            agent("b-done", "/work/beta", "done", 0, true),
            agent("vm", "/srv/gamma", "idle", 0, false),
        ];
        let groups = group_by_project(&agents);
        let names: Vec<_> = groups.iter().map(|g| g.display.as_str()).collect();
        assert_eq!(
            names,
            ["alpha", "beta", "/srv/gamma"],
            "creatable machines first, then by name"
        );
        let beta: Vec<_> = groups[1].agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(beta, ["b-blocked", "b-done", "b-idle"]);
        assert_eq!(groups[1].blocked, 1);
        assert_eq!(groups[2].key, "agw:m1|/srv/gamma");
    }

    #[test]
    fn every_status_has_a_word_and_a_glyph() {
        for s in [
            Status::Working,
            Status::Blocked,
            Status::Idle,
            Status::Done,
            Status::Unknown,
            Status::Exited,
            Status::Stopped,
        ] {
            assert!(!status_word(s).is_empty() && !status_glyph(s).is_empty());
        }
        assert!(status_order(Status::Blocked) < status_order(Status::Working));
    }

    #[gpui::test]
    fn the_store_follows_engine_events(cx: &mut gpui::TestAppContext) {
        let store = cx.new(|_| AgentStore::new(vec![]));
        let (bridge, rx) = crate::bridge::Bridge::new();
        cx.update(|cx| AgentStore::listen(&store, rx, cx));

        bridge.send(AppEvent::Engine(Event::AgentsChanged(vec![agent(
            "a1",
            "/work/alpha",
            "blocked",
            0,
            true,
        )])));
        bridge.send(AppEvent::Engine(Event::BlockedCount(1)));
        cx.run_until_parked();
        store.read_with(cx, |s, _| {
            assert_eq!(s.agents.len(), 1);
            assert_eq!(s.blocked, 1);
            assert_eq!(s.groups()[0].display, "alpha");
            assert_eq!(s.updates, 2);
        });

        bridge.send(AppEvent::Engine(Event::AgentsChanged(vec![])));
        cx.run_until_parked();
        store.read_with(cx, |s, _| assert!(s.agents.is_empty()));
    }

    #[gpui::test]
    fn the_same_list_again_redraws_nothing(cx: &mut gpui::TestAppContext) {
        let one = || vec![agent("a1", "/work/alpha", "working", 0, true)];
        let store = cx.new(|_| AgentStore::new(one()));
        let notified = std::rc::Rc::new(std::cell::Cell::new(0));
        let n = notified.clone();
        let _sub = cx.update(|cx| cx.observe(&store, move |_, _| n.set(n.get() + 1)));
        let (bridge, rx) = crate::bridge::Bridge::new();
        cx.update(|cx| AgentStore::listen(&store, rx, cx));
        bridge.send(AppEvent::Engine(Event::AgentsChanged(one())));
        cx.run_until_parked();
        assert_eq!(notified.get(), 0);
        bridge.send(AppEvent::Engine(Event::AgentsChanged(vec![])));
        cx.run_until_parked();
        assert_eq!(notified.get(), 1);
    }
}
