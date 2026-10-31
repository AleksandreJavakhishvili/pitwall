//! The listener (tailnet-peers.md §4 "Rules for the listener", §5): on
//! while "Let my other machines see this Pitwall's agents" is.
//!
//! - It binds only to this node's Tailscale addresses, one socket each
//!   (never `0.0.0.0`, `::` or a LAN address), and rebinds when they change
//!   (looked at every `status_poll`; Tailscale may start after Pitwall).
//! - On accept, before a byte is read: the source must be in the tailnet
//!   ranges, and `whois` must name an untagged machine of this node's own
//!   Tailscale user. Otherwise the connection is closed without a byte.
//! - Then `peerHello` → `peerWelcome` (version, label, pairing open or
//!   closed, agent count: all an unpaired caller learns), and `peer.auth`:
//!   the caller's key signs our nonce; a known machine must come with the
//!   node, user and key it paired with; a new one is paired when it asks
//!   (no code: same user, and this side opted in). We sign its nonce back.
//! - After that, View only: `agent.list`, `events.subscribe` (agents,
//!   attention, blocked), `term.watch` / `term.unwatch` (an agent's
//!   terminal, view-only: [`crate::watch`]) and `peer.ping`. Everything
//!   else is `denied`, and terminal data from the viewer (input) closes
//!   the connection.
//!
//! Connections, pairings and refusals go to the audit log. One thread
//! accepts per address and one runs each connection (no async runtime).

use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pitwall_core::engine::input;
use pitwall_core::term::TermHost;
use pitwall_core::Shared;
use pitwall_daemon::events::{self, Hub, Queue};
use pitwall_daemon::methods::peer_access;
use pitwall_daemon::Caller;
use pitwall_proto::frame::{self, Frame};
use pitwall_proto::peer::{
    transcript, Pairing, PeerAuth, PeerAuthed, PeerClientHello, PeerServerHello, PeerWelcome, Signer, TermUnwatch, TermWatch, TermWatching, IDENTITY_CHANGED,
    NOT_PAIRED, PEER_AUTH, PEER_MAX_FRAME, PEER_PING, PERMS_VIEW, TERM_UNWATCH, TERM_WATCH,
};
use pitwall_proto::{code, method, AgentView, ErrorBody, EventsSubscribe, EventsSubscribed, Range, Reject, Request, Response, ServerMsg};
use serde_json::Value;

use crate::dial::read_json;
use crate::key::{self, InstanceKey};
use crate::store::{AuditEntry, Paired};
use crate::tailnet::WhoIs;
use crate::watch::{Viewer, Watches, Writer};
use crate::{lock, now_ms, Ctx, PeerEvent};

/// This Pitwall's agents, as the listener serves them: its engine's own
/// records only (never mirrors, so A↔B never loops).
pub trait Feed: Send + Sync {
    fn views(&self) -> Vec<AgentView>;
    /// A queue of `topics` that starts with the agent list and blocked
    /// count now (`events::snapshot`).
    fn subscribe(&self, topics: Vec<&'static str>) -> Arc<Queue>;
    /// This Pitwall's account names by id: a viewer groups the agents by
    /// them (its own account ids mean nothing here).
    fn account_names(&self) -> HashMap<String, String> {
        HashMap::new()
    }
    /// Agent `agent_id`'s terminal while it is attached (`term.watch`).
    /// Whatever the agent is: Pitwall hosts every terminal the same way.
    fn terminal(&self, _agent_id: &str) -> Option<Arc<TermHost>> {
        None
    }
}

/// `views` with their accounts' names (`AgentView::account_name`).
pub fn name_accounts(mut views: Vec<AgentView>, names: &HashMap<String, String>) -> Vec<AgentView> {
    for v in &mut views {
        v.account_name = v.account.as_ref().and_then(|id| names.get(id)).cloned();
    }
    views
}

/// An `agents` event with its accounts' names; any other as it is.
fn named_event(feed: &dyn Feed, mut ev: pitwall_proto::Event) -> pitwall_proto::Event {
    if ev.event != pitwall_proto::event::AGENTS_CHANGED {
        return ev;
    }
    if let Ok(views) = serde_json::from_value::<Vec<AgentView>>(ev.data.clone()) {
        if let Ok(v) = serde_json::to_value(name_accounts(views, &feed.account_names())) {
            ev.data = v;
        }
    }
    ev
}

/// The engine and the server's event hub (the one `events.subscribe` on
/// the local socket uses).
pub struct EngineFeed {
    pub engine: Shared,
    pub hub: Arc<Hub>,
}

impl Feed for EngineFeed {
    fn views(&self) -> Vec<AgentView> {
        self.engine.views()
    }

    fn subscribe(&self, topics: Vec<&'static str>) -> Arc<Queue> {
        let q = self.hub.subscribe(topics);
        events::snapshot(&self.engine, &q);
        q
    }

    fn account_names(&self) -> HashMap<String, String> {
        let list = pitwall_core::accounts::Accounts::load(self.engine.paths());
        list.accounts.into_iter().map(|a| (a.id, a.name)).collect()
    }

    fn terminal(&self, agent_id: &str) -> Option<Arc<TermHost>> {
        input::terminal(&self.engine, agent_id)
    }
}

/// How often an accept loop looks for a stop between connections. (Waking
/// a blocking accept by connecting to it doesn't work everywhere: macOS
/// doesn't route a node's own Tailscale IPv6 address back to it.)
const ACCEPT_POLL: Duration = Duration::from_millis(50);

/// A running listener; dropping it stops it and closes every connection.
pub struct Listener {
    inner: Arc<Inner>,
    supervisor: Option<JoinHandle<()>>,
}

struct Inner {
    ctx: Arc<Ctx>,
    feed: Arc<dyn Feed>,
    port: u16,
    stop: AtomicBool,
    wake: Condvar,
    sockets: Mutex<Vec<Socket>>,
    /// Why nothing listens (no Tailscale, the port is taken), if so.
    error: Mutex<Option<String>>,
    conns: Mutex<Vec<Conn>>,
    next: AtomicU64,
    /// Pairings per node, for the rate limit.
    pairs: Mutex<HashMap<String, Vec<Instant>>>,
}

struct Socket {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Socket {
    fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// An open connection, to close it on revocation or stop.
struct Conn {
    id: u64,
    node: String,
    /// The viewer's id once `peer.auth` succeeded.
    viewer: Option<String>,
    stream: TcpStream,
}

impl Listener {
    /// Start listening on `port` (0: any, tests) for `feed`'s agents.
    pub(crate) fn start(ctx: Arc<Ctx>, feed: Arc<dyn Feed>, port: u16) -> Listener {
        let inner = Arc::new(Inner {
            ctx,
            feed,
            port,
            stop: AtomicBool::new(false),
            wake: Condvar::new(),
            sockets: Mutex::new(Vec::new()),
            error: Mutex::new(None),
            conns: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
            pairs: Mutex::new(HashMap::new()),
        });
        inner.sync();
        let sup = inner.clone();
        let supervisor = std::thread::Builder::new()
            .name("pitwall-peer-listen".into())
            .spawn(move || {
                let mut guard = lock(&sup.error);
                while !sup.stop.load(Ordering::SeqCst) {
                    guard = sup.wake.wait_timeout(guard, sup.ctx.timing.status_poll).unwrap_or_else(|e| e.into_inner()).0;
                    if sup.stop.load(Ordering::SeqCst) {
                        break;
                    }
                    drop(guard);
                    sup.sync();
                    guard = lock(&sup.error);
                }
            })
            .ok();
        Listener { inner, supervisor }
    }

    /// Where it listens now.
    pub fn addrs(&self) -> Vec<SocketAddr> {
        lock(&self.inner.sockets).iter().map(|s| s.addr).collect()
    }

    /// Why it doesn't listen, if it doesn't.
    pub fn error(&self) -> Option<String> {
        lock(&self.inner.error).clone()
    }

    /// Close a viewer's connections (it was revoked).
    pub fn drop_viewer(&self, id: &str) {
        for c in lock(&self.inner.conns).iter().filter(|c| c.viewer.as_deref() == Some(id)) {
            let _ = c.stream.shutdown(Shutdown::Both);
        }
    }

    /// Close the connections of viewers not in `keep` (revoked elsewhere,
    /// e.g. by `pitwall peers revoke`).
    pub fn keep_viewers(&self, keep: &[String]) {
        for c in lock(&self.inner.conns).iter().filter(|c| c.viewer.as_ref().is_some_and(|v| !keep.contains(v))) {
            let _ = c.stream.shutdown(Shutdown::Both);
        }
    }

    /// Open connections (tests, Settings).
    pub fn connections(&self) -> usize {
        lock(&self.inner.conns).len()
    }

    fn shutdown(&mut self) {
        if self.inner.stop.swap(true, Ordering::SeqCst) {
            return;
        }
        {
            let _g = lock(&self.inner.error);
            self.inner.wake.notify_all();
        }
        if let Some(t) = self.supervisor.take() {
            let _ = t.join();
        }
        for s in lock(&self.inner.sockets).drain(..).collect::<Vec<_>>() {
            s.stop();
        }
        for c in lock(&self.inner.conns).iter() {
            let _ = c.stream.shutdown(Shutdown::Both);
        }
        self.inner.ctx.store.audit(AuditEntry::new("stop"));
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Inner {
    /// Listen on this node's Tailscale addresses now: bind new ones, close
    /// ones that went away.
    fn sync(self: &Arc<Self>) {
        let want: Vec<IpAddr> = match self.ctx.tailnet.status().and_then(|s| s.me().map(|me| self.ctx.tailnet.bind_ips(me))) {
            Ok(ips) => ips,
            Err(e) => {
                *lock(&self.error) = Some(e.message);
                return;
            }
        };
        let mut sockets = lock(&self.sockets);
        let (keep, gone): (Vec<Socket>, Vec<Socket>) = sockets.drain(..).partition(|s| want.contains(&s.addr.ip()));
        *sockets = keep;
        for s in gone {
            s.stop();
        }
        let mut port = sockets.first().map_or(self.port, |s| s.addr.port());
        let mut errors = Vec::new();
        let have: Vec<IpAddr> = sockets.iter().map(|s| s.addr.ip()).collect();
        for ip in want.iter().filter(|ip| !have.contains(ip)) {
            match TcpListener::bind(SocketAddr::new(*ip, port)).and_then(|l| l.local_addr().map(|a| (l, a))) {
                Ok((listener, addr)) => {
                    // One port on every address, even when the OS picked it.
                    port = addr.port();
                    let stop = Arc::new(AtomicBool::new(false));
                    let thread = self.accept(listener, stop.clone());
                    sockets.push(Socket { addr, stop, thread });
                    self.ctx.tailnet.listening(addr);
                    self.ctx.store.audit(AuditEntry::new("listen").addr(addr));
                }
                Err(e) => errors.push(format!("{ip}: {e}")),
            }
        }
        *lock(&self.error) = match (sockets.is_empty(), want.is_empty()) {
            (true, true) => Some("this device has no Tailscale address to listen on".into()),
            (true, false) => Some(format!("can't listen on port {port}: {}", errors.join("; "))),
            _ => None,
        };
    }

    fn accept(self: &Arc<Self>, listener: TcpListener, stop: Arc<AtomicBool>) -> Option<JoinHandle<()>> {
        listener.set_nonblocking(true).ok()?;
        let inner = self.clone();
        std::thread::Builder::new()
            .name("pitwall-peer-accept".into())
            .spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let stream = match listener.accept() {
                        Ok((stream, _)) => stream,
                        Err(_) => {
                            // Nothing waiting (or a transient error): look again soon.
                            std::thread::sleep(ACCEPT_POLL);
                            continue;
                        }
                    };
                    if stream.set_nonblocking(false).is_err() {
                        continue;
                    }
                    let inner = inner.clone();
                    let _ = std::thread::Builder::new().name("pitwall-peer-conn".into()).spawn(move || inner.serve(stream));
                }
            })
            .ok()
    }

    fn refuse(&self, peer: SocketAddr, who: Option<&WhoIs>, why: &str) {
        let mut e = AuditEntry::new("refused").addr(peer).detail(why);
        if let Some(w) = who {
            e = e.login(w.user_profile.login_name.clone()).node(w.node.stable_id.clone());
        }
        self.ctx.store.audit(e);
    }

    /// One connection, start to end.
    fn serve(self: Arc<Self>, stream: TcpStream) {
        let (Ok(peer), Ok(local)) = (stream.peer_addr(), stream.local_addr()) else { return };
        if !self.ctx.tailnet.address_ok(peer.ip()) {
            return self.refuse(peer, None, "not from the tailnet");
        }
        if lock(&self.conns).len() >= self.ctx.limits.total {
            return self.refuse(peer, None, "too many connections");
        }
        let status = match self.ctx.tailnet.status() {
            Ok(s) => s,
            Err(e) => return self.refuse(peer, None, &e.message),
        };
        let Ok(me) = status.me() else { return self.refuse(peer, None, "Tailscale isn't running") };
        let who = match self.ctx.tailnet.whois(peer) {
            Ok(Some(w)) => w,
            _ => return self.refuse(peer, None, "not a node of this tailnet"),
        };
        if who.tagged() {
            return self.refuse(peer, Some(&who), "a tagged device");
        }
        if !who.same_user(&status, me) {
            return self.refuse(peer, Some(&who), "another Tailscale user's machine");
        }
        let node = who.node.stable_id.clone();
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        {
            let mut conns = lock(&self.conns);
            if conns.iter().filter(|c| c.node == node).count() >= self.ctx.limits.per_peer {
                drop(conns);
                return self.refuse(peer, Some(&who), "too many connections from this machine");
            }
            let Ok(clone) = stream.try_clone() else { return };
            conns.push(Conn { id, node: node.clone(), viewer: None, stream: clone });
        }
        let mut session = Session {
            inner: &self,
            conn: id,
            peer,
            local,
            who,
            my_label: me.label(),
            nonce: key::nonce(),
            viewer: None,
            sub: Subscription::default(),
            watches: Watches::default(),
        };
        let closer = stream.try_clone().ok();
        let _ = session.run(stream);
        // Done reading: writers (events, watches) stop at once.
        if let Some(c) = closer {
            let _ = c.shutdown(Shutdown::Both);
        }
        drop(session);
        lock(&self.conns).retain(|c| c.id != id);
    }

    /// Whether `node` may pair now (and count the try when `take`).
    fn may_pair(&self, node: &str, take: bool) -> bool {
        let mut pairs = lock(&self.pairs);
        let tries = pairs.entry(node.to_string()).or_default();
        tries.retain(|t| t.elapsed() < self.ctx.limits.pair_window);
        let ok = tries.len() < self.ctx.limits.pairs;
        if ok && take {
            tries.push(Instant::now());
        }
        ok
    }
}

/// One authenticated (or not yet) connection.
struct Session<'a> {
    inner: &'a Inner,
    conn: u64,
    peer: SocketAddr,
    local: SocketAddr,
    who: WhoIs,
    my_label: String,
    nonce: String,
    /// The viewer's record once `peer.auth` succeeded.
    viewer: Option<Paired>,
    sub: Subscription,
    /// Terminals it watches (after `sub`: dropped, they stop, quietly).
    watches: Watches,
}

fn denied(msg: impl Into<String>) -> ErrorBody {
    ErrorBody::new(code::DENIED, msg)
}

impl Session<'_> {
    fn run(&mut self, stream: TcpStream) -> std::io::Result<()> {
        let ctx = &self.inner.ctx;
        stream.set_read_timeout(Some(ctx.timing.hello))?;
        // A viewer that stops reading doesn't hold a writer forever.
        stream.set_write_timeout(Some(ctx.timing.dead_after))?;
        let _ = stream.set_nodelay(true);
        let mut reader = BufReader::new(stream.try_clone()?);
        let writer: Writer = Arc::new(Mutex::new(BufWriter::new(stream.try_clone()?)));
        let hello = match read_json::<PeerClientHello>(&mut reader) {
            Ok(Some(PeerClientHello::PeerHello(h))) => h,
            Ok(None) => return Ok(()),
            Err(_) => {
                let reject = PeerServerHello::Reject(Reject { code: code::BAD_HELLO.into(), daemon: ctx.version.clone(), protocol: Range::ours() });
                return frame::write_json(&mut *lock(&writer), &reject);
            }
        };
        let Some(protocol) = Range::ours().negotiate(hello.protocol) else {
            let reject = PeerServerHello::Reject(Reject { code: code::INCOMPATIBLE.into(), daemon: ctx.version.clone(), protocol: Range::ours() });
            return frame::write_json(&mut *lock(&writer), &reject);
        };
        let welcome = PeerWelcome {
            protocol,
            daemon: ctx.version.clone(),
            label: self.my_label.clone(),
            pairing: if self.inner.may_pair(&self.who.node.stable_id, false) { Pairing::Open } else { Pairing::Closed },
            agents: self.inner.feed.views().len() as u32,
            nonce: self.nonce.clone(),
        };
        frame::write_json(&mut *lock(&writer), &PeerServerHello::PeerWelcome(welcome))?;
        let mut bucket = Bucket::new(ctx.limits.requests_per_sec, ctx.limits.burst);
        loop {
            let req = match read_request(&mut reader) {
                Ok(Incoming::Request(r)) => r,
                Ok(Incoming::Input) => {
                    // View only: nothing from the viewer reaches a terminal.
                    self.audit_denied("terminal input", "view only");
                    return Ok(());
                }
                // A probe ends here; so does a viewer that went quiet.
                Ok(Incoming::Closed) | Err(_) => return Ok(()),
            };
            if !bucket.take() {
                frame::write_json(&mut *lock(&writer), &Response::err(req.id, ErrorBody::new(code::CONFLICT, "too many requests; slow down")))?;
                continue;
            }
            let (resp, close) = match &self.viewer {
                None => self.unauthenticated(req, protocol, &hello.nonce),
                Some(_) => self.authenticated(req),
            };
            frame::write_json(&mut *lock(&writer), &resp)?;
            if close {
                return Ok(());
            }
            if self.viewer.is_some() {
                // Pinged every `ping`: silence this long means it is gone.
                stream.set_read_timeout(Some(ctx.timing.dead_after))?;
            }
            // After the answer, so the viewer sees it before any event.
            self.sub.start(&writer, self.inner.feed.clone());
            if let Some(v) = &self.viewer {
                let viewer = Viewer {
                    label: v.label.clone(),
                    login: self.who.user_profile.login_name.clone(),
                    node: self.who.node.stable_id.clone(),
                    stable_id: v.stable_id.clone(),
                    public_key: v.public_key.clone(),
                };
                self.watches.start_pending(&self.inner.ctx, &self.inner.feed, &writer, &viewer);
            }
        }
    }

    /// Before `peer.auth`: nothing else answers. Returns (response, close).
    fn unauthenticated(&mut self, req: Request, protocol: u32, viewer_nonce: &str) -> (Response, bool) {
        if req.method != PEER_AUTH {
            self.audit_denied(&req.method, "before pairing");
            return (Response::err(req.id, denied("pair first")), false);
        }
        match self.auth(req.params, protocol, viewer_nonce) {
            Ok(authed) => (Response::ok(req.id, serde_json::to_value(authed).unwrap_or_default()), false),
            Err(e) => (Response::err(req.id, e), true),
        }
    }

    fn auth(&mut self, params: Value, protocol: u32, viewer_nonce: &str) -> Result<PeerAuthed, ErrorBody> {
        let ctx = &self.inner.ctx;
        let p: PeerAuth = serde_json::from_value(params).map_err(|e| ErrorBody::new(code::BAD_PARAMS, e.to_string()))?;
        let (viewer_addr, my_addr) = (self.peer.to_string(), self.local.to_string());
        let node = self.who.node.stable_id.clone();
        let login = self.who.user_profile.login_name.clone();
        let audit = |event: &str| AuditEntry::new(event).peer(p.label.clone()).login(login.clone()).node(node.clone()).addr(self.peer);
        if !InstanceKey::verify(&p.key, &transcript(Signer::Viewer, &self.nonce, &viewer_addr, &my_addr, protocol), &p.sig) {
            ctx.store.audit(audit("refused").detail("bad signature"));
            return Err(denied("the signature doesn't match the key"));
        }
        let record = Paired {
            id: node.clone(),
            label: if p.label.trim().is_empty() { self.who.label() } else { p.label.trim().to_string() },
            stable_id: node.clone(),
            login: login.clone(),
            public_key: p.key.clone(),
            added_at: now_ms(),
            last_seen: Some(now_ms()),
        };
        let known = ctx.store.pairings().viewers.into_iter().find(|v| v.stable_id == node || v.public_key == p.key);
        let same = known.as_ref().is_some_and(|v| v.stable_id == node && v.public_key == p.key && v.login == login);
        let event = match (known, same) {
            (Some(_), true) => "connect",
            (Some(_), false) if !p.pair => {
                ctx.store.audit(audit("identity_changed"));
                return Err(ErrorBody::new(IDENTITY_CHANGED, format!("{}'s identity changed; pair again", record.label)));
            }
            (None, _) if !p.pair => {
                ctx.store.audit(audit("refused").detail("not paired"));
                return Err(ErrorBody::new(NOT_PAIRED, format!("this machine isn't paired with {}; pair it first", self.my_label)));
            }
            (known, _) => {
                if !self.inner.may_pair(&node, true) {
                    ctx.store.audit(audit("refused").detail("too many pairing tries"));
                    return Err(denied("too many pairing tries from this machine; wait a few minutes"));
                }
                if known.is_some() { "repair" } else { "pair" }
            }
        };
        let saved = ctx.store.update(|f| {
            let added_at = f.viewers.iter().find(|v| v.stable_id == node && event == "connect").map(|v| v.added_at);
            f.viewers.retain(|v| v.stable_id != node && v.public_key != p.key);
            let rec = Paired { added_at: added_at.unwrap_or(record.added_at), ..record.clone() };
            f.viewers.push(rec.clone());
            rec
        })?;
        ctx.store.audit(audit(event));
        if event != "connect" {
            ctx.sink.emit(PeerEvent::PairingsChanged);
        }
        if let Some(c) = lock(&self.inner.conns).iter_mut().find(|c| c.id == self.conn) {
            c.viewer = Some(saved.id.clone());
        }
        self.viewer = Some(saved);
        let sig = ctx.key.sign(&transcript(Signer::Listener, viewer_nonce, &viewer_addr, &my_addr, protocol));
        Ok(PeerAuthed { key: ctx.key.public_key(), sig, label: self.my_label.clone(), perms: PERMS_VIEW.into() })
    }

    /// After `peer.auth`: View only, while still paired.
    fn authenticated(&mut self, req: Request) -> (Response, bool) {
        let viewer = self.viewer.clone().expect("authenticated");
        let still = self.inner.ctx.store.pairings().viewers.iter().any(|v| v.stable_id == viewer.stable_id && v.public_key == viewer.public_key);
        if !still {
            return (Response::err(req.id, ErrorBody::new(NOT_PAIRED, format!("this machine was removed from {}", self.my_label))), true);
        }
        let res = match req.method.as_str() {
            PEER_PING => Ok(Value::Null),
            TERM_WATCH => self.watch(req.params),
            TERM_UNWATCH => serde_json::from_value::<TermUnwatch>(req.params)
                .map_err(|e| ErrorBody::new(code::BAD_PARAMS, e.to_string()))
                .and_then(|p| self.watches.remove(p.stream))
                .map(|_| Value::Null),
            m if peer_access(m).is_none() => {
                self.audit_denied(m, "view only");
                Err(denied(format!("{m} isn't allowed from another machine (view only)")))
            }
            method::AGENT_LIST => serde_json::to_value(name_accounts(self.inner.feed.views(), &self.inner.feed.account_names())).map_err(|e| ErrorBody::new(code::OTHER, e.to_string())),
            method::EVENTS_SUBSCRIBE => self.subscribe(req.params),
            m => Err(ErrorBody::new(code::UNKNOWN_METHOD, format!("unknown method \"{m}\""))),
        };
        (
            match res {
                Ok(v) => Response::ok(req.id, v),
                Err(e) => Response::err(req.id, e),
            },
            false,
        )
    }

    /// `events.subscribe`: the topics anyone outside Pitwall's own window
    /// gets (agents, attention, blocked; `approvals` is denied).
    fn subscribe(&mut self, params: Value) -> Result<Value, ErrorBody> {
        let p: EventsSubscribe = if params.is_null() { EventsSubscribe::default() } else { serde_json::from_value(params).map_err(|e| ErrorBody::new(code::BAD_PARAMS, e.to_string()))? };
        let topics = events::topics(&Caller::outside(), &p)?;
        match &self.sub.queue {
            Some(q) => q.set_topics(topics.clone()),
            None => self.sub.queue = Some(self.inner.feed.subscribe(topics.clone())),
        }
        serde_json::to_value(EventsSubscribed { topics: topics.iter().map(|t| t.to_string()).collect() }).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
    }

    /// `term.watch`: one of the agents `agent.list` gives (its terminal,
    /// whatever kind of agent or shell it is), view-only.
    fn watch(&mut self, params: Value) -> Result<Value, ErrorBody> {
        let p: TermWatch = serde_json::from_value(params).map_err(|e| ErrorBody::new(code::BAD_PARAMS, e.to_string()))?;
        let feed = &self.inner.feed;
        if !feed.views().iter().any(|a| a.id == p.agent_id) {
            return Err(ErrorBody::new(code::NOT_FOUND, format!("no agent {} on {}", p.agent_id, self.my_label)));
        }
        let host = feed.terminal(&p.agent_id).ok_or_else(|| ErrorBody::new(code::NOT_RUNNING, format!("{} isn't running on {}", p.agent_id, self.my_label)))?;
        let (cols, rows) = host.size();
        let stream = self.watches.add(&p.agent_id, host)?;
        serde_json::to_value(TermWatching { stream, cols, rows }).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
    }

    fn audit_denied(&self, m: &str, why: &str) {
        let label = self.viewer.as_ref().map(|v| v.label.clone()).unwrap_or_else(|| self.who.label());
        let e = AuditEntry::new("denied").peer(label).login(self.who.user_profile.login_name.clone()).node(self.who.node.stable_id.clone()).method(m).detail(why);
        self.inner.ctx.store.audit(e);
    }
}

/// What a viewer sent.
enum Incoming {
    Request(Request),
    /// Terminal data: input, which View doesn't allow.
    Input,
    Closed,
}

/// The next request, skipping frame types this side doesn't know; frames
/// over `PEER_MAX_FRAME` and undecodable JSON are errors.
fn read_request(r: &mut impl Read) -> std::io::Result<Incoming> {
    loop {
        match frame::read_max(r, PEER_MAX_FRAME)? {
            Some(Frame::Json(body)) => {
                return serde_json::from_slice(&body).map(Incoming::Request).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e));
            }
            Some(Frame::Term { .. }) => return Ok(Incoming::Input),
            Some(Frame::Unknown(_)) => continue,
            None => return Ok(Incoming::Closed),
        }
    }
}

/// A connection's events: its queue in the hub, and the thread writing it
/// (as the local socket's server does).
#[derive(Default)]
struct Subscription {
    queue: Option<Arc<Queue>>,
    pusher: Option<JoinHandle<()>>,
}

impl Subscription {
    fn start(&mut self, writer: &Writer, feed: Arc<dyn Feed>) {
        let Some(q) = self.queue.clone().filter(|_| self.pusher.is_none()) else { return };
        let writer = writer.clone();
        self.pusher = std::thread::Builder::new()
            .name("pitwall-peer-events".into())
            .spawn(move || {
                while let Some(ev) = q.pop() {
                    let ev = named_event(&*feed, ev);
                    let mut w = lock(&writer);
                    if w.write_all(&frame::json(&ServerMsg::Event(ev))).and_then(|_| w.flush()).is_err() {
                        q.close();
                    }
                }
            })
            .ok();
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(q) = &self.queue {
            q.close();
        }
        if let Some(t) = self.pusher.take() {
            let _ = t.join();
        }
    }
}

/// Requests per second with bursts.
struct Bucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn new(rate: u32, burst: u32) -> Bucket {
        Bucket { rate: rate as f64, burst: burst as f64, tokens: burst as f64, at: Instant::now() }
    }

    fn take(&mut self) -> bool {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.at).as_secs_f64() * self.rate).min(self.burst);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}
