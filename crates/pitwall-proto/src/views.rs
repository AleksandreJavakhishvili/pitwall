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
    /// Its project's worktrees can be listed (`list_worktrees`): the
    /// provider runs commands there and it works in a git repository.
    /// Missing from older servers: no.
    #[serde(default)]
    pub worktrees: bool,
    /// Its folder can be browsed, read and searched (read-only code
    /// explorer, docs/spec/explorer.md): the provider runs commands there.
    /// Missing from older servers: no.
    #[serde(default)]
    pub explorer: bool,
    /// Its files are on this computer: "Open in editor" can hand one to the
    /// user's editor (otherwise only "Copy path"). Missing: no.
    #[serde(default)]
    pub open_in_editor: bool,
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

/// How a worktree was matched to an agent (docs/spec/worktrees-view.md), in
/// the order the rules are tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum WorktreeVia {
    /// It is the agent's own working folder.
    Own,
    /// It lives under the agent's folder in a location its tool manages
    /// (`worktree_dirs` of the agent definition, e.g. `.claude/worktrees`).
    ToolDir,
    /// A process in the agent's terminal works in it.
    Process,
    /// No agent: one of the project's other worktrees.
    Other,
}

/// What may be done with one worktree from Pitwall. The UI reads only this.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeCaps {
    /// Its changes can be read (its folder exists).
    pub diff: bool,
    /// `git add -A && git commit` there.
    pub commit: bool,
    /// Merge its branch into the project's current branch (it is on a
    /// branch, and the main checkout is on another one).
    pub merge: bool,
    /// `git worktree remove` (never `--force`): not locked, not an agent's
    /// own folder (remove the agent instead).
    pub remove: bool,
    /// "Open a terminal there" (Pitwall can start terminals on its machine).
    pub terminal: bool,
}

/// One linked worktree of a project (the main checkout is the project itself).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeView {
    pub path: String,
    pub path_display: String,
    /// Its folder's name.
    pub name: String,
    /// `None` when detached.
    pub branch: Option<String>,
    pub head: Option<String>,
    pub locked: bool,
    pub lock_reason: Option<String>,
    /// Its folder is gone (git would prune it).
    pub prunable: bool,
    /// The agent it belongs to, if any.
    pub agent_id: Option<String>,
    pub via: WorktreeVia,
    pub caps: WorktreeCaps,
}

/// Every worktree of one project (git repository) on one machine
/// (`list_worktrees`). Per-worktree changes are fetched separately, only
/// for the ones shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectWorktrees {
    /// Opaque; what the per-worktree commands take.
    pub id: String,
    /// The main checkout.
    pub repo: String,
    pub repo_display: String,
    /// The main checkout's current branch (merge target); `None` when detached.
    pub branch: Option<String>,
    pub machine: MachineView,
    /// Agents working in this repository.
    pub agent_ids: Vec<String>,
    pub worktrees: Vec<WorktreeView>,
    /// The list couldn't be read (the last good one is kept in `worktrees`).
    pub error: Option<String>,
}

/// One update of an agent's screen as styled text, for drawing it without a
/// terminal emulator (the Wall's view-only tiles, docs/spec/wall.md). The
/// engine sends one when the screen changed, at most ~10 per second per
/// watcher, and only while someone watches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScreenFrame {
    pub cols: u16,
    pub rows: u16,
    /// `[col, row]` while the program shows its cursor.
    pub cursor: Option<(u16, u16)>,
    /// `lines` holds every row (first frame, or the size changed); otherwise
    /// only the rows that changed since the previous frame.
    pub full: bool,
    pub lines: Vec<ScreenLine>,
}

/// `[row, runs]`: a row's runs up to the last cell that draws anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ScreenLine(pub u16, pub Vec<ScreenRun>);

/// `[text, fg, bg, attrs]`: cells sharing one style. Each character is one
/// cell, except in a run with [`screen_attr::WIDE`] (one character, two
/// cells) or [`screen_attr::CLUSTER`] (one character plus combining marks,
/// one cell). Colours: 0 = the theme's default, 1–256 = palette entry + 1,
/// `0x100_0000 | 0xRRGGBB` = truecolor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ScreenRun(pub String, pub u32, pub u32, pub u16);

/// [`ScreenRun`] attribute bits.
pub mod screen_attr {
    pub const BOLD: u16 = 1;
    pub const ITALIC: u16 = 1 << 1;
    pub const DIM: u16 = 1 << 2;
    pub const INVERSE: u16 = 1 << 3;
    pub const HIDDEN: u16 = 1 << 4;
    pub const STRIKE: u16 = 1 << 5;
    /// Underline style in bits 6–8: 1 single, 2 double, 3 curly, 4 dotted, 5 dashed.
    pub const UNDERLINE_SHIFT: u16 = 6;
    pub const UNDERLINE_MASK: u16 = 0b111 << UNDERLINE_SHIFT;
    pub const WIDE: u16 = 1 << 9;
    pub const CLUSTER: u16 = 1 << 10;
    /// Truecolor marker in a colour value.
    pub const RGB: u32 = 0x100_0000;
}
