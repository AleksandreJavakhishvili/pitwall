//! Collaboration: teams and shared agents (docs/spec/collaboration/). People
//! on the same network share some of their agents; others send them
//! messages, each in their own conversation, over A2A (Pitwall callers) or
//! MCP (plain Claude Code), both fronts over one core (§8.3).
//!
//! This module holds the contract the pieces are built against:
//! - [`turn`]: conversations, turns, replies, changes and refusals;
//! - [`team`]: teams as signed records (§4.2) and the team endpoints' shapes;
//! - [`Conversations`] and [`Teams`]: what the network handler calls;
//! - share settings are [`ShareSettings`] (stored, §5.7), in `pitwall-proto`
//!   so the CLI can read them.
//!
//! Collaboration sees networks only through [`crate::net`]: never a backend
//! module (`net::tailscale`, `net::cloud`), checked by `tests/boundaries.rs`
//! (§15.8). Its sessions are sandboxed through
//! [`crate::sandbox::SessionSandbox`].

pub mod audit;
mod core;
pub mod engine_host;
pub mod front;
pub mod live;
pub mod people;
pub mod sandboxed;
mod service;
pub mod session;
pub mod session_file;
pub mod shares;
pub mod store;
pub mod team;
pub mod teams;
pub mod time;
pub mod turn;
pub mod usage;

#[cfg(test)]
mod interop;
#[cfg(test)]
mod tests;

pub use pitwall_proto::collab::{
    Access, Audience, Capability, LimitOverride, Limits, PerCaller, Programs, RefusalReason, Scope, ShareSettings,
    ShareState, Start,
};
pub use self::core::{Collab, CollabDeps, CollabSettings, HookWiring, Notice, OwnerTurn, RunMode, DEFAULT_MAX_SESSIONS};
pub use service::{Conversations, Teams};
pub use turn::{
    Ask, Asker, Changes, ChangesPart, ConversationId, ConversationSummary, Failure, Left, Refusal, Reply, ShareCard,
    Turn, TurnId, TurnStatus, Who,
};

/// Request bodies over this are refused (§8.1).
pub const MAX_BODY: usize = 2 * 1024 * 1024;
/// Open streams per caller (§8.1).
pub const MAX_STREAMS: usize = 8;
/// A patch series inlines in an MCP result up to this many characters, and
/// comes through `get_changes` in parts of this size above it (§8.3.3).
pub const CHANGES_PART_CHARS: usize = 40_000;
/// MCP `wait_seconds`: default and cap, below Claude Code's 5-minute idle
/// timeout for HTTP tool calls (§8.3.3).
pub const WAIT_SECONDS_DEFAULT: u32 = 120;
pub const WAIT_SECONDS_MAX: u32 = 240;
/// `Pitwall-Chain` longer than this is refused with `loop` (§8.6).
pub const MAX_CHAIN_DEPTH: usize = 2;

/// Plain headers a Pitwall caller may add on either front (§8.2): labels,
/// trusted as much as the person whose device sent them, never access.
pub const AGENT_HEADER: &str = "Pitwall-Agent";
pub const CHAIN_HEADER: &str = "Pitwall-Chain";
/// A2A: the version header every request carries, and the message metadata
/// key that starts a new conversation (§8.3.2).
pub const A2A_VERSION: &str = "1.0";
pub const NEW_CONVERSATION_META: &str = "pitwall/newConversation";
