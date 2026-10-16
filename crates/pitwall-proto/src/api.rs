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

pub mod collab;

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

    // Collaboration (`caps::COLLAB`; `api::collab`): owner-only local
    // methods (collaboration/protocol.md §8.5). Never on a network, never
    // from a shared session (`forbidden`).
    /// → `ShareView[]`
    pub const SHARE_LIST: &str = "share.list";
    /// [`ShareRef`](super::collab::ShareRef) → `ShareView`
    pub const SHARE_GET: &str = "share.get";
    /// [`ShareCreate`](super::collab::ShareCreate) → `ShareView`: a draft.
    pub const SHARE_CREATE: &str = "share.create";
    /// [`ShareSet`](super::collab::ShareSet) → [`ShareChanged`](super::collab::ShareChanged):
    /// the diff it applied; widening a live share asks.
    pub const SHARE_SET: &str = "share.set";
    /// [`ShareRef`](super::collab::ShareRef) → `ShareView` (always asks,
    /// with the full summary, unless from Pitwall's own window).
    pub const SHARE_GO_LIVE: &str = "share.go_live";
    /// [`ShareRef`](super::collab::ShareRef) → `ShareView` (narrowing: direct).
    pub const SHARE_PAUSE: &str = "share.pause";
    /// [`ShareRef`](super::collab::ShareRef) → `ShareView` (widening: asks).
    pub const SHARE_RESUME: &str = "share.resume";
    /// [`ShareRef`](super::collab::ShareRef) → `null`: stop sharing (the
    /// agent stays as it was).
    pub const SHARE_STOP: &str = "share.stop";
    /// [`ShareGuest`](super::collab::ShareGuest) → `ProbeResult[]` (drafts only).
    pub const SHARE_GUEST: &str = "share.guest";
    /// [`ShareRef`](super::collab::ShareRef) → `SharedSessionView[]`
    pub const SHARE_SESSIONS: &str = "share.sessions";
    /// [`ShareBlock`](super::collab::ShareBlock) → `null` (blocking: direct;
    /// letting in again asks).
    pub const SHARE_BLOCK: &str = "share.block";
    /// [`ShareSummaryRequest`](super::collab::ShareSummaryRequest) → `ShareSummary`
    pub const SHARE_SUMMARY: &str = "share.summary";
    /// → `SetupLine[]`: the `claude mcp add` lines for this machine.
    pub const SHARE_SETUP: &str = "share.setup";
    /// [`ShareTurns`](super::collab::ShareTurns) → `OwnerTurnView[]`: one
    /// conversation's turns, as its owner sees them (the session's Messages).
    pub const SHARE_TURNS: &str = "share.turns";
    /// [`ShareAudit`](super::collab::ShareAudit) → `AuditEntryView[]`, newest first.
    pub const SHARE_AUDIT: &str = "share.audit";
    /// [`NoticesAfter`](super::collab::NoticesAfter) → `OwnerNotice[]`:
    /// needs-you, no answer, the day's agent time used, the agent's limit.
    pub const SHARE_NOTICES: &str = "share.notices";
    /// → `TeamCard[]`: mine, then the discovered ones.
    pub const TEAM_LIST: &str = "team.list";
    /// [`TeamRef`](super::collab::TeamRef) → `TeamDetail`
    pub const TEAM_SHOW: &str = "team.show";
    /// [`TeamCreate`](super::collab::TeamCreate) → `TeamDetail`
    pub const TEAM_CREATE: &str = "team.create";
    /// [`TeamJoin`](super::collab::TeamJoin) → `TeamCard`
    pub const TEAM_JOIN: &str = "team.join";
    /// [`TeamRef`](super::collab::TeamRef) → `null`
    pub const TEAM_LEAVE: &str = "team.leave";
    /// [`TeamApprove`](super::collab::TeamApprove) → `TeamDetail` (admins)
    pub const TEAM_APPROVE: &str = "team.approve";
    /// [`TeamRef`](super::collab::TeamRef) → `InviteLink` (admins)
    pub const TEAM_INVITE: &str = "team.invite";
    /// [`TeamAddMember`](super::collab::TeamAddMember) → `MemberAdded` (admins)
    pub const TEAM_ADD_MEMBER: &str = "team.add_member";
    /// → `PersonView[]`: people in my teams and their shared agents.
    pub const PEOPLE_LIST: &str = "people.list";
    /// [`AskSend`](super::collab::AskSend) → `AskView` (waits for the turn,
    /// up to `timeout`). Agents need "Let my agents ask" (`not_allowed`).
    pub const ASK_SEND: &str = "ask.send";
    /// [`AskFilter`](super::collab::AskFilter) → `AskView[]`
    pub const ASK_LIST: &str = "ask.list";
    /// [`AskRef`](super::collab::AskRef) → `AskView`
    pub const ASK_GET: &str = "ask.get";
    /// → `CollabSettingsView`
    pub const COLLAB_SETTINGS_GET: &str = "collab.settings.get";
    /// `CollabSettings` → `CollabSettingsView` (turning something on asks
    /// unless from Pitwall's own window).
    pub const COLLAB_SETTINGS_SET: &str = "collab.settings.set";
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
    /// `share.*`, `team.*`, `people.list`, `ask.*`, `collab.settings.*`
    /// (a Pitwall that hosts collaboration).
    pub const COLLAB: &str = "collab";
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
    /// The login it runs with: an account's name or id, or `default`
    /// (default: the project's account, else the user's own).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
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
