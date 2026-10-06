//! One agent: its stored record plus what is only known while Pitwall runs.

use std::sync::Arc;

use crate::hooks::{HookEffect, HookState};
use crate::model::{AgentRecord, AgentView, Source, Status};
use crate::paths;
use crate::session::{Session, DEFAULT_COLS, DEFAULT_ROWS};

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
    pub watch: Option<super::worktree::Watch>,
}

impl Agent {
    /// `now`: the engine clock's mono ms.
    pub fn new(rec: AgentRecord, session: Option<Arc<Session>>, now: u64) -> Self {
        let status = if session.is_some() { Status::Unknown } else { Status::Stopped };
        Agent {
            rec,
            session,
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
            last_auto_send: 0,
            watch: None,
        }
    }

    /// Swap in a freshly spawned process and reset per-run state.
    pub fn attach_session(&mut self, session: Arc<Session>, now: u64) {
        if let Some(old) = self.session.replace(session) {
            old.hangup();
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
            .unwrap_or((DEFAULT_COLS, DEFAULT_ROWS));
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
            current_task_id: super::tasks::current_task_id(r),
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
        let mut a = Agent::new(record("a", "/tmp"), None, 5);
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
