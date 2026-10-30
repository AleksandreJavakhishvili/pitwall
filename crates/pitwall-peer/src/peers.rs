//! [`Peers`]: what the app (and `pitwall peers`) calls. It owns the
//! listener while "Let my other machines see this Pitwall's agents" is on,
//! one link per paired machine while "Show agents from my other machines"
//! is, and the mirror of their agents.
//!
//! Nothing runs until [`Peers::start`], and nothing is probed or dialed
//! while the switches are off. Discovery lists the user's own online
//! machines from Tailscale's status and probes Pitwall's port on each, only
//! when asked.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use pitwall_proto::peer::{LinkState, LinkView, Pairing};
use pitwall_proto::{code, AgentView, ErrorBody};

use crate::link::{Link, Mirror};
use crate::server::{Feed, Listener};
use crate::store::{slug, AuditEntry, Paired, PeerSettings, Store};
use crate::tailnet::{Node, Status, Tailnet, TailnetState};
use crate::{dial, lock, now_ms, Ctx, InstanceKey, Limits, PeerEvent, PeerSink, Timing};

pub struct PeersConfig {
    /// Pitwall's data folder (`peers/` goes in it).
    pub root: PathBuf,
    pub tailnet: Arc<dyn Tailnet>,
    /// This Pitwall's version, told to peers.
    pub version: String,
    pub sink: Arc<dyn PeerSink>,
    /// This Pitwall's agents, for the listener (`None`: this host can't
    /// share them, e.g. the CLI).
    pub feed: Option<Arc<dyn Feed>>,
    pub timing: Timing,
    pub limits: Limits,
}

/// Tailscale and the listener, for Settings → Peers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailnetInfo {
    pub state: TailnetState,
    /// Why it isn't ready, in words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// This machine's name and owner on the tailnet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// This machine's Tailscale addresses (where sharing listens).
    pub addresses: Vec<std::net::IpAddr>,
    pub settings: PeerSettings,
    /// Where the listener listens now (empty while sharing is off).
    pub listening: Vec<SocketAddr>,
    /// Why sharing is on but nothing listens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_error: Option<String>,
    /// This Pitwall's key (`ed25519:…`), to compare by eye.
    pub public_key: String,
}

/// What a probe found on a machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Probe {
    /// A Pitwall that lets the user's machines see it.
    #[serde(rename_all = "camelCase")]
    Pitwall { version: String, label: String, pairing: Pairing, agents: u32 },
    /// Nothing answered (no Pitwall, or sharing is off there).
    NoAnswer { message: String },
    /// A Pitwall too old or too new for this one.
    Incompatible { message: String },
    /// Offline in Tailscale: not probed.
    NotProbed,
}

/// One of the user's own machines on the tailnet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub stable_id: String,
    pub label: String,
    pub dns: String,
    pub os: String,
    pub online: bool,
    /// Its pairing's id when already paired.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired: Option<String>,
    pub probe: Probe,
}

/// A paired machine this Pitwall shows, and how its link is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerEntry {
    #[serde(flatten)]
    pub paired: Paired,
    /// `None` while "Show agents from my other machines" is off.
    pub link: Option<LinkView>,
    pub agents: usize,
    pub blocked: u32,
}

#[derive(Default)]
struct Running {
    started: bool,
    listener: Option<Listener>,
    links: BTreeMap<String, Link>,
}

pub struct Peers {
    ctx: Arc<Ctx>,
    feed: Option<Arc<dyn Feed>>,
    mirror: Arc<Mirror>,
    run: Mutex<Running>,
}

fn not_found(what: &str) -> ErrorBody {
    ErrorBody::new(code::NOT_FOUND, what)
}

impl Peers {
    /// Peers for a data folder: loads (or makes) the instance key. Nothing
    /// runs until [`start`](Self::start).
    pub fn new(cfg: PeersConfig) -> Result<Arc<Peers>, ErrorBody> {
        let key = InstanceKey::load_or_create(&cfg.root)?;
        let mirror = Mirror::new(cfg.sink.clone());
        let ctx = Arc::new(Ctx {
            tailnet: cfg.tailnet,
            store: Store::new(&cfg.root),
            key,
            sink: cfg.sink,
            version: cfg.version,
            timing: cfg.timing,
            limits: cfg.limits,
        });
        Ok(Arc::new(Peers { ctx, feed: cfg.feed, mirror, run: Mutex::new(Running::default()) }))
    }

    /// Run what the settings say: the listener while sharing, links while
    /// showing. Later [`set_settings`](Self::set_settings) apply at once.
    pub fn start(&self) {
        lock(&self.run).started = true;
        self.apply();
    }

    /// Stop everything (the app quits).
    pub fn stop(&self) {
        let mut run = lock(&self.run);
        run.started = false;
        run.listener = None;
        let links = std::mem::take(&mut run.links);
        drop(run);
        drop(links);
    }

    pub fn settings(&self) -> PeerSettings {
        self.ctx.store.settings()
    }

    /// Save the switches and the port, and apply them when started.
    pub fn set_settings(&self, s: PeerSettings) -> Result<PeerSettings, ErrorBody> {
        self.ctx.store.set_settings(&s)?;
        self.apply();
        Ok(s)
    }

    fn apply(&self) {
        let s = self.settings();
        let mut run = lock(&self.run);
        if !run.started {
            return;
        }
        let share = s.share && self.feed.is_some();
        let port_changed = run.listener.as_ref().is_some_and(|l| l.addrs().first().is_some_and(|a| s.port != 0 && a.port() != s.port));
        if !share || port_changed {
            run.listener = None;
        }
        if share && run.listener.is_none() {
            let feed = self.feed.clone().expect("checked");
            run.listener = Some(Listener::start(self.ctx.clone(), feed, s.port));
        }
        let want: Vec<Paired> = if s.show { self.ctx.store.pairings().peers } else { Vec::new() };
        let gone: Vec<String> = run.links.keys().filter(|id| !want.iter().any(|p| &p.id == *id)).cloned().collect();
        let mut stopped = Vec::new();
        for id in gone {
            stopped.extend(run.links.remove(&id));
            self.mirror.remove(&id);
        }
        for p in want {
            if !run.links.contains_key(&p.id) {
                let link = Link::start(self.ctx.clone(), p.clone(), s.port, self.mirror.clone());
                run.links.insert(p.id.clone(), link);
            }
        }
        drop(run);
        drop(stopped);
    }

    /// Apply what another process (`pitwall peers set / pair / unpair /
    /// revoke` on the same data folder) wrote: the switches, the port, the
    /// paired machines, and close revoked viewers' connections. The host
    /// calls it when `peers/` changes.
    pub fn reload(&self) {
        self.apply();
        let viewers: Vec<String> = self.viewers().into_iter().map(|v| v.id).collect();
        if let Some(l) = &lock(&self.run).listener {
            l.keep_viewers(&viewers);
        }
        self.ctx.sink.emit(PeerEvent::PairingsChanged);
    }

    /// Tailscale's state, this machine, and the listener.
    pub fn info(&self) -> TailnetInfo {
        let settings = self.settings();
        let (listening, listen_error) = match &lock(&self.run).listener {
            Some(l) => (l.addrs(), l.error()),
            None => (Vec::new(), None),
        };
        let mut info = TailnetInfo {
            state: TailnetState::NotRunning,
            message: None,
            machine: None,
            login: None,
            addresses: Vec::new(),
            settings,
            listening,
            listen_error,
            public_key: self.ctx.key.public_key(),
        };
        match self.ctx.tailnet.status() {
            Ok(s) => {
                info.state = s.state();
                if let Ok(me) = s.me() {
                    info.machine = Some(me.label());
                    info.login = Some(s.login_of(me)).filter(|l| !l.is_empty());
                    info.addresses = self.ctx.tailnet.bind_ips(me);
                }
                info.message = s.me().err().map(|e| e.message);
            }
            Err(e) => {
                info.state = if e.code == code::UNSUPPORTED { TailnetState::NotInstalled } else { TailnetState::NotRunning };
                info.message = Some(e.message);
            }
        }
        info
    }

    fn showing(&self) -> Result<Status, ErrorBody> {
        if !self.settings().show {
            return Err(ErrorBody::new(code::UNSUPPORTED, "turn on \"Show agents from my other machines\" first"));
        }
        let status = self.ctx.tailnet.status()?;
        status.me()?;
        Ok(status)
    }

    /// The user's own machines on the tailnet (same Tailscale user, not
    /// tagged), each online one probed for a Pitwall. Only when asked.
    pub fn discover(&self) -> Result<Vec<Candidate>, ErrorBody> {
        let status = self.showing()?;
        let paired = self.ctx.store.pairings().peers;
        let nodes: Vec<Node> = status.own_machines().into_iter().cloned().collect();
        let port = self.settings().port;
        let probes: Vec<_> = nodes
            .iter()
            .map(|n| {
                let (ctx, n) = (self.ctx.clone(), n.clone());
                std::thread::spawn(move || if n.online { probe(&ctx, &n, port) } else { Probe::NotProbed })
            })
            .collect();
        let mut out: Vec<Candidate> = nodes
            .iter()
            .zip(probes)
            .map(|(n, t)| Candidate {
                stable_id: n.id.clone(),
                label: n.label(),
                dns: n.dns().to_string(),
                os: n.os.clone(),
                online: n.online,
                paired: paired.iter().find(|p| p.stable_id == n.id).map(|p| p.id.clone()),
                probe: t.join().unwrap_or(Probe::NoAnswer { message: "probe failed".into() }),
            })
            .collect();
        out.sort_by(|a, b| b.online.cmp(&a.online).then(a.label.cmp(&b.label)));
        Ok(out)
    }

    /// One of the user's own machines, by name, MagicDNS name, IP or
    /// StableID: refuses tagged devices and other users' machines.
    fn own(status: &Status, name: &str) -> Result<Node, ErrorBody> {
        let me = status.me()?;
        let node = status.find(name).ok_or_else(|| not_found(&format!("no machine called {name} on your tailnet")))?;
        if node.tagged() {
            return Err(ErrorBody::new(code::DENIED, format!("{} is a tagged device; only your own machines can be paired", node.label())));
        }
        if node.user_id != me.user_id {
            return Err(ErrorBody::new(code::DENIED, format!("{} belongs to {}; only your own machines can be paired", node.label(), status.login_of(node))));
        }
        Ok(node.clone())
    }

    /// Pair with one of the user's machines (or pair again after its
    /// identity changed): both Pitwalls prove their keys and remember each
    /// other. No code: same Tailscale user, and that machine opted in.
    pub fn pair(&self, name: &str) -> Result<PeerEntry, ErrorBody> {
        let status = self.showing()?;
        let node = Self::own(&status, name)?;
        let label = status.me().map(|m| m.label()).unwrap_or_default();
        let mut d = dial::dial(&self.ctx, &node, self.settings().port)?;
        let authed = d.auth(&self.ctx.key, &label, true, None)?;
        drop(d);
        let login = status.login_of(&node);
        let paired = self.ctx.store.update(|f| {
            let old = f.peers.iter().position(|p| p.stable_id == node.id);
            let taken: Vec<&str> = f.peers.iter().filter(|p| p.stable_id != node.id).map(|p| p.id.as_str()).collect();
            let id = old.map(|i| f.peers[i].id.clone()).unwrap_or_else(|| slug(&authed.label, &taken));
            let rec = Paired {
                id,
                label: authed.label.clone(),
                stable_id: node.id.clone(),
                login: login.clone(),
                public_key: authed.key.clone(),
                added_at: now_ms(),
                last_seen: Some(now_ms()),
            };
            match old {
                Some(i) => f.peers[i] = rec.clone(),
                None => f.peers.push(rec.clone()),
            }
            rec
        })?;
        self.ctx.store.audit(AuditEntry::new("pair").peer(paired.id.clone()).login(login).node(node.id.clone()));
        // A new link: the old one may have stopped on a changed identity.
        lock(&self.run).links.remove(&paired.id);
        self.ctx.sink.emit(PeerEvent::PairingsChanged);
        self.apply();
        Ok(self.entry(paired))
    }

    fn find_peer(&self, name: &str) -> Result<Paired, ErrorBody> {
        let peers = self.ctx.store.pairings().peers;
        peers
            .into_iter()
            .find(|p| p.id == name || p.label == name || p.stable_id == name)
            .ok_or_else(|| not_found(&format!("no paired machine called {name}")))
    }

    /// Stop showing a machine and forget its key.
    pub fn unpair(&self, name: &str) -> Result<Paired, ErrorBody> {
        let p = self.find_peer(name)?;
        self.ctx.store.update(|f| f.peers.retain(|q| q.id != p.id))?;
        self.ctx.store.audit(AuditEntry::new("unpair").peer(p.id.clone()).login(p.login.clone()).node(p.stable_id.clone()));
        let link = lock(&self.run).links.remove(&p.id);
        drop(link);
        self.mirror.remove(&p.id);
        self.ctx.sink.emit(PeerEvent::PairingsChanged);
        Ok(p)
    }

    fn entry(&self, paired: Paired) -> PeerEntry {
        let link = self.mirror.link(&paired.id).filter(|_| lock(&self.run).links.contains_key(&paired.id));
        PeerEntry { agents: self.mirror.views_of(&paired.id).len(), blocked: self.mirror.blocked_of(&paired.id), link, paired }
    }

    /// Paired machines this Pitwall shows, with their links.
    pub fn peers(&self) -> Vec<PeerEntry> {
        self.ctx.store.pairings().peers.into_iter().map(|p| self.entry(p)).collect()
    }

    /// The paired machines' agents, mirrored (read-only).
    pub fn views(&self) -> Vec<AgentView> {
        self.mirror.views()
    }

    pub fn mirror(&self) -> &Arc<Mirror> {
        &self.mirror
    }

    /// Machines that may see this Pitwall's agents.
    pub fn viewers(&self) -> Vec<Paired> {
        self.ctx.store.pairings().viewers
    }

    /// Stop a machine from seeing this Pitwall: forget it and close its
    /// connections at once.
    pub fn revoke(&self, name: &str) -> Result<Paired, ErrorBody> {
        let v = self
            .viewers()
            .into_iter()
            .find(|v| v.id == name || v.label == name || v.stable_id == name)
            .ok_or_else(|| not_found(&format!("no machine called {name} sees this Pitwall")))?;
        self.ctx.store.update(|f| f.viewers.retain(|q| q.id != v.id))?;
        if let Some(l) = &lock(&self.run).listener {
            l.drop_viewer(&v.id);
        }
        self.ctx.store.audit(AuditEntry::new("revoke").peer(v.label.clone()).login(v.login.clone()).node(v.stable_id.clone()));
        self.ctx.sink.emit(PeerEvent::PairingsChanged);
        Ok(v)
    }

    /// Reconnect every offline link now (wake from sleep, network change).
    pub fn kick(&self) {
        for l in lock(&self.run).links.values() {
            l.kick();
        }
    }

    /// The newest `limit` audit lines, oldest first.
    pub fn audit(&self, limit: usize) -> Vec<AuditEntry> {
        self.ctx.store.audit_tail(limit)
    }

    /// The listener's open connections (tests, Settings).
    pub fn connections(&self) -> usize {
        lock(&self.run).listener.as_ref().map_or(0, |l| l.connections())
    }

    /// Whether a link to `peer` is in `state` now.
    pub fn link_is(&self, peer: &str, state: LinkState) -> bool {
        self.mirror.link(peer).is_some_and(|l| l.state == state)
    }
}

impl Drop for Peers {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Say hello to `node` and hang up: what an unpaired machine may learn.
fn probe(ctx: &Ctx, node: &Node, port: u16) -> Probe {
    match dial::dial(ctx, node, port) {
        Ok(d) => {
            let w = d.welcome;
            Probe::Pitwall { version: w.daemon, label: w.label, pairing: w.pairing, agents: w.agents }
        }
        Err(e) if e.code == code::INCOMPATIBLE => Probe::Incompatible { message: e.message },
        Err(e) => Probe::NoAnswer { message: e.message },
    }
}
