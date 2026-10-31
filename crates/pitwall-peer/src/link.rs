//! The viewer's side (tailnet-peers.md §2): one [`Link`] per paired peer,
//! and the [`Mirror`] of its agents.
//!
//! A link finds the peer's current address by its `StableID` in Tailscale's
//! status, connects, proves both keys (`peer.auth`, never pairing on its
//! own), subscribes to `agents`, `attention` and `blocked`, and keeps the
//! peer's `AgentView`s. It pings every `ping` and calls the peer gone after
//! `dead_after` without a frame. Then it reconnects with backoff (1 s, 2 s,
//! 5 s, 10 s, 30 s) while Tailscale says the peer is online; while it says
//! offline, the link looks at the status every `status_poll` and dials only
//! every `offline_retry` (10 min). A changed node, user or key stops it
//! ("identity changed; pair again").
//!
//! Mirrored agents are read-only: ids `peer:<peer>:<id>`, machine
//! `peer:<peer>` labelled with the peer's name (`studio › vm-1` for its
//! other machines), all caps off, and `machine.link` saying how the link is.

use std::collections::BTreeMap;
use std::io::{BufReader, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use pitwall_proto::peer::{LinkState, LinkView, IDENTITY_CHANGED, NOT_PAIRED, PEER_PING};
use pitwall_proto::{event, frame, method, topic, AgentCaps, AgentView, AttentionView, BlockedCount, ErrorBody, EventsSubscribe, Request, ServerMsg};

use crate::dial::{self, read_json};
use crate::store::{AuditEntry, Paired};
use crate::{lock, now_ms, Ctx, PeerEvent, PeerSink};

/// The provider name of a peer's mirrored agents' machine.
pub fn provider_of(peer: &str) -> String {
    format!("peer:{peer}")
}

/// A mirrored agent's id.
pub fn mirrored_id(peer: &str, id: &str) -> String {
    format!("peer:{peer}:{id}")
}

/// A mirrored id back to its peer and the peer's own id for the agent
/// (`peer:studio:a1` → `("studio", "a1")`).
pub fn local_id(mirrored: &str) -> Option<(&str, &str)> {
    let (peer, id) = mirrored.strip_prefix("peer:")?.split_once(':')?;
    (!peer.is_empty() && !id.is_empty()).then_some((peer, id))
}

/// `view` from peer `peer` (named `label`) as the viewer shows it.
pub fn mirrored(peer: &str, label: &str, link: &LinkView, mut view: AgentView) -> AgentView {
    view.id = mirrored_id(peer, &view.id);
    let machine = &mut view.machine;
    machine.label = if machine.provider == "local" { label.to_string() } else { format!("{label} › {}", machine.label) };
    machine.provider = provider_of(peer);
    machine.can_create = false;
    machine.link = Some(link.clone());
    view.location = provider_of(peer);
    // View only: nothing can be typed, stopped, reviewed or changed.
    view.caps = AgentCaps::default();
    view
}

struct Entry {
    label: String,
    views: Vec<AgentView>,
    blocked: u32,
    link: LinkView,
}

/// Every paired peer's agents, as last heard, with change notifications.
pub struct Mirror {
    peers: Mutex<BTreeMap<String, Entry>>,
    sink: Arc<dyn PeerSink>,
}

impl Mirror {
    pub fn new(sink: Arc<dyn PeerSink>) -> Arc<Mirror> {
        Arc::new(Mirror { peers: Mutex::new(BTreeMap::new()), sink })
    }

    /// All peers' agents, mirrored (peers by id, agents in the peer's order).
    pub fn views(&self) -> Vec<AgentView> {
        let peers = lock(&self.peers);
        peers.iter().flat_map(|(id, e)| e.views.iter().map(|v| mirrored(id, &e.label, &e.link, v.clone()))).collect()
    }

    pub fn views_of(&self, peer: &str) -> Vec<AgentView> {
        let peers = lock(&self.peers);
        peers.get(peer).map(|e| e.views.iter().map(|v| mirrored(peer, &e.label, &e.link, v.clone())).collect()).unwrap_or_default()
    }

    pub fn link(&self, peer: &str) -> Option<LinkView> {
        lock(&self.peers).get(peer).map(|e| e.link.clone())
    }

    /// Blocked agents on connected peers (stale counts don't badge).
    pub fn blocked(&self) -> u32 {
        lock(&self.peers).values().filter(|e| e.link.state == LinkState::Connected).map(|e| e.blocked).sum()
    }

    pub fn blocked_of(&self, peer: &str) -> u32 {
        lock(&self.peers).get(peer).map_or(0, |e| e.blocked)
    }

    pub(crate) fn add(&self, peer: &Paired) {
        let link = LinkView { peer: peer.id.clone(), state: LinkState::Connecting, since: now_ms(), last_seen: peer.last_seen };
        let known = {
            let mut peers = lock(&self.peers);
            let known = peers.contains_key(&peer.id);
            peers.entry(peer.id.clone()).or_insert(Entry { label: peer.label.clone(), views: Vec::new(), blocked: 0, link }).label = peer.label.clone();
            known
        };
        // Paired again: its last rows stay until the new link says more.
        if known {
            self.set_state(&peer.id, LinkState::Connecting);
        }
    }

    pub(crate) fn remove(&self, peer: &str) {
        if lock(&self.peers).remove(peer).is_some() {
            self.sink.emit(PeerEvent::AgentsChanged { peer: peer.into() });
        }
    }

    pub(crate) fn set_state(&self, peer: &str, state: LinkState) {
        let link = {
            let mut peers = lock(&self.peers);
            let Some(e) = peers.get_mut(peer) else { return };
            if e.link.state == state {
                if state == LinkState::Connected {
                    e.link.last_seen = Some(now_ms());
                }
                return;
            }
            let now = now_ms();
            if e.link.state == LinkState::Connected {
                e.link.last_seen = Some(now);
            }
            e.link.state = state;
            e.link.since = now;
            if state == LinkState::Connected {
                e.link.last_seen = Some(now);
            }
            e.link.clone()
        };
        self.sink.emit(PeerEvent::LinkChanged { peer: peer.into(), link });
        self.sink.emit(PeerEvent::AgentsChanged { peer: peer.into() });
    }

    fn set_views(&self, peer: &str, views: Vec<AgentView>) {
        match lock(&self.peers).get_mut(peer) {
            Some(e) => e.views = views,
            None => return,
        }
        self.sink.emit(PeerEvent::AgentsChanged { peer: peer.into() });
    }

    fn set_blocked(&self, peer: &str, count: u32) {
        match lock(&self.peers).get_mut(peer) {
            Some(e) => e.blocked = count,
            None => return,
        }
        self.sink.emit(PeerEvent::BlockedChanged { peer: peer.into(), count });
    }

    fn attention(&self, peer: &str, mut a: AttentionView) {
        let Some(label) = lock(&self.peers).get(peer).map(|e| e.label.clone()) else { return };
        a.agent_id = mirrored_id(peer, &a.agent_id);
        self.sink.emit(PeerEvent::Attention { peer: peer.into(), label, attention: a });
    }

    /// An event from the peer.
    fn apply(&self, peer: &str, ev: pitwall_proto::Event) {
        match ev.event.as_str() {
            event::AGENTS_CHANGED => {
                if let Ok(views) = serde_json::from_value(ev.data) {
                    self.set_views(peer, views);
                }
            }
            event::BLOCKED_CHANGED => {
                if let Ok(b) = serde_json::from_value::<BlockedCount>(ev.data) {
                    self.set_blocked(peer, b.count);
                }
            }
            event::ATTENTION => {
                if let Ok(a) = serde_json::from_value(ev.data) {
                    self.attention(peer, a);
                }
            }
            _ => {}
        }
    }
}

/// A running link; dropping it stops it.
pub(crate) struct Link {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

struct Shared {
    ctx: Arc<Ctx>,
    peer: Paired,
    port: u16,
    mirror: Arc<Mirror>,
    ctl: Mutex<Ctl>,
    wake: Condvar,
}

#[derive(Default)]
struct Ctl {
    stop: bool,
    kicked: bool,
    /// The open connection, to cut it on stop.
    stream: Option<TcpStream>,
}

/// How one try ended.
enum Outcome {
    /// It was connected, then the connection ended.
    Lost,
    /// Couldn't connect (or Tailscale isn't up).
    Failed,
    /// Tailscale says the peer is offline (not dialed).
    Asleep,
    IdentityChanged(String),
}

impl Link {
    pub fn start(ctx: Arc<Ctx>, peer: Paired, port: u16, mirror: Arc<Mirror>) -> Link {
        mirror.add(&peer);
        let shared = Arc::new(Shared { ctx, peer, port, mirror, ctl: Mutex::new(Ctl::default()), wake: Condvar::new() });
        let run = shared.clone();
        let thread = std::thread::Builder::new().name("pitwall-peer-link".into()).spawn(move || run.run()).ok();
        Link { shared, thread }
    }

    /// Try again now (the user asked, the Mac woke, the network changed).
    pub fn kick(&self) {
        lock(&self.shared.ctl).kicked = true;
        self.shared.wake.notify_all();
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        {
            let mut ctl = lock(&self.shared.ctl);
            ctl.stop = true;
            if let Some(s) = &ctl.stream {
                let _ = s.shutdown(Shutdown::Both);
            }
        }
        self.shared.wake.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Shared {
    fn stopped(&self) -> bool {
        lock(&self.ctl).stop
    }

    /// Wait `d` or until stopped or kicked; whether it was kicked.
    fn wait(&self, d: std::time::Duration) -> bool {
        let deadline = Instant::now() + d;
        let mut ctl = lock(&self.ctl);
        while !ctl.stop && !ctl.kicked {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            ctl = self.wake.wait_timeout(ctl, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        std::mem::take(&mut ctl.kicked)
    }

    fn run(&self) {
        let id = self.peer.id.clone();
        let timing = &self.ctx.timing;
        let mut failures = 0usize;
        let mut last_dial: Option<Instant> = None;
        while !self.stopped() {
            let outcome = self.once(&mut last_dial);
            if self.stopped() {
                break;
            }
            let wait = match outcome {
                Outcome::IdentityChanged(why) => {
                    self.mirror.set_state(&id, LinkState::IdentityChanged);
                    self.ctx.store.audit(AuditEntry::new("identity_changed").peer(id.clone()).node(self.peer.stable_id.clone()).detail(why));
                    // Until it is paired again (which starts a new link).
                    while !self.stopped() {
                        self.wait(timing.offline_retry);
                    }
                    break;
                }
                Outcome::Lost => {
                    failures = 0;
                    timing.backoff.first().copied().unwrap_or_default()
                }
                Outcome::Failed => {
                    let d = timing.backoff.get(failures).or(timing.backoff.last()).copied().unwrap_or_default();
                    failures += 1;
                    d
                }
                Outcome::Asleep => timing.status_poll,
            };
            self.mirror.set_state(&id, LinkState::Offline);
            if self.wait(wait) {
                last_dial = None;
            }
        }
    }

    /// One try: find, dial, prove, subscribe, follow until it ends.
    fn once(&self, last_dial: &mut Option<Instant>) -> Outcome {
        let ctx = &self.ctx;
        let Ok(status) = ctx.tailnet.status() else { return Outcome::Failed };
        let Ok(me) = status.me() else { return Outcome::Failed };
        // Gone from the tailnet (removed, or the status is partial): look again later.
        let Some(node) = status.node(&self.peer.stable_id) else { return Outcome::Asleep };
        if node.tagged() || node.user_id != me.user_id || status.login_of(node) != self.peer.login {
            return Outcome::IdentityChanged(format!("{} now belongs to {}", self.peer.label, status.login_of(node)));
        }
        if !node.online && last_dial.is_some_and(|t| t.elapsed() < ctx.timing.offline_retry) {
            return Outcome::Asleep;
        }
        *last_dial = Some(Instant::now());
        let mut d = match dial::dial(ctx, node, self.port) {
            Ok(d) => d,
            Err(_) => return Outcome::Failed,
        };
        let label = status.me().map(|m| m.label()).unwrap_or_default();
        match d.auth(&ctx.key, &label, false, Some(&self.peer.public_key)) {
            Ok(_) => {}
            Err(e) if e.code == IDENTITY_CHANGED || e.code == NOT_PAIRED => return Outcome::IdentityChanged(e.message),
            Err(_) => return Outcome::Failed,
        }
        let sub = EventsSubscribe { topics: topic::DEFAULT.iter().map(|t| t.to_string()).collect() };
        if d.send(2, method::EVENTS_SUBSCRIBE, &sub).is_err() {
            return Outcome::Failed;
        }
        {
            let mut ctl = lock(&self.ctl);
            if ctl.stop {
                return Outcome::Lost;
            }
            ctl.stream = d.stream.try_clone().ok();
        }
        let _ = d.stream.set_read_timeout(Some(ctx.timing.dead_after));
        self.mirror.set_state(&self.peer.id, LinkState::Connected);
        let _ = ctx.store.update(|f| {
            if let Some(p) = f.peers.iter_mut().find(|p| p.id == self.peer.id) {
                p.last_seen = Some(now_ms());
            }
        });
        let pinger = Pinger::start(&d.stream, ctx.timing.ping);
        self.follow(&mut d.reader);
        drop(pinger);
        lock(&self.ctl).stream = None;
        Outcome::Lost
    }

    /// Read events into the mirror until the connection ends.
    fn follow(&self, reader: &mut BufReader<TcpStream>) {
        loop {
            match read_json::<ServerMsg>(reader) {
                Ok(Some(ServerMsg::Event(ev))) => self.mirror.apply(&self.peer.id, ev),
                // The subscription's answer and pongs.
                Ok(Some(ServerMsg::Response(r))) => {
                    if let Some(ErrorBody { code, .. }) = r.error {
                        if code == NOT_PAIRED {
                            return;
                        }
                    }
                }
                Ok(None) | Err(_) => return,
            }
        }
    }
}

/// Pings a peer every `every` until dropped.
pub(crate) struct Pinger {
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Pinger {
    pub(crate) fn start(stream: &TcpStream, every: std::time::Duration) -> Pinger {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let flag = stop.clone();
        let stream = stream.try_clone().ok();
        let thread = std::thread::Builder::new()
            .name("pitwall-peer-ping".into())
            .spawn(move || {
                let Some(mut stream) = stream else { return };
                let mut id = 1_000u64;
                let mut stopped = lock(&flag.0);
                while !*stopped {
                    stopped = flag.1.wait_timeout(stopped, every).unwrap_or_else(|e| e.into_inner()).0;
                    if *stopped {
                        break;
                    }
                    id += 1;
                    let ping = frame::json(&Request { id, method: PEER_PING.into(), params: serde_json::Value::Null });
                    if stream.write_all(&ping).and_then(|_| stream.flush()).is_err() {
                        break;
                    }
                }
            })
            .ok();
        Pinger { stop, thread }
    }
}

impl Drop for Pinger {
    fn drop(&mut self) {
        *lock(&self.stop.0) = true;
        self.stop.1.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
