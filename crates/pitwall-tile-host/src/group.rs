//! The agents' questions group (platform-services.md §14.2): the host side
//! of `agents`, `badges` and `asks`.
//!
//! - `agents` goes through the app's method table ([`Methods`]): the same
//!   `agent.list` and `queue.add` adapters call, with the same limits and
//!   the same Activity. The host checks the grant first, so a tile without
//!   it gets `denied` without a call.
//! - `badges` and `asks` are checked and cut to their limits here and
//!   handed to the app as [`UiRequest`]s (the whole set each time); the app
//!   draws them on sidebar rows and in the floating control and sends the
//!   answers back as `ask-answered` events.

use std::collections::BTreeSet;
use std::sync::Arc;

use pitwall_core::extensions::permissions::{AGENTS_QUEUE, AGENTS_READ, PROJECTS_READ};
use serde_json::{json, Value};

use crate::bindings::pitwall::tile::{agents, asks, badges};
use crate::host::{cut, HostState};
use crate::ui::HostError;
use crate::{LogLevel, UiRequest};

/// The most badges in a set.
pub const MAX_BADGES: usize = 500;
/// A badge's note, in characters.
pub const MAX_NOTE: usize = 16;
/// A badge's count.
pub const MAX_COUNT: u32 = 9999;
/// A badge's tooltip, in characters.
pub const MAX_TOOLTIP: usize = 120;
/// The most open questions in a set.
pub const MAX_ASKS: usize = 50;
/// An ask's title, its question, a suggestion's label and description, in
/// characters.
pub const MAX_ASK_TITLE: usize = 200;
pub const MAX_QUESTION: usize = 2000;
pub const MAX_LABEL: usize = 80;
pub const MAX_DESCRIPTION: usize = 200;
/// The most suggestions an ask offers.
pub const MAX_SUGGESTIONS: usize = 8;
/// An ask's id, in characters.
pub const MAX_ASK_ID: usize = 128;
/// The most characters `queue-add` takes (the method table's limit for
/// extensions).
pub const MAX_QUEUE_TEXT: usize = 4000;

/// The app's method table (the daemon's `METHODS`, as adapters reach it),
/// called on the tile's worker thread as extension `extension`: it must
/// not need the UI thread.
pub trait Methods: Send + Sync {
    /// Call `method` with `params` as extension `extension`.
    fn call(&self, extension: &str, method: &str, params: Value) -> Result<Value, MethodError>;
}

/// Why a method call failed: the method table's code (`denied`, `busy`,
/// `bad_params`, `not_found`, …) and its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodError {
    pub code: String,
    pub message: String,
}

impl MethodError {
    pub fn new(code: &str, message: impl Into<String>) -> MethodError {
        MethodError { code: code.to_string(), message: message.into() }
    }
}

/// No method table: every call is `unsupported` (a host without the app).
pub struct NoMethods;

impl Methods for NoMethods {
    fn call(&self, _extension: &str, _method: &str, _params: Value) -> Result<Value, MethodError> {
        Err(MethodError::new("unsupported", "This Pitwall can't answer calls here."))
    }
}

/// What the group needs of a running tile: its manifest's switches, the
/// method table, and the refusals already noted in its Log.
pub(crate) struct Group {
    /// The manifest contributes `badges`.
    pub badges: bool,
    /// The manifest contributes `asks`.
    pub asks: bool,
    pub methods: Arc<dyn Methods>,
    noted: BTreeSet<String>,
}

impl Group {
    pub fn new(badges: bool, asks: bool, methods: Arc<dyn Methods>) -> Group {
        Group { badges, asks, methods, noted: BTreeSet::new() }
    }
}

impl HostState {
    /// Note `what` in the Log, once per run of the tile.
    fn note_once(&mut self, key: &str, what: String) {
        if self.group.noted.insert(key.to_string()) {
            self.host_log(LogLevel::Warn, what);
        }
    }

    fn method(&mut self, method: &str, params: Value) -> Result<Value, HostError> {
        let ext = self.extension.clone();
        self.group.methods.call(&ext, method, params).map_err(|e| {
            self.host_log(LogLevel::Warn, format!("{method}: {}", cut(&e.message, 300)));
            match e.code.as_str() {
                "denied" | "off" => HostError::Denied,
                "busy" => HostError::Busy,
                "too_large" => HostError::TooLarge,
                "bad_params" | "not_found" => HostError::Invalid(e.message),
                _ => HostError::Failed(e.message),
            }
        })
    }
}

/// An agent of `agent.list`, as the tile sees it.
fn agent_of(v: &Value) -> Option<agents::Agent> {
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let state = match v.get("status").and_then(Value::as_str).unwrap_or("") {
        "working" => agents::AgentState::Working,
        "blocked" => agents::AgentState::Blocked,
        "idle" => agents::AgentState::Idle,
        "done" => agents::AgentState::Done,
        "exited" | "stopped" => agents::AgentState::Stopped,
        _ => agents::AgentState::Unknown,
    };
    Some(agents::Agent {
        id: s("id")?,
        name: s("name").unwrap_or_default(),
        kind: s("kind").unwrap_or_default(),
        kind_name: s("kindName").unwrap_or_default(),
        project: s("project").unwrap_or_default(),
        branch: s("branch"),
        waiting_on_you: state == agents::AgentState::Blocked,
        state,
        queued: v.get("queue").and_then(Value::as_array).map_or(0, |q| q.len() as u32),
    })
}

impl agents::Host for HostState {
    fn list(&mut self) -> Result<Vec<agents::Agent>, HostError> {
        if !self.grants.has(AGENTS_READ) {
            return Err(HostError::Denied);
        }
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        let v = self.method("agent.list", json!({}))?;
        Ok(v.as_array().map(|a| a.iter().filter_map(agent_of).collect()).unwrap_or_default())
    }

    fn queue_add(&mut self, agent: String, text: String) -> Result<(), HostError> {
        if !self.grants.has(AGENTS_QUEUE) {
            return Err(HostError::Denied);
        }
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        if text.trim().is_empty() {
            return Err(HostError::Invalid("the text is empty".into()));
        }
        if text.trim_end().chars().count() > MAX_QUEUE_TEXT {
            return Err(HostError::TooLarge);
        }
        self.method("queue.add", json!({ "agentId": agent, "text": text })).map(|_| ())
    }
}

/// `s` cut to `max` characters (not bytes), with "…" when cut.
fn chars(s: &str, max: usize) -> String {
    let s = s.trim();
    match s.char_indices().nth(max) {
        Some((at, _)) => {
            let mut end = at;
            if let Some((i, _)) = s[..at].char_indices().last() {
                end = i;
            }
            format!("{}…", &s[..end])
        }
        None => s.to_string(),
    }
}

impl badges::Host for HostState {
    fn set(&mut self, list: Vec<badges::Badge>) -> Result<(), HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        if !self.group.badges {
            self.note_once("badges", "badges.set: the manifest doesn't contribute \"badges\"".into());
            return Err(HostError::Denied);
        }
        if list.len() > MAX_BADGES {
            return Err(HostError::TooLarge);
        }
        let mut kept = Vec::with_capacity(list.len());
        for mut b in list {
            let need = match &b.row {
                badges::Row::Agent(_) => AGENTS_READ,
                badges::Row::Project(_) => PROJECTS_READ,
            };
            if !self.grants.has(need) {
                self.note_once(&format!("badges:{need}"), format!("badges.set: a badge on {} needs the “{need}” permission; it isn't shown", if need == AGENTS_READ { "an agent" } else { "a project" }));
                continue;
            }
            b.note = b.note.map(|n| chars(&n, MAX_NOTE)).filter(|n| !n.is_empty());
            b.count = b.count.map(|c| c.min(MAX_COUNT));
            if b.count.is_none() && b.note.is_none() {
                continue;
            }
            b.tooltip = b.tooltip.map(|t| chars(&t, MAX_TOOLTIP)).filter(|t| !t.is_empty());
            if let Some(t) = b.open.take() {
                if self.known_views.contains(&t) {
                    b.open = Some(t);
                } else {
                    self.note_once(&format!("badges:open:{t}"), format!("badges.set: “{}” isn't one of its tiles; the badge opens nothing", cut(&t, 80)));
                }
            }
            kept.push(b);
        }
        self.pending.push(crate::host::Pending::Ui(UiRequest::Badges(kept)));
        Ok(())
    }
}

impl asks::Host for HostState {
    fn set(&mut self, open: Vec<asks::Ask>) -> Result<(), HostError> {
        if !self.count_call() {
            return Err(HostError::Busy);
        }
        if !self.group.asks {
            self.note_once("asks", "asks.set: the manifest doesn't contribute \"asks\"".into());
            return Err(HostError::Denied);
        }
        if open.len() > MAX_ASKS {
            return Err(HostError::TooLarge);
        }
        let mut ids = BTreeSet::new();
        let mut kept = Vec::with_capacity(open.len());
        for mut a in open {
            if a.id.is_empty() || a.id.chars().count() > MAX_ASK_ID || a.id.chars().any(char::is_control) {
                return Err(HostError::Invalid(format!("an ask's id must be 1 to {MAX_ASK_ID} characters without control characters")));
            }
            if !ids.insert(a.id.clone()) {
                return Err(HostError::Invalid(format!("two asks have the id “{}”", cut(&a.id, 80))));
            }
            if a.question.trim().is_empty() {
                return Err(HostError::Invalid(format!("ask “{}” has no question", cut(&a.id, 80))));
            }
            if a.from.is_some() && !self.grants.has(AGENTS_READ) {
                self.note_once("asks:from", format!("asks.set: naming the agent that asks needs the “{AGENTS_READ}” permission; the extension's name is shown instead"));
                a.from = None;
            }
            a.title = chars(&a.title, MAX_ASK_TITLE);
            a.question = chars(&a.question, MAX_QUESTION);
            a.suggestions.truncate(MAX_SUGGESTIONS);
            for s in &mut a.suggestions {
                s.label = chars(&s.label, MAX_LABEL);
                s.description = s.description.as_deref().map(|d| chars(d, MAX_DESCRIPTION)).filter(|d| !d.is_empty());
            }
            a.suggestions.retain(|s| !s.label.is_empty());
            kept.push(a);
        }
        self.pending.push(crate::host::Pending::Ui(UiRequest::Asks(kept)));
        Ok(())
    }

    fn reply(&mut self, id: String, refused: Option<String>) {
        if self.count_call() {
            let refused = refused.map(|r| chars(&r, 200)).filter(|r| !r.is_empty());
            self.pending.push(crate::host::Pending::Ui(UiRequest::AskResult { id, refused }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_reads_as_the_method_table_gives_it() {
        let v = json!({"id": "a1", "name": "api", "kind": "claude", "kindName": "Claude Code", "project": "/p", "branch": "main", "status": "blocked", "queue": [{"id": "q1", "text": "x"}]});
        let a = agent_of(&v).unwrap();
        assert_eq!((a.id.as_str(), a.kind_name.as_str(), a.state, a.waiting_on_you, a.queued), ("a1", "Claude Code", agents::AgentState::Blocked, true, 1));
        let b = agent_of(&json!({"id": "a2", "status": "exited"})).unwrap();
        assert_eq!((b.state, b.waiting_on_you, b.branch), (agents::AgentState::Stopped, false, None));
        assert!(agent_of(&json!({"name": "no id"})).is_none());
    }

    #[test]
    fn text_is_cut_by_characters() {
        assert_eq!(chars("  short ", 16), "short");
        assert_eq!(chars("ééééééééééééééééééé", 16).chars().count(), 16);
        assert!(chars("ééééééééééééééééééé", 16).ends_with('…'));
    }
}
