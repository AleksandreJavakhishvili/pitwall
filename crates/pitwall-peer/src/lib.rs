//! Pitwall peers over Tailscale, phase 1 (docs/spec/tailnet-peers.md): a
//! Pitwall shows the agents of the user's other machines, read-only (list,
//! status, needs-you), next to its own.
//!
//! - [`tailnet`]: the local Tailscale (`status`, `whois`) behind
//!   [`Tailnet`]; [`TailscaleCli`] is the real one, [`fake`] a scripted one
//!   for tests.
//! - [`key`]: this Pitwall's Ed25519 instance key (`peers/identity.key`).
//! - [`store`]: settings, pairings (`peers.json`) and the audit log.
//! - [`server`]: the listener ("Let my other machines see this Pitwall's
//!   agents"), on this node's Tailscale addresses only, for machines of the
//!   same Tailscale user; View only.
//! - [`link`]: one link per paired peer and the [`Mirror`] of its agents.
//! - [`peers`]: [`Peers`], what the app calls.
//!
//! Only the user's own machines take part: same Tailscale user (id and
//! login), never tagged devices or other users. Pairing needs no code, but
//! each side proves its Pitwall key on every connection.

#![cfg_attr(test, recursion_limit = "256")]

mod dial;
#[cfg(any(test, feature = "testing"))]
pub mod fake;
pub mod key;
pub mod link;
pub mod peers;
mod platform;
pub mod server;
pub mod store;
pub mod tailnet;

#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use pitwall_proto::AttentionView;

pub use key::InstanceKey;
pub use link::Mirror;
pub use peers::{Candidate, PeerEntry, Peers, PeersConfig, Probe, TailnetInfo};
pub use server::{EngineFeed, Feed};
pub use store::{AuditEntry, Paired, PeerSettings};
pub use tailnet::{Tailnet, TailnetState, TailscaleCli};

/// What changed, for the host (the app turns these into its own events).
#[derive(Debug, Clone, PartialEq)]
pub enum PeerEvent {
    /// A peer's agents or its link changed: [`Peers::views`] has the new
    /// list.
    AgentsChanged { peer: String },
    LinkChanged { peer: String, link: pitwall_proto::peer::LinkView },
    /// An agent on a peer turned blocked or done (live only, never from
    /// stale data); `agent_id` is the mirrored id.
    Attention { peer: String, label: String, attention: AttentionView },
    /// How many agents are blocked on a peer.
    BlockedChanged { peer: String, count: u32 },
    /// Paired machines (either way) changed.
    PairingsChanged,
}

/// Where [`PeerEvent`]s go. Called on peers' own threads: never block.
pub trait PeerSink: Send + Sync {
    fn emit(&self, ev: PeerEvent);
}

/// Drops every event.
pub struct NullPeerSink;

impl PeerSink for NullPeerSink {
    fn emit(&self, _ev: PeerEvent) {}
}

/// How links and the listener pace themselves (tailnet-peers.md §2 "When
/// the peer sleeps"). Tests shorten them.
#[derive(Debug, Clone)]
pub struct Timing {
    /// Waits between reconnects while the peer is online in Tailscale.
    pub backoff: Vec<Duration>,
    /// How often a link pings its peer…
    pub ping: Duration,
    /// …and how long without a frame means it is gone.
    pub dead_after: Duration,
    /// How often Tailscale's status is looked at while a peer is offline,
    /// and the listener's addresses are checked.
    pub status_poll: Duration,
    /// A peer Tailscale calls offline is still dialed this often.
    pub offline_retry: Duration,
    /// Connecting, and the handshake.
    pub connect: Duration,
    pub hello: Duration,
}

impl Default for Timing {
    fn default() -> Timing {
        let s = Duration::from_secs;
        Timing {
            backoff: vec![s(1), s(2), s(5), s(10), s(30)],
            ping: s(15),
            dead_after: s(45),
            status_poll: s(30),
            offline_retry: s(600),
            connect: s(5),
            hello: s(5),
        }
    }
}

/// What the listener allows per machine (tailnet-peers.md §5 "Limits").
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Open connections per paired machine.
    pub per_peer: usize,
    /// Open connections in all (threads are per connection).
    pub total: usize,
    /// Pairings per node within `pair_window`.
    pub pairs: usize,
    pub pair_window: Duration,
    /// Requests per second per connection, sustained…
    pub requests_per_sec: u32,
    /// …with bursts up to this many.
    pub burst: u32,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { per_peer: 4, total: 32, pairs: 3, pair_window: Duration::from_secs(600), requests_per_sec: 100, burst: 300 }
    }
}

/// What the listener, links and [`Peers`] share.
pub(crate) struct Ctx {
    pub tailnet: Arc<dyn Tailnet>,
    pub store: store::Store,
    pub key: InstanceKey,
    pub sink: Arc<dyn PeerSink>,
    /// This Pitwall's version (`welcome.daemon`).
    pub version: String,
    pub timing: Timing,
    pub limits: Limits,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// ms since the epoch.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
