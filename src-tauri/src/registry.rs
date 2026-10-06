//! The set of agents and their live state. The registry lock is only ever held
//! for in-memory bookkeeping — never across PTY writes, git, or disk I/O.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use tauri::AppHandle;

use crate::hooks::{HookEffect, HookState};
use crate::model::{mono_ms, AgentRecord, AgentView, QueueItem, Source, Status};
use crate::paths;
use crate::session::Session;

pub struct Agent {
    pub rec: AgentRecord,
    pub session: Option<Arc<Session>>,
    pub status: Status,
    pub source: Source,
    pub detail: Option<String>,
    /// mono ms when `status` last changed.
    pub status_since: u64,
    pub turn_active: bool,
    /// Latest hook-reported state in this run (`Some` ⇒ hooks are authoritative).
    pub hook: Option<(HookState, Option<String>)>,
    pub screen_state: Option<pitwall_detect::Detected>,
    pub screen_detail: Option<String>,
    pub detect_seq: u64,
    pub branch: Option<String>,
    pub added: u32,
    pub removed: u32,
    pub files_changed: u32,
    pub git_at: u64,
    pub git_seq: u64,
    pub git_inflight: bool,
    pub git_wanted: bool,
    pub last_auto_send: u64,
    /// Looking for the worktree the agent makes itself (worktree.rs).
    pub watch: Option<crate::worktree::Watch>,
}

impl Agent {
    pub fn new(rec: AgentRecord, session: Option<Arc<Session>>) -> Self {
        let status = if session.is_some() { Status::Unknown } else { Status::Stopped };
        Agent {
            rec,
            session,
            status,
            source: Source::Activity,
            detail: None,
            status_since: mono_ms(),
            turn_active: false,
            hook: None,
            screen_state: None,
            screen_detail: None,
            detect_seq: u64::MAX,
            branch: None,
            added: 0,
            removed: 0,
            files_changed: 0,
            git_at: 0,
            git_seq: 0,
            git_inflight: false,
            git_wanted: true,
            last_auto_send: 0,
            watch: None,
        }
    }

    /// Swap in a freshly spawned process and reset per-run state.
    pub fn attach_session(&mut self, session: Arc<Session>) {
        if let Some(old) = self.session.replace(session) {
            old.hangup();
        }
        self.status = Status::Unknown;
        self.source = Source::Activity;
        self.detail = None;
        self.status_since = mono_ms();
        self.turn_active = false;
        self.hook = None;
        self.screen_state = None;
        self.screen_detail = None;
        self.detect_seq = u64::MAX;
        self.git_wanted = true;
    }

    pub fn running(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| !s.exited.load(std::sync::atomic::Ordering::Relaxed))
    }

    pub fn view(&self) -> AgentView {
        let r = &self.rec;
        let (cols, rows) = self
            .session
            .as_ref()
            .map(|s| s.size())
            .or_else(|| r.term_size())
            .unwrap_or((crate::session::DEFAULT_COLS, crate::session::DEFAULT_ROWS));
        AgentView {
            id: r.id.clone(),
            name: r.name.clone(),
            kind: r.kind.clone(),
            kind_name: r.kind_name.clone(),
            cwd: r.cwd.clone(),
            cwd_display: paths::tildify(&r.cwd),
            project: r.project.clone(),
            project_display: paths::tildify(&r.project),
            branch: self.branch.clone(),
            worktree: r.worktree.is_some(),
            worktree_pending: r.worktree_pending,
            location: "local",
            status: self.status,
            status_source: self.source,
            status_detail: self.detail.clone(),
            running: self.running(),
            cols,
            rows,
            added: self.added,
            removed: self.removed,
            files_changed: self.files_changed,
            queue: r.queue.clone(),
            auto_send: r.auto_send,
            last_sent: r.last_sent.clone(),
            last_sent_at: r.last_sent_at,
            created_at: r.created_at,
            current_task_id: crate::tasks::current_task_id(r),
        }
    }

    pub fn apply_hook(&mut self, effect: HookEffect) -> bool {
        let mut persist = false;
        if let Some(id) = effect.session_id {
            if self.rec.session_id.as_deref() != Some(id.as_str()) {
                self.rec.session_id = Some(id);
                persist = true;
            }
        }
        if effect.prompt_submitted && !self.rec.has_conversation {
            self.rec.has_conversation = true;
            persist = true;
        }
        if let Some(state) = effect.state {
            if !(effect.session_start && self.hook.is_some()) {
                self.hook = Some((state, effect.detail));
            }
        }
        persist
    }

    pub fn mark_seen(&mut self) {
        if self.status == Status::Done {
            self.status = Status::Idle;
            self.status_since = mono_ms();
        }
        self.turn_active = false;
        if let Some((HookState::Done, _)) = self.hook {
            self.hook = Some((HookState::Idle, None));
        }
    }
}

#[derive(Default)]
struct Dirty {
    view: bool,
    persist: bool,
}

pub struct Core {
    pub app: AppHandle,
    agents: Mutex<Vec<Agent>>,
    dirty: Mutex<Dirty>,
    wake: Condvar,
}

pub type Shared = Arc<Core>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Core {
    pub fn new(app: AppHandle, agents: Vec<Agent>) -> Shared {
        Arc::new(Core {
            app,
            agents: Mutex::new(agents),
            dirty: Mutex::new(Dirty::default()),
            wake: Condvar::new(),
        })
    }

    pub fn agents(&self) -> MutexGuard<'_, Vec<Agent>> {
        lock(&self.agents)
    }

    /// Run `f` on one agent under the lock.
    pub fn with<T>(&self, id: &str, f: impl FnOnce(&mut Agent) -> T) -> Result<T, String> {
        let mut agents = self.agents();
        let agent = agents
            .iter_mut()
            .find(|a| a.rec.id == id)
            .ok_or_else(|| format!("no agent {id}"))?;
        Ok(f(agent))
    }

    pub fn session(&self, id: &str) -> Result<Arc<Session>, String> {
        self.with(id, |a| a.session.clone())?
            .ok_or_else(|| "agent is not running".to_string())
    }

    pub fn views(&self) -> Vec<AgentView> {
        self.agents().iter().map(Agent::view).collect()
    }

    pub fn records(&self) -> Vec<AgentRecord> {
        self.agents().iter().map(|a| a.rec.clone()).collect()
    }

    pub fn changed(&self, persist: bool) {
        let mut d = lock(&self.dirty);
        d.view = true;
        d.persist |= persist;
        self.wake.notify_one();
    }

    /// Block until something is dirty; returns (view, persist).
    pub fn wait_dirty(&self) -> (bool, bool) {
        let mut d = lock(&self.dirty);
        while !d.view && !d.persist {
            d = self.wake.wait(d).unwrap_or_else(|e| e.into_inner());
        }
        let out = (d.view, d.persist);
        *d = Dirty::default();
        out
    }

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
        self.with(id, Agent::mark_seen)?;
        self.changed(false);
        Ok(())
    }

    pub fn apply_hook(&self, id: &str, effect: HookEffect) {
        if let Ok(persist) = self.with(id, |a| a.apply_hook(effect)) {
            self.changed(persist);
        }
    }

    /// Record a prompt as sent (verbatim) and return the session to write to.
    pub fn record_sent(&self, id: &str, text: &str) -> Result<Arc<Session>, String> {
        let session = self.with(id, |a| {
            let s = a.session.clone().filter(|_| a.running());
            if s.is_some() {
                note_sent(a, text);
            }
            s
        })?;
        let session = session.ok_or("agent is not running")?;
        self.changed(true);
        Ok(session)
    }

    /// Take a queue item out and record it as sent.
    pub fn take_queued(&self, id: &str, item_id: &str) -> Result<(Arc<Session>, String, AgentView), String> {
        let out = self.with(id, |a| {
            let session = a.session.clone().filter(|_| a.running()).ok_or("agent is not running")?;
            let pos = a
                .rec
                .queue
                .iter()
                .position(|q| q.id == item_id)
                .ok_or("queue item not found")?;
            let item = a.rec.queue.remove(pos);
            note_sent(a, &item.text);
            Ok::<_, &str>((session, item.text, a.view()))
        })?;
        let out = out.map_err(String::from)?;
        self.changed(true);
        Ok(out)
    }
}

pub fn note_sent(a: &mut Agent, text: &str) {
    a.rec.last_sent = Some(text.to_string());
    a.rec.last_sent_at = Some(crate::model::unix_ms());
    a.rec.has_conversation = true;
    a.last_auto_send = mono_ms();
}
