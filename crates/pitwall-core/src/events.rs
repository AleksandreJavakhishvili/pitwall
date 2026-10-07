//! What the engine tells its host. The core only says *what happened*; the
//! host decides how to show it (the app: webview events, notifications, the
//! Dock badge; later the daemon: socket events). One narrow trait, so a new
//! host is one small implementation.

use crate::model::{AgentView, Attention};
use crate::onboarding::project_list::Project;
use crate::onboarding::scan::ScanProgress;

#[derive(Debug, Clone)]
pub enum Event {
    /// The agent list or something shown about an agent changed (throttled).
    AgentsChanged(Vec<AgentView>),
    /// An agent turned blocked or done: it needs the user.
    Attention(Attention),
    /// How many agents are blocked now; sent whenever the number changes.
    BlockedCount(usize),
    /// One onboarding scan step started or finished.
    ScanProgress(ScanProgress),
    /// The user's project list changed.
    ProjectsChanged(Vec<Project>),
}

/// Receives engine events. Called from engine threads: must not block for long.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: Event);
}

/// Drops everything (headless use, and tests that don't look at events).
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _event: Event) {}
}
