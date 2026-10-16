//! Polling `status` (every [`TailscaleConfig::poll`](super::TailscaleConfig),
//! while anyone listens for revocations or the listener runs): who the
//! provider removed, and whether this node's addresses changed.
//!
//! - A node that disappears from the tailnet, or now belongs to someone
//!   else, is a [`Revocation::Device`]; a login that no longer appears at
//!   all (the person was removed, or their last device was) is a
//!   [`Revocation::Identity`]. Their cached `whois` answers are dropped and
//!   their open connections closed, so the next request is refused.
//! - The listener rebinds when this node's Tailscale IPs change
//!   (tailnet-peers.md §4 "Rebinding").

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Weak};

use super::{identity_of, lock, Inner, Status};
use crate::net::{DeviceAddr, Identity, NetState, Revocation};

/// The poller thread; stops when its owner drops it (or the backend goes).
pub(crate) struct Watcher {
    _stop: Sender<()>,
}

/// One poll's view of the tailnet.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Snapshot {
    /// Node `StableID` → its owner's identity.
    nodes: BTreeMap<String, Identity>,
    /// Logins (not tagged devices) seen.
    logins: BTreeMap<String, Identity>,
    /// This node's addresses.
    pub(crate) mine: Vec<IpAddr>,
}

impl Snapshot {
    fn of(status: &Status) -> Snapshot {
        let mut nodes = BTreeMap::new();
        let mut logins = BTreeMap::new();
        for p in status.peer.values().chain(status.self_.iter()) {
            let user = status.user(p.user_id).cloned().unwrap_or_default();
            let identity = identity_of(&user, &p.id, &p.tags, &p.dns_name);
            if !identity.subject.starts_with("node:") {
                logins.insert(identity.subject.clone(), identity.clone());
            }
            nodes.insert(p.id.clone(), identity);
        }
        let mine = status.self_.as_ref().map(|s| s.tailscale_ips.clone()).unwrap_or_default();
        Snapshot { nodes, logins, mine }
    }
}

/// Start the poller if it isn't running.
pub(crate) fn ensure(inner: &Arc<Inner>) {
    let mut watcher = lock(&inner.watcher);
    if watcher.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<()>();
    let weak: Weak<Inner> = Arc::downgrade(inner);
    let every = inner.config.poll;
    let spawned = std::thread::Builder::new().name("pw-tailscale-watch".into()).spawn(move || loop {
        match rx.recv_timeout(every) {
            Err(RecvTimeoutError::Timeout) => {}
            _ => return,
        }
        let Some(inner) = weak.upgrade() else { return };
        if !inner.has_subscribers() && lock(&inner.serving).is_none() {
            continue;
        }
        inner.forget_status();
        tick(&inner);
    });
    if spawned.is_ok() {
        *watcher = Some(Watcher { _stop: tx });
    }
    drop(watcher);
    // The first view, so the next poll can tell who disappeared.
    if lock(&inner.seen).is_none() {
        tick(inner);
    }
}

/// One poll: announce who disappeared, rebind if this node's addresses
/// changed.
pub(crate) fn tick(inner: &Inner) {
    let Ok(status) = inner.status() else { return };
    if status.net_state() != NetState::Ready {
        return;
    }
    let now = Snapshot::of(&status);
    let before = lock(&inner.seen).replace(now.clone());
    let Some(before) = before else { return };
    let network = inner.id();

    let mut revoked = Vec::new();
    for (node, owner) in &before.nodes {
        if now.nodes.get(node).is_none_or(|o| !o.is(owner)) {
            inner.forget_node(node);
            revoked.push(Revocation::Device { device: DeviceAddr { network: network.clone(), node: node.clone() } });
        }
    }
    let gone: BTreeSet<&String> = before.logins.keys().filter(|l| !now.logins.contains_key(*l)).collect();
    for login in gone {
        revoked.push(Revocation::Identity { identity: before.logins[login].clone() });
    }

    if let Some(serving) = lock(&inner.serving).as_ref() {
        for r in &revoked {
            serving.close(r);
        }
        if before.mine != now.mine {
            serving.rebind(inner, &now.mine);
        }
    }
    for r in revoked {
        inner.announce(r);
    }
}
