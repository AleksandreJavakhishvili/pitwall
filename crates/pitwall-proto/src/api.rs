//! Methods, their parameters and results, and approvals.
//!
//! Method names follow api.md's commands (`list_agents` → `agent.list`,
//! `create_agent` → `agent.create`, `adopt_session` → `session.add`), so
//! api.md stays the reference and the CLI mirrors it. More methods are
//! added the same way: a name here, a params/result shape, a row in the
//! server's method table (with its risk).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::views::{AgentView, ScannedPlace};

pub mod method {
    /// → `AgentView[]`
    pub const AGENT_LIST: &str = "agent.list";
    /// [`AgentCreate`](super::AgentCreate) → `AgentView`
    pub const AGENT_CREATE: &str = "agent.create";
    /// [`AgentRef`](super::AgentRef) → `AgentView`: its git numbers read
    /// now, whatever the polling pace (`refresh_changes`).
    pub const AGENT_REFRESH: &str = "agent.refresh";
    /// → [`ProviderMachines`](super::ProviderMachines)`[]`
    pub const MACHINE_LIST: &str = "machine.list";
    /// [`FormRequest`](super::FormRequest) → [`CreateForm`](super::CreateForm):
    /// how new agents are made on one machine (read-only; the provider
    /// lists its choices there).
    pub const MACHINE_FORM: &str = "machine.form";
    /// [`SessionFilter`](super::SessionFilter) → `ScannedPlace[]`
    pub const SESSION_LIST: &str = "session.list";
    /// [`SessionAdd`](super::SessionAdd) → [`SessionAdded`](super::SessionAdded)
    pub const SESSION_ADD: &str = "session.add";
    /// Pending approvals → [`ApprovalView`](super::ApprovalView)`[]`
    /// (verified UI clients only).
    pub const APPROVAL_LIST: &str = "approval.list";
    /// [`ApprovalAnswer`](super::ApprovalAnswer) → `null` (verified UI
    /// clients only).
    pub const APPROVAL_ANSWER: &str = "approval.answer";
    /// → [`SettingView`](crate::SettingView)`[]`: every setting with its
    /// current value, allowed values and description.
    pub const SETTINGS_LIST: &str = "settings.list";
    /// [`SettingKey`](crate::SettingKey) → `SettingView`
    pub const SETTINGS_GET: &str = "settings.get";
    /// [`SettingSet`](crate::SettingSet) → `SettingView` (applied live;
    /// `approval` settings wait for the user's OK when an agent asks).
    pub const SETTINGS_SET: &str = "settings.set";
    /// [`SettingKey`](crate::SettingKey) → `SettingView`: back to the default.
    pub const SETTINGS_RESET: &str = "settings.reset";

    // Managing what Pitwall has (`caps::MANAGE`; crate::manage).
    /// [`AgentRef`](super::AgentRef) → `AgentView` (asks the user).
    pub const AGENT_STOP: &str = "agent.stop";
    /// [`AgentRef`](super::AgentRef) → `AgentView` (asks the user).
    pub const AGENT_RESTART: &str = "agent.restart";
    /// [`AgentRemove`](crate::AgentRemove) → `null` (asks the user).
    pub const AGENT_REMOVE: &str = "agent.remove";
    /// [`AgentRename`](crate::AgentRename) → `AgentView`
    pub const AGENT_RENAME: &str = "agent.rename";
    /// [`AgentRef`](super::AgentRef) → [`AgentDiagnosis`](crate::AgentDiagnosis)
    pub const AGENT_STATUS: &str = "agent.status";
    /// [`AgentWait`](crate::AgentWait) → [`WaitResult`](crate::WaitResult);
    /// error `timeout` when it didn't get there in time.
    pub const AGENT_WAIT: &str = "agent.wait";
    /// [`QueueAdd`](crate::QueueAdd) → `AgentView`
    pub const QUEUE_ADD: &str = "queue.add";
    /// [`QueueFilter`](crate::QueueFilter) → [`AgentQueue`](crate::AgentQueue)`[]`
    pub const QUEUE_LIST: &str = "queue.list";
    /// [`QueueItemRef`](crate::QueueItemRef) → `AgentView`
    pub const QUEUE_REMOVE: &str = "queue.remove";
    /// [`QueueItemRef`](crate::QueueItemRef) → `AgentView`: send it now.
    pub const QUEUE_SEND: &str = "queue.send";
    /// → [`ProjectView`](crate::ProjectView)`[]`
    pub const PROJECT_LIST: &str = "project.list";
    /// [`ProjectPath`](crate::ProjectPath) → `ProjectView[]`
    pub const PROJECT_ADD: &str = "project.add";
    /// [`ProjectPath`](crate::ProjectPath) → `ProjectView[]` (never deletes files).
    pub const PROJECT_REMOVE: &str = "project.remove";
    /// → [`RulesOverview`](crate::RulesOverview)
    pub const RULES_LIST: &str = "rules.list";
    /// → [`RuleSets`](crate::RuleSets)
    pub const RULES_SETS: &str = "rules.sets";
    /// [`RulesApply`](crate::RulesApply) → [`RulesApplied`](crate::RulesApplied)
    /// (asks the user before writing into a main checkout).
    pub const RULES_APPLY: &str = "rules.apply";
    /// [`RulesDefault`](crate::RulesDefault) → `RuleSets`
    pub const RULES_DEFAULT: &str = "rules.default";
    /// [`ReviewRequest`](crate::ReviewRequest) → [`ReviewChanges`](crate::ReviewChanges)
    pub const REVIEW_CHANGES: &str = "review.changes";

    // Spaces and windows (`caps::SPACES`: the app's UI state).
    /// → [`SpaceView`](crate::SpaceView)`[]`
    pub const SPACE_LIST: &str = "space.list";
    /// [`SpaceCreate`](crate::SpaceCreate) → `SpaceView`
    pub const SPACE_CREATE: &str = "space.create";
    /// [`SpaceRename`](crate::SpaceRename) → `SpaceView`
    pub const SPACE_RENAME: &str = "space.rename";
    /// [`SpaceMove`](crate::SpaceMove) → `SpaceView`
    pub const SPACE_MOVE: &str = "space.move";
    /// [`AgentMove`](crate::AgentMove) → `SpaceView` (the space it is in now)
    pub const AGENT_MOVE: &str = "agent.move";
}

/// Feature flags in `welcome.caps`.
pub mod caps {
    pub const AGENTS: &str = "agents";
    pub const SESSIONS: &str = "sessions";
    pub const APPROVALS: &str = "approvals";
    /// `settings.*` (a Pitwall that hosts the settings: the app).
    pub const SETTINGS: &str = "settings";
    /// Agents stop/restart/remove/rename/status/wait, `queue.*`,
    /// `project.*`, `rules.*`, `review.changes`.
    pub const MANAGE: &str = "manage";
    /// `space.*` and `agent.move` (a Pitwall with windows: the app).
    pub const SPACES: &str = "spaces";
}

pub mod event {
    /// The pending approvals changed: `ApprovalView[]`.
    pub const APPROVALS_CHANGED: &str = "approvals.changed";
}

/// One agent, by id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRef {
    pub agent_id: String,
}

/// Start a new agent (or terminal) in Pitwall: on this Mac by default, or
/// on another machine (`machine`, e.g. an agw VM), where it is made with
/// that machine's form (`machine.form`) and the user approves it first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentCreate {
    /// A kind id (`claude`, `codex`, `shell`, …); on a machine whose
    /// platform decides what runs (a form without `folder`), ignored.
    #[serde(default)]
    pub kind: String,
    /// The folder it works in (absolute; `~` is expanded); only for a form
    /// with `folder`.
    #[serde(default)]
    pub project: String,
    /// The provider of `machine`; found from `machine` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Where to make it (default: this Mac).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// The machine's form fields (field id → value).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, String>,
    /// Shown name; defaults to the kind's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Continue this conversation (the kind's resume flags).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
}

/// Narrow `session.list` to one provider and/or machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

impl SessionFilter {
    /// Keep only what matches.
    pub fn apply(&self, places: Vec<ScannedPlace>) -> Vec<ScannedPlace> {
        places
            .into_iter()
            .filter(|p| self.provider.as_ref().is_none_or(|want| *want == p.provider))
            .map(|mut p| {
                if let (Some(want), Some(ms)) = (&self.machine, p.machines.as_mut()) {
                    ms.retain(|m| m.id == *want);
                }
                p
            })
            .collect()
    }
}

/// "Add to Pitwall" a session that exists on another machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionAdd {
    pub provider: String,
    pub machine: String,
    /// The provider's own handle (an agw session name).
    pub native: String,
    /// Also start it there when it is stopped. That changes the other
    /// machine, so the user approves it in Pitwall first.
    #[serde(default)]
    pub start: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionAdded {
    pub agent: AgentView,
    /// It was already tracked by Pitwall (adding is idempotent).
    pub already: bool,
    /// It was stopped and has been started (`start`).
    pub started: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MachineEntry {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
}

/// One provider and its machines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProviderMachines {
    pub provider: String,
    pub label: String,
    pub version: Option<String>,
    /// New agents can be started here (`agent new`).
    pub can_create: bool,
    /// Existing sessions here can be added (`session add`).
    pub can_add_sessions: bool,
    pub machines: Vec<MachineEntry>,
    /// Why the machines couldn't be listed.
    pub error: Option<String>,
}

// ------------------------------------------------------------ approvals

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    /// The user may let the same caller do it again without asking
    /// ("Remember for this caller").
    Low,
    /// Asked every time.
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum RequesterKind {
    /// A process inside one of Pitwall's agents or terminals.
    Agent,
    /// A process outside Pitwall (the user's own terminal, a script).
    Outside,
}

/// Who asked, as the server established it from the connecting process —
/// never from anything the caller said about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Requester {
    pub kind: RequesterKind,
    /// The Pitwall agent it runs in.
    pub agent_id: Option<String>,
    /// How it is shown: the agent's name, or "A terminal outside Pitwall".
    pub name: String,
    pub pid: Option<u32>,
    /// The connecting program's name, if known.
    pub process: Option<String>,
}

/// A risky request waiting for the user (shown as Pitwall's approval dialog).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalView {
    pub id: String,
    /// The method ("session.add").
    pub action: String,
    /// "start the session \"work\" on vm-1 (agw)" — after the requester's name.
    pub summary: String,
    /// More lines for the dialog.
    pub details: Vec<String>,
    pub requester: Requester,
    pub risk: Risk,
    /// "Remember for this caller" may be offered (low risk only).
    pub rememberable: bool,
    /// Unix ms.
    #[ts(type = "number")]
    pub created_at: u64,
    /// Unix ms; unanswered by then = denied.
    #[ts(type = "number")]
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalAnswer {
    pub id: String,
    pub allow: bool,
    /// Allow this caller the same action without asking, until Pitwall
    /// quits (ignored unless `rememberable`).
    #[serde(default)]
    pub remember: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::ScannedMachine;
    use serde_json::json;

    #[test]
    fn params_golden_json() {
        let add: SessionAdd = serde_json::from_value(json!({"provider":"agw","machine":"vm-1","native":"work"})).unwrap();
        assert!(!add.start, "start defaults to off");
        assert_eq!(
            serde_json::to_value(SessionAdd { start: true, ..add }).unwrap(),
            json!({"provider":"agw","machine":"vm-1","native":"work","start":true})
        );
        let create = AgentCreate { kind: "claude".into(), project: "/p".into(), name: Some("api".into()), ..Default::default() };
        let wire = json!({"kind":"claude","project":"/p","name":"api"});
        assert_eq!(serde_json::to_value(&create).unwrap(), wire);
        assert_eq!(serde_json::from_value::<AgentCreate>(wire).unwrap(), create);
        let answer: ApprovalAnswer = serde_json::from_value(json!({"id":"a1","allow":true})).unwrap();
        assert!(!answer.remember);
    }

    #[test]
    fn session_filters_narrow_by_provider_and_machine() {
        let m = |id: &str| ScannedMachine { id: id.into(), label: id.into(), detail: None, sessions: vec![] };
        let place = |p: &str| ScannedPlace { provider: p.into(), label: p.into(), version: None, machines: Some(vec![m("a"), m("b")]) };
        let all = vec![place("agw"), place("ssh")];
        assert_eq!(SessionFilter::default().apply(all.clone()), all);
        let got = SessionFilter { provider: Some("agw".into()), machine: Some("b".into()) }.apply(all);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].machines.as_ref().unwrap().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["b"]);
    }
}
