//! The Tailscale backend (collaboration network.md §15.5): devices on the
//! user's tailnet, through the system's Tailscale.
//!
//! - **identity:** `{tailscale, UserProfile.LoginName}` from `whois` on the
//!   connection; the device is the node's `StableID`. Tagged devices have
//!   no user (Tailscale calls their owner `tagged-devices`), so each one is
//!   its own identity, `node:<StableID>`: two tagged devices are never the
//!   same caller.
//! - **groups:** from the Pitwall app capability in `whois`'s `CapMap`
//!   ([`CAP_NAME`], teams.md §4.8).
//! - **serve:** an HTTP/1.1 listener on 47421 bound only to this node's
//!   Tailscale IPs, source checked against the tailnet ranges and `whois`
//!   before a byte is read ([`serve`]); per-caller limits.
//! - **client:** plain HTTP (WireGuard encrypts) straight to the peer's
//!   tailnet IP from `status`, never DNS ([`client`]).
//! - **endpoint:** `http://<MagicDNS name>:47421`, no client auth.
//! - **discover:** candidates from `status` ([`Discovery`]); the
//!   network-neutral [`Prober`](super::probe::Prober) probes them.
//! - **revocations:** `status` polled every 30 s while anyone listens or
//!   the listener runs: nodes that disappear or change owner, logins that
//!   disappear ([`watch`]).
//!
//! Everything Tailscale-specific comes through [`LocalApi`] (the `tailscale`
//! CLI in production), so the backend runs in tests on a scripted tailnet
//! with listeners on 127.0.0.1 (`TestHooks`, test builds only).

mod api;
mod client;
mod http1;
mod serve;
mod watch;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub use self::api::{
    is_tailnet_ip, CliApi, LocalApi, PeerStatus, Status, Tailnet, UserProfile, WhoIs, WhoIsNode, TAGGED_DEVICES,
};
use super::{
    Caller, ClientAuth, DeviceAddr, Endpoint, GroupRef, Handler, HttpClient, Identity, NetState, Network, NetworkId,
    PeerHint, Revocation, ServeHandle,
};
use crate::error::{ErrorCode, PwError, Result};

/// The provider name of Tailscale identities and network ids.
pub const PROVIDER: &str = "tailscale";

/// Collaboration's port (protocol.md §8.1), beside peers' 47420.
pub const DEFAULT_PORT: u16 = 47421;

/// The Pitwall app capability in a grant's `app` field (teams.md §4.8).
/// The domain is open question 3 ("a domain the project controls"); until
/// it is settled this is the project's GitHub Pages domain.
pub const CAP_NAME: &str = "aleksandrejavakhishvili.github.io/cap/pitwall";

/// The suffix of the group an admin entry of [`CAP_NAME`] grants:
/// `{"group": "platform", "admin": true}` puts the caller in `platform`
/// and `platform:admin` (a binding's `adminGroup`).
pub const ADMIN_SUFFIX: &str = ":admin";

/// The default tag that opts a device in to discovery (teams.md §3).
pub const DEFAULT_TAG: &str = "tag:pitwall";

/// Which tailnet devices [`Network::discover`] offers (Settings → Networks
/// → Tailscale → "Look for teams on", teams.md §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Discovery {
    /// Devices with the tag when any device in the tailnet has a tag,
    /// every online device otherwise (the spec's default).
    Auto { tag: String },
    /// Only devices with this tag.
    Tagged(String),
    /// Every online device in the tailnet.
    Online,
    /// Only these devices, by MagicDNS name (`studio` or
    /// `studio.example.ts.net`), host name or tailnet IP.
    Named(Vec<String>),
}

impl Default for Discovery {
    fn default() -> Discovery {
        Discovery::Auto { tag: DEFAULT_TAG.into() }
    }
}

/// Per-caller limits on the listener (protocol.md §8.1), the same on every
/// network. A caller is a person (linked identities share them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Open connections per caller (each open stream holds one).
    pub connections: usize,
    /// Sustained requests per second per caller…
    pub requests_per_sec: u32,
    /// …with bursts up to this many.
    pub burst: u32,
    /// Request body bytes (2 MiB, `collab::MAX_BODY`).
    pub max_body: usize,
    /// Open streaming responses (SSE) per caller (`collab::MAX_STREAMS`).
    pub streams: usize,
    /// Connections in all, authenticated or not (threads are per
    /// connection).
    pub total_connections: usize,
    /// A request's line and headers.
    pub max_head: usize,
    /// An idle keep-alive connection, or a slow request, is closed after
    /// this long.
    pub idle: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            connections: 16,
            requests_per_sec: 100,
            burst: 300,
            max_body: 2 * 1024 * 1024,
            streams: 8,
            total_connections: 256,
            max_head: 16 * 1024,
            idle: Duration::from_secs(60),
        }
    }
}

/// How the backend runs.
#[derive(Clone)]
pub struct TailscaleConfig {
    /// The listener's port, and the port peers are reached on.
    pub port: u16,
    pub discovery: Discovery,
    pub limits: Limits,
    /// How long a `status` answer is reused (tailnet-peers.md §4: 5 s).
    pub status_ttl: Duration,
    /// How long a `whois` answer is reused per address (60 s).
    pub whois_ttl: Duration,
    /// How often `status` is polled for revocations and address changes
    /// while anyone listens or the listener runs (30 s).
    pub poll: Duration,
    /// Tests only: listen on and connect to 127.0.0.1, with the scripted
    /// tailnet telling ports and who connects. Never in a release build.
    #[cfg(any(test, feature = "testing"))]
    pub test: Option<Arc<dyn TestHooks>>,
}

impl Default for TailscaleConfig {
    fn default() -> TailscaleConfig {
        TailscaleConfig {
            port: DEFAULT_PORT,
            discovery: Discovery::default(),
            limits: Limits::default(),
            status_ttl: Duration::from_secs(5),
            whois_ttl: Duration::from_secs(60),
            poll: Duration::from_secs(30),
            #[cfg(any(test, feature = "testing"))]
            test: None,
        }
    }
}

/// What a scripted tailnet does in tests that the kernel and Tailscale do
/// in production: every device listens on 127.0.0.1, so ports differ per
/// device and `whois` can only tell callers apart by their source port.
#[cfg(any(test, feature = "testing"))]
pub trait TestHooks: Send + Sync {
    /// Device `node` listens on `addr`.
    fn bound(&self, node: &str, addr: SocketAddr);
    /// Where device `node` listens, if it does.
    fn port_of(&self, node: &str) -> Option<u16>;
    /// Device `from` opened a connection from `local`, before sending a
    /// byte (so the scripted `whois` knows it).
    fn connected(&self, from: &str, local: SocketAddr);
}

/// The Tailscale [`Network`]. Cheap to clone; clones share caches,
/// subscribers and the listener.
#[derive(Clone)]
pub struct TailscaleNetwork {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    /// Itself, for threads that must own it (the listener's).
    me: std::sync::Weak<Inner>,
    api: Arc<dyn LocalApi>,
    config: TailscaleConfig,
    status: Mutex<Option<(Instant, std::result::Result<Status, PwError>)>>,
    whois: Mutex<HashMap<SocketAddr, (Instant, Option<WhoIs>)>>,
    id: Mutex<Option<NetworkId>>,
    subscribers: Mutex<Vec<Sender<Revocation>>>,
    serving: Mutex<Option<serve::Serving>>,
    next_serve: Mutex<u64>,
    watcher: Mutex<Option<watch::Watcher>>,
    /// What the last poll saw, to tell who disappeared.
    seen: Mutex<Option<watch::Snapshot>>,
    clients: client::Agents,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl TailscaleNetwork {
    /// The backend on this machine's Tailscale (the `tailscale` CLI).
    pub fn local(config: TailscaleConfig) -> TailscaleNetwork {
        TailscaleNetwork::new(Arc::new(CliApi::new()), config)
    }

    /// [`local`](Self::local), when Tailscale is installed here (its CLI
    /// was found); `None` otherwise.
    pub fn installed(config: TailscaleConfig) -> Option<TailscaleNetwork> {
        let api = CliApi::new();
        api.installed().then(|| TailscaleNetwork::new(Arc::new(api), config))
    }

    /// The backend on any [`LocalApi`] (tests script one).
    pub fn new(api: Arc<dyn LocalApi>, config: TailscaleConfig) -> TailscaleNetwork {
        TailscaleNetwork {
            inner: Arc::new_cyclic(|me| Inner {
                me: me.clone(),
                api,
                config,
                status: Mutex::new(None),
                whois: Mutex::new(HashMap::new()),
                id: Mutex::new(None),
                subscribers: Mutex::new(Vec::new()),
                serving: Mutex::new(None),
                next_serve: Mutex::new(0),
                watcher: Mutex::new(None),
                seen: Mutex::new(None),
                clients: client::Agents::default(),
            }),
        }
    }

    pub fn config(&self) -> &TailscaleConfig {
        &self.inner.config
    }

    /// Where the listener listens, while it does.
    pub fn listening(&self) -> Vec<SocketAddr> {
        lock(&self.inner.serving).as_ref().map(|s| s.addrs()).unwrap_or_default()
    }

    /// Poll `status` now (as the watcher does every [`TailscaleConfig::poll`]):
    /// announce revocations and rebind the listener if the addresses changed.
    pub fn refresh(&self) {
        self.inner.forget_status();
        watch::tick(&self.inner);
    }
}

impl Inner {
    /// `status`, cached for `status_ttl`.
    pub(crate) fn status(&self) -> Result<Status> {
        let mut cache = lock(&self.status);
        if let Some((at, answer)) = cache.as_ref() {
            if at.elapsed() < self.config.status_ttl {
                return answer.clone();
            }
        }
        let answer = self.api.status();
        if let Ok(status) = &answer {
            if let Some(name) = status.tailnet_name() {
                *lock(&self.id) = Some(NetworkId::new(PROVIDER, &name));
            }
        }
        *cache = Some((Instant::now(), answer.clone()));
        answer
    }

    pub(crate) fn forget_status(&self) {
        *lock(&self.status) = None;
    }

    /// A ready status (this node signed in), or why not.
    fn ready(&self) -> Result<Status> {
        let status = self.status()?;
        match status.net_state() {
            NetState::Ready => Ok(status),
            state => Err(PwError::unreachable(format!("Tailscale isn't ready ({})", state_word(&state)))),
        }
    }

    pub(crate) fn id(&self) -> NetworkId {
        if let Some(id) = lock(&self.id).clone() {
            return id;
        }
        let _ = self.status();
        lock(&self.id).clone().unwrap_or_else(|| NetworkId::new(PROVIDER, "unknown"))
    }

    /// Whether a connection may come from, or go to, `ip`: the tailnet
    /// ranges, plus loopback in tests.
    pub(crate) fn address_ok(&self, ip: IpAddr) -> bool {
        is_tailnet_ip(ip) || (self.testing() && ip.is_loopback())
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn testing(&self) -> bool {
        self.config.test.is_some()
    }

    #[cfg(not(any(test, feature = "testing")))]
    pub(crate) fn testing(&self) -> bool {
        false
    }

    /// `whois` for a connection's source, cached for `whois_ttl` per
    /// address (per address and port in tests, where every device is
    /// 127.0.0.1).
    pub(crate) fn whois(&self, addr: SocketAddr) -> Result<Option<WhoIs>> {
        let key = if addr.ip().is_loopback() { addr } else { SocketAddr::new(addr.ip(), 0) };
        if let Some((at, answer)) = lock(&self.whois).get(&key) {
            if at.elapsed() < self.config.whois_ttl {
                return Ok(answer.clone());
            }
        }
        let answer = self.api.whois(addr)?;
        // Only answers that name a node are kept: a device that just joined
        // isn't refused for a minute.
        if answer.is_some() {
            let mut cache = lock(&self.whois);
            cache.retain(|_, (at, _)| at.elapsed() < self.config.whois_ttl);
            cache.insert(key, (Instant::now(), answer.clone()));
        }
        Ok(answer)
    }

    /// Drop cached `whois` answers for a node (it was removed or changed
    /// hands), so the next request asks again.
    pub(crate) fn forget_node(&self, stable_id: &str) {
        lock(&self.whois).retain(|_, (_, w)| w.as_ref().is_none_or(|w| w.node.stable_id != stable_id));
    }

    /// The proven caller behind `addr`, or `None`: not a node of this
    /// tailnet. Built only here, after `whois`.
    pub(crate) fn caller(&self, addr: SocketAddr, handler: &dyn Handler) -> Result<Option<Caller>> {
        if !self.address_ok(addr.ip()) {
            return Ok(None);
        }
        let Some(whois) = self.whois(addr)? else { return Ok(None) };
        if whois.node.stable_id.is_empty() {
            return Ok(None);
        }
        let network = self.id();
        let identity = identity_of(&whois.user_profile, &whois.node.stable_id, &whois.node.tags, &whois.node.name);
        let groups = groups_of(&network, &whois);
        let person = handler.person_of(&identity);
        let device = DeviceAddr { network: network.clone(), node: whois.node.stable_id.clone() };
        Ok(Some(Caller::new(identity, person, Some(device), network, groups)))
    }

    pub(crate) fn announce(&self, revocation: Revocation) {
        lock(&self.subscribers).retain(|tx| tx.send(revocation.clone()).is_ok());
    }

    pub(crate) fn has_subscribers(&self) -> bool {
        !lock(&self.subscribers).is_empty()
    }
}

fn state_word(state: &NetState) -> String {
    match state {
        NetState::Off => "off".into(),
        NetState::SignedOut => "signed out".into(),
        NetState::Connecting => "connecting".into(),
        NetState::Ready => "ready".into(),
        NetState::Error { message } => message.clone(),
    }
}

/// The identity of a node's user: the login, or for tagged devices the
/// node itself (`node:<StableID>`).
pub(crate) fn identity_of(user: &UserProfile, stable_id: &str, tags: &[String], node_name: &str) -> Identity {
    let login = user.login_name.as_str();
    if !tags.is_empty() || login.is_empty() || login == TAGGED_DEVICES {
        let label = node_name.split('.').next().filter(|s| !s.is_empty()).map(str::to_string);
        return Identity { name: label, ..Identity::new(PROVIDER, &format!("node:{stable_id}")) };
    }
    Identity {
        email: login.contains('@').then(|| login.to_string()),
        name: Some(user.display_name.clone()).filter(|n| !n.is_empty()),
        ..Identity::new(PROVIDER, login)
    }
}

/// Groups from the Pitwall app capability (§4.8). Each grant parameter is
/// `{"group": "platform", "admin"?: bool, "logins"?: [login]}`: the caller
/// is in `platform`, and in `platform:admin` when `admin` is true; with
/// `logins`, only those logins are. Anything else is ignored.
pub(crate) fn groups_of(network: &NetworkId, whois: &WhoIs) -> Vec<GroupRef> {
    let login = whois.user_profile.login_name.as_str();
    let mut ids: Vec<String> = Vec::new();
    for param in whois.cap_map.get(CAP_NAME).into_iter().flatten() {
        let Some(group) = param.get("group").and_then(|g| g.as_str()).filter(|g| !g.is_empty()) else { continue };
        if let Some(logins) = param.get("logins").and_then(|l| l.as_array()) {
            if !logins.iter().any(|l| l.as_str() == Some(login)) {
                continue;
            }
        }
        ids.push(group.to_string());
        if param.get("admin").and_then(|a| a.as_bool()) == Some(true) {
            ids.push(format!("{group}{ADMIN_SUFFIX}"));
        }
    }
    ids.sort();
    ids.dedup();
    ids.into_iter().map(|id| GroupRef { network: network.clone(), id }).collect()
}

impl Network for TailscaleNetwork {
    fn id(&self) -> NetworkId {
        self.inner.id()
    }

    fn state(&self) -> NetState {
        match self.inner.status() {
            Ok(status) => status.net_state(),
            Err(e) if e.is(ErrorCode::Unsupported) || e.is(ErrorCode::NotRunning) => NetState::Off,
            Err(e) => NetState::Error { message: e.message },
        }
    }

    fn me(&self) -> Option<(Identity, DeviceAddr)> {
        let status = self.inner.ready().ok()?;
        let me = status.self_.as_ref()?;
        let user = status.user(me.user_id).cloned().unwrap_or_default();
        let identity = identity_of(&user, &me.id, &me.tags, &me.dns_name);
        Some((identity, DeviceAddr { network: self.inner.id(), node: me.id.clone() }))
    }

    fn serve(&self, handler: Arc<dyn Handler>) -> Result<ServeHandle> {
        serve::start(&self.inner, handler)
    }

    fn client(&self, to: &DeviceAddr) -> Result<Box<dyn HttpClient>> {
        client::to(&self.inner, to)
    }

    fn endpoint(&self) -> Endpoint {
        let port = lock(&self.inner.serving).as_ref().map_or(self.inner.config.port, |s| s.port());
        let host = self.inner.status().ok().and_then(|s| s.self_).map(|me| {
            if !me.dns().is_empty() {
                me.dns().to_string()
            } else {
                me.tailscale_ips.first().map(ip_host).unwrap_or_default()
            }
        });
        // Empty until Tailscale names this device (signed out, stopped).
        let base_url = host.filter(|h| !h.is_empty()).map(|h| format!("http://{h}:{port}")).unwrap_or_default();
        Endpoint { base_url, client_auth: ClientAuth::None }
    }

    fn discover(&self) -> Result<Vec<PeerHint>> {
        let status = self.inner.ready()?;
        Ok(candidates(&status, &self.inner.config.discovery, &self.inner.id()))
    }

    fn groups(&self, who: &Identity) -> Result<Vec<GroupRef>> {
        if who.provider != PROVIDER {
            return Ok(Vec::new());
        }
        let status = self.inner.ready()?;
        let network = self.inner.id();
        // Grants are per source device; any of the person's devices will do.
        for peer in status.peer.values() {
            let user = status.user(peer.user_id).cloned().unwrap_or_default();
            if !identity_of(&user, &peer.id, &peer.tags, &peer.dns_name).is(who) {
                continue;
            }
            let Some(ip) = peer.tailscale_ips.first() else { continue };
            if let Some(whois) = self.inner.whois(SocketAddr::new(*ip, 0))? {
                return Ok(groups_of(&network, &whois));
            }
        }
        Ok(Vec::new())
    }

    fn revocations(&self) -> Receiver<Revocation> {
        let (tx, rx) = std::sync::mpsc::channel();
        lock(&self.inner.subscribers).push(tx);
        watch::ensure(&self.inner);
        rx
    }
}

/// `100.64.1.2` or `[fd7a:…]`, for URLs.
pub(crate) fn ip_host(ip: &IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    }
}

/// At most this many candidates per refresh (teams.md §3).
pub const MAX_CANDIDATES: usize = 256;

/// The devices `discovery` offers, from a status: never this device;
/// online first; at most [`MAX_CANDIDATES`].
pub(crate) fn candidates(status: &Status, discovery: &Discovery, network: &NetworkId) -> Vec<PeerHint> {
    let any_tags = status.peer.values().any(|p| p.tagged());
    let wanted = |p: &PeerStatus| match discovery {
        Discovery::Auto { tag } if any_tags => p.tags.iter().any(|t| t == tag),
        Discovery::Auto { .. } | Discovery::Online => p.online,
        Discovery::Tagged(tag) => p.tags.iter().any(|t| t == tag),
        Discovery::Named(names) => names.iter().any(|n| {
            let n = n.trim_end_matches('.');
            n.eq_ignore_ascii_case(p.dns())
                || n.eq_ignore_ascii_case(&p.label())
                || n.eq_ignore_ascii_case(&p.host_name)
                || p.tailscale_ips.iter().any(|ip| ip.to_string() == n)
        }),
    };
    let me = status.self_.as_ref().map(|s| s.id.as_str());
    let mut hints: Vec<PeerHint> = status
        .peer
        .values()
        .filter(|p| Some(p.id.as_str()) != me && !p.id.is_empty() && wanted(p))
        .map(|p| {
            let user = status.user(p.user_id).cloned().unwrap_or_default();
            let owner = (!p.tagged() && user.login_name != TAGGED_DEVICES && !user.login_name.is_empty())
                .then(|| identity_of(&user, &p.id, &p.tags, &p.dns_name));
            PeerHint {
                addr: DeviceAddr { network: network.clone(), node: p.id.clone() },
                owner,
                label: p.label(),
                online: p.online,
            }
        })
        .collect();
    hints.sort_by(|a, b| b.online.cmp(&a.online).then_with(|| a.label.cmp(&b.label)));
    hints.truncate(MAX_CANDIDATES);
    hints
}
