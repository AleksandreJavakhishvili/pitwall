//! Conversations and turns as the core hands them to both fronts (§8.3).
//! A conversation id is the A2A `contextId` and the MCP `conversation`; a
//! turn id is the A2A task id and the MCP `turn`, so one conversation
//! continues across fronts.

use serde::{Deserialize, Serialize};

use crate::net::{Caller, Identity};
pub use pitwall_proto::collab::{Capability, ChangesState, Front, RefusalReason, ReplySource};

/// One caller's conversation with one shared agent (`c_8f2k`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConversationId(pub String);

impl ConversationId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One message and what came of it: `<conversation>/<n>` (`c_8f2k/7`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TurnId(pub String);

impl TurnId {
    pub fn new(conversation: &ConversationId, n: u32) -> TurnId {
        TurnId(format!("{}/{n}", conversation.0))
    }

    pub fn conversation(&self) -> ConversationId {
        ConversationId(self.0.rsplit_once('/').map_or(self.0.as_str(), |(c, _)| c).to_string())
    }

    /// Its number in the conversation, from 1.
    pub fn number(&self) -> Option<u32> {
        self.0.rsplit_once('/').and_then(|(_, n)| n.parse().ok())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where a turn is. The fronts map it: A2A task states (§8.3.2) and MCP's
/// `status` (§8.3.3).
///
/// | here | A2A | MCP `status` |
/// |---|---|---|
/// | `queued` | `SUBMITTED` ("2nd in line") | `working` |
/// | `working` | `WORKING` | `working` |
/// | `waiting_for_owner` | `WORKING` ("waiting for alice") | `waiting_for_owner` |
/// | `changes_held` | `WORKING` ("changes waiting for alice's OK") | `waiting_for_owner` |
/// | `done` | `COMPLETED` | `done` |
/// | `failed` | `FAILED` | `failed` |
/// | `canceled` | `CANCELED` | `failed` |
/// | `refused` | `REJECTED` | `isError` with `refused` |
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TurnStatus {
    /// Waiting for a free session; `position` 1 is next.
    Queued { position: u32 },
    Working,
    /// The agent needs the owner (§7.3).
    WaitingForOwner,
    /// The reply is out; its changes wait for the owner's Send (§7.5).
    ChangesHeld,
    Done,
    /// Whatever reply exists is in [`Turn::reply`].
    Failed { reason: Failure },
    /// By the caller, or the owner stopped sharing mid-turn.
    Canceled,
    Refused(Refusal),
}

impl TurnStatus {
    /// Nothing more will happen to the turn.
    pub fn is_final(&self) -> bool {
        matches!(self, TurnStatus::Done | TurnStatus::Failed { .. } | TurnStatus::Canceled | TurnStatus::Refused(_))
    }
}

/// Why a turn failed (§7.2, §6.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    /// Nothing was captured when the turn ended.
    NoAnswer,
    /// Over `turnTimeoutMin`.
    Timeout,
    /// The agent exited.
    Crashed,
}

/// A limit said no (§5.8). `message` is the plain sentence both fronts show
/// ("Daily limit reached for alice/api-expert; resets at 00:00").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Refusal {
    pub reason: RefusalReason,
    pub message: String,
    /// When a daily limit resets (`quota`, `share_total`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    /// Until when the agent's vendor limit holds (`agent_limit`), when it
    /// showed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

/// What the agent said in the turn (§7.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub text: String,
    pub source: ReplySource,
}

/// A turn's patch series, summed up (§7.5). The text itself comes from
/// [`Conversations::patch`](super::Conversations::patch).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    pub state: ChangesState,
    /// The owner's real commit the series applies to.
    pub base_commit: String,
    pub files: u32,
    pub added: u32,
    pub removed: u32,
    pub patches: u32,
    /// Parts of [`CHANGES_PART_CHARS`](super::CHANGES_PART_CHARS) for
    /// `get_changes`; 1 when it fits inline.
    pub parts: u32,
}

/// One part of a patch series (MCP `get_changes`, §8.3.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangesPart {
    pub turn: TurnId,
    /// From 1.
    pub part: u32,
    pub parts: u32,
    pub text: String,
}

/// What is left for the caller today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Left {
    pub messages_today: u32,
    pub resets_at: String,
}

/// One turn, as both fronts report it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub id: TurnId,
    pub conversation: ConversationId,
    /// The share's slug.
    pub share: String,
    pub status: TurnStatus,
    /// A short line for people ("2nd in line", "waiting for alice").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<Reply>,
    /// Only at "Can change code", when the turn changed the copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<Changes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<Left>,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<String>,
}

/// One message (§7.1): the caller's text, delivered verbatim.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    /// The share's slug.
    pub share: String,
    pub message: String,
    /// One of the caller's conversations (A2A `contextId`); `None`: the
    /// caller's current one, created if there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<ConversationId>,
    /// Start a new conversation (A2A `pitwall/newConversation`, MCP
    /// `new_conversation`, CLI `--new`).
    #[serde(default)]
    pub new_conversation: bool,
    /// "Redo on my code": the caller's commits as a patch series, applied
    /// to the copy before the turn (§7.5 step 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_changes: Option<String>,
}

/// Who is asking, as a front hands it to the core: the network's proof
/// plus labels the request carried. Labels key the conversation (§6.1) and
/// name it for the owner; they never widen access, which is the person's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asker {
    pub who: Who,
    /// `Pitwall-Agent` (`client-fix@bob-mbp`), §8.2.
    pub agent: Option<String>,
    /// A plain MCP client's name from `initialize` (`Claude Code`).
    pub client: Option<String>,
    /// `Pitwall-Chain`: turns and shares the message came through (§8.6).
    pub chain: Vec<String>,
    pub front: Front,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Who {
    /// Someone a network proved.
    Network(Box<Caller>),
    /// The owner trying a draft as a guest from their own Ask panel (§5.4);
    /// never arrives over a network.
    Guest,
}

/// A share as a caller may see it (§5.7 "Callers see only the share's
/// tool"): the fronts build the A2A Agent Card and the MCP tool from it.
/// Never paths, branches, accounts, exclusions, skills or other callers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareCard {
    /// The owner, as the caller's network knows them.
    pub owner: Identity,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub capability: Capability,
    /// The agent kind's id and name (`claude`, "Claude Code").
    pub kind: String,
    pub kind_name: String,
    /// Changes whenever the card does (A2A `version`).
    pub version: u64,
    /// The caller's daily limit and what is left of it.
    pub messages_per_day: u32,
    pub left: u32,
}

/// One of the caller's conversations with this owner (`GET /sessions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: ConversationId,
    pub share: String,
    pub turns: u32,
    /// The one a message without a conversation goes to.
    pub current: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_at: Option<String>,
}
