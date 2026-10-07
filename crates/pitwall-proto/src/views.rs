//! What clients are shown about agents and the places they run. The engine
//! (`pitwall-core`) builds these; the app, the CLI and the UI read them.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
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

/// Which input decided an agent's status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Hooks,
    Screen,
    Activity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub id: String,
    pub text: String,
}

/// What the UI may offer for one agent right now (architecture.md §3). The
/// UI reads only this; it never asks which provider or kind an agent has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MachineView {
    pub provider: String,
    pub id: String,
    pub label: String,
    /// New agents and terminals can be started in its folders ("New agent
    /// here", "Open terminal here"). False on a platform's machine (agw),
    /// where new sessions come from its own form (New agent → Runs on).
    pub can_create: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
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
    #[ts(type = "number | null")]
    pub last_sent_at: Option<u64>,
    #[ts(type = "number")]
    pub created_at: u64,
    pub current_task_id: Option<String>,
    pub caps: AgentCaps,
}

/// One provider's machines and the sessions there that can be added to
/// Pitwall (onboarding's "Running now", `pitwall session list`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScannedPlace {
    pub provider: String,
    /// How it is shown ("agw").
    pub label: String,
    pub version: Option<String>,
    /// `None` when its machines or sessions couldn't be listed.
    pub machines: Option<Vec<ScannedMachine>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScannedMachine {
    pub id: String,
    pub label: String,
    /// Where it lives (an agw vm-site), if the provider says.
    pub detail: Option<String>,
    /// Running sessions first, then by name.
    pub sessions: Vec<ScannedSession>,
}

/// A session found on a machine; `provider` + `machine` + `native` is what
/// "Add to Pitwall" (`adopt_session`, `session.add`) takes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScannedSession {
    pub provider: String,
    pub machine: String,
    /// The provider's own handle (an agw session name).
    pub native: String,
    pub name: String,
    /// Pitwall kind id it was matched to (id or alias), else the platform's name.
    pub kind: String,
    pub kind_name: String,
    /// The platform's own name for what runs there ("claude-code").
    pub program: String,
    pub workspace: Option<String>,
    /// The user it runs as when that isn't the machine's main user.
    pub user: Option<String>,
    pub cwd: Option<String>,
    /// "running" | "stopped" | "unknown"
    #[ts(type = "\"running\" | \"stopped\" | \"unknown\"")]
    pub status: String,
    /// A Pitwall agent already tracks this session.
    pub in_pitwall: bool,
}
