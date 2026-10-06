//! Shapes that cross the JS boundary or go to disk. See docs/CONTRACT.md.

use serde::{Deserialize, Serialize};

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
    /// The agent can work in its own git worktree (`worktree_args`).
    pub worktree: bool,
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
    Some(crate::session::clamp_size(cols?, rows?))
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
    pub tasks: Vec<crate::tasks::Task>,
    /// Last known terminal size; restarts and resumes start the process at it.
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
}

impl AgentRecord {
    pub fn term_size(&self) -> Option<(u16, u16)> {
        size_of(self.cols, self.rows)
    }

    /// Returns whether it changed.
    pub fn set_term_size(&mut self, (cols, rows): (u16, u16)) -> bool {
        let (cols, rows) = crate::session::clamp_size(cols, rows);
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
    pub kind: String,
    pub kind_name: String,
    pub cwd: String,
    pub cwd_display: String,
    pub project: String,
    pub project_display: String,
    pub branch: Option<String>,
    pub worktree: bool,
    /// Asked for its own worktree; waiting to see where the agent made it.
    pub worktree_pending: bool,
    pub location: &'static str,
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

/// Wall-clock milliseconds since the Unix epoch.
pub fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Monotonic milliseconds since process start (for internal timing).
pub fn mono_ms() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64 + 1
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
}
