//! The engine: the set of agents and their live state, plus the background
//! work around them (status ticker, change emitter, task snapshots, hook
//! ingest). The host builds one with [`Deps`] and talks to it through the
//! service modules (`input`, `lifecycle`, `review`, …).
//!
//! The registry lock is only ever held for in-memory bookkeeping — never
//! across terminal writes, git, or disk I/O.

pub(crate) mod agent;
pub mod changes;
pub mod input;
pub mod lifecycle;
mod status;
pub(crate) mod tasks;
mod ticker;
pub(crate) mod worktree;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use serde_json::Value;

use crate::clock::Clock;
use crate::events::{Event, EventSink};
use crate::hooks::{self, HookEffect};
use crate::kind::KindCatalog;
use crate::model::{AgentRecord, AgentView, QueueItem};
use crate::onboarding::project_list::ProjectList;
use crate::paths::Paths;
use crate::session::Session;
use crate::store::{Snapshot, Store};
pub(crate) use agent::Agent;
pub use tasks::Task;

/// What the host provides.
pub struct Deps {
    pub paths: Paths,
    pub events: Arc<dyn EventSink>,
    pub clock: Arc<dyn Clock>,
    pub store: Arc<dyn Store>,
    /// The `pitwall-hold` executable, or why there is none (reported when an
    /// agent is started).
    pub holder_bin: Result<PathBuf, String>,
}

#[derive(Default)]
struct Dirty {
    view: bool,
    persist: bool,
}

pub struct Engine {
    agents: Mutex<Vec<Agent>>,
    dirty: Mutex<Dirty>,
    wake: Condvar,
    paths: Paths,
    events: Arc<dyn EventSink>,
    clock: Arc<dyn Clock>,
    store: Arc<dyn Store>,
    holder_bin: Result<PathBuf, String>,
    kinds: Arc<KindCatalog>,
    projects: ProjectList,
    snapshots: tasks::Worker,
}

pub type Shared = Arc<Engine>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Engine {
    /// Load the saved agents. Agents whose terminal holder survived the last
    /// run are running again, without a restart; the rest are stopped.
    /// Starts no background work (see [`start`](Self::start)).
    pub fn open(deps: Deps) -> Shared {
        let Deps { paths, events, clock, store, holder_bin } = deps;
        let agents = store
            .load()
            .agents
            .into_iter()
            .map(|rec| {
                let session = lifecycle::reattach(&paths, &clock, &rec);
                Agent::new(rec, session, clock.mono_ms())
            })
            .collect();
        Arc::new(Engine {
            agents: Mutex::new(agents),
            dirty: Mutex::new(Dirty::default()),
            wake: Condvar::new(),
            kinds: Arc::new(KindCatalog::new(paths.user_agents_dir())),
            projects: ProjectList::new(paths.projects_file()),
            snapshots: tasks::Worker::default(),
            paths,
            events,
            clock,
            store,
            holder_bin,
        })
    }

    /// Background work: resolve installed kinds, install the hook relay and
    /// listen for hooks, the snapshot worker, the status ticker and the
    /// throttled change emitter / state saver.
    pub fn start(self: &Shared) {
        self.kinds.warm_up();
        if let Err(e) = hooks::install_script(&self.paths) {
            eprintln!("pitwall: could not install hook script: {e}");
        }
        let engine = self.clone();
        if let Err(e) = hooks::serve(&self.paths.hook_socket(), move |id, payload| engine.ingest_hook(id, &payload)) {
            // Hooks are optional: screen/activity detection still works.
            eprintln!("pitwall: hooks disabled: {e}");
        }
        self.snapshots.start(self.clone());
        ticker::start(self.clone());
    }

    /// One hook payload from agent `id`.
    pub fn ingest_hook(self: &Shared, id: &str, payload: &Value) {
        // First: an agent that made its own worktree reports where it works.
        worktree::on_hook(self, id, payload);
        self.apply_hook(id, hooks::map_hook(payload));
        tasks::on_hook(self, id, payload);
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn kinds(&self) -> &KindCatalog {
        &self.kinds
    }

    pub fn projects(&self) -> &ProjectList {
        &self.projects
    }

    pub(crate) fn emit(&self, event: Event) {
        self.events.emit(event);
    }

    pub(crate) fn now(&self) -> u64 {
        self.clock.mono_ms()
    }

    pub(crate) fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    pub(crate) fn holder_bin(&self) -> Result<&Path, String> {
        self.holder_bin.as_deref().map_err(Clone::clone)
    }

    pub(crate) fn submit(&self, jobs: Vec<tasks::Job>) {
        self.snapshots.submit(jobs);
    }

    // ------------------------------------------------------------ registry

    pub(crate) fn agents(&self) -> MutexGuard<'_, Vec<Agent>> {
        lock(&self.agents)
    }

    /// Run `f` on one agent under the lock.
    pub(crate) fn with<T>(&self, id: &str, f: impl FnOnce(&mut Agent) -> T) -> Result<T, String> {
        let mut agents = self.agents();
        let agent = agents
            .iter_mut()
            .find(|a| a.rec.id == id)
            .ok_or_else(|| format!("no agent {id}"))?;
        Ok(f(agent))
    }

    pub(crate) fn session(&self, id: &str) -> Result<Arc<Session>, String> {
        self.with(id, |a| a.session.clone())?
            .ok_or_else(|| "agent is not running".to_string())
    }

    pub fn views(&self) -> Vec<AgentView> {
        self.agents().iter().map(Agent::view).collect()
    }

    pub fn records(&self) -> Vec<AgentRecord> {
        self.agents().iter().map(|a| a.rec.clone()).collect()
    }

    /// Something shown changed (`persist`: also something stored).
    pub(crate) fn changed(&self, persist: bool) {
        let mut d = lock(&self.dirty);
        d.view = true;
        d.persist |= persist;
        self.wake.notify_one();
    }

    /// Block until something is dirty; returns (view, persist).
    fn wait_dirty(&self) -> (bool, bool) {
        let mut d = lock(&self.dirty);
        while !d.view && !d.persist {
            d = self.wake.wait(d).unwrap_or_else(|e| e.into_inner());
        }
        let out = (d.view, d.persist);
        *d = Dirty::default();
        out
    }

    /// Write the agents' records to the store now.
    pub fn save(&self) -> Result<(), String> {
        self.store.save(&Snapshot { agents: self.records() })
    }

    // ------------------------------------------------------------ queue & seen

    pub fn queue_add(&self, id: &str, text: String) -> Result<AgentView, String> {
        let view = self.with(id, |a| {
            a.rec.queue.push(QueueItem {
                id: uuid::Uuid::new_v4().to_string(),
                text,
            });
            a.view()
        })?;
        self.changed(true);
        Ok(view)
    }

    pub fn queue_remove(&self, id: &str, item_id: &str) -> Result<AgentView, String> {
        let view = self.with(id, |a| {
            a.rec.queue.retain(|q| q.id != item_id);
            a.view()
        })?;
        self.changed(true);
        Ok(view)
    }

    pub fn set_auto_send(&self, id: &str, enabled: bool) -> Result<AgentView, String> {
        let view = self.with(id, |a| {
            a.rec.auto_send = enabled;
            a.view()
        })?;
        self.changed(true);
        Ok(view)
    }

    pub fn mark_seen(&self, id: &str) -> Result<(), String> {
        let now = self.now();
        self.with(id, |a| a.mark_seen(now))?;
        self.changed(false);
        Ok(())
    }

    fn apply_hook(&self, id: &str, effect: HookEffect) {
        if let Ok(persist) = self.with(id, |a| a.apply_hook(effect)) {
            self.changed(persist);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Status;
    use crate::testing::{record, Harness};

    #[test]
    fn opens_saved_agents_as_stopped() {
        let h = Harness::new(vec![record("a", "/tmp"), record("b", "/tmp")]);
        let views = h.engine.views();
        assert_eq!(views.len(), 2);
        assert!(views.iter().all(|v| v.status == Status::Stopped && !v.running));
        assert!(h.engine.session("a").is_err());
    }

    #[test]
    fn queue_edits_are_persisted_changes() {
        let h = Harness::new(vec![record("a", "/tmp")]);
        let v = h.engine.queue_add("a", "first".into()).unwrap();
        assert_eq!(v.queue.len(), 1);
        assert_eq!(h.engine.wait_dirty(), (true, true));
        h.engine.queue_remove("a", &v.queue[0].id).unwrap();
        assert!(h.engine.views()[0].queue.is_empty());
        assert!(h.engine.queue_add("nope", "x".into()).is_err());
        h.engine.mark_seen("a").unwrap();
        assert_eq!(h.engine.wait_dirty(), (true, true), "flags accumulate until taken");
        h.engine.mark_seen("a").unwrap();
        assert_eq!(h.engine.wait_dirty(), (true, false), "seen is view-only");
    }

    #[test]
    fn hooks_update_the_agent() {
        let h = Harness::new(vec![record("a", "/tmp")]);
        h.engine.ingest_hook("a", &serde_json::json!({"hook_event_name": "UserPromptSubmit", "session_id": "s-9", "prompt": "hi"}));
        let rec = h.engine.records().remove(0);
        assert_eq!(rec.session_id.as_deref(), Some("s-9"));
        assert!(rec.has_conversation);
        assert_eq!(rec.tasks.len(), 1, "a prompt starts a task");
        assert_eq!(rec.tasks[0].prompt, "hi");
        // Unknown agents are ignored.
        h.engine.ingest_hook("ghost", &serde_json::json!({"hook_event_name": "Stop"}));
    }

    #[test]
    fn save_writes_records_to_the_store() {
        let h = Harness::new(vec![record("a", "/tmp")]);
        h.engine.save().unwrap();
        assert_eq!(h.store.saves(), 1);
        assert_eq!(h.store.load().agents[0].id, "a");
    }
}
