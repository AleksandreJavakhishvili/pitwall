//! Two (or three) Pitwalls on a [`FakeTailnet`], each with its own data
//! folder, talking over 127.0.0.1: pairing, the mirror, identity changes,
//! refusals, denied methods and reconnects. Waits are "until it happened"
//! with a generous cap, never fixed sleeps.

use std::io::Read;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pitwall_core::events::{Event, EventSink};
use pitwall_core::model::Attention;
use pitwall_core::testing::TempDir;
use pitwall_daemon::events::{self, Hub, Queue};
use pitwall_proto::frame;
use pitwall_proto::peer::{LinkState, Pairing, PeerClientHello, PeerHello, IDENTITY_CHANGED, PEER_PING};
use pitwall_proto::{code, method, AgentView, ErrorBody, Range, ServerMsg};
use serde_json::{json, Value};

use crate::dial::{self, read_json};
use crate::fake::FakeWorld;
use crate::store::{Paired, Store};
use crate::tailnet::Tailnet;
use crate::*;

fn agent(id: &str, name: &str, status: &str) -> AgentView {
    serde_json::from_value(json!({
        "id": id, "name": name, "kind": "claude", "kindName": "Claude Code", "terminal": false, "sessionId": null,
        "cwd": "/Users/me/api", "cwdDisplay": "~/api", "project": "/Users/me/api", "projectDisplay": "~/api",
        "branch": null, "worktree": false, "worktreePending": false, "location": "local",
        "machine": {"provider": "local", "id": "this-mac", "label": "This Mac", "canCreate": true},
        "agentInTerminal": false, "restartAs": null, "status": status,
        "statusSource": "screen", "statusDetail": null, "running": true, "cols": 120, "rows": 40,
        "added": 0, "removed": 0, "filesChanged": 0, "queue": [], "autoSend": true, "lastSent": null,
        "lastSentAt": null, "createdAt": 1, "currentTaskId": null,
        "caps": {"input": true, "restart": true, "resume": false, "stop": true, "removeWorktree": false,
                 "diff": true, "review": true, "merge": false, "rules": true, "hooks": true, "removeKeepsSession": false}
    }))
    .unwrap()
}

/// A Pitwall's agents without an engine: a list and the hub.
struct TestFeed {
    views: Mutex<Vec<AgentView>>,
    hub: Arc<Hub>,
}

impl TestFeed {
    fn new(views: Vec<AgentView>) -> Arc<TestFeed> {
        Arc::new(TestFeed { views: Mutex::new(views), hub: Hub::new(Arc::new(pitwall_core::events::NullSink)) })
    }

    fn set(&self, views: Vec<AgentView>) {
        *lock(&self.views) = views.clone();
        self.hub.emit(Event::AgentsChanged(views));
    }
}

impl Feed for TestFeed {
    fn views(&self) -> Vec<AgentView> {
        lock(&self.views).clone()
    }

    fn subscribe(&self, topics: Vec<&'static str>) -> Arc<Queue> {
        let q = self.hub.subscribe(topics);
        events::snapshot_of(&lock(&self.views), &q);
        q
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<PeerEvent>>);

impl PeerSink for Recorder {
    fn emit(&self, ev: PeerEvent) {
        lock(&self.0).push(ev);
    }
}

impl Recorder {
    fn attention(&self) -> Vec<pitwall_proto::AttentionView> {
        lock(&self.0).iter().filter_map(|e| if let PeerEvent::Attention { attention, .. } = e { Some(attention.clone()) } else { None }).collect()
    }
}

fn timing() -> Timing {
    let ms = Duration::from_millis;
    Timing {
        backoff: vec![ms(20), ms(50)],
        ping: ms(200),
        dead_after: Duration::from_secs(5),
        status_poll: ms(100),
        offline_retry: Duration::from_secs(600),
        connect: Duration::from_secs(5),
        hello: Duration::from_secs(5),
    }
}

/// One Pitwall: its folder, its tailnet view, its peers.
struct Pw {
    dir: TempDir,
    peers: Arc<Peers>,
    events: Arc<Recorder>,
    feed: Arc<TestFeed>,
}

fn pitwall(world: &Arc<FakeWorld>, node: &str, share: bool, show: bool) -> Pw {
    let dir = TempDir::new(&format!("peer-{node}"));
    pitwall_in(world, node, dir, share, show, Limits::default())
}

fn pitwall_in(world: &Arc<FakeWorld>, node: &str, dir: TempDir, share: bool, show: bool, limits: Limits) -> Pw {
    let events = Arc::new(Recorder::default());
    let feed = TestFeed::new(vec![agent("a1", "api", "working")]);
    let peers = Peers::new(PeersConfig {
        root: dir.path().to_path_buf(),
        tailnet: world.tailnet(node),
        version: "0.2.1".into(),
        sink: events.clone(),
        feed: Some(feed.clone()),
        timing: timing(),
        limits,
    })
    .unwrap();
    peers.set_settings(PeerSettings { share, show, port: 0 }).unwrap();
    peers.start();
    Pw { dir, peers, events, feed }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// alice's `laptop` and `studio`, and bob's `bobs`.
fn world() -> Arc<FakeWorld> {
    let w = FakeWorld::new();
    w.add("nLAPTOP", "laptop", "alice@example.com").add("nSTUDIO", "studio", "alice@example.com").add("nBOB", "bobs", "bob@example.com");
    w
}

fn listening(world: &FakeWorld, node: &str) -> u16 {
    let mut port = None;
    wait_until(&format!("{node} listens"), || {
        port = world.port_of(node);
        port.is_some()
    });
    port.unwrap()
}

#[test]
fn pair_without_a_code_and_mirror_the_agents() {
    let w = world();
    let studio = pitwall(&w, "nSTUDIO", true, false);
    listening(&w, "nSTUDIO");
    let laptop = pitwall(&w, "nLAPTOP", false, true);

    // Discovery: the user's own machines only, probed for a Pitwall.
    let found = laptop.peers.discover().unwrap();
    assert_eq!(found.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["studio"], "bob's machine isn't offered");
    assert_eq!(found[0].probe, Probe::Pitwall { version: "0.2.1".into(), label: "studio".into(), pairing: Pairing::Open, agents: 1 });
    assert_eq!(found[0].paired, None);

    let entry = laptop.peers.pair("studio").unwrap();
    assert_eq!((entry.paired.id.as_str(), entry.paired.login.as_str(), entry.paired.stable_id.as_str()), ("studio", "alice@example.com", "nSTUDIO"));
    assert_eq!(entry.paired.public_key, studio.peers.info().public_key);
    // The listener remembers the viewer too, and can revoke it.
    let viewers = studio.peers.viewers();
    assert_eq!((viewers.len(), viewers[0].label.as_str()), (1, "laptop"));
    assert_eq!(viewers[0].public_key, laptop.peers.info().public_key);

    wait_until("the mirror has studio's agent", || laptop.peers.views().len() == 1);
    wait_until("the link is connected", || laptop.peers.link_is("studio", LinkState::Connected));
    let v = &laptop.peers.views()[0];
    assert_eq!(v.id, "peer:studio:a1");
    assert_eq!((v.machine.provider.as_str(), v.machine.label.as_str(), v.machine.can_create), ("peer:studio", "studio", false));
    assert_eq!(v.machine.link.as_ref().unwrap().state, LinkState::Connected);
    assert_eq!(v.caps, pitwall_proto::AgentCaps::default(), "read-only");

    // Live: list changes, attention and the blocked count.
    let mut vm = agent("a2", "night", "blocked");
    vm.machine.provider = "agw".into();
    vm.machine.label = "vm-1".into();
    studio.feed.set(vec![agent("a1", "api", "idle"), vm]);
    wait_until("the mirror follows", || laptop.peers.views().len() == 2);
    assert_eq!(laptop.peers.views()[1].machine.label, "studio › vm-1");
    studio.feed.hub.emit(Event::Attention(Attention { agent_id: "a2".into(), name: "night".into(), reason: "blocked", detail: Some("Allow Bash?".into()) }));
    studio.feed.hub.emit(Event::BlockedCount(1));
    wait_until("attention arrives", || !laptop.events.attention().is_empty());
    assert_eq!(laptop.events.attention()[0].agent_id, "peer:studio:a2");
    wait_until("the blocked count arrives", || laptop.peers.mirror().blocked() == 1);
    assert_eq!(laptop.peers.peers()[0].agents, 2);

    let audit: Vec<String> = studio.peers.audit(50).into_iter().map(|e| e.event).collect();
    assert!(audit.contains(&"pair".to_string()) && audit.contains(&"connect".to_string()), "{audit:?}");
    assert_eq!(laptop.peers.discover().unwrap()[0].paired.as_deref(), Some("studio"));
}

#[test]
fn nothing_listens_while_sharing_is_off() {
    let w = world();
    let studio = pitwall(&w, "nSTUDIO", false, false);
    let laptop = pitwall(&w, "nLAPTOP", false, true);
    assert!(studio.peers.info().listening.is_empty());
    assert!(matches!(laptop.peers.discover().unwrap()[0].probe, Probe::NoAnswer { .. }));
    assert_eq!(laptop.peers.pair("studio").unwrap_err().code, code::UNREACHABLE);
    // Turning it on listens; off again closes everything.
    studio.peers.set_settings(PeerSettings { share: true, show: false, port: 0 }).unwrap();
    assert_eq!(studio.peers.info().listening.len(), 1);
    assert!(studio.peers.info().listening.iter().all(|a| a.ip().is_loopback()));
    laptop.peers.pair("studio").unwrap();
    wait_until("connected", || laptop.peers.link_is("studio", LinkState::Connected));
    studio.peers.set_settings(PeerSettings { share: false, show: false, port: 0 }).unwrap();
    wait_until("the link notices", || laptop.peers.link_is("studio", LinkState::Offline));
    assert_eq!(laptop.peers.views().len(), 1, "last known rows stay");
    assert_eq!(laptop.peers.views()[0].machine.link.as_ref().unwrap().state, LinkState::Offline);
}

#[test]
fn the_viewer_needs_its_own_switch() {
    let w = world();
    let _studio = pitwall(&w, "nSTUDIO", true, false);
    let laptop = pitwall(&w, "nLAPTOP", false, false);
    assert_eq!(laptop.peers.discover().unwrap_err().code, code::UNSUPPORTED);
    assert_eq!(laptop.peers.pair("studio").unwrap_err().code, code::UNSUPPORTED);
}

/// Connect as `node` and say hello; what comes back (`None`: closed
/// without a byte).
fn raw_hello(w: &Arc<FakeWorld>, from: &str, to: &str) -> Option<Value> {
    let port = listening(w, to);
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    w.tailnet(from).dialed(s.local_addr().unwrap());
    let hello = PeerClientHello::PeerHello(PeerHello { protocol: Range::ours(), client: "test".into(), nonce: "n".into() });
    let _ = frame::write_json(&mut s, &hello);
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    read_json::<Value>(&mut s).ok().flatten()
}

#[test]
fn other_users_and_tagged_devices_are_refused() {
    let w = world();
    w.add_tagged("nCI", "ci", "tag:ci");
    let studio = pitwall(&w, "nSTUDIO", true, false);
    assert!(raw_hello(&w, "nLAPTOP", "nSTUDIO").is_some(), "alice's laptop gets a welcome");
    assert_eq!(raw_hello(&w, "nBOB", "nSTUDIO"), None, "bob's machine: closed without a byte");
    assert_eq!(raw_hello(&w, "nCI", "nSTUDIO"), None, "a tagged device: closed without a byte");
    let refused: Vec<String> = studio.peers.audit(50).into_iter().filter(|e| e.event == "refused").filter_map(|e| e.detail).collect();
    assert_eq!(refused, ["another Tailscale user's machine", "a tagged device"]);

    // And the viewer never offers or pairs with them.
    let laptop = pitwall(&w, "nLAPTOP", false, true);
    assert_eq!(laptop.peers.pair("bobs").unwrap_err().code, code::DENIED);
    assert_eq!(laptop.peers.pair("ci").unwrap_err().code, code::DENIED);
    assert_eq!(laptop.peers.pair("nowhere").unwrap_err().code, code::NOT_FOUND);
}

#[test]
fn connections_from_outside_the_tailnet_get_nothing() {
    let w = world();
    w.set_whois_wait(Duration::from_millis(50));
    let studio = pitwall(&w, "nSTUDIO", true, false);
    let port = listening(&w, "nSTUDIO");
    // Nobody said who this is: whois finds no node.
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut buf = [0u8; 1];
    assert_eq!(s.read(&mut buf).unwrap_or(0), 0, "closed without a byte");
    wait_until("it is audited", || studio.peers.audit(10).iter().any(|e| e.detail.as_deref() == Some("not a node of this tailnet")));
}

/// A viewer's handshake by hand, with its own key.
fn viewer_ctx(w: &Arc<FakeWorld>, node: &str, dir: &TempDir) -> Ctx {
    Ctx {
        tailnet: w.tailnet(node),
        store: Store::new(dir.path()),
        key: InstanceKey::load_or_create(dir.path()).unwrap(),
        sink: Arc::new(NullPeerSink),
        version: "0.2.1".into(),
        timing: timing(),
        limits: Limits::default(),
    }
}

fn studio_node(w: &Arc<FakeWorld>) -> crate::tailnet::Node {
    w.tailnet("nLAPTOP").status().unwrap().node("nSTUDIO").unwrap().clone()
}

fn call(d: &mut dial::Dialed, id: u64, m: &str, params: Value) -> Result<Value, ErrorBody> {
    d.send(id, m, params).unwrap();
    loop {
        match read_json::<ServerMsg>(&mut d.reader).unwrap().expect("an answer") {
            ServerMsg::Response(r) if r.id == id => return r.into_result(),
            _ => continue,
        }
    }
}

#[test]
fn a_view_peer_may_only_list_and_follow() {
    let w = world();
    let _studio = pitwall(&w, "nSTUDIO", true, false);
    listening(&w, "nSTUDIO");
    let dir = TempDir::new("peer-viewer");
    let ctx = viewer_ctx(&w, "nLAPTOP", &dir);
    let mut d = dial::dial(&ctx, &studio_node(&w), 0).unwrap();
    // Before peer.auth nothing answers.
    assert_eq!(call(&mut d, 7, method::AGENT_LIST, Value::Null).unwrap_err().code, code::DENIED);
    d.auth(&ctx.key, "laptop", true, None).unwrap();
    assert_eq!(call(&mut d, 8, method::AGENT_LIST, Value::Null).unwrap().as_array().unwrap().len(), 1);
    assert_eq!(call(&mut d, 9, PEER_PING, Value::Null).unwrap(), Value::Null);
    for (i, m) in [method::AGENT_CREATE, method::AGENT_STOP, method::AGENT_REMOVE, method::QUEUE_ADD, method::APPROVAL_ANSWER, method::SETTINGS_SET, method::MACHINE_LIST, method::SESSION_LIST, "term.attach"].iter().enumerate() {
        assert_eq!(call(&mut d, 20 + i as u64, m, json!({})).unwrap_err().code, code::DENIED, "{m}");
    }
    assert_eq!(call(&mut d, 40, method::EVENTS_SUBSCRIBE, json!({"topics":["approvals"]})).unwrap_err().code, code::DENIED);
    let sub = call(&mut d, 41, method::EVENTS_SUBSCRIBE, json!({"topics":["agents","blocked"]})).unwrap();
    assert_eq!(sub, json!({"topics":["agents","blocked"]}));
}

#[test]
fn a_changed_key_on_either_side_needs_pairing_again() {
    let w = world();
    let studio = pitwall(&w, "nSTUDIO", true, false);
    listening(&w, "nSTUDIO");
    let laptop = pitwall(&w, "nLAPTOP", false, true);
    laptop.peers.pair("studio").unwrap();
    wait_until("connected", || laptop.peers.link_is("studio", LinkState::Connected));

    // The laptop's Pitwall gets a new key (its data folder was reset, or
    // someone else's process on that node): studio refuses it.
    let Pw { dir, peers, .. } = laptop;
    peers.stop();
    drop(peers);
    std::fs::remove_file(InstanceKey::path(dir.path())).unwrap();
    let laptop = pitwall_in(&w, "nLAPTOP", dir, false, true, Limits::default());
    wait_until("the link says identity changed", || laptop.peers.link_is("studio", LinkState::IdentityChanged));
    assert!(studio.peers.audit(50).iter().any(|e| e.event == "identity_changed"));
    // Pairing again (the user's explicit choice) replaces the old key.
    laptop.peers.pair("studio").unwrap();
    wait_until("connected again", || laptop.peers.link_is("studio", LinkState::Connected));
    assert_eq!(studio.peers.viewers().len(), 1);
    assert_eq!(studio.peers.viewers()[0].public_key, laptop.peers.info().public_key);

    // Studio's Pitwall gets a new key: the laptop refuses it.
    let Pw { dir: sdir, peers: speers, .. } = studio;
    speers.stop();
    drop(speers);
    std::fs::remove_file(InstanceKey::path(sdir.path())).unwrap();
    // It still knows the laptop as a viewer, but the laptop knows its old key.
    let _studio = pitwall_in(&w, "nSTUDIO", sdir, true, false, Limits::default());
    laptop.peers.kick();
    wait_until("the laptop notices", || laptop.peers.link_is("studio", LinkState::IdentityChanged));
}

#[test]
fn a_machine_that_changed_owner_is_refused() {
    let w = world();
    let _studio = pitwall(&w, "nSTUDIO", true, false);
    listening(&w, "nSTUDIO");
    let laptop = pitwall(&w, "nLAPTOP", false, true);
    laptop.peers.pair("studio").unwrap();
    wait_until("connected", || laptop.peers.link_is("studio", LinkState::Connected));
    w.edit("nSTUDIO", |n| {
        n.login = "bob@example.com".into();
        n.user_id = 99;
    });
    // The open connection ends (studio restarts its listener), and the
    // link won't dial a machine that now belongs to someone else.
    laptop.peers.stop();
    laptop.peers.start();
    wait_until("identity changed", || laptop.peers.link_is("studio", LinkState::IdentityChanged));
}

#[test]
fn revoking_a_viewer_closes_its_link() {
    let w = world();
    let studio = pitwall(&w, "nSTUDIO", true, false);
    listening(&w, "nSTUDIO");
    let laptop = pitwall(&w, "nLAPTOP", false, true);
    laptop.peers.pair("studio").unwrap();
    wait_until("connected", || laptop.peers.link_is("studio", LinkState::Connected));
    studio.peers.revoke("laptop").unwrap();
    assert!(studio.peers.viewers().is_empty());
    wait_until("the laptop must pair again", || laptop.peers.link_is("studio", LinkState::IdentityChanged));
    assert!(studio.peers.audit(50).iter().any(|e| e.event == "revoke"));
    // Unpairing on the laptop forgets studio and its agents.
    laptop.peers.unpair("studio").unwrap();
    assert!(laptop.peers.peers().is_empty() && laptop.peers.views().is_empty());
}

#[test]
fn links_reconnect_after_the_peer_comes_back() {
    let w = world();
    let studio = pitwall(&w, "nSTUDIO", true, false);
    listening(&w, "nSTUDIO");
    let laptop = pitwall(&w, "nLAPTOP", false, true);
    laptop.peers.pair("studio").unwrap();
    wait_until("connected", || laptop.peers.link_is("studio", LinkState::Connected));

    // Studio's Pitwall quits (or the machine sleeps).
    studio.peers.stop();
    wait_until("offline", || laptop.peers.link_is("studio", LinkState::Offline));
    let since = laptop.peers.mirror().link("studio").unwrap().since;
    assert_eq!(laptop.peers.views().len(), 1, "last known rows stay");
    assert_eq!(laptop.peers.mirror().blocked(), 0, "stale counts don't badge");
    // Asleep in Tailscale: the link keeps looking at the status but
    // doesn't dial (until `offline_retry`).
    w.set_online("nSTUDIO", false);
    let polls = w.status_calls("nLAPTOP");
    wait_until("it looked twice", || w.status_calls("nLAPTOP") >= polls + 2);
    let dials = w.dials("nLAPTOP");
    wait_until("it looked three more times", || w.status_calls("nLAPTOP") >= polls + 5);
    assert_eq!(w.dials("nLAPTOP"), dials, "no dialing a sleeping machine");
    assert!(laptop.peers.link_is("studio", LinkState::Offline));
    assert_eq!(laptop.peers.mirror().link("studio").unwrap().since, since, "offline since the first failure");
    studio.peers.start();
    // Awake again: it reconnects on its own.
    w.set_online("nSTUDIO", true);
    wait_until("reconnected", || laptop.peers.link_is("studio", LinkState::Connected));
    assert!(laptop.peers.mirror().link("studio").unwrap().last_seen.is_some());
}

#[test]
fn pairing_is_rate_limited_per_machine() {
    let w = world();
    let limits = Limits { pairs: 1, ..Limits::default() };
    let _studio = pitwall_in(&w, "nSTUDIO", TempDir::new("peer-studio"), true, false, limits);
    listening(&w, "nSTUDIO");
    let laptop = pitwall(&w, "nLAPTOP", false, true);
    laptop.peers.pair("studio").unwrap();
    // The same Pitwall again is just a connection, not a new pairing.
    laptop.peers.pair("studio").unwrap();
    assert!(matches!(laptop.peers.discover().unwrap()[0].probe, Probe::Pitwall { pairing: Pairing::Closed, .. }));
    // Another key from the same node within the window is refused.
    let dir = TempDir::new("peer-viewer");
    let ctx = viewer_ctx(&w, "nLAPTOP", &dir);
    let mut d = dial::dial(&ctx, &studio_node(&w), 0).unwrap();
    let err = d.auth(&ctx.key, "laptop", true, None).unwrap_err();
    assert_eq!(err.code, code::DENIED, "{err:?}");
}

#[test]
fn an_unpaired_viewer_must_pair_first() {
    let w = world();
    let _studio = pitwall(&w, "nSTUDIO", true, false);
    listening(&w, "nSTUDIO");
    let dir = TempDir::new("peer-viewer");
    let ctx = viewer_ctx(&w, "nLAPTOP", &dir);
    let mut d = dial::dial(&ctx, &studio_node(&w), 0).unwrap();
    assert_eq!(d.auth(&ctx.key, "laptop", false, None).unwrap_err().code, pitwall_proto::peer::NOT_PAIRED);
    // A paired viewer whose expected key doesn't match refuses the peer.
    let mut d = dial::dial(&ctx, &studio_node(&w), 0).unwrap();
    assert_eq!(d.auth(&ctx.key, "laptop", true, Some("ed25519:other")).unwrap_err().code, IDENTITY_CHANGED);
}

#[test]
fn pairings_survive_a_restart() {
    let w = world();
    let _studio = pitwall(&w, "nSTUDIO", true, false);
    listening(&w, "nSTUDIO");
    let laptop = pitwall(&w, "nLAPTOP", false, true);
    laptop.peers.pair("studio").unwrap();
    let Pw { dir, peers, .. } = laptop;
    drop(peers);
    let stored: Vec<Paired> = Store::new(dir.path()).pairings().peers;
    assert_eq!(stored.len(), 1);
    let laptop = pitwall_in(&w, "nLAPTOP", dir, false, true, Limits::default());
    wait_until("connected after a restart", || laptop.peers.link_is("studio", LinkState::Connected));
}
