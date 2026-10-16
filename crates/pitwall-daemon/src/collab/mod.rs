//! Collaboration's local methods (docs/spec/collaboration/protocol.md
//! §8.5): `share.*`, `team.*`, `people.list`, `ask.*`, `collab.settings.*`
//! for the owner's UI, CLI and agents.
//!
//! - [`CollabBackend`]: what holds shares, teams, people and conversations:
//!   [`core::CoreCollab`] over the core's `Collab` service in Pitwall;
//!   [`memory::MemoryCollab`] in tests and as the fallback when the core
//!   can't open.
//! - [`service::CollabService`]: who may call what (the permission matrix),
//!   approvals with the exact [`ShareDiff`](pitwall_proto::collab::ShareDiff)
//!   for widening, summaries, the sensitive-file scan, budgets for agents
//!   that ask. The socket server and the app's own window both call it.
//! - [`diff`], [`summary`], [`scan`]: the pure parts.
//!
//! Shared sessions never get here: every method on the socket answers them
//! `forbidden` (`methods::dispatch`).

pub mod core;
pub mod diff;
pub mod memory;
pub mod scan;
pub mod service;
pub mod summary;

#[cfg(test)]
mod tests;

use pitwall_proto::api::collab::{
    AskSend, CollabSettings, InviteLink, MemberAdded, NetworkRow, PersonView, ProbeResult, SetupLine, ShareUsage, TeamCreate,
    TeamDetail, TeamJoin,
};
use pitwall_proto::collab::{AskView, DirectoryEntry, Identity, ShareSettings, SharedSessionView, TeamCard, TeamId};
use pitwall_proto::ErrorBody;

pub use service::{CollabService, Who};

/// Me, as my networks know me.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Me {
    /// `alice@example.com`.
    pub login: String,
    /// `alice`: the first part of addresses (`alice/api-expert`) and of the
    /// MCP setup line.
    pub name: String,
}

/// What holds collaboration's state on the owner's machine:
/// [`core::CoreCollab`] in Pitwall, [`memory::MemoryCollab`] in tests. Calls come on connection threads and may block (`ask` waits for a
/// turn). Decisions about **who may** do something are the
/// [`CollabService`]'s, never the backend's: by the time a call gets here
/// it was allowed (and approved where needed).
pub trait CollabBackend: Send + Sync {
    fn me(&self) -> Me;

    /// `agent_id` is one of the shared sessions (a conversation's own
    /// agent): its processes may not use the socket at all.
    fn is_shared_session(&self, agent_id: &str) -> bool;

    // --- settings
    fn settings(&self) -> CollabSettings;
    fn set_settings(&self, s: CollabSettings) -> Result<(), ErrorBody>;
    /// Settings → Networks (`reachable` from [`settings`](Self::settings)).
    fn networks(&self) -> Vec<NetworkRow>;
    /// The `claude mcp add` line per network this machine is reachable on.
    fn setup_lines(&self) -> Vec<SetupLine>;

    // --- shares (stored settings, §5.7)
    fn shares(&self) -> Vec<ShareSettings>;
    /// Create or replace (by slug; `previous` when the slug changed).
    fn save_share(&self, s: &ShareSettings, previous: Option<&str>) -> Result<(), ErrorBody>;
    /// Publish `s` (a draft, now `live`): approved by the owner already.
    /// The guest conversations tried on the draft go unless `keep_guest`.
    fn go_live(&self, s: &ShareSettings, keep_guest: bool) -> Result<(), ErrorBody> {
        let _ = keep_guest;
        self.save_share(s, None)
    }
    /// Stop sharing: the share and its sessions go; the agent stays.
    fn remove_share(&self, slug: &str) -> Result<(), ErrorBody>;
    fn sessions(&self, slug: &str) -> Vec<SharedSessionView>;
    fn usage(&self, slug: &str) -> ShareUsage;
    /// Guest messages sent to this share, oldest first.
    fn probes(&self, slug: &str) -> Vec<ProbeResult>;
    /// One guest message (`probe`: its id, or `message`), through the same
    /// pipeline as a real caller (§5.4).
    fn guest(&self, slug: &str, probe: &str, message: &str) -> Result<ProbeResult, ErrorBody>;
    /// Person ids blocked from all my shares.
    fn blocked(&self) -> Vec<String>;
    fn set_blocked(&self, person: &str, blocked: bool) -> Result<(), ErrorBody>;
    /// The sensitive-file scan of a scope (default: [`scan::PatternScan`]).
    fn scan(&self, scope: &pitwall_proto::collab::Scope) -> scan::ScanResult {
        use scan::Scanner;
        scan::PatternScan::default().scan(scope)
    }

    // --- teams (§4)
    /// Mine, then discovered ones (`mine: None`).
    fn teams(&self) -> Vec<TeamCard>;
    fn team(&self, team: &TeamId) -> Option<TeamDetail>;
    fn create_team(&self, req: &TeamCreate) -> Result<TeamDetail, ErrorBody>;
    fn join_team(&self, req: &TeamJoin) -> Result<TeamCard, ErrorBody>;
    fn leave_team(&self, team: &TeamId) -> Result<(), ErrorBody>;
    fn approve(&self, team: &TeamId, person: &str, allow: bool) -> Result<TeamDetail, ErrorBody>;
    fn invite(&self, team: &TeamId) -> Result<InviteLink, ErrorBody>;
    fn add_member(&self, team: &TeamId, who: &Identity) -> Result<MemberAdded, ErrorBody>;

    // --- people and asking (caller side)
    fn people(&self) -> Vec<PersonView>;
    /// Shared agents I may use (People → Directory, the Ask panel).
    fn directory(&self) -> Vec<DirectoryEntry>;
    /// Send one message and wait for the turn, up to `req.timeout`.
    /// `agent`: the asking agent's label (`client-fix@bob-mbp`), from the
    /// process tree, never from an argument.
    fn ask(&self, req: &AskSend, agent: Option<&str>) -> Result<AskView, ErrorBody>;
    /// My messages and replies, oldest first.
    fn asks(&self) -> Vec<AskView>;
}
