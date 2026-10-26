//! A scripted tailnet for tests (tailnet-peers.md §9 "Fake tailnet"): no
//! Tailscale anywhere. Every Pitwall listens on `127.0.0.1` (the OS picks
//! the port, which the world remembers per node), and since every caller
//! is `127.0.0.1` too, `whois` tells them apart by the source port each
//! side reports right after connecting. Deterministic on macOS, Windows
//! and Linux, no loopback aliases, no root.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use pitwall_proto::{code, ErrorBody};

use crate::lock;
use crate::tailnet::{Node, Status, Tailnet, UserProfile, WhoIs, WhoIsNode, TAGGED_DEVICES};

/// One made-up machine.
#[derive(Debug, Clone)]
pub struct FakeNode {
    pub id: String,
    pub name: String,
    pub user_id: i64,
    pub login: String,
    pub tags: Vec<String>,
    pub online: bool,
}

/// The tailnet every [`FakeTailnet`] of a test shares.
pub struct FakeWorld {
    nodes: Mutex<Vec<FakeNode>>,
    /// node id → where it listens.
    ports: Mutex<HashMap<String, u16>>,
    /// caller's source port → its node id.
    callers: Mutex<HashMap<u16, String>>,
    called: Condvar,
    /// How long `whois` waits for a caller to say who it is.
    whois_wait: Mutex<Duration>,
    /// `status` fails (Tailscale stopped) while set.
    down: Mutex<bool>,
    /// Per node: `status` calls and connections opened.
    counts: Mutex<HashMap<String, (usize, usize)>>,
}

impl FakeWorld {
    pub fn new() -> Arc<FakeWorld> {
        Arc::new(FakeWorld {
            nodes: Mutex::new(Vec::new()),
            ports: Mutex::new(HashMap::new()),
            callers: Mutex::new(HashMap::new()),
            called: Condvar::new(),
            whois_wait: Mutex::new(Duration::from_secs(5)),
            down: Mutex::new(false),
            counts: Mutex::new(HashMap::new()),
        })
    }

    /// Add a machine of user `login` (ids are given per login).
    pub fn add(&self, id: &str, name: &str, login: &str) -> &Self {
        let mut nodes = lock(&self.nodes);
        let user_id = match nodes.iter().find(|n| n.login == login) {
            Some(n) => n.user_id,
            None => nodes.iter().map(|n| n.user_id).max().unwrap_or(0) + 1,
        };
        nodes.push(FakeNode { id: id.into(), name: name.into(), user_id, login: login.into(), tags: Vec::new(), online: true });
        self
    }

    /// Add a tagged device (no user).
    pub fn add_tagged(&self, id: &str, name: &str, tag: &str) -> &Self {
        self.add(id, name, TAGGED_DEVICES);
        self.edit(id, |n| n.tags = vec![tag.into()]);
        self
    }

    pub fn edit(&self, id: &str, f: impl FnOnce(&mut FakeNode)) {
        if let Some(n) = lock(&self.nodes).iter_mut().find(|n| n.id == id) {
            f(n);
        }
    }

    pub fn set_online(&self, id: &str, online: bool) {
        self.edit(id, |n| n.online = online);
    }

    pub fn set_down(&self, down: bool) {
        *lock(&self.down) = down;
    }

    pub fn set_whois_wait(&self, d: Duration) {
        *lock(&self.whois_wait) = d;
    }

    /// Where node `id` listens, once it does.
    pub fn port_of(&self, id: &str) -> Option<u16> {
        lock(&self.ports).get(id).copied()
    }

    /// How often node `id` asked for the status.
    pub fn status_calls(&self, id: &str) -> usize {
        lock(&self.counts).get(id).map_or(0, |c| c.0)
    }

    /// How many connections node `id` opened.
    pub fn dials(&self, id: &str) -> usize {
        lock(&self.counts).get(id).map_or(0, |c| c.1)
    }

    /// Node `id`'s view of the tailnet.
    pub fn tailnet(self: &Arc<Self>, id: &str) -> Arc<FakeTailnet> {
        Arc::new(FakeTailnet { world: self.clone(), me: id.into() })
    }

    fn node(n: &FakeNode, i: usize) -> Node {
        Node {
            id: n.id.clone(),
            host_name: n.name.clone(),
            dns_name: format!("{}.example.ts.net.", n.name),
            os: "macOS".into(),
            user_id: n.user_id,
            tailscale_ips: vec![IpAddr::V4(Ipv4Addr::new(100, 64, 0, i as u8 + 1))],
            tags: n.tags.clone(),
            online: n.online,
        }
    }
}

/// One node's [`Tailnet`] in a [`FakeWorld`].
pub struct FakeTailnet {
    world: Arc<FakeWorld>,
    me: String,
}

impl Tailnet for FakeTailnet {
    fn status(&self) -> Result<Status, ErrorBody> {
        lock(&self.world.counts).entry(self.me.clone()).or_default().0 += 1;
        if *lock(&self.world.down) {
            return Err(ErrorBody::new(code::NOT_RUNNING, "Tailscale isn't running"));
        }
        let nodes = lock(&self.world.nodes);
        let mut s = Status { backend_state: "Running".into(), magic_dns_suffix: "example.ts.net".into(), tun: Some(true), ..Status::default() };
        for (i, n) in nodes.iter().enumerate() {
            let node = FakeWorld::node(n, i);
            s.user.insert(n.user_id.to_string(), UserProfile { id: n.user_id, login_name: n.login.clone(), display_name: n.name.clone() });
            if n.id == self.me {
                s.self_ = Some(node);
            } else {
                s.peer.insert(format!("nodekey:{}", n.id), node);
            }
        }
        Ok(s)
    }

    fn whois(&self, addr: SocketAddr) -> Result<Option<WhoIs>, ErrorBody> {
        let deadline = Instant::now() + *lock(&self.world.whois_wait);
        let mut callers = lock(&self.world.callers);
        let id = loop {
            if let Some(id) = callers.get(&addr.port()) {
                break id.clone();
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            callers = self.world.called.wait_timeout(callers, left).unwrap_or_else(|e| e.into_inner()).0;
        };
        drop(callers);
        let nodes = lock(&self.world.nodes);
        Ok(nodes.iter().find(|n| n.id == id).map(|n| WhoIs {
            node: WhoIsNode { stable_id: n.id.clone(), name: format!("{}.example.ts.net.", n.name), tags: n.tags.clone() },
            user_profile: UserProfile { id: n.user_id, login_name: n.login.clone(), display_name: n.name.clone() },
        }))
    }

    fn bind_ips(&self, _me: &Node) -> Vec<IpAddr> {
        vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]
    }

    fn address_ok(&self, ip: IpAddr) -> bool {
        ip.is_loopback()
    }

    fn dial_addr(&self, node: &Node, _port: u16) -> Option<SocketAddr> {
        // Port 1 when it never listened: nothing answers there.
        Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.world.port_of(&node.id).unwrap_or(1)))
    }

    fn listening(&self, addr: SocketAddr) {
        lock(&self.world.ports).insert(self.me.clone(), addr.port());
    }

    fn dialed(&self, local: SocketAddr) {
        lock(&self.world.counts).entry(self.me.clone()).or_default().1 += 1;
        lock(&self.world.callers).insert(local.port(), self.me.clone());
        self.world.called.notify_all();
    }
}
