//! The core's two faces toward the network handler (§8.3, §8.4). Both
//! fronts (A2A and MCP) call [`Conversations`] and only translate; the
//! Pitwall team endpoints call [`Teams`]. Everything that matters (audience,
//! limits, sessions, replies, held results, the audit log) happens behind
//! these, once, whichever front a request came through.
//!
//! Errors: a share the asker may not use, an unknown share, turn or
//! conversation, and someone else's turn or conversation are all
//! [`NotFound`](crate::error::ErrorCode::NotFound) and must look the same
//! (A2A `404`, MCP "unknown tool"). Limits are not errors: a refused
//! message is a [`Turn`] with [`TurnStatus::Refused`](super::TurnStatus).

use std::sync::mpsc::Receiver;

use super::team::{InviteRedeem, JoinRequest, PendingJoin, PitwallInfo, TeamHead, TeamId, TeamRecord};
use super::turn::{Ask, Asker, ChangesPart, ConversationId, ConversationSummary, ShareCard, Turn, TurnId};
use crate::error::Result;
use crate::net::Caller;

/// Conversations with the owner's shared agents (§6, §7, §8.3).
pub trait Conversations: Send + Sync {
    /// Live shares this asker may use (§8.2): one MCP tool each
    /// (`tools/list`), the A2A cards and `GET /pitwall/v1/directory`. Empty
    /// for a caller outside every audience; a blocked person sees nothing.
    fn shares(&self, asker: &Asker) -> Vec<ShareCard>;

    /// Send one message (§7.1). Checks audience, share state, limits and
    /// the loop chain, writes the audit line, and returns at once with the
    /// turn queued or working (or refused); follow it with [`turn`] or
    /// [`watch`](Conversations::watch).
    ///
    /// [`turn`]: Conversations::turn
    fn ask(&self, asker: &Asker, ask: Ask) -> Result<Turn>;

    /// One of the asker's turns (A2A `GetTask`, MCP `check_turn`).
    fn turn(&self, asker: &Asker, turn: &TurnId) -> Result<Turn>;

    /// A turn as it changes: the current state first, then every change,
    /// each a whole [`Turn`]; the channel closes after a final state (A2A
    /// streaming and `SubscribeToTask`, MCP progress while waiting).
    fn watch(&self, asker: &Asker, turn: &TurnId) -> Result<Receiver<Turn>>;

    /// The asker's turns, oldest first; of one conversation when given (A2A
    /// `ListTasks`, `pitwall ask list`).
    fn turns(&self, asker: &Asker, conversation: Option<&ConversationId>) -> Result<Vec<Turn>>;

    /// Stop a running turn (as Stop for a turn) or drop a queued one (A2A
    /// `CancelTask`, MCP `notifications/cancelled`). A final turn stays as
    /// it is.
    fn cancel(&self, asker: &Asker, turn: &TurnId) -> Result<Turn>;

    /// A turn's whole patch series once sent (A2A `changes` artifact,
    /// `GET /pitwall/v1/results/{task}`). Held or discarded series: `NotFound`.
    fn patch(&self, asker: &Asker, turn: &TurnId) -> Result<String>;

    /// Part `part` (from 1) of the series in pieces of
    /// [`CHANGES_PART_CHARS`](super::CHANGES_PART_CHARS) (MCP `get_changes`).
    fn patch_part(&self, asker: &Asker, turn: &TurnId, part: u32) -> Result<ChangesPart>;

    /// The asker's conversations with this owner (`GET /pitwall/v1/sessions`).
    fn conversations(&self, asker: &Asker) -> Result<Vec<ConversationSummary>>;

    /// Forget one (`DELETE /pitwall/v1/sessions/{conversation}`): its
    /// session, copy and turns go.
    fn forget(&self, asker: &Asker, conversation: &ConversationId) -> Result<()>;

    /// A tick whenever the live shares or their audiences change (MCP
    /// `notifications/tools/list_changed`, the `directory.changed` event).
    /// Each call subscribes anew.
    fn shares_changed(&self) -> Receiver<()>;
}

/// The Pitwall team endpoints (§8.4) and replication (§4.3). Records carry
/// their own signatures; the caller only decides who may ask.
pub trait Teams: Send + Sync {
    /// `GET /pitwall/v1/info`, the discovery probe (§3). Any caller.
    fn info(&self, caller: &Caller) -> PitwallInfo;

    /// `GET /pitwall/v1/teams:discover`: the latest record of each listed
    /// team this Pitwall knows (never invite-only teams). Any caller.
    fn listed(&self, caller: &Caller) -> Vec<TeamRecord>;

    /// `POST /pitwall/v1/teams/{t}/heads`: the caller's heads for teams both
    /// are in; answers with ours, so the side behind pulls. Member devices.
    fn heads(&self, caller: &Caller, theirs: &[TeamHead]) -> Result<Vec<TeamHead>>;

    /// `GET /pitwall/v1/teams/{t}/records?after=n`. Member devices.
    fn records(&self, caller: &Caller, team: &TeamId, after: u64) -> Result<Vec<TeamRecord>>;

    /// `POST /pitwall/v1/teams/{t}/announce`: an admin changed the team;
    /// pull from the caller. Member devices.
    fn announce(&self, caller: &Caller, head: TeamHead) -> Result<()>;

    /// `POST /pitwall/v1/teams/{t}/join` (§4.6). Anyone with Pitwall.
    fn request_join(&self, caller: &Caller, team: &TeamId, request: JoinRequest) -> Result<()>;

    /// `POST /pitwall/v1/teams/{t}/invites:redeem` (§4.6).
    fn redeem_invite(&self, caller: &Caller, team: &TeamId, redeem: InviteRedeem) -> Result<()>;

    /// `GET /pitwall/v1/teams/{t}/requests`: pending join requests. Admins.
    fn join_requests(&self, caller: &Caller, team: &TeamId) -> Result<Vec<PendingJoin>>;
}
