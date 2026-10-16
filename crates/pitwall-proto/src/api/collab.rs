//! Collaboration's local methods (docs/spec/collaboration/protocol.md
//! §8.5–8.7): `share.*`, `team.*`, `people.list`, `ask.*` and
//! `collab.settings.*`, their parameters and results. Offered when
//! `welcome.caps` has [`caps::COLLAB`](crate::caps::COLLAB).
//!
//! These are owner-only methods on the local socket: never on a network,
//! never from a shared session. The shapes that are stored or shown
//! elsewhere (`ShareSettings`, `TeamCard`, `AskView`, …) are in
//! [`crate::collab`]; here are only the requests and the views the UI and
//! the CLI need on top of them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::collab::{
    Audience, Capability, DirectoryEntry, GroupRef, Identity, LimitOverride, NetworkId, Programs, ShareDiff, ShareSettings, ShareState,
    Start, TeamCard, TeamId, TeamRole,
};

// ------------------------------------------------------------ shares

/// One share, by the agent it shares (id or name) or its slug.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShareRef {
    pub share: String,
}

/// What to change in a share's settings (`share.create`, `share.set`). Every
/// field is optional: what isn't given stays (or, on `share.create`, takes
/// the Share dialog's defaults: fresh start, no network beyond the model,
/// no environment variables, the sensitive files excluded).
///
/// `settings` replaces everything at once (the Share dialog's Save); the
/// other fields are the CLI's flags, applied on top of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct SharePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub settings: Option<ShareSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub capability: Option<Capability>,
    /// Added to the audience.
    #[serde(skip_serializing_if = "AudienceEdit::is_empty")]
    pub add: AudienceEdit,
    /// Taken out of the audience.
    #[serde(skip_serializing_if = "AudienceEdit::is_empty")]
    pub remove: AudienceEdit,
    /// The picked paths, all of them (`--paths src/,docs/`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub paths: Option<Vec<String>>,
    /// More exclusions (`--exclude`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// Exclusions to drop again, the scan's findings included (`--include`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub uncommitted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub start: Option<Start>,
    /// Environment variables passed in, all of them.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub env: Option<Vec<String>>,
    /// Hosts the egress proxy allows besides the model's, all of them.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hosts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub programs: Option<Programs>,
    /// Soft: instructions to the agent ("not guaranteed").
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub off_limits: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub messages_per_day: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sessions_running: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub queue: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub turn_timeout_min: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub agent_minutes_per_day: Option<u32>,
    /// Per-person overrides by person id; `null` removes one.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<String, Option<LimitOverride>>,
}

/// People, teams, groups and networks added to or taken out of an audience.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(default)]
pub struct AudienceEdit {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<Identity>,
    /// Team ids, or names the owner's Pitwall resolves.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub networks: Vec<NetworkId>,
}

impl AudienceEdit {
    pub fn is_empty(&self) -> bool {
        self.people.is_empty() && self.teams.is_empty() && self.groups.is_empty() && self.networks.is_empty()
    }
}

/// `share.create`: a draft of `agent` (nobody else can see it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShareCreate {
    /// The agent to share (id or name).
    pub agent: String,
    #[serde(default)]
    pub patch: SharePatch,
}

/// `share.set`: change a share. Widening asks the owner unless it comes from
/// Pitwall's own window (or the share is still a draft).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShareSet {
    pub share: String,
    pub patch: SharePatch,
    /// Only work out the diff; change nothing (the Share dialog's Save shows it first).
    #[serde(default)]
    pub dry_run: bool,
}

/// `share.summary`: the plain-language summary of a saved share, or of
/// settings not saved yet (the Share dialog's Summary step).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShareSummaryRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub share: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub settings: Option<ShareSettings>,
}

/// One piece of a summary sentence; `strong` pieces are bold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SummaryPart {
    pub text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub strong: bool,
}

/// Which answer a summary sentence states (§5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SummaryTopic {
    Who,
    Capability,
    Scope,
    Start,
    Access,
    Account,
    Limits,
    /// Instructions to the agent: not guaranteed.
    Soft,
}

/// One fixed sentence of the summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SummarySentence {
    pub topic: SummaryTopic,
    pub parts: Vec<SummaryPart>,
}

impl SummarySentence {
    pub fn text(&self) -> String {
        self.parts.iter().map(|p| p.text.as_str()).collect()
    }
}

/// The plain-language page Pitwall makes from a share's answers: fixed
/// sentences, never written by a model (§5.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
pub struct ShareSummary {
    pub sentences: Vec<SummarySentence>,
    /// All of it as one text (the CLI, the approval dialog).
    pub text: String,
}

/// A file the sensitive-file scan found in a share's scope (§5.4 step 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SensitiveFile {
    /// Relative to the repository.
    pub path: String,
    /// "matches *.pem", "contains a private key".
    pub why: String,
}

/// One caller's use of a share, today and over 30 days (§11 Usage).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct UsageCounts {
    pub messages: u32,
    pub results: u32,
    pub minutes: u32,
    #[ts(type = "number | null")]
    pub tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CallerUsage {
    /// The person id limits are keyed by.
    pub person: String,
    /// For display (`bob@example.com`).
    pub login: String,
    pub today: UsageCounts,
    pub days30: UsageCounts,
    /// Their daily message limit (an override, else the share's).
    pub messages_per_day: u32,
    pub blocked: bool,
}

/// A share's use (`share.get`, the Usage view).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShareUsage {
    /// Agent time today, all callers together, against `limits.agentMinutesPerDay`.
    pub agent_minutes_today: u32,
    /// Conversations running now, against `limits.sessionsRunning`.
    pub running: u32,
    /// The agent hit its vendor's limit: "limited by Claude until 14:30".
    pub limited_until: Option<String>,
    pub callers: Vec<CallerUsage>,
}

/// One share as its owner sees it (`share.get`, `share.list`, the result of
/// `share.create` and the go-live approval).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShareView {
    pub settings: ShareSettings,
    /// `alice/api-expert`.
    pub address: String,
    /// The shared agent's name and kind, for display.
    pub agent_name: String,
    pub kind_name: String,
    pub summary: ShareSummary,
    /// What the sensitive-file scan found (and is excluded unless included again).
    pub sensitive: Vec<SensitiveFile>,
    /// Files in the copy after exclusions, when counted.
    pub files: Option<u32>,
    pub usage: ShareUsage,
    /// Guest probes run on it, newest last.
    pub probes: Vec<ProbeResult>,
}

/// What `share.set` did (or, with `dryRun`, would do).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShareChanged {
    pub share: ShareView,
    pub diff: ShareDiff,
    /// The change waited for the owner's OK (it widened access).
    pub approved: bool,
    pub dry_run: bool,
}

/// `share.guest`: ask the draft as a guest, through the same pipeline as a
/// real caller (§5.4 "Try as guest"). `probes` sends the standard probes;
/// `message` one message of the owner's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShareGuest {
    pub share: String,
    #[serde(default)]
    pub probes: bool,
    /// Only these probes (by id, `env-file`); empty with `probes`: all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub message: Option<String>,
}

/// One guest message and what came of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    /// The probe's id (`env-file`), or `message` for the owner's own.
    pub probe: String,
    /// Sent verbatim.
    pub message: String,
    pub reply: Option<String>,
    /// What the agent tried and the sandbox or proxy refused
    /// (`read ~/.ssh/id_ed25519: denied`).
    pub tried: Vec<String>,
    pub at: String,
}

/// `share.block`: stop all of a person's messages to my shares at once
/// (or to one share), or let them in again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShareBlock {
    /// A person id or a login (`bob@example.com`).
    pub person: String,
    /// One share; all of them when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub share: Option<String>,
    #[serde(default = "yes")]
    pub blocked: bool,
}

fn yes() -> bool {
    true
}

/// The line a plain Claude Code user runs once to reach this machine's
/// shares on one network (§4.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SetupLine {
    pub network: NetworkId,
    /// `alice`: the owner's name in the caller's MCP setup.
    pub owner: String,
    /// `claude mcp add --transport http alice http://studio.example.ts.net:47421/mcp`.
    pub line: String,
}

// ------------------------------------------------------------ teams

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct TeamRef {
    /// A team id, a fingerprint (`9F2-C1A`) or a name.
    pub team: String,
}

/// `team.create`: a new team with me as its only admin and member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct TeamCreate {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub listed: bool,
    #[serde(default)]
    pub join: crate::collab::JoinPolicy,
}

/// `team.join`: ask to join a listed team, or join with an invite link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct TeamJoin {
    /// A team id, fingerprint or name, or a `pitwall://team/join?…` link.
    pub team: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// `team.approve`: an admin answers a join request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct TeamApprove {
    pub team: String,
    /// The requester's person id or login.
    pub person: String,
    #[serde(default = "yes")]
    pub allow: bool,
}

/// `team.add_member`: an admin adds someone by login, also people without
/// Pitwall (§4.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct TeamAddMember {
    pub team: String,
    /// `erin@example.com`.
    pub login: String,
    /// The network the login is on (`tailscale:example.ts.net`, or just a
    /// provider, `tailscale`); default: the only one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub network: Option<String>,
}

/// One device of a member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    /// Its name on the network (`studio`).
    pub label: String,
    pub network: NetworkId,
    pub online: bool,
}

/// One member on the team page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MemberView {
    pub person: String,
    /// For display (`bob@example.com`).
    pub login: String,
    pub identities: Vec<Identity>,
    pub devices: Vec<DeviceView>,
    pub admin: bool,
    /// No Pitwall: a plain Claude Code (MCP) user ("Claude only").
    pub claude_only: bool,
}

/// A pending join request (admins).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequestView {
    pub person: String,
    pub login: String,
    pub note: String,
    /// "from bob@example.com on bob-mbp (example.ts.net)".
    pub from: String,
    pub at: String,
}

/// A team's page (`team.show`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TeamDetail {
    pub card: TeamCard,
    pub members: Vec<MemberView>,
    /// "dave@example.com", with `lastChangeAt`.
    pub last_change_by: Option<String>,
    pub last_change_at: Option<String>,
    pub requests: Vec<JoinRequestView>,
    /// For people without Pitwall: one line per member who shares with the team.
    pub setup: Vec<SetupLine>,
    /// "Your change was re-applied after dave's".
    pub conflict: Option<String>,
}

/// `team.invite`: a one-time link (7 days, one use).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct InviteLink {
    pub link: String,
    pub expires_at: String,
}

/// What `team.add_member` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MemberAdded {
    pub team: TeamDetail,
    pub member: MemberView,
    /// The lines to send them when they have no Pitwall.
    pub setup: Vec<SetupLine>,
    pub note: String,
}

// ------------------------------------------------------------ people

/// Someone in my teams (People → People, `pitwall people list`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PersonView {
    pub person: String,
    pub login: String,
    pub identities: Vec<Identity>,
    /// Team ids we share.
    pub teams: Vec<TeamId>,
    pub devices: Vec<DeviceView>,
    /// Their shared agents I may use.
    pub shares: Vec<DirectoryEntry>,
    /// Blocked from all my shares.
    pub blocked: bool,
    pub claude_only: bool,
    pub role: Option<TeamRole>,
}

// ------------------------------------------------------------ asking

/// `ask.send`: one message to a teammate's shared agent; waits for the turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct AskSend {
    /// `alice/api-expert`.
    pub share: String,
    pub message: String,
    /// Start a new conversation instead of continuing mine.
    #[serde(default)]
    pub new: bool,
    /// Give up waiting after this many seconds (the turn goes on; `ask.get` it later).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub timeout: Option<u64>,
}

/// `ask.list`: my messages and replies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct AskFilter {
    /// Only this owner's shares (`alice`, `alice@example.com`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub share: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct AskRef {
    pub turn: String,
}

// ------------------------------------------------------------ settings

/// One network on Settings → Networks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NetworkRow {
    pub id: NetworkId,
    /// "Tailscale", "Pitwall Cloud".
    pub label: String,
    /// `off`, `signed_out`, `connecting`, `ready`, or `error: …`.
    pub state: String,
    /// "alice@example.com on studio".
    pub me: Option<String>,
    /// This machine's base URL there.
    pub endpoint: Option<String>,
    /// "Let people on example.ts.net reach this machine".
    pub reachable: bool,
}

/// Where team discovery looks (§3, "Look for teams on").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LookFor {
    /// Devices tagged `tag:pitwall`.
    Tagged,
    /// Every online device of people on my network (capped).
    #[default]
    Everyone,
    /// Only devices I add by name.
    Named,
}

/// Settings → Collaboration: everything off by default (§9.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct CollabSettings {
    /// "Use teams": discovery, joining, asking. Outgoing only.
    pub use_teams: bool,
    /// "Let people on <network> reach this machine", per network id.
    pub reachable: BTreeMap<String, bool>,
    /// "Let my agents ask shared agents".
    pub agents_ask: bool,
    /// Messages per hour per asking agent while `agentsAsk` is on.
    pub agent_budget_per_hour: u32,
    /// Shared conversations on this machine at once, across all shares.
    pub session_cap: u32,
    pub look_for: LookFor,
    /// Devices to probe with `look_for: named`.
    pub named_devices: Vec<String>,
}

impl Default for CollabSettings {
    fn default() -> CollabSettings {
        CollabSettings {
            use_teams: false,
            reachable: BTreeMap::new(),
            agents_ask: false,
            agent_budget_per_hour: 20,
            session_cap: 4,
            look_for: LookFor::Everyone,
            named_devices: Vec::new(),
        }
    }
}

/// `collab.settings.get`'s answer: the switches and the networks they are about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CollabSettingsView {
    pub settings: CollabSettings,
    pub networks: Vec<NetworkRow>,
    /// "Copy my MCP setup line".
    pub setup: Vec<SetupLine>,
    /// Hosting shared sessions works on this OS (macOS in C1a, §9.2).
    pub can_host: bool,
}

/// A share's state as a word (`draft`, `live`, `paused`).
pub fn state_word(s: ShareState) -> &'static str {
    match s {
        ShareState::Draft => "draft",
        ShareState::Live => "live",
        ShareState::Paused => "paused",
    }
}

/// A capability in the UI's words.
pub fn capability_words(c: Capability) -> &'static str {
    match c {
        Capability::Read => "read only",
        Capability::Write => "can change code",
        Capability::WriteApproval => "can change code, I approve before it's sent",
    }
}

/// The capability chip (`READ`, `WRITE`, `WRITE ✓`).
pub fn capability_chip(c: Capability) -> &'static str {
    match c {
        Capability::Read => "READ",
        Capability::Write => "WRITE",
        Capability::WriteApproval => "WRITE ✓",
    }
}

/// The same audience, for a draft that names nobody yet.
pub fn audience_is_empty(a: &Audience) -> bool {
    a.people.is_empty() && a.teams.is_empty() && a.groups.is_empty() && a.networks.is_empty()
}

/// The owner-side views and requests, for `src/gen`.
#[cfg(test)]
pub(crate) fn export(cfg: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    ShareView::export_all(cfg)?;
    ShareChanged::export_all(cfg)?;
    ShareCreate::export_all(cfg)?;
    ShareSet::export_all(cfg)?;
    ShareGuest::export_all(cfg)?;
    ShareBlock::export_all(cfg)?;
    ShareSummaryRequest::export_all(cfg)?;
    SetupLine::export_all(cfg)?;
    TeamDetail::export_all(cfg)?;
    TeamCreate::export_all(cfg)?;
    TeamJoin::export_all(cfg)?;
    TeamApprove::export_all(cfg)?;
    TeamAddMember::export_all(cfg)?;
    InviteLink::export_all(cfg)?;
    MemberAdded::export_all(cfg)?;
    PersonView::export_all(cfg)?;
    AskSend::export_all(cfg)?;
    AskFilter::export_all(cfg)?;
    CollabSettingsView::export_all(cfg)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_patch_says_only_what_changes() {
        let p = SharePatch {
            capability: Some(Capability::WriteApproval),
            add: AudienceEdit { teams: vec!["payments".into()], ..Default::default() },
            ..Default::default()
        };
        assert_eq!(serde_json::to_value(&p).unwrap(), json!({"capability": "write_approval", "add": {"teams": ["payments"]}}));
        let back: SharePatch = serde_json::from_value(json!({"remove": {"people": [{"provider": "tailscale", "subject": "bob@example.com"}]}})).unwrap();
        assert_eq!(back.remove.people[0].subject, "bob@example.com");
        assert!(back.add.is_empty());
        let block: ShareBlock = serde_json::from_value(json!({"person": "bob@example.com"})).unwrap();
        assert!(block.blocked, "blocking is the default");
    }

    #[test]
    fn collaboration_starts_off() {
        let s = CollabSettings::default();
        assert!(!s.use_teams && !s.agents_ask && s.reachable.is_empty());
        assert_eq!((s.agent_budget_per_hour, s.session_cap), (20, 4));
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["useTeams"], false);
        assert_eq!(v["lookFor"], "everyone");
        assert_eq!(serde_json::from_value::<CollabSettings>(json!({})).unwrap(), s);
    }

    #[test]
    fn words_and_chips() {
        assert_eq!(capability_chip(Capability::WriteApproval), "WRITE ✓");
        assert_eq!(capability_words(Capability::Read), "read only");
        assert_eq!(state_word(ShareState::Paused), "paused");
    }
}
