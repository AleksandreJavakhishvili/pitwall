//! Collaboration's serde shapes (docs/spec/collaboration/): who people are on
//! a network (§15.2), a share's stored settings (§5.7), and what the UI and
//! the CLI are shown (§8.7). The traits and the logic are in
//! `pitwall-core::{net, collab}`; these are only the shapes that are stored
//! or cross the daemon protocol, so the CLI needn't pull in the engine.
//!
//! Every name here is network-neutral: identities, groups and network ids
//! are the same shapes on Tailscale, a Pitwall Cloud or the Fake network
//! (§15.8), so nothing is migrated when a second network arrives.
//!
//! Times are RFC 3339 strings (`2026-10-17T00:00:00+02:00`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ---------------------------------------------------------- who (§15.2)

/// One network backend instance: `<provider>:<name>`, e.g.
/// `tailscale:example.ts.net` or `cloud:acme`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub struct NetworkId(pub String);

impl NetworkId {
    pub fn new(provider: &str, name: &str) -> NetworkId {
        NetworkId(format!("{provider}:{name}"))
    }

    /// The part before the first `:` (`tailscale`).
    pub fn provider(&self) -> &str {
        self.0.split_once(':').map_or(self.0.as_str(), |(p, _)| p)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NetworkId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Who a network says someone is. `subject` is the provider's stable id
/// (a Tailscale login name, a Cloud account id), never a display name.
/// Two identities are the same login when [`Identity::is`] says so; `email`
/// and `name` are for display only and never used to match (§15.7).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub struct Identity {
    pub provider: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
}

impl Identity {
    pub fn new(provider: &str, subject: &str) -> Identity {
        Identity { provider: provider.into(), subject: subject.into(), email: None, name: None }
    }

    /// Same provider and subject: the same login, whatever the display fields.
    pub fn is(&self, other: &Identity) -> bool {
        self.provider == other.provider && self.subject == other.subject
    }

    /// `provider:subject`, the default key of an identity nobody linked.
    pub fn key(&self) -> String {
        format!("{}:{}", self.provider, self.subject)
    }
}

/// A person: one or more identities known to be the same human (§15.7).
/// Audiences, conversations and limits are keyed by it on the owner's side.
/// An identity nobody linked is its own person, keyed `provider:subject`
/// ([`PersonId::of`]); linked identities share one id chosen by the owner's
/// Pitwall.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub struct PersonId(pub String);

impl PersonId {
    /// The person an unlinked identity is.
    pub fn of(identity: &Identity) -> PersonId {
        PersonId(identity.key())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where one device is on one network: `node` is the network's stable id
/// for it (a Tailscale `StableID`, a Cloud device id).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub struct DeviceAddr {
    pub network: NetworkId,
    pub node: String,
}

/// A group the network's provider manages (a Tailscale group carried as an
/// app capability, a Cloud or SSO group), §4.8. Scoped to one network
/// instance: `group:platform` on two tailnets is two groups.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub struct GroupRef {
    pub network: NetworkId,
    pub id: String,
}

/// A team's id: the hash of its genesis record (`t_9f2c…`), §4.1. Names
/// the team forever; two teams called "Platform" are different ids.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub struct TeamId(pub String);

impl TeamId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ------------------------------------------------- share settings (§5.7)

/// What a shared agent may do (§5.2). A property of the share, enforced by
/// the session's sandbox, never per message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Replies only; its copy is not writable.
    #[default]
    Read,
    /// Can change its copy; changes come back as a patch series.
    Write,
    /// As `Write`, but each turn's changes wait for the owner's Send (§7.5).
    WriteApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ShareState {
    /// Only the owner (and their guest sessions) can see it.
    #[default]
    Draft,
    Live,
    Paused,
}

/// A share's stored settings, `collab/shares/<slug>.json` (§5.7). Callers
/// never see this: only a [`DirectoryEntry`] / the share's tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShareSettings {
    /// The ordinary agent being shared (its agent id).
    pub agent: String,
    /// `api-expert`, as in `alice/api-expert`; unique per owner.
    pub slug: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub state: ShareState,
    #[serde(default)]
    pub capability: Capability,
    #[serde(default)]
    pub audience: Audience,
    pub scope: Scope,
    #[serde(default)]
    pub start: Start,
    #[serde(default)]
    pub access: Access,
    /// Soft: written into the session's instructions, "not guaranteed".
    #[serde(default)]
    pub off_limits: String,
    #[serde(default)]
    pub limits: Limits,
}

/// Who may send messages (§5.4 step 1). A caller is in it when its person
/// is named, is a member of one of the teams, has one of the groups, or
/// arrives on one of the networks (§8.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Audience {
    #[serde(default)]
    pub people: Vec<Identity>,
    #[serde(default)]
    pub teams: Vec<TeamId>,
    #[serde(default)]
    pub groups: Vec<GroupRef>,
    /// Everyone on these networks (amber in the UI).
    #[serde(default)]
    pub networks: Vec<NetworkId>,
}

/// What the session's copy holds (§5.4 step 3, §6.2). Never shown to callers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Scope {
    pub repo: String,
    /// The branch or commit the copy is made from.
    #[serde(rename = "ref")]
    pub git_ref: String,
    /// Picked paths, relative to `repo` (`src/`).
    #[serde(default)]
    pub paths: Vec<String>,
    /// Left out: the sensitive-file scan's findings plus the owner's own.
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub uncommitted: bool,
    /// Always false in v1 (§14 question 12).
    #[serde(default)]
    pub history: bool,
}

/// Where new sessions start (§5.4 step 4, §6.4).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum Start {
    #[default]
    Fresh,
    /// The owner-approved summary; only this text reaches sessions.
    Summary {
        text: String,
        #[serde(rename = "approvedAt")]
        approved_at: String,
    },
}

/// Secrets and access (§5.4 step 5): what the session's environment and
/// sandbox let through.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct Access {
    /// Names of the owner's environment variables passed in (none by default).
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub network: NetworkAccess,
    /// The agent account (login) sessions run on, by id (`accounts.json`);
    /// `None`: the owner's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub account: Option<String>,
    /// Programs it may run (§5.4 step 5, §6.3). `None`: the defaults for the
    /// capability (an allowlist at Read only, a denylist at Can change code,
    /// §14 question 16). Not in §5.7's example; added here so the answer has
    /// a home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub programs: Option<Programs>,
}

/// Hosts the egress proxy allows besides the kind's model hosts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct NetworkAccess {
    #[serde(default)]
    pub allow: Vec<String>,
}

/// An allow- or denylist of executables (names such as `git`, or paths).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "mode", content = "list", rename_all = "snake_case")]
pub enum Programs {
    Allow(Vec<String>),
    Deny(Vec<String>),
}

/// The owner's limits (§5.8). Defaults are the dialog's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct Limits {
    pub per_caller: PerCaller,
    /// Conversations running at once for this share.
    pub sessions_running: u32,
    /// Messages waiting for a free session before `busy`.
    pub queue: u32,
    /// A longer turn is stopped and fails with `timeout`.
    pub turn_timeout_min: u32,
    /// Agent time per day, all callers together; then `share_total`.
    pub agent_minutes_per_day: u32,
    /// Per person, keyed by [`PersonId`].
    pub overrides: BTreeMap<String, LimitOverride>,
    /// A session with nothing in flight stops after this long (and resumes).
    pub idle_stop_min: u32,
    /// Conversations are kept this long after their last message.
    pub keep_sessions_days: u32,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            per_caller: PerCaller::default(),
            sessions_running: 2,
            queue: 10,
            turn_timeout_min: 15,
            agent_minutes_per_day: 120,
            overrides: BTreeMap::new(),
            idle_stop_min: 15,
            keep_sessions_days: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct PerCaller {
    /// Reset at the owner's local midnight; then `quota`.
    pub messages_per_day: u32,
}

impl Default for PerCaller {
    fn default() -> PerCaller {
        PerCaller { messages_per_day: 20 }
    }
}

/// One person's exception: a different daily limit, or blocked (the share
/// disappears for them).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LimitOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub messages_per_day: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub blocked: bool,
}

// ------------------------------------------- turns, shared by core and views

/// Why a message was refused by a limit (§5.8, §8.3.2 `rejected`). The same
/// reasons on both fronts and as CLI error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    /// The caller's messages for today are used up.
    Quota,
    /// Every session is busy and the queue is full.
    Busy,
    /// The owner paused the share.
    Paused,
    /// The share's agent time for today is used up.
    ShareTotal,
    /// The agent's own vendor limit; `until` when the agent showed it.
    AgentLimit,
    /// The message came through this share already (`Pitwall-Chain`, §8.6).
    Loop,
}

/// Where a reply was read from (§7.2), shown to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum ReplySource {
    Hook,
    SessionFile,
    NonInteractive,
    /// From the terminal output; marked as such to the caller.
    Terminal,
}

/// A turn's patch series and where it is (§7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ChangesState {
    /// Waiting for the owner's Send (`write_approval`).
    Held,
    Sent,
    Discarded,
}

/// Which front a conversation came in through (§8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Front {
    A2a,
    Mcp,
}

// ---------------------------------------------------------- views (§8.7)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum JoinPolicy {
    #[default]
    Request,
    Closed,
}

/// My standing in a team.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    Member,
    Admin,
    Requested,
}

/// A team on People → Teams (mine, or discovered).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TeamCard {
    pub team: TeamId,
    pub name: String,
    pub description: String,
    /// Admins by identity, for display (`alice@example.com`).
    pub admins: Vec<String>,
    pub members: u32,
    #[ts(type = "number")]
    pub version: u64,
    pub listed: bool,
    pub join: JoinPolicy,
    /// The team id's first 6 characters in groups (`9F2-C1A`), §4.5.
    pub fingerprint: String,
    /// Follows a provider's group (§4.8).
    pub bound: bool,
    /// `None`: not mine (discovered).
    pub mine: Option<TeamRole>,
}

/// A shared agent I may use (People → Directory, the Ask panel's picker).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub card_url: String,
    /// The share's A2A Agent Card, as served.
    #[ts(type = "Record<string, unknown>")]
    pub card: serde_json::Value,
    /// The owner, by identity for display.
    pub owner: String,
    pub slug: String,
    pub capability: Capability,
    /// The agent kind's id (`claude`).
    pub kind: String,
    /// Messages left for me today; `None`: unknown.
    pub left: Option<u32>,
    pub online: bool,
    pub last_seen: Option<String>,
    /// Why I may use it: team ids, group ids, `direct`, or a network id.
    pub via: Vec<String>,
}

/// Who wrote a message in my thread with a shared agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AskFrom {
    Me,
    /// The owner typed into the session (§7.3).
    Owner,
}

/// A turn as the caller sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AskStatus {
    Queued,
    Working,
    WaitingForOwner,
    Done,
    Failed,
    Canceled,
    Refused,
}

/// A turn's patch series, summed up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChangesView {
    pub state: ChangesState,
    pub files: u32,
    pub added: u32,
    pub removed: u32,
    pub base_commit: String,
    pub patches: u32,
    /// Where the caller stored it (`collab/results/<turn>/`), once received.
    pub path: Option<String>,
}

/// One message and its reply in the caller's thread (Ask panel,
/// `pitwall ask list`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AskView {
    pub turn: String,
    pub conversation: String,
    /// `alice/api-expert`.
    pub share: String,
    pub text: String,
    pub from: AskFrom,
    pub status: AskStatus,
    /// "2nd in line", "waiting for alice", "changes waiting for alice's OK",
    /// or a refusal's sentence.
    pub status_message: Option<String>,
    pub reply: Option<String>,
    pub reply_source: Option<ReplySource>,
    pub changes: Option<ChangesView>,
    pub at: String,
    pub done_at: Option<String>,
}

/// A shared session's state on the owner's side (§6.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    New,
    Starting,
    Ready,
    Working,
    /// Needs the owner (§7.3).
    Blocked,
    /// Idle-stopped; resumes on the next message.
    Stopped,
    Closed,
}

/// Who a shared session belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionCaller {
    /// The person, by identity for display.
    pub login: String,
    /// The asking agent's `Pitwall-Agent` label (`client-fix@bob-mbp`).
    pub agent: Option<String>,
    /// A plain MCP client's name from `initialize` (`Claude Code`).
    pub client: Option<String>,
    pub front: Front,
    /// The owner's own try-as-guest session (§5.4).
    pub guest: bool,
    /// Asked from a Pitwall (A2A with Pitwall's labels).
    pub pitwall: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsage {
    pub minutes_today: u32,
    pub messages_today: u32,
    /// When the agent reports them.
    #[ts(type = "number | null")]
    pub tokens: Option<u64>,
}

/// One caller's conversation with one of my shared agents (owner sidebar,
/// `pitwall share sessions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SharedSessionView {
    pub conversation: String,
    /// The share's slug.
    pub share: String,
    pub caller: SessionCaller,
    pub state: SessionState,
    /// The ordinary engine agent the session is.
    pub agent_id: String,
    pub turns: u32,
    pub results: u32,
    /// Results waiting for my Send.
    pub held: u32,
    pub usage: SessionUsage,
    pub last_at: Option<String>,
}

/// One changed share setting (`share.set`, the Share dialog's Save).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ShareDiffLine {
    /// Dotted path into [`ShareSettings`] (`audience.teams`, `limits.queue`).
    pub field: String,
    #[ts(type = "unknown")]
    pub from: serde_json::Value,
    #[ts(type = "unknown")]
    pub to: serde_json::Value,
    /// Widening always asks and can't be remembered (§5.5).
    pub widens: bool,
}

/// What a change to a share's settings did, line by line.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct ShareDiff(pub Vec<ShareDiffLine>);

impl ShareDiff {
    pub fn widens(&self) -> bool {
        self.0.iter().any(|l| l.widens)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// §5.7's example, with the two §15.8 adjustments (groups and override
    /// keys use the network-neutral shapes).
    fn golden_share() -> serde_json::Value {
        json!({
            "agent": "a1b2",
            "slug": "api-expert",
            "name": "API expert",
            "description": "Questions about the API service: auth, billing, jobs",
            "state": "draft",
            "capability": "write_approval",
            "audience": {
                "people": [{"provider": "tailscale", "subject": "carol@example.com"}],
                "teams": ["t_9f2c"],
                "groups": [{"network": "tailscale:example.ts.net", "id": "platform"}],
                "networks": ["tailscale:example.ts.net"]
            },
            "scope": {
                "repo": "~/code/api", "ref": "main",
                "paths": ["src/", "docs/"],
                "exclude": [".env", "config/master.key", "deploy/id_ed25519"],
                "uncommitted": false, "history": false
            },
            "start": {"from": "fresh"},
            "access": {"env": [], "network": {"allow": []}, "account": "work"},
            "offLimits": "Don't discuss pricing or the reorg",
            "limits": {
                "perCaller": {"messagesPerDay": 20}, "sessionsRunning": 2,
                "queue": 10, "turnTimeoutMin": 15, "agentMinutesPerDay": 120,
                "overrides": {
                    "tailscale:dave@example.com": {"messagesPerDay": 50},
                    "tailscale:eve@example.com": {"blocked": true}
                },
                "idleStopMin": 15, "keepSessionsDays": 30
            }
        })
    }

    #[test]
    fn share_settings_match_the_spec_shape_both_ways() {
        let golden = golden_share();
        let share: ShareSettings = serde_json::from_value(golden.clone()).unwrap();
        assert_eq!(share.capability, Capability::WriteApproval);
        assert_eq!(share.scope.git_ref, "main");
        assert_eq!(share.access.account.as_deref(), Some("work"));
        assert!(share.limits.overrides["tailscale:eve@example.com"].blocked);
        assert_eq!(share.audience.networks[0].provider(), "tailscale");
        assert_eq!(serde_json::to_value(&share).unwrap(), golden);
    }

    #[test]
    fn a_minimal_share_takes_the_dialog_defaults() {
        let share: ShareSettings = serde_json::from_value(json!({
            "agent": "a1", "slug": "docs", "name": "Docs", "description": "",
            "scope": {"repo": "~/code/my-app", "ref": "main"}
        }))
        .unwrap();
        assert_eq!(share.state, ShareState::Draft);
        assert_eq!(share.capability, Capability::Read);
        assert_eq!(share.start, Start::Fresh);
        assert!(share.audience.people.is_empty() && share.access.env.is_empty());
        assert_eq!(share.limits, Limits::default());
        assert_eq!(share.limits.per_caller.messages_per_day, 20);
        assert_eq!(share.limits.agent_minutes_per_day, 120);
    }

    #[test]
    fn summary_start_and_programs_round_trip() {
        let start = json!({"from": "summary", "text": "We use JWTs.", "approvedAt": "2026-10-16T09:00:00Z"});
        let parsed: Start = serde_json::from_value(start.clone()).unwrap();
        assert!(matches!(&parsed, Start::Summary { text, .. } if text == "We use JWTs."));
        assert_eq!(serde_json::to_value(&parsed).unwrap(), start);

        let programs = json!({"mode": "allow", "list": ["git", "rg"]});
        let parsed: Programs = serde_json::from_value(programs.clone()).unwrap();
        assert_eq!(parsed, Programs::Allow(vec!["git".into(), "rg".into()]));
        assert_eq!(serde_json::to_value(&parsed).unwrap(), programs);
    }

    #[test]
    fn identities_match_by_provider_and_subject_only() {
        let a = Identity::new("tailscale", "bob@example.com");
        let b = Identity { email: Some("bob@example.com".into()), ..a.clone() };
        assert!(a.is(&b));
        assert!(!a.is(&Identity::new("cloud", "bob@example.com")));
        assert_eq!(PersonId::of(&a).as_str(), "tailscale:bob@example.com");
        assert_eq!(serde_json::to_value(&a).unwrap(), json!({"provider": "tailscale", "subject": "bob@example.com"}));
        assert_eq!(NetworkId::new("cloud", "acme").as_str(), "cloud:acme");
    }

    #[test]
    fn refusals_and_reply_sources_use_the_spec_words() {
        let words: Vec<_> = [
            RefusalReason::Quota,
            RefusalReason::Busy,
            RefusalReason::Paused,
            RefusalReason::ShareTotal,
            RefusalReason::AgentLimit,
            RefusalReason::Loop,
        ]
        .iter()
        .map(|r| serde_json::to_value(r).unwrap())
        .collect();
        assert_eq!(words, ["quota", "busy", "paused", "share_total", "agent_limit", "loop"]);
        assert_eq!(serde_json::to_value(ReplySource::SessionFile).unwrap(), "session-file");
        assert_eq!(serde_json::to_value(ReplySource::NonInteractive).unwrap(), "non-interactive");
        assert_eq!(serde_json::to_value(Front::A2a).unwrap(), "a2a");
    }

    #[test]
    fn views_serialize_in_the_spec_shape() {
        let card = TeamCard {
            team: TeamId("t_9f2c".into()),
            name: "Platform".into(),
            description: "API, jobs, infra".into(),
            admins: vec!["alice@example.com".into()],
            members: 6,
            version: 7,
            listed: true,
            join: JoinPolicy::Request,
            fingerprint: "9F2-C1A".into(),
            bound: false,
            mine: None,
        };
        let v = serde_json::to_value(&card).unwrap();
        assert_eq!(v["mine"], serde_json::Value::Null);
        assert_eq!(v["join"], "request");

        let ask = AskView {
            turn: "c_8f2k/7".into(),
            conversation: "c_8f2k".into(),
            share: "alice/api-expert".into(),
            text: "where are refresh tokens rotated?".into(),
            from: AskFrom::Me,
            status: AskStatus::WaitingForOwner,
            status_message: Some("waiting for alice".into()),
            reply: None,
            reply_source: None,
            changes: Some(ChangesView {
                state: ChangesState::Held,
                files: 2,
                added: 14,
                removed: 3,
                base_commit: "3f2a1c9".into(),
                patches: 1,
                path: None,
            }),
            at: "2026-10-16T09:12:00Z".into(),
            done_at: None,
        };
        let v = serde_json::to_value(&ask).unwrap();
        assert_eq!(v["status"], "waiting_for_owner");
        assert_eq!(v["statusMessage"], "waiting for alice");
        assert_eq!(v["changes"]["baseCommit"], "3f2a1c9");
        assert_eq!(serde_json::from_value::<AskView>(v).unwrap(), ask);

        let diff = ShareDiff(vec![ShareDiffLine {
            field: "audience.people".into(),
            from: json!([]),
            to: json!([{"provider": "tailscale", "subject": "dave@example.com"}]),
            widens: true,
        }]);
        assert!(diff.widens());
        assert!(serde_json::to_value(&diff).unwrap().is_array());
    }
}
