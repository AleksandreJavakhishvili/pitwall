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
pub mod terminals;
mod ticker;
pub(crate) mod worktree;

use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use serde_json::Value;

use crate::clock::Clock;
use crate::error::{PwError, Result as PwResult};
use crate::events::{Event, EventSink};
use crate::exec::{Cmd, Exec, Out, Stat};
use crate::hooks::{self, HookEffect};
use crate::kind::{self, KindCatalog};
use crate::model::{AgentRecord, AgentView, KindCaps, KindView, QueueItem};
use crate::onboarding::project_list::ProjectList;
use crate::paths::Paths;
use crate::provider::{CreateForm, Locator, Provider, Providers, TermSize};
use pitwall_proto::{MachineEntry, ProviderMachines};
use crate::store::{Snapshot, Store};
use crate::term::TermHost;
pub(crate) use agent::{Agent, Facts};
pub use tasks::Task;

/// What the host provides.
pub struct Deps {
    pub paths: Paths,
    pub events: Arc<dyn EventSink>,
    pub clock: Arc<dyn Clock>,
    pub store: Arc<dyn Store>,
    /// Where agents run (architecture.md §2.2). The first one that can create
    /// agents gets new ones; stored agents find theirs by locator.
    pub providers: Vec<Arc<dyn Provider>>,
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
    providers: Providers,
    kinds: Arc<KindCatalog>,
    projects: ProjectList,
    snapshots: tasks::Worker,
    /// Worktree listings per project (crate::worktrees).
    pub(crate) worktrees: crate::worktrees::Cache,
}

pub type Shared = Arc<Engine>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Engine {
    /// Load the saved agents. Agents whose session survived the last run
    /// (their provider can attach to it) are running again, without a
    /// restart; the rest are stopped. Starts no background work (see
    /// [`start`](Self::start)).
    pub fn open(deps: Deps) -> Shared {
        let Deps { paths, events, clock, store, providers } = deps;
        let engine = Arc::new(Engine {
            agents: Mutex::new(Vec::new()),
            dirty: Mutex::new(Dirty::default()),
            wake: Condvar::new(),
            kinds: Arc::new(KindCatalog::new(paths.user_agents_dir())),
            projects: ProjectList::new(paths.projects_file()),
            snapshots: tasks::Worker::default(),
            worktrees: Default::default(),
            providers: Providers::new(providers),
            paths,
            events,
            clock,
            store,
        });
        let agents = engine
            .store
            .load()
            .agents
            .into_iter()
            .map(|mut rec| {
                // v1 records: every agent ran on this Mac.
                let loc = rec.locator.get_or_insert_with(|| Locator::local(&rec.id)).clone();
                let host = lifecycle::reattach(&engine, &loc, &rec);
                let mut agent = Agent::new(rec, host, engine.now());
                agent.facts = engine.facts(&loc, &agent.rec);
                agent
            })
            .collect();
        *engine.agents() = agents;
        engine
    }

    /// Background work: resolve installed kinds, install the hook relay and
    /// listen for hooks, the snapshot worker, the status ticker and the
    /// throttled change emitter / state saver.
    pub fn start(self: &Shared) {
        // Resolving which kinds are installed is slow (login shells): start now.
        let engine = self.clone();
        std::thread::spawn(move || {
            let _ = engine.list_kinds();
        });
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
        terminals::start(self.clone());
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

    pub fn providers(&self) -> &Providers {
        &self.providers
    }

    /// Agent `id`'s locator and provider.
    pub(crate) fn provider_of(&self, id: &str) -> PwResult<(Locator, Arc<dyn Provider>)> {
        let loc = self.with(id, |a| a.rec.locator()).map_err(PwError::not_found)?;
        let p = self.providers.for_locator(&loc)?;
        Ok((loc, p))
    }

    /// The machine where agent `id` works: git, snapshots, Review, worktree
    /// discovery and rules reach its files through this. A machine that
    /// can't be reached answers every call with that error. Takes the
    /// registry lock: never call it from inside [`with`](Self::with).
    pub(crate) fn exec_for(&self, id: &str) -> Arc<dyn Exec> {
        let loc = self.with(id, |a| a.rec.locator()).unwrap_or_else(|_| Locator::local(id));
        self.providers.exec(&loc).unwrap_or_else(|e| Arc::new(NoExec(e)))
    }

    /// What the engine needs to know about `loc`'s provider and `rec`'s kind
    /// there (capabilities, machine label, home folder).
    pub(crate) fn facts(&self, loc: &Locator, rec: &AgentRecord) -> Facts {
        let Ok(p) = self.providers.for_locator(loc) else { return Facts::default() };
        let caps = p.caps();
        let kind = self.kinds.for_record(&rec.kind, rec.custom_command.as_deref());
        Facts {
            provider: caps,
            kind: kind.map(|k| KindCaps::of(&k, &caps)).unwrap_or_default(),
            machine_label: self.providers.machine_label(loc),
            home: p.exec(&loc.machine).and_then(|x| x.home()).ok(),
            inner: rec.inner_agent.as_ref().and_then(|i| self.kinds.find(&i.kind)).map(|k| KindCaps::of(&k, &caps)),
        }
    }

    /// Kinds for the New-agent dialog, on the machine new agents go to, with
    /// what each can do there. Blocking (the provider may resolve programs).
    pub fn list_kinds(&self) -> Result<Vec<KindView>, String> {
        let (p, m) = self.providers.default_target()?;
        let caps = p.caps();
        let mut views: Vec<KindView> = p
            .kinds(&m.id, &self.kinds)?
            .into_iter()
            .map(|k| {
                let kc = KindCaps::of(&k.kind, &caps);
                KindView { id: k.kind.id, name: k.kind.name, installed: k.installed, path: k.path, worktree: kc.worktree, caps: kc }
            })
            .collect();
        if caps.custom_command {
            let custom = kind::custom_kind("");
            let kc = KindCaps::of(&custom, &caps);
            views.push(KindView { id: custom.id, name: "Custom command".into(), installed: true, path: None, worktree: kc.worktree, caps: kc });
        }
        Ok(views)
    }

    /// How new agents are made on `machine` of `provider` (default: where
    /// new agents go): the New-agent dialog's fields and `machine.form`.
    /// Blocking (a platform lists its choices; cached briefly there).
    pub fn create_form(&self, provider: Option<&str>, machine: Option<&str>) -> Result<CreateForm, String> {
        let (p, m) = self.providers.target(provider, machine)?;
        if !p.caps().create {
            return Err(format!("new agents can't be started on {}", m.label));
        }
        let mut form = p.create_form(&m.id)?;
        form.provider = p.id().to_string();
        form.machine = m.id.to_string();
        form.machine_label = m.label;
        Ok(form)
    }

    /// Every provider with its machines and what can be done there
    /// (`machine.list`, the New-agent dialog's "Runs on"). Blocking.
    pub fn machine_list(&self) -> Vec<ProviderMachines> {
        self.providers
            .all()
            .iter()
            .map(|p| {
                let caps = p.caps();
                let (machines, error) = match self.providers.machines(p) {
                    Ok(ms) => (
                        ms.into_iter().map(|m| MachineEntry { id: m.id.to_string(), label: m.label, detail: m.detail }).collect(),
                        None,
                    ),
                    Err(e) => (vec![], Some(e.to_string())),
                };
                ProviderMachines {
                    provider: p.id().to_string(),
                    label: p.label(),
                    version: p.version(),
                    can_create: caps.create,
                    can_add_sessions: caps.attach_existing,
                    machines,
                    error,
                }
            })
            .collect()
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

    /// Agent `id`'s terminal, while attached.
    pub(crate) fn host(&self, id: &str) -> Result<Arc<TermHost>, String> {
        self.with(id, |a| a.host.clone())?
            .ok_or_else(|| "agent is not running".to_string())
    }

    /// Start using `io` as agent `id`'s terminal at `size`.
    pub(crate) fn host_for(&self, io: Box<dyn crate::provider::TermIo>, size: TermSize) -> Arc<TermHost> {
        TermHost::new(io, size, self.clock.clone())
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

/// The `Exec` of a machine that can't be reached: every call fails with why.
struct NoExec(PwError);

impl Exec for NoExec {
    fn run(&self, _cmd: &Cmd) -> PwResult<Out> {
        Err(self.0.clone())
    }
    fn read_file(&self, _path: &str, _max: u64) -> PwResult<Option<Vec<u8>>> {
        Err(self.0.clone())
    }
    fn write_file(&self, _path: &str, _bytes: &[u8]) -> PwResult<()> {
        Err(self.0.clone())
    }
    fn remove_file(&self, _path: &str) -> PwResult<()> {
        Err(self.0.clone())
    }
    fn remove_dir(&self, _path: &str) -> PwResult<()> {
        Err(self.0.clone())
    }
    fn copy_file(&self, _from: &str, _to: &str) -> PwResult<()> {
        Err(self.0.clone())
    }
    fn stat(&self, _path: &str) -> PwResult<Option<Stat>> {
        Err(self.0.clone())
    }
    fn real_path(&self, _path: &str) -> PwResult<String> {
        Err(self.0.clone())
    }
    fn temp_dir(&self) -> PwResult<String> {
        Err(self.0.clone())
    }
    fn home(&self) -> PwResult<String> {
        Err(self.0.clone())
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
        assert!(h.engine.host("a").is_err());
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
        let mut rec = record("a", "/tmp");
        rec.kind = "claude".into(); // a terminal's hooks name the agent inside it (terminals.rs)
        let h = Harness::new(vec![rec]);
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
