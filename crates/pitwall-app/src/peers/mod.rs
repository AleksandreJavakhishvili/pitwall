//! The user's other machines over Tailscale in the app
//! (docs/spec/tailnet-peers.md, "Phase 1 as built" and "The app"): the
//! hosted [`Peers`] (`host.rs` makes it), what the UI shows of it
//! ([`PeersModel`]) and the offers to show a machine's agents.
//!
//! - Their agents reach the sidebar through the bridge like the engine's
//!   (`AppEvent::Peer`, `AgentStore`): mirrored, every cap off, so the UI
//!   offers nothing it can't do.
//! - Settings → Peers ([`crate::settings`]'s `peers.rs`) draws this model:
//!   the two switches, the port, discovered and paired machines, viewers
//!   and the audit log.
//! - `pitwall peers set / pair / unpair / revoke` write `peers/` directly:
//!   [`watch`] sees the files change and reloads them, so they apply live.
//! - Offers: while "Show my other machines' agents" is off and Tailscale
//!   lists other machines of this user, the sidebar shows one card offering
//!   it ([`card`]); once it is on, every machine newly found running a
//!   Pitwall that shares gets one toast "studio is running Pitwall with N
//!   agents — show them here?" ([`to_offer`]). Answers are remembered in
//!   `peers/app.json`.
//!
//! Nothing here touches the network itself: Tailscale's status is read
//! (`tailscale status`, read-only), and every probe, dial and listen is the
//! backend's, which does none of it while its switches are off.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global};
use serde::{Deserialize, Serialize};

use pitwall_peer::tailnet::Tailnet;
use pitwall_peer::{AuditEntry, Candidate, Paired, PeerEntry, PeerSettings, Peers, Probe, TailnetInfo};
use pitwall_proto::peer::{LinkState, LinkView};

use crate::agents::{AgentStore, StoreEvent};

#[cfg(test)]
mod tests;

/// The hosted peers, for code without a handle (the agent store reads the
/// mirror from it).
pub struct PeersHandle(pub Arc<Peers>);

impl Global for PeersHandle {}

/// The app's one [`PeersModel`].
pub struct ModelHandle(pub Entity<PeersModel>);

impl Global for ModelHandle {}

/// The model, when peers run (not when the data folder was refused).
pub fn model(cx: &App) -> Option<Entity<PeersModel>> {
    cx.try_global::<ModelHandle>().map(|h| h.0.clone())
}

/// How often Tailscale's status is looked at (new machines online, this
/// machine logged in or out).
pub const CHECK: Duration = Duration::from_secs(60);
/// How often `peers/` is looked at for the CLI's changes.
pub const WATCH: Duration = Duration::from_secs(1);

/// What the app remembers about offers (`peers/app.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Offers {
    /// Machines (Tailscale StableIDs) already offered: never again.
    pub asked: BTreeSet<String>,
    /// The sidebar card offering "Show my other machines' agents" was
    /// answered.
    pub card_done: bool,
}

impl Offers {
    fn file(root: &Path) -> PathBuf {
        root.join("peers").join("app.json")
    }

    pub fn load(root: &Path) -> Offers {
        std::fs::read_to_string(Offers::file(root)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, root: &Path) {
        let path = Offers::file(root);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }
}

/// One of the user's other machines as Tailscale lists them (read-only
/// status; nothing is probed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    pub stable_id: String,
    pub label: String,
    pub online: bool,
}

/// Machines found running a Pitwall that shares, not paired and never
/// offered: each gets one offer.
pub fn to_offer(found: &[Candidate], offers: &Offers) -> Vec<Candidate> {
    found
        .iter()
        .filter(|c| matches!(c.probe, Probe::Pitwall { .. }) && c.paired.is_none() && !offers.asked.contains(&c.stable_id))
        .cloned()
        .collect()
}

/// The sidebar card offering to show the other machines' agents: showing
/// is off, Tailscale lists other machines of this user, and it wasn't
/// answered.
pub fn card(settings: &PeerSettings, machines: &[Machine], offers: &Offers) -> bool {
    !settings.show && !offers.card_done && !machines.is_empty()
}

/// The offer's words: "studio is running Pitwall with 3 agents".
pub fn offer_title(c: &Candidate) -> String {
    let n = match c.probe {
        Probe::Pitwall { agents, .. } => agents,
        _ => 0,
    };
    let agents = if n == 1 { "1 agent".to_string() } else { format!("{n} agents") };
    format!("{} is running Pitwall with {agents}", c.label)
}

/// "5 min ago" for a time `ms` before `now` (ms since the epoch).
pub fn ago(ms: u64, now: u64) -> String {
    let s = now.saturating_sub(ms) / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}

/// How a link is doing, in words: "connected", "connecting…", "offline ·
/// last seen 5 min ago", "pair again".
pub fn link_text(link: &LinkView, now: u64) -> String {
    match link.state {
        LinkState::Connected => "connected".into(),
        LinkState::Connecting => "connecting…".into(),
        LinkState::Offline => match link.last_seen {
            Some(t) => format!("offline · last seen {}", ago(t, now)),
            None => "offline".into(),
        },
        LinkState::IdentityChanged => "identity changed · pair again".into(),
    }
}

/// ms since the epoch.
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// What `peers/` and the mirror say now.
struct Snapshot {
    settings: PeerSettings,
    entries: Vec<PeerEntry>,
    viewers: Vec<Paired>,
    audit: Vec<AuditEntry>,
}

impl Snapshot {
    fn of(p: &Peers) -> Snapshot {
        let mut audit = p.audit(30);
        audit.reverse();
        Snapshot { settings: p.settings(), entries: p.peers(), viewers: p.viewers(), audit }
    }
}

/// What the model tells the windows.
#[derive(Debug, Clone)]
pub enum PeersEvent {
    /// Offer to show this machine's agents (once per machine).
    Offer(Candidate),
    /// The offer for this machine (StableID) was answered: other windows
    /// drop theirs.
    Answered(String),
}

/// The user's other machines, as the UI shows them.
pub struct PeersModel {
    peers: Arc<Peers>,
    tailnet: Arc<dyn Tailnet>,
    root: PathBuf,
    pub settings: PeerSettings,
    /// Paired machines this Mac shows, with their links.
    pub entries: Vec<PeerEntry>,
    /// Machines that may see this Mac's agents.
    pub viewers: Vec<Paired>,
    /// The newest audit lines, newest first.
    pub audit: Vec<AuditEntry>,
    /// Tailscale and the listener (`None` until read).
    pub info: Option<TailnetInfo>,
    /// The user's other machines in Tailscale's status.
    pub machines: Vec<Machine>,
    /// The last discovery (`None`: not run yet).
    pub found: Option<Vec<Candidate>>,
    pub discover_error: Option<String>,
    pub discovering: bool,
    /// Machines with a pair / unpair / revoke in flight.
    pub busy: BTreeSet<String>,
    /// The last call that failed, in words.
    pub error: Option<String>,
    pub offers: Offers,
    /// Online machines when discovery last ran: it runs again when one
    /// more comes online.
    probed: BTreeSet<String>,
}

impl EventEmitter<PeersEvent> for PeersModel {}

impl PeersModel {
    pub fn new(peers: Arc<Peers>, tailnet: Arc<dyn Tailnet>, root: PathBuf) -> PeersModel {
        let mut m = PeersModel {
            settings: peers.settings(),
            peers,
            tailnet,
            offers: Offers::load(&root),
            root,
            entries: Vec::new(),
            viewers: Vec::new(),
            audit: Vec::new(),
            info: None,
            machines: Vec::new(),
            found: None,
            discover_error: None,
            discovering: false,
            busy: BTreeSet::new(),
            error: None,
            probed: BTreeSet::new(),
        };
        m.read();
        m
    }

    pub fn peers(&self) -> &Arc<Peers> {
        &self.peers
    }

    /// Read what's kept on disk and in the mirror.
    fn read(&mut self) {
        let snap = Snapshot::of(&self.peers);
        self.take(snap);
    }

    fn take(&mut self, s: Snapshot) {
        self.settings = s.settings;
        self.entries = s.entries;
        self.viewers = s.viewers;
        self.audit = s.audit;
    }

    /// Something changed (an event, a reload): read again (off the UI
    /// thread: the peers' lock may be held while the listener starts) and
    /// redraw.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let peers = self.peers.clone();
        let read = cx.background_executor().spawn(async move { Snapshot::of(&peers) });
        cx.spawn(async move |this, cx| {
            let snap = read.await;
            let _ = this.update(cx, |m, cx| {
                let was_showing = m.settings.show;
                m.take(snap);
                if m.settings.show && !was_showing {
                    m.check(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Read Tailscale's status and the listener (off the UI thread: it runs
    /// `tailscale status`), then discover if a machine came online while
    /// showing.
    pub fn check(&mut self, cx: &mut Context<Self>) {
        let (peers, tailnet) = (self.peers.clone(), self.tailnet.clone());
        let read = cx.background_executor().spawn(async move {
            let info = peers.info();
            let machines: Vec<Machine> = tailnet
                .status()
                .map(|s| {
                    s.own_machines()
                        .into_iter()
                        .map(|n| Machine { stable_id: n.id.clone(), label: n.label(), online: n.online })
                        .collect()
                })
                .unwrap_or_default();
            (info, machines, Snapshot::of(&peers))
        });
        let task = cx.spawn(async move |this, cx| {
            let (info, mut machines, snap) = read.await;
            machines.sort_by(|a, b| b.online.cmp(&a.online).then(a.label.cmp(&b.label)));
            let _ = this.update(cx, |m, cx| {
                m.info = Some(info);
                m.machines = machines;
                m.take(snap);
                let online: BTreeSet<String> = m.machines.iter().filter(|n| n.online).map(|n| n.stable_id.clone()).collect();
                if m.settings.show && !online.is_subset(&m.probed) && !m.discovering {
                    m.probed = online;
                    m.discover(true, cx);
                }
                cx.notify();
            });
        });
        task.detach();
    }

    /// Look for the user's machines running a Pitwall that shares (probes
    /// each online one; the backend refuses while showing is off). `offer`:
    /// new ones get their one offer.
    pub fn discover(&mut self, offer: bool, cx: &mut Context<Self>) {
        self.discovering = true;
        cx.notify();
        let peers = self.peers.clone();
        let run = cx.background_executor().spawn(async move { peers.discover() });
        let task = cx.spawn(async move |this, cx| {
            let r = run.await;
            let _ = this.update(cx, |m, cx| {
                m.discovering = false;
                match r {
                    Ok(found) => {
                        m.discover_error = None;
                        if offer {
                            let new = to_offer(&found, &m.offers);
                            if !new.is_empty() {
                                m.offers.asked.extend(new.iter().map(|c| c.stable_id.clone()));
                                m.offers.save(&m.root);
                            }
                            for c in new {
                                cx.emit(PeersEvent::Offer(c));
                            }
                        }
                        m.found = Some(found);
                    }
                    Err(e) => m.discover_error = Some(e.message),
                }
                cx.notify();
            });
        });
        task.detach();
    }

    /// Change the switches or the port.
    pub fn set_settings(&mut self, s: PeerSettings, cx: &mut Context<Self>) {
        self.settings = s;
        cx.notify();
        self.call("settings", cx, move |p| p.set_settings(s).map(|_| ()));
    }

    /// Show `name`'s agents here (pairs; turns showing on first if needed).
    pub fn pair(&mut self, stable_id: &str, cx: &mut Context<Self>) {
        let mut s = self.settings;
        let id = stable_id.to_string();
        self.call(stable_id, cx, move |p| {
            if !s.show {
                s.show = true;
                p.set_settings(s)?;
            }
            p.pair(&id).map(|_| ())
        });
    }

    pub fn unpair(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        self.call(&id.clone(), cx, move |p| p.unpair(&id).map(|_| ()));
    }

    pub fn revoke(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        self.call(&id.clone(), cx, move |p| p.revoke(&id).map(|_| ()));
    }

    /// Reconnect every offline link now.
    pub fn kick(&mut self, cx: &mut Context<Self>) {
        self.call("kick", cx, |p| {
            p.kick();
            Ok(())
        });
    }

    /// Run `f` on the peers off the UI thread, `key` busy meanwhile; then
    /// read everything again.
    fn call(
        &mut self,
        key: &str,
        cx: &mut Context<Self>,
        f: impl FnOnce(&Peers) -> Result<(), pitwall_proto::ErrorBody> + Send + 'static,
    ) {
        let key = key.to_string();
        self.busy.insert(key.clone());
        self.error = None;
        cx.notify();
        let peers = self.peers.clone();
        let run = cx.background_executor().spawn(async move { f(&peers) });
        let task = cx.spawn(async move |this, cx| {
            let r = run.await;
            let _ = this.update(cx, |m, cx| {
                m.busy.remove(&key);
                if let Err(e) = r {
                    m.error = Some(e.message);
                }
                m.check(cx);
            });
        });
        task.detach();
    }

    /// The offer's answer: Show pairs, Not now leaves it (asked either way).
    pub fn answer(&mut self, c: &Candidate, show: bool, cx: &mut Context<Self>) {
        if self.offers.asked.insert(c.stable_id.clone()) {
            self.offers.save(&self.root);
        }
        cx.emit(PeersEvent::Answered(c.stable_id.clone()));
        if show {
            self.pair(&c.stable_id, cx);
        }
    }

    /// The sidebar card: on turns showing on (and discovery offers what it
    /// finds); either way it doesn't come back.
    pub fn answer_card(&mut self, show: bool, cx: &mut Context<Self>) {
        self.offers.card_done = true;
        self.offers.save(&self.root);
        if show {
            let s = PeerSettings { show: true, ..self.settings };
            self.set_settings(s, cx);
        }
        cx.notify();
    }

    pub fn card(&self) -> bool {
        card(&self.settings, &self.machines, &self.offers)
    }
}

/// Start peers and the model (once, after the store listens): run what the
/// settings say, follow the store's peer events, watch `peers/` for the
/// CLI's changes and look at Tailscale now and then.
pub fn init(peers: Arc<Peers>, tailnet: Arc<dyn Tailnet>, root: PathBuf, store: &Entity<AgentStore>, cx: &mut App) -> Entity<PeersModel> {
    cx.set_global(PeersHandle(peers.clone()));
    let model = cx.new(|_| PeersModel::new(peers.clone(), tailnet, root.clone()));
    cx.set_global(ModelHandle(model.clone()));
    let m = model.downgrade();
    cx.subscribe(store, move |_, e: &StoreEvent, cx| {
        if matches!(e, StoreEvent::Peers) {
            let _ = m.update(cx, |m, cx| m.refresh(cx));
        }
    })
    .detach();
    // Start off the UI thread (the listener reads Tailscale's status), then
    // the first look.
    let start = cx.background_executor().spawn({
        let peers = peers.clone();
        async move { peers.start() }
    });
    let m = model.downgrade();
    cx.spawn(async move |cx| {
        start.await;
        loop {
            if m.update(cx, |m, cx| m.check(cx)).is_err() {
                break;
            }
            cx.background_executor().timer(CHECK).await;
        }
    })
    .detach();
    watch(peers, root, model.downgrade(), cx);
    model
}

/// What changes when a file changes.
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

fn stamps(root: &Path) -> Vec<Option<(SystemTime, u64)>> {
    let dir = root.join("peers");
    ["settings.json", "peers.json"].iter().map(|f| stamp(&dir.join(f))).collect()
}

/// `pitwall peers …` changed `peers/`: apply it in this Pitwall (the
/// listener, links, revoked viewers) and redraw.
fn watch(peers: Arc<Peers>, root: PathBuf, model: gpui::WeakEntity<PeersModel>, cx: &mut App) {
    let mut seen = stamps(&root);
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(WATCH).await;
        let now = stamps(&root);
        if now == seen {
            if model.upgrade().is_none() {
                break;
            }
            continue;
        }
        seen = now;
        let p = peers.clone();
        cx.background_executor().spawn(async move { p.reload() }).await;
        if model.update(cx, |m, cx| m.refresh(cx)).is_err() {
            break;
        }
    })
    .detach();
}
