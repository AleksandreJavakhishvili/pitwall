//! Peers (tailnet-peers.md §6): what two Pitwalls of the same Tailscale
//! user say on the peer port, in the same frames as the local socket.
//!
//! ```json
//! → {"peerHello":{"protocol":{"min":1,"max":1},"client":"pitwall/0.2.1","nonce":"…"}}
//! ← {"peerWelcome":{"protocol":1,"daemon":"0.2.1","label":"studio","pairing":"open","agents":3,"nonce":"…"}}
//! → {"id":1,"method":"peer.auth","params":{"key":"ed25519:…","sig":"ed25519:…","label":"laptop","pair":true}}
//! ← {"id":1,"result":{"key":"ed25519:…","sig":"ed25519:…","label":"studio","perms":"view"}}
//! → {"id":2,"method":"events.subscribe","params":{"topics":["agents","attention","blocked"]}}
//! ```
//!
//! An agent's terminal, view-only, on a connection of its own:
//!
//! ```text
//! → {"id":2,"method":"term.watch","params":{"agentId":"a1"}}
//! ← {"id":2,"result":{"stream":1,"cols":120,"rows":40}}
//! ← {"event":"term.reset","data":{"stream":1,"cols":120,"rows":40}}
//! ← [type 1, stream 1] the screen and scrollback so far, in ≤ 64 KiB pieces
//! ← [type 1, stream 1] live output…
//! ← {"event":"term.resized","data":{"stream":1,"cols":100,"rows":30}}
//! ← {"event":"term.ended","data":{"stream":1,"reason":"exited"}}
//! ```
//!
//! A probe stops after the welcome: an unpaired caller learns the version,
//! the machine's name, whether it may pair and how many agents run there,
//! nothing else. Until `peer.auth` succeeds every other method is `denied`.
//! Each side signs the other's nonce ([`transcript`]).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::msg::{Range, Reject};

/// Where a Pitwall that lets the user's other machines see it listens (on
/// its Tailscale addresses only).
pub const PEER_PORT: u16 = 47420;

/// Larger frames from a peer are a protocol error (tailnet-peers.md §5).
pub const PEER_MAX_FRAME: u32 = 4 * 1024 * 1024;

/// The only access level in v1: list, status, attention and view-only
/// terminal streams.
pub const PERMS_VIEW: &str = "view";

/// `peer.auth`: [`PeerAuth`] → [`PeerAuthed`].
pub const PEER_AUTH: &str = "peer.auth";
/// `peer.ping` → `null`: the viewer's keep-alive (every 15 s; a link with
/// no frame for 45 s is gone).
pub const PEER_PING: &str = "peer.ping";

/// `term.watch`: [`TermWatch`] → [`TermWatching`], then the agent's
/// terminal on that stream, view-only: a [`TERM_RESET`] event and the
/// snapshot as terminal frames (type 1), then live output as it comes,
/// [`TERM_RESIZED`] when the source's size changes and [`TERM_ENDED`] when
/// the stream stops. Terminal frames from the viewer are refused (the
/// connection is closed): no input in View.
pub const TERM_WATCH: &str = "term.watch";
/// `term.unwatch`: [`TermUnwatch`] → `null`.
pub const TERM_UNWATCH: &str = "term.unwatch";
/// Event [`TermScreen`]: start over at this size; the snapshot follows
/// (sent first, and again after the viewer fell behind or the agent
/// restarted).
pub const TERM_RESET: &str = "term.reset";
/// Event [`TermScreen`]: the source's terminal is this size now.
pub const TERM_RESIZED: &str = "term.resized";
/// Event [`TermEnded`]: nothing more comes on the stream.
pub const TERM_ENDED: &str = "term.ended";
/// Terminal frames from the listener carry at most this many bytes.
pub const TERM_CHUNK: usize = 64 * 1024;
/// Streams one connection may watch at once.
pub const MAX_WATCHES: usize = 4;

/// `term.watch` params: one of the agents `agent.list` lists (the
/// listener's own id, not the viewer's `peer:<peer>:<id>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TermWatch {
    pub agent_id: String,
}

/// `term.watch` result: the stream id its terminal frames carry, and the
/// source's size now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TermWatching {
    pub stream: u32,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TermUnwatch {
    pub stream: u32,
}

/// [`TERM_RESET`] and [`TERM_RESIZED`]: the source's size. The viewer
/// renders at it and never resizes the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TermScreen {
    pub stream: u32,
    pub cols: u16,
    pub rows: u16,
}

/// Why a watched stream stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TermEnd {
    /// The program exited.
    Exited,
    /// The agent is no longer attached (stopped, or its machine went away).
    Stopped,
    /// The agent is no longer listed.
    Gone,
    /// The viewer asked (`term.unwatch`).
    Unwatched,
}

/// [`TERM_ENDED`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TermEnded {
    pub stream: u32,
    pub reason: TermEnd,
}

/// Error code: the peer's machine, Tailscale user or Pitwall key isn't the
/// one paired ("pair again").
pub const IDENTITY_CHANGED: &str = "identity_changed";
/// Error code: the caller isn't paired here (and didn't ask to pair).
pub const NOT_PAIRED: &str = "not_paired";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PeerHello {
    pub protocol: Range,
    /// "pitwall/0.2.1"
    pub client: String,
    /// Random, base64url; the listener signs it in [`PeerAuthed`].
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PeerClientHello {
    PeerHello(PeerHello),
}

/// Whether a new machine may pair now (`closed`: too many tries lately).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Pairing {
    Open,
    Closed,
}

/// All an unpaired caller learns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PeerWelcome {
    pub protocol: u32,
    /// The listener's Pitwall version.
    pub daemon: String,
    /// Its machine's name (`studio`).
    pub label: String,
    pub pairing: Pairing,
    /// How many agents run there.
    pub agents: u32,
    /// Random, base64url; the viewer signs it in [`PeerAuth`].
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PeerServerHello {
    PeerWelcome(PeerWelcome),
    Reject(Reject),
}

/// `peer.auth` params: the viewer's Pitwall key and its signature over the
/// listener's nonce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PeerAuth {
    /// `ed25519:<base64url>`
    pub key: String,
    pub sig: String,
    /// The viewer's machine name, as the listener lists it.
    pub label: String,
    /// Pair if not paired yet (the user asked to); `false` on reconnects.
    #[serde(default)]
    pub pair: bool,
}

/// `peer.auth` result: the listener's key and its signature over the
/// viewer's nonce.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PeerAuthed {
    pub key: String,
    pub sig: String,
    pub label: String,
    /// [`PERMS_VIEW`]
    pub perms: String,
}

/// Who signs a [`transcript`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signer {
    Viewer,
    Listener,
}

/// What a side signs: the other side's nonce, both ends of this connection
/// (`viewer` → `listener`, as `ip:port`) and the protocol, so a signature
/// can't be replayed on another connection or reflected back.
pub fn transcript(signer: Signer, nonce: &str, viewer: &str, listener: &str, protocol: u32) -> Vec<u8> {
    let who = match signer {
        Signer::Viewer => "viewer",
        Signer::Listener => "listener",
    };
    format!("pitwall-peer-auth/1\n{who}\n{nonce}\n{viewer}\n{listener}\n{protocol}").into_bytes()
}

/// How a viewer's link to a peer is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LinkState {
    Connected,
    Connecting,
    /// Unreachable (asleep, off the tailnet); rows are last known.
    Offline,
    /// Its machine, Tailscale user or Pitwall key changed: pair again.
    IdentityChanged,
}

/// `MachineView.link` of a peer's agent on the viewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LinkView {
    /// The viewer's id for the pairing.
    pub peer: String,
    pub state: LinkState,
    /// When it entered `state` (ms since the epoch).
    #[ts(type = "number")]
    pub since: u64,
    /// Last time it was connected (ms), if ever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | null")]
    pub last_seen: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn round_trip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(v: &T, wire: Value) {
        assert_eq!(serde_json::to_value(v).unwrap(), wire);
        assert_eq!(&serde_json::from_value::<T>(wire).unwrap(), v);
    }

    #[test]
    fn peer_handshake_golden_json() {
        let hello = PeerClientHello::PeerHello(PeerHello { protocol: Range { min: 1, max: 1 }, client: "pitwall/0.2.1".into(), nonce: "n1".into() });
        round_trip(&hello, json!({"peerHello":{"protocol":{"min":1,"max":1},"client":"pitwall/0.2.1","nonce":"n1"}}));
        let welcome = PeerServerHello::PeerWelcome(PeerWelcome {
            protocol: 1,
            daemon: "0.2.1".into(),
            label: "studio".into(),
            pairing: Pairing::Open,
            agents: 3,
            nonce: "n2".into(),
        });
        round_trip(
            &welcome,
            json!({"peerWelcome":{"protocol":1,"daemon":"0.2.1","label":"studio","pairing":"open","agents":3,"nonce":"n2"}}),
        );
        let auth = PeerAuth { key: "ed25519:k".into(), sig: "ed25519:s".into(), label: "laptop".into(), pair: true };
        round_trip(&auth, json!({"key":"ed25519:k","sig":"ed25519:s","label":"laptop","pair":true}));
        assert!(!serde_json::from_value::<PeerAuth>(json!({"key":"k","sig":"s","label":"l"})).unwrap().pair);
        let authed = PeerAuthed { key: "ed25519:k2".into(), sig: "ed25519:s2".into(), label: "studio".into(), perms: PERMS_VIEW.into() };
        round_trip(&authed, json!({"key":"ed25519:k2","sig":"ed25519:s2","label":"studio","perms":"view"}));
        let link = LinkView { peer: "studio".into(), state: LinkState::IdentityChanged, since: 5, last_seen: None };
        round_trip(&link, json!({"peer":"studio","state":"identityChanged","since":5}));
    }

    #[test]
    fn term_watch_golden_json() {
        round_trip(&TermWatch { agent_id: "a1".into() }, json!({"agentId":"a1"}));
        round_trip(&TermWatching { stream: 1, cols: 120, rows: 40 }, json!({"stream":1,"cols":120,"rows":40}));
        round_trip(&TermUnwatch { stream: 1 }, json!({"stream":1}));
        round_trip(&TermScreen { stream: 2, cols: 80, rows: 24 }, json!({"stream":2,"cols":80,"rows":24}));
        round_trip(&TermEnded { stream: 2, reason: TermEnd::Exited }, json!({"stream":2,"reason":"exited"}));
        round_trip(&TermEnded { stream: 3, reason: TermEnd::Unwatched }, json!({"stream":3,"reason":"unwatched"}));
        // A chunk and its header fit a peer frame many times over.
        assert!(TERM_CHUNK as u32 + 9 < PEER_MAX_FRAME);
    }

    #[test]
    fn transcripts_differ_by_signer_and_connection() {
        let t = |s, n, v| transcript(s, n, v, "100.64.0.2:47420", 1);
        assert_ne!(t(Signer::Viewer, "n", "100.64.0.1:5000"), t(Signer::Listener, "n", "100.64.0.1:5000"));
        assert_ne!(t(Signer::Viewer, "n", "100.64.0.1:5000"), t(Signer::Viewer, "n", "100.64.0.1:5001"));
        assert_ne!(t(Signer::Viewer, "n", "100.64.0.1:5000"), t(Signer::Viewer, "m", "100.64.0.1:5000"));
    }
}
