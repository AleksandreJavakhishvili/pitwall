//! Shapes that cross the host boundary or go to disk. See docs/CONTRACT.md.
//! (Step 5 moves the wire shapes into pitwall-proto.)

use serde::{Deserialize, Serialize};

use crate::kind::{AgentKind, HookMode};
use crate::provider::{HookTransport, Locator, ProviderCaps};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Working,
    Blocked,
    Idle,
    Done,
    Unknown,
    Exited,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Hooks,
    Screen,
    Activity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KindView {
    pub id: String,
    pub name: String,
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The agent can work in its own git worktree (= `caps.worktree`; kept
    /// for older UIs).
    pub worktree: bool,
    pub caps: KindCaps,
}

/// What a kind can do on a machine: its definition (architecture.md §2.6)
/// combined with the provider's capabilities (§3). Never hard-coded per kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KindCaps {
    /// Its own worktree flag, and the provider can find where it went.
    pub worktree: bool,
    /// Resume args, and the provider can resume.
    pub resume: bool,
    /// A rulesync target, and the provider can write rules where it works.
    pub rules: bool,
    /// Hooks, and the provider can deliver them.
    pub hooks: bool,
    /// Runs the command the user types (the "Custom command" kind).
    pub custom_command: bool,
}

impl KindCaps {
    pub fn of(kind: &AgentKind, p: &ProviderCaps) -> KindCaps {
        KindCaps {
            worktree: !kind.worktree_args.is_empty() && p.exec && p.process_cwd,
            resume: !kind.resume_args.is_empty() && p.resume,
            rules: kind.rulesync_target.is_some() && p.rules,
            hooks: kind.hooks != HookMode::None && p.hooks != HookTransport::None,
            custom_command: kind.id == crate::kind::CUSTOM && p.custom_command,
        }
    }
}

/// What the UI may offer for one agent right now (architecture.md §3). The
/// UI reads only this; it never asks which provider or kind an agent has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCaps {
    /// Running and attached: keystrokes and prompts reach it.
    pub input: bool,
    /// Can be started again (its provider's `start`).
    pub restart: bool,
    /// A restart continues its conversation (provider and kind can resume,
    /// and it has one).
    pub resume: bool,
    pub stop: bool,
    /// Works in its own worktree, which can be removed with it.
    pub remove_worktree: bool,
    /// Its changes can be read: the provider runs commands there and it
    /// works in a git repository (not known yet counts as yes).
    pub diff: bool,
    pub review: bool,
    /// Commit & merge back: a worktree on a branch.
    pub merge: bool,
    pub rules: bool,
    pub hooks: bool,
    /// Pitwall adopted a session that was already there: removing it from
    /// Pitwall only stops tracking it, the session keeps running.
    pub remove_keeps_session: bool,
}

/// Where an agent runs, for display and grouping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineView {
    pub provider: String,
    pub id: String,
    pub label: String,
    /// New agents and terminals can be started there ("New agent here").
    pub can_create: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentRequest {
    pub name: String,
    pub kind: String,
    pub project_path: String,
    #[serde(default)]
    pub custom_command: Option<String>,
    /// Ask the agent to work in its own worktree (its native flag).
    #[serde(default)]
    pub worktree: bool,
    /// Continue this existing conversation (kind's resume args) instead of starting fresh.
    #[serde(default)]
    pub resume_session_id: Option<String>,
    /// Extra rule set for this agent (on top of the project default).
    #[serde(default)]
    pub rule_set_id: Option<String>,
    /// Write rule files into the main checkout (agents without a worktree).
    #[serde(default)]
    pub apply_to_main_checkout: bool,
    /// Show the agent under this project (sidebar grouping) while it still
    /// runs in `project_path` — e.g. a conversation started in `~`.
    #[serde(default)]
    pub display_project: Option<String>,
    /// Terminal size the UI will show the agent at; the process starts at it.
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
}

/// (cols, rows) when both are given.
pub fn size_of(cols: Option<u16>, rows: Option<u16>) -> Option<(u16, u16)> {
    Some(crate::term::clamp_size(cols?, rows?))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    /// Main repository the worktree belongs to.
    pub repo: String,
    /// The worktree's root folder.
    pub path: String,
    /// Branch when it was found; the live one (`git branch --show-current`)
    /// wins wherever it matters. Empty = detached.
    #[serde(default)]
    pub branch: String,
}

/// Everything about an agent that survives an app restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRecord {
    pub id: String,
    /// Where it lives (provider, machine, native handle). Missing in v1
    /// state files: those agents are all `local:this-mac/<id>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<Locator>,
    pub name: String,
    pub kind: String,
    pub kind_name: String,
    #[serde(default)]
    pub custom_command: Option<String>,
    /// Where the agent runs (and resumes its conversation).
    pub cwd: String,
    /// What it's grouped under; usually the repo of `cwd`, but may be a
    /// project the user picked for a conversation started elsewhere.
    pub project: String,
    #[serde(default)]
    pub worktree: Option<WorktreeInfo>,
    /// The agent was asked to make its own worktree, which hasn't been found
    /// yet (see worktree.rs). `cwd` is still the folder it was launched in.
    #[serde(default)]
    pub worktree_pending: bool,
    #[serde(default)]
    pub base_commit: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    /// A prompt has been sent in this session, so resuming it is meaningful.
    #[serde(default)]
    pub has_conversation: bool,
    #[serde(default)]
    pub queue: Vec<QueueItem>,
    #[serde(default = "yes")]
    pub auto_send: bool,
    #[serde(default)]
    pub last_sent: Option<String>,
    #[serde(default)]
    pub last_sent_at: Option<u64>,
    pub created_at: u64,
    /// Per-task snapshots for the review screen (tasks.rs).
    #[serde(default)]
    pub tasks: Vec<crate::engine::tasks::Task>,
    /// Last known terminal size; restarts and resumes start the process at it.
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
    /// The session existed before Pitwall knew it (adopted from its
    /// provider's `discover`): Pitwall never deletes it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub adopted: bool,
    /// Terminals: the agent the user last started by hand in it, kept after
    /// it exits so a restart continues it inside the shell
    /// (docs/spec/terminals.md §3). Missing in older state files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_agent: Option<InnerAgent>,
}

/// An agent started by hand in a terminal, as remembered on its record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InnerAgent {
    pub kind: String,
    pub kind_name: String,
    /// Its conversation, once known (hooks, arguments or newest transcript).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Its process while it runs (after Pitwall restarts, the same process
    /// keeps its conversation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Unix ms when it exited; `None` while it runs (or is being restarted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_at: Option<u64>,
}

impl AgentRecord {
    /// Where it lives (`local:this-mac/<id>` for records from before providers).
    pub fn locator(&self) -> Locator {
        self.locator.clone().unwrap_or_else(|| Locator::local(&self.id))
    }

    pub fn term_size(&self) -> Option<(u16, u16)> {
        size_of(self.cols, self.rows)
    }

    /// Returns whether it changed.
    pub fn set_term_size(&mut self, (cols, rows): (u16, u16)) -> bool {
        let (cols, rows) = crate::term::clamp_size(cols, rows);
        let changed = self.term_size() != Some((cols, rows));
        self.cols = Some(cols);
        self.rows = Some(rows);
        changed
    }
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    pub id: String,
    pub name: String,
    /// What it is now: for a terminal running an agent the user started in
    /// it, that agent's kind (docs/spec/terminals.md).
    pub kind: String,
    pub kind_name: String,
    /// A plain terminal (kind `shell` when nothing else runs in it).
    pub terminal: bool,
    /// The conversation it is in, when known (a terminal: its agent's).
    pub session_id: Option<String>,
    pub cwd: String,
    pub cwd_display: String,
    pub project: String,
    pub project_display: String,
    pub branch: Option<String>,
    pub worktree: bool,
    /// Asked for its own worktree; waiting to see where the agent made it.
    pub worktree_pending: bool,
    /// The provider it runs on ("local"); display only.
    pub location: String,
    pub machine: MachineView,
    /// A terminal running an agent the user started in it by hand.
    pub agent_in_terminal: bool,
    /// A terminal that restarts as the agent last started in it (its kind
    /// name): the shell starts and continues that agent inside it.
    pub restart_as: Option<String>,
    pub status: Status,
    pub status_source: Source,
    pub status_detail: Option<String>,
    pub running: bool,
    /// Current PTY size.
    pub cols: u16,
    pub rows: u16,
    pub added: u32,
    pub removed: u32,
    pub files_changed: u32,
    pub queue: Vec<QueueItem>,
    pub auto_send: bool,
    pub last_sent: Option<String>,
    pub last_sent_at: Option<u64>,
    pub created_at: u64,
    pub current_task_id: Option<String>,
    pub caps: AgentCaps,
}

/// "Add to Pitwall" on a session found on another machine (`discover`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptSessionRequest {
    pub provider: String,
    pub machine: String,
    /// The provider's own handle (an agw session name).
    pub native: String,
    /// Terminal size the UI will show it at.
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attention {
    pub agent_id: String,
    pub name: String,
    pub reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexHooksStatus {
    pub installed: bool,
    pub path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_remember_the_terminal_size() {
        let mut rec: AgentRecord = serde_json::from_value(serde_json::json!({
            "id": "a", "name": "a", "kind": "shell", "kindName": "Shell",
            "cwd": "/", "project": "/", "createdAt": 1
        }))
        .unwrap();
        assert_eq!(rec.term_size(), None); // older state files have no size
        assert!(rec.set_term_size((90, 30)));
        assert!(!rec.set_term_size((90, 30)));
        let back: AgentRecord = serde_json::from_value(serde_json::to_value(&rec).unwrap()).unwrap();
        assert_eq!(back.term_size(), Some((90, 30)));
        assert!(rec.set_term_size((0, 9999)));
        assert_eq!(rec.term_size(), Some((2, 1000)));
        assert_eq!(size_of(Some(80), None), None);
    }

    #[test]
    fn the_remembered_agent_of_a_terminal_is_optional_on_disk() {
        let old = serde_json::json!({
            "id": "t", "name": "t", "kind": "shell", "kindName": "Shell",
            "cwd": "/", "project": "/", "createdAt": 1
        });
        let mut rec: AgentRecord = serde_json::from_value(old).unwrap();
        assert_eq!(rec.inner_agent, None, "older state files have none");
        assert!(serde_json::to_value(&rec).unwrap().get("innerAgent").is_none(), "not written when absent");
        rec.inner_agent = Some(InnerAgent {
            kind: "claude".into(),
            kind_name: "Claude Code".into(),
            session_id: Some("s1".into()),
            pid: None,
            left_at: Some(5),
        });
        let v = serde_json::to_value(&rec).unwrap();
        assert_eq!(v["innerAgent"], serde_json::json!({ "kind": "claude", "kindName": "Claude Code", "sessionId": "s1", "leftAt": 5 }));
        let back: AgentRecord = serde_json::from_value(v).unwrap();
        assert_eq!(back.inner_agent, rec.inner_agent);
        let bare: InnerAgent = serde_json::from_value(serde_json::json!({ "kind": "codex", "kindName": "Codex" })).unwrap();
        assert_eq!((bare.session_id, bare.pid, bare.left_at), (None, None, None));
    }
}
