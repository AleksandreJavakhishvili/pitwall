//! One agent: its stored record plus what is only known while Pitwall runs.

use std::sync::Arc;

use crate::hooks::{HookEffect, HookState};
use crate::model::{AgentCaps, AgentRecord, AgentView, KindCaps, MachineView, Source, Status};
use crate::paths;
use crate::provider::ProviderCaps;
use crate::term::{TermHost, DEFAULT_COLS, DEFAULT_ROWS};

/// What the engine knows about where an agent runs and what its kind can do
/// (looked up when it is loaded or started; feeds [`AgentCaps`]).
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub provider: ProviderCaps,
    /// The record's kind on its provider.
    pub kind: KindCaps,
    pub machine_label: String,
    /// The user's home on the agent's machine ("~/…" display).
    pub home: Option<String>,
    /// Terminals: what the agent remembered on the record
    /// (`AgentRecord::inner_agent`) can do on this provider; `None` when
    /// there is none or its kind is unknown.
    pub inner: Option<KindCaps>,
}

pub struct Agent {
    pub rec: AgentRecord,
    /// Its terminal while attached (from its provider).
    pub host: Option<Arc<TermHost>>,
    pub facts: Facts,
    /// `cwd` is (`Some(true)`) or isn't (`Some(false)`) in a git repository;
    /// `None` until git was asked. Not a repo: no diffs, no git polling.
    pub git_repo: Option<bool>,
    /// A dropped attachment (`!eof_is_exit`) is being re-attached.
    pub relinking: bool,
    /// Waiting to be attached to its session again (saved agents at
    /// startup, unreachable machines; connect.rs). No terminal meanwhile.
    pub connect: Option<super::connect::Connect>,
    /// `facts` are only what was known without asking the machine; the
    /// rest is being looked up (connect.rs).
    pub facts_pending: bool,
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
    /// Current git polling interval (ms): starts at the provider's, doubles
    /// while refreshes find nothing new, back to the start on a change
    /// (ticker.rs `git_due`). 0 = not backed off yet.
    pub git_every: u64,
    /// Its checkout's file watch, where the provider can (gitwatch.rs).
    pub fs: super::gitwatch::FsWatch,
    pub last_auto_send: u64,
    /// Looking for the worktree the agent makes itself (worktree.rs).
    pub watch: Option<super::worktree::Watch>,
    /// Terminals: the agent the user started in it, while it runs (terminals.rs).
    pub inner: Option<super::terminals::Inner>,
    /// Terminals: when to look at the foreground process next.
    pub fg: super::terminals::Watch,
    /// Terminals: a conversation id a hook reported (possibly before the
    /// poller noticed the agent).
    pub hook_session: Option<String>,
    /// Terminals: Enter was pressed in the shell after the remembered agent
    /// exited (the user went on using the shell; terminals.rs `FORGET_AFTER_MS`).
    pub shell_used: bool,
    /// `rec.cwd` resolved on its machine (symlinks, letter case), for
    /// matching worktree paths; (cwd it was resolved from, result).
    pub real_cwd: Option<(String, String)>,
}

impl Agent {
    /// `now`: the engine clock's mono ms.
    pub fn new(rec: AgentRecord, host: Option<Arc<TermHost>>, now: u64) -> Self {
        let status = if host.is_some() { Status::Unknown } else { Status::Stopped };
        Agent {
            rec,
            host,
            facts: Facts::default(),
            git_repo: None,
            relinking: false,
            connect: None,
            facts_pending: false,
            status,
            source: Source::Activity,
            detail: None,
            status_since: now,
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
            git_every: 0,
            fs: Default::default(),
            last_auto_send: 0,
            watch: None,
            inner: None,
            fg: Default::default(),
            hook_session: None,
            shell_used: false,
            real_cwd: None,
        }
    }

    /// Swap in a freshly started agent's terminal and reset per-run state.
    /// An old terminal still open is closed (in the background).
    pub fn attach_host(&mut self, host: Arc<TermHost>, now: u64) {
        if let Some(old) = self.host.replace(host) {
            if !old.ended() {
                std::thread::spawn(move || old.close(super::lifecycle::STOP_GRACE));
            }
        }
        self.status = Status::Unknown;
        self.source = Source::Activity;
        self.detail = None;
        self.status_since = now;
        self.turn_active = false;
        self.hook = None;
        self.screen_state = None;
        self.screen_detail = None;
        self.detect_seq = u64::MAX;
        self.git_wanted = true;
        self.git_every = 0;
        self.git_repo = None;
        self.relinking = false;
        self.connect = None;
        self.inner = None;
        self.fg = Default::default();
        self.hook_session = None;
        self.shell_used = false;
    }

    /// A plain terminal (it may be running an agent the user started in it).
    pub fn is_terminal(&self) -> bool {
        self.rec.kind == super::terminals::TERMINAL_KIND
    }

    /// The kind it is now: the agent running in a terminal, else its own.
    pub fn kind(&self) -> &str {
        self.inner.as_ref().map_or(&self.rec.kind, |i| &i.kind)
    }

    /// Terminals: the agent a restart continues inside the shell (the one
    /// last started in it), when its provider runs command lines and its
    /// kind is known there.
    pub fn restart_inner(&self) -> Option<(&crate::model::InnerAgent, KindCaps)> {
        let caps = self.facts.inner.filter(|_| self.is_terminal() && self.facts.provider.custom_command)?;
        Some((self.rec.inner_agent.as_ref()?, caps))
    }

    pub fn running(&self) -> bool {
        self.host.as_ref().is_some_and(|h| !h.ended())
    }

    /// The agent's local process, when its provider runs agents on this Mac.
    pub fn local_pid(&self) -> Option<u32> {
        self.host.as_ref().filter(|_| self.facts.provider.local_process).and_then(|h| h.pid())
    }

    /// What the UI may offer for it now (architecture.md §3).
    pub fn caps(&self) -> AgentCaps {
        let (p, k, r) = (&self.facts.provider, &self.facts.kind, &self.rec);
        let running = self.running();
        let diff = p.exec && self.git_repo != Some(false);
        AgentCaps {
            input: running,
            // Not while it may still attach to the session it has.
            restart: p.start && !self.connect.as_ref().is_some_and(|c| c.busy()),
            resume: match self.restart_inner() {
                Some((inner, caps)) => caps.resume && inner.session_id.is_some(),
                None => k.resume && r.has_conversation && r.session_id.is_some(),
            },
            stop: running,
            remove_worktree: p.exec && r.worktree.is_some(),
            diff,
            review: diff,
            merge: diff && r.worktree.is_some() && self.branch.is_some(),
            rules: k.rules,
            hooks: k.hooks,
            worktrees: diff,
            explorer: p.exec,
            remove_keeps_session: r.adopted,
        }
    }

    pub fn view(&self) -> AgentView {
        let r = &self.rec;
        let loc = r.locator();
        let home = self.facts.home.as_deref();
        let (cols, rows) = self
            .host
            .as_ref()
            .map(|s| s.size())
            .or_else(|| r.term_size())
            .unwrap_or((DEFAULT_COLS, DEFAULT_ROWS));
        AgentView {
            id: r.id.clone(),
            name: r.name.clone(),
            kind: self.kind().to_string(),
            kind_name: self.inner.as_ref().map_or(&r.kind_name, |i| &i.kind_name).clone(),
            terminal: self.is_terminal(),
            session_id: match &self.inner {
                Some(i) => i.session_id.clone(),
                None if self.is_terminal() => None,
                None => r.session_id.clone(),
            },
            cwd: r.cwd.clone(),
            cwd_display: paths::tildify_in(home, &r.cwd),
            project: r.project.clone(),
            project_display: paths::tildify_in(home, &r.project),
            branch: self.branch.clone(),
            worktree: r.worktree.is_some(),
            worktree_pending: r.worktree_pending,
            location: loc.provider.to_string(),
            machine: MachineView {
                provider: loc.provider.to_string(),
                id: loc.machine.to_string(),
                label: if self.facts.machine_label.is_empty() { loc.machine.to_string() } else { self.facts.machine_label.clone() },
                can_create: self.facts.provider.create && !self.facts.provider.platform_create,
            },
            agent_in_terminal: self.is_terminal() && self.inner.is_some(),
            restart_as: self.restart_inner().map(|(i, _)| i.kind_name.clone()),
            engineer: r.engineer,
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
            current_task_id: super::tasks::current_task_id(r),
            caps: self.caps(),
        }
    }

    pub fn apply_hook(&mut self, effect: HookEffect) -> bool {
        let mut persist = false;
        if let (Some(id), true) = (&effect.session_id, self.is_terminal()) {
            // A terminal has no conversation of its own; the agent in it does.
            self.hook_session = Some(id.clone());
            if let Some(inner) = &mut self.inner {
                inner.session_id = Some(id.clone());
                persist |= super::terminals::remember_session(&mut self.rec, inner.pid, id);
            }
        } else if let Some(id) = effect.session_id {
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

    pub fn mark_seen(&mut self, now: u64) {
        if self.status == Status::Done {
            self.status = Status::Idle;
            self.status_since = now;
        }
        self.turn_active = false;
        if let Some((HookState::Done, _)) = self.hook {
            self.hook = Some((HookState::Idle, None));
        }
    }

    /// Record a prompt as sent (verbatim) at `now` (mono ms).
    pub fn note_sent(&mut self, text: &str, now: u64) {
        self.rec.last_sent = Some(text.to_string());
        self.rec.last_sent_at = Some(crate::clock::unix_ms());
        self.rec.has_conversation = true;
        self.last_auto_send = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::record;

    #[test]
    fn hooks_set_session_and_conversation() {
        let mut rec = record("a", "/tmp");
        rec.kind = "claude".into();
        let mut a = Agent::new(rec, None, 5);
        assert_eq!((a.status, a.status_since), (Status::Stopped, 5));
        let persist = a.apply_hook(HookEffect {
            session_id: Some("s1".into()),
            state: Some(HookState::Working),
            prompt_submitted: true,
            ..Default::default()
        });
        assert!(persist);
        assert_eq!(a.rec.session_id.as_deref(), Some("s1"));
        assert!(a.rec.has_conversation);
        // SessionStart never overrides a known hook state.
        a.apply_hook(HookEffect { state: Some(HookState::Idle), session_start: true, ..Default::default() });
        assert_eq!(a.hook.as_ref().map(|h| h.0), Some(HookState::Working));
    }

    #[test]
    fn seen_done_turns_idle() {
        let mut a = Agent::new(record("a", "/tmp"), None, 1);
        a.status = Status::Done;
        a.hook = Some((HookState::Done, None));
        a.mark_seen(42);
        assert_eq!((a.status, a.status_since), (Status::Idle, 42));
        assert_eq!(a.hook, Some((HookState::Idle, None)));
    }
}
