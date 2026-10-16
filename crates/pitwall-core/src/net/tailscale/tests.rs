//! The Tailscale backend against the guarantees every backend must meet
//! (network.md §15.4; the same checklist as `net/tests.rs` for Fake), on a
//! scripted tailnet: a fake [`LocalApi`] per device and real sockets on
//! 127.0.0.1 ([`TestHooks`]).

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::error::ErrorCode;
use crate::net::{http, Body, Caller, Handler, PersonId, Request, Response, Timeouts};
use crate::testing::FakeExec;

// ------------------------------------------------------- a scripted tailnet

/// One tailnet: users, nodes, grants, and which local port is which node.
#[derive(Clone, Default)]
struct FakeTailnet {
    world: Arc<Mutex<World>>,
}

#[derive(Default)]
struct World {
    suffix: String,
    users: BTreeMap<i64, UserProfile>,
    nodes: BTreeMap<String, Node>,
    /// Pitwall app capability parameters per login.
    caps: HashMap<String, Vec<serde_json::Value>>,
    ports: HashMap<String, u16>,
    /// Local address of a test connection → the node that opened it.
    conns: HashMap<SocketAddr, String>,
    whois_calls: usize,
}

#[derive(Clone)]
struct Node {
    user: i64,
    host: String,
    tags: Vec<String>,
    online: bool,
    state: String,
    /// The last byte of its tailnet IP (connections go to 127.0.0.1).
    index: u8,
}

impl FakeTailnet {
    fn new(suffix: &str) -> FakeTailnet {
        let net = FakeTailnet::default();
        net.world().suffix = suffix.to_string();
        net
    }

    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }

    fn user_id(&self, login: &str) -> i64 {
        let mut w = self.world();
        if let Some(u) = w.users.values().find(|u| u.login_name == login) {
            return u.id;
        }
        let id = 1000 + w.users.len() as i64;
        let display = login.split('@').next().unwrap_or(login).to_string();
        w.users.insert(id, UserProfile { id, login_name: login.to_string(), display_name: display });
        id
    }

    /// Node `host` (StableID `n-<host>`) signed in as `login`, online.
    fn node(&self, login: &str, host: &str) -> TailscaleNetwork {
        self.node_with(login, host, Vec::new(), TailscaleConfig { ..self.config() })
    }

    fn config(&self) -> TailscaleConfig {
        TailscaleConfig {
            status_ttl: Duration::ZERO,
            whois_ttl: Duration::ZERO,
            poll: Duration::from_secs(3600),
            test: Some(Arc::new(self.clone())),
            ..TailscaleConfig::default()
        }
    }

    fn node_with(&self, login: &str, host: &str, tags: Vec<String>, config: TailscaleConfig) -> TailscaleNetwork {
        let user = self.user_id(login);
        let index = 1 + self.world().nodes.len() as u8;
        let node = Node { user, host: host.to_string(), tags, online: true, state: "Running".into(), index };
        self.world().nodes.insert(format!("n-{host}"), node);
        TailscaleNetwork::new(Arc::new(FakeApi { net: self.clone(), me: format!("n-{host}") }), config)
    }

    fn addr(&self, host: &str) -> DeviceAddr {
        DeviceAddr { network: NetworkId::new(PROVIDER, &self.world().suffix), node: format!("n-{host}") }
    }

    fn grant(&self, login: &str, param: serde_json::Value) {
        self.world().caps.entry(login.to_string()).or_default().push(param);
    }

    fn set_online(&self, host: &str, online: bool) {
        self.world().nodes.get_mut(&format!("n-{host}")).unwrap().online = online;
    }

    fn set_state(&self, host: &str, state: &str) {
        self.world().nodes.get_mut(&format!("n-{host}")).unwrap().state = state.to_string();
    }

    /// The admin removes a device.
    fn remove_node(&self, host: &str) {
        self.world().nodes.remove(&format!("n-{host}"));
    }

    /// The admin removes a person, and with them their devices.
    fn remove_user(&self, login: &str) {
        let mut w = self.world();
        let Some(id) = w.users.values().find(|u| u.login_name == login).map(|u| u.id) else { return };
        w.users.remove(&id);
        w.nodes.retain(|_, n| n.user != id);
    }

    fn peer_status(&self, w: &World, id: &str, n: &Node) -> PeerStatus {
        let ip = IpAddr::from([100, 64, 0, n.index]);
        PeerStatus {
            id: id.to_string(),
            host_name: n.host.clone(),
            dns_name: format!("{}.{}.", n.host, w.suffix),
            os: "macOS".into(),
            user_id: n.user,
            tailscale_ips: vec![ip],
            tags: n.tags.clone(),
            online: n.online,
        }
    }
}

impl TestHooks for FakeTailnet {
    fn bound(&self, node: &str, addr: SocketAddr) {
        self.world().ports.insert(node.to_string(), addr.port());
    }

    fn port_of(&self, node: &str) -> Option<u16> {
        self.world().ports.get(node).copied()
    }

    fn connected(&self, from: &str, local: SocketAddr) {
        self.world().conns.insert(local, from.to_string());
    }
}

/// One device's local Tailscale.
struct FakeApi {
    net: FakeTailnet,
    me: String,
}

impl LocalApi for FakeApi {
    fn status(&self) -> Result<Status> {
        let w = self.net.world();
        let Some(me) = w.nodes.get(&self.me) else {
            return Ok(Status { backend_state: "NeedsLogin".into(), ..Status::default() });
        };
        if me.state != "Running" {
            return Ok(Status { backend_state: me.state.clone(), ..Status::default() });
        }
        let peer = w
            .nodes
            .iter()
            .filter(|(id, _)| **id != self.me)
            .map(|(id, n)| (format!("nodekey:{id}"), self.net.peer_status(&w, id, n)))
            .collect();
        Ok(Status {
            backend_state: "Running".into(),
            self_: Some(self.net.peer_status(&w, &self.me, me)),
            peer,
            user: w.users.iter().map(|(id, u)| (id.to_string(), u.clone())).collect(),
            magic_dns_suffix: w.suffix.clone(),
            current_tailnet: Some(Tailnet { name: "example.com".into(), magic_dns_suffix: w.suffix.clone() }),
        })
    }

    fn whois(&self, addr: SocketAddr) -> Result<Option<WhoIs>> {
        // The client registers its connection right after `connect`
        // returns, which may be just after the listener accepted it.
        if addr.ip().is_loopback() {
            for _ in 0..50 {
                if self.net.world().conns.contains_key(&addr) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let mut w = self.net.world();
        w.whois_calls += 1;
        // Test connections by their local port; anything else by tailnet IP.
        let id = if addr.ip().is_loopback() {
            w.conns.get(&addr).cloned()
        } else {
            w.nodes.iter().find(|(_, n)| IpAddr::from([100, 64, 0, n.index]) == addr.ip()).map(|(id, _)| id.clone())
        };
        let Some(id) = id else { return Ok(None) };
        let Some(node) = w.nodes.get(&id).cloned() else { return Ok(None) };
        let Some(user) = w.users.get(&node.user).cloned() else { return Ok(None) };
        let caps = w.caps.get(&user.login_name).cloned().unwrap_or_default();
        Ok(Some(WhoIs {
            node: WhoIsNode { stable_id: id, name: format!("{}.{}.", node.host, w.suffix), tags: node.tags },
            user_profile: user,
            cap_map: if caps.is_empty() { BTreeMap::new() } else { BTreeMap::from([(CAP_NAME.to_string(), caps)]) },
        }))
    }
}

// ------------------------------------------------------------------ helpers

/// Answers with what the network proved: `person|subject|device|groups`.
fn echo() -> Arc<dyn Handler> {
    Arc::new(|_req: Request, caller: Caller| {
        let groups: Vec<_> = caller.groups().iter().map(|g| g.id.as_str()).collect();
        let text = format!(
            "{}|{}|{}|{}",
            caller.person().as_str(),
            caller.identity().subject,
            caller.device().map_or("-", |d| d.node.as_str()),
            groups.join(",")
        );
        Response::new(Body::from(text))
    })
}

fn get(net: &dyn Network, to: &DeviceAddr, path: &str) -> Result<String> {
    let req = http::Request::get(path).body(Body::Empty).unwrap();
    let res = net.client(to)?.send(req)?;
    Ok(String::from_utf8(res.into_body().read_all(1 << 20)?).unwrap())
}

fn status_of(net: &dyn Network, to: &DeviceAddr, req: Request) -> u16 {
    net.client(to).unwrap().send(req).unwrap().status().as_u16()
}

/// A raw connection to `node`'s listener, from no tailnet node at all.
fn raw(tailnet: &FakeTailnet, host: &str) -> TcpStream {
    let port = tailnet.port_of(&format!("n-{host}")).unwrap();
    let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s
}

/// A raw connection that the tailnet knows is from `from`.
fn raw_from(tailnet: &FakeTailnet, from: &str, to: &str) -> TcpStream {
    let s = raw(tailnet, to);
    tailnet.connected(&format!("n-{from}"), s.local_addr().unwrap());
    s
}

fn read_to_end(mut s: TcpStream) -> Vec<u8> {
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    out
}

// --------------------------------------------------- §15.4, as for Fake

#[test]
fn two_instances_talk_and_the_caller_is_what_the_network_proved() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    let bob = net.node("bob@example.com", "bob-mbp");
    net.grant("bob@example.com", serde_json::json!({"group": "platform"}));
    let _serving = alice.serve(echo()).unwrap();

    let answer = get(&bob, &net.addr("studio"), "/pitwall/v1/info").unwrap();
    assert_eq!(answer, "tailscale:bob@example.com|bob@example.com|n-bob-mbp|platform");
    let group = GroupRef { network: alice.id(), id: "platform".into() };
    assert_eq!(alice.groups(&Identity::new(PROVIDER, "bob@example.com")).unwrap(), vec![group]);
    assert_eq!(alice.id().as_str(), "tailscale:example.ts.net");
    let (me, addr) = alice.me().unwrap();
    assert_eq!((me.subject.as_str(), me.email.as_deref(), me.name.as_deref()), ("alice@example.com", Some("alice@example.com"), Some("alice")));
    assert_eq!(addr, net.addr("studio"));
    assert_eq!(alice.state(), NetState::Ready);
    let port = alice.listening()[0].port();
    assert_eq!(
        alice.endpoint(),
        Endpoint { base_url: format!("http://studio.example.ts.net:{port}"), client_auth: ClientAuth::None }
    );
    assert!(alice.listening().iter().all(|a| a.ip().is_loopback()));
}

#[test]
fn headers_and_bodies_cant_change_who_is_calling() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    let mallory = net.node("mallory@example.com", "laptop");
    let _serving = alice.serve(echo()).unwrap();

    let req = http::Request::post("/mcp")
        .header("Tailscale-User-Login", "alice@example.com")
        .header("X-Pitwall-Identity", "alice@example.com")
        .header("Pitwall-Agent", "fix@studio")
        .header("Host", "studio")
        .body(Body::from(r#"{"identity":{"provider":"tailscale","subject":"alice@example.com"}}"#))
        .unwrap();
    let res = mallory.client(&net.addr("studio")).unwrap().send(req).unwrap();
    let text = String::from_utf8(res.into_body().read_all(1024).unwrap()).unwrap();
    assert!(text.starts_with("tailscale:mallory@example.com|mallory@example.com|n-laptop"), "{text}");
}

#[test]
fn revocation_stops_requests_and_is_announced() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    let bob = net.node("bob@example.com", "bob-mbp");
    let carol = net.node("carol@example.com", "carol-mbp");
    let _serving = alice.serve(echo()).unwrap();
    let revoked = alice.revocations();
    assert!(get(&bob, &net.addr("studio"), "/").is_ok());

    net.remove_node("bob-mbp");
    let err = get(&bob, &net.addr("studio"), "/").unwrap_err();
    assert!(err.is(ErrorCode::Unreachable), "{err:?}");
    alice.refresh();
    assert_eq!(revoked.try_recv().unwrap(), Revocation::Device { device: net.addr("bob-mbp") });
    // Bob had no other device: his login is gone too.
    assert!(matches!(revoked.try_recv().unwrap(), Revocation::Identity { identity } if identity.subject == "bob@example.com"));

    assert!(get(&carol, &net.addr("studio"), "/").is_ok());
    net.remove_user("carol@example.com");
    assert!(get(&carol, &net.addr("studio"), "/").unwrap_err().is(ErrorCode::Unreachable));
    alice.refresh();
    assert_eq!(revoked.try_recv().unwrap(), Revocation::Device { device: net.addr("carol-mbp") });
    assert!(matches!(revoked.try_recv().unwrap(), Revocation::Identity { identity } if identity.subject == "carol@example.com"));
    assert!(alice.groups(&Identity::new(PROVIDER, "carol@example.com")).unwrap().is_empty());
    assert!(revoked.try_recv().is_err());
}

/// A connection already open when its node is removed is refused on its
/// next request, even by a raw client that keeps the socket.
#[test]
fn a_removed_node_is_refused_on_an_open_connection() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    net.node("bob@example.com", "bob-mbp");
    let _serving = alice.serve(echo()).unwrap();

    let mut s = raw_from(&net, "bob-mbp", "studio");
    s.write_all(b"GET / HTTP/1.1\r\nHost: studio\r\n\r\n").unwrap();
    let mut buf = [0u8; 512];
    let n = s.read(&mut buf).unwrap();
    assert!(String::from_utf8_lossy(&buf[..n]).ends_with("tailscale:bob@example.com|bob@example.com|n-bob-mbp|"));

    net.remove_node("bob-mbp");
    s.write_all(b"GET / HTTP/1.1\r\nHost: studio\r\n\r\n").unwrap();
    assert!(read_to_end(s).is_empty(), "closed without a byte");
}

/// When the poller sees a node go, its open streams are cut at once.
#[test]
fn revocation_closes_open_streams() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    let bob = net.node("bob@example.com", "bob-mbp");
    let writers = Arc::new(Mutex::new(Vec::new()));
    let keep = writers.clone();
    let _serving = alice
        .serve(Arc::new(move |_req: Request, _caller: Caller| {
            let (tx, body) = Body::channel();
            tx.send("data: hello\n\n");
            keep.lock().unwrap().push(tx);
            Response::new(body)
        }))
        .unwrap();
    let _revoked = alice.revocations();

    let mut req = http::Request::get("/events").body(Body::Empty).unwrap();
    req.extensions_mut().insert(Timeouts { connect: Duration::from_secs(2), idle: None });
    let res = bob.client(&net.addr("studio")).unwrap().send(req).unwrap();
    let mut reader = res.into_body().into_reader();
    let mut first = [0u8; 13];
    reader.read_exact(&mut first).unwrap();
    assert_eq!(&first, b"data: hello\n\n");

    net.remove_node("bob-mbp");
    alice.refresh();
    let mut rest = Vec::new();
    let ended = reader.read_to_end(&mut rest);
    assert!(ended.is_err() || rest.is_empty(), "the stream ends");
}

#[test]
fn only_serve_reaches_a_handler_and_stopping_closes_it() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    let bob = net.node("bob@example.com", "bob-mbp");
    assert!(get(&bob, &net.addr("studio"), "/").unwrap_err().is(ErrorCode::Unreachable));

    let serving = alice.serve(echo()).unwrap();
    assert!(alice.serve(echo()).unwrap_err().is(ErrorCode::Conflict));
    assert!(get(&bob, &net.addr("studio"), "/").is_ok());
    let port = alice.listening()[0].port();
    serving.stop();
    assert!(alice.listening().is_empty());
    assert!(get(&bob, &net.addr("studio"), "/").unwrap_err().is(ErrorCode::Unreachable));
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err(), "the port is closed");

    // Dropping the handle stops too, and a stale handle can't stop a newer serve.
    let first = alice.serve(echo()).unwrap();
    first.stop();
    let second = alice.serve(echo()).unwrap();
    drop(first);
    assert!(!alice.listening().is_empty());
    assert!(get(&bob, &net.addr("studio"), "/").is_ok());
    drop(second);
    assert!(alice.listening().is_empty());
}

#[test]
fn offline_signed_out_and_foreign_devices_cant_be_reached() {
    let net = FakeTailnet::new("example.ts.net");
    let other = FakeTailnet::new("elsewhere.ts.net");
    let alice = net.node("alice@example.com", "studio");
    let bob = net.node("bob@example.com", "bob-mbp");
    let _serving = alice.serve(echo()).unwrap();

    net.set_online("studio", false);
    assert!(get(&bob, &net.addr("studio"), "/").unwrap_err().contains("offline"));
    net.set_online("studio", true);

    net.set_state("bob-mbp", "NeedsLogin");
    assert!(get(&bob, &net.addr("studio"), "/").is_err());
    assert!(bob.me().is_none());
    assert_eq!(bob.state(), NetState::SignedOut);
    net.set_state("bob-mbp", "Running");

    let foreign = bob.client(&other.addr("studio")).err().expect("not on this network");
    assert!(foreign.is(ErrorCode::Unreachable));
    assert!(get(&bob, &net.addr("nobody"), "/").unwrap_err().is(ErrorCode::Unreachable));
}

#[test]
fn discovery_lists_the_other_devices() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    net.node("bob@example.com", "bob-mbp");
    net.node("erin@example.com", "erin-mbp");
    net.set_online("erin-mbp", false);

    // No tags in the tailnet: every online device.
    let hints = alice.discover().unwrap();
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].label, "bob-mbp");
    assert!(hints[0].online);
    assert_eq!(hints[0].owner.as_ref().unwrap().subject, "bob@example.com");
    assert_eq!(hints[0].addr, net.addr("bob-mbp"));

    // Once the tailnet uses tags: only tag:pitwall devices, which have no owner.
    let config = net.config();
    net.node_with("ci@example.com", "ci-runner", vec!["tag:ci".into()], config.clone());
    net.node_with("ops@example.com", "build-box", vec!["tag:pitwall".into()], config.clone());
    let hints = alice.discover().unwrap();
    assert_eq!(hints.iter().map(|h| h.label.as_str()).collect::<Vec<_>>(), ["build-box"]);
    assert!(hints[0].owner.is_none());

    let named = net.node_with(
        "alice@example.com",
        "laptop",
        Vec::new(),
        TailscaleConfig { discovery: Discovery::Named(vec!["erin-mbp.example.ts.net.".into(), "ci-runner".into()]), ..config },
    );
    let hints = named.discover().unwrap();
    assert_eq!(hints.iter().map(|h| (h.label.as_str(), h.online)).collect::<Vec<_>>(), [("ci-runner", true), ("erin-mbp", false)]);
}

/// One handler on two networks; the owner links Bob's two identities
/// (§15.7), so both arrive as one person. Unlinked, they stay apart.
#[test]
fn several_networks_one_handler_and_linked_identities() {
    struct Linked(Arc<dyn Handler>);
    impl Handler for Linked {
        fn handle(&self, req: Request, caller: Caller) -> Response {
            self.0.handle(req, caller)
        }
        fn person_of(&self, identity: &Identity) -> PersonId {
            match identity.subject.as_str() {
                "bob@example.com" | "bob@acme.com" => PersonId("bob".into()),
                _ => PersonId::of(identity),
            }
        }
    }

    let home = FakeTailnet::new("example.ts.net");
    let work = FakeTailnet::new("acme.ts.net");
    let handler: Arc<dyn Handler> = Arc::new(Linked(echo()));
    let _a = home.node("alice@example.com", "studio").serve(handler.clone()).unwrap();
    let _b = work.node("alice@acme.com", "studio").serve(handler).unwrap();

    let bob_home = home.node("bob@example.com", "bob-mbp");
    let bob_work = work.node("bob@acme.com", "bob-mbp");
    let carol = work.node("carol@acme.com", "carol-mbp");
    assert!(get(&bob_home, &home.addr("studio"), "/").unwrap().starts_with("bob|"));
    assert!(get(&bob_work, &work.addr("studio"), "/").unwrap().starts_with("bob|"));
    assert!(get(&carol, &work.addr("studio"), "/").unwrap().starts_with("tailscale:carol@acme.com|"));
}

#[test]
fn streaming_bodies_arrive_chunk_by_chunk() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    let bob = net.node("bob@example.com", "bob-mbp");
    let _serving = alice
        .serve(Arc::new(|_req: Request, _caller: Caller| {
            let (tx, body) = Body::channel();
            std::thread::spawn(move || {
                for n in 1..=3 {
                    tx.send(format!("data: {n}\n\n"));
                    std::thread::sleep(Duration::from_millis(20));
                }
            });
            let mut res = Response::new(body);
            res.headers_mut().insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("text/event-stream"));
            res
        }))
        .unwrap();

    let res = bob.client(&net.addr("studio")).unwrap().send(Request::new(Body::Empty)).unwrap();
    assert!(res.body().is_stream());
    assert_eq!(res.headers()[http::header::CONTENT_TYPE], "text/event-stream");
    let mut reader = res.into_body().into_reader();
    let mut first = [0u8; 9];
    reader.read_exact(&mut first).unwrap();
    assert_eq!(&first, b"data: 1\n\n");
    let mut text = String::new();
    reader.read_to_string(&mut text).unwrap();
    assert_eq!(text, "data: 2\n\ndata: 3\n\n");
}

// --------------------------------------------------------- the listener

#[test]
fn unknown_sources_are_closed_without_a_byte_and_never_reach_the_handler() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    let _serving = alice
        .serve(Arc::new(move |_req: Request, _caller: Caller| {
            counted.fetch_add(1, Ordering::SeqCst);
            Response::new(Body::Empty)
        }))
        .unwrap();

    let mut s = raw(&net, "studio");
    let _ = s.write_all(b"GET /pitwall/v1/info HTTP/1.1\r\nHost: studio\r\nTailscale-User-Login: alice@example.com\r\n\r\n");
    assert!(read_to_end(s).is_empty());
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert!(net.world().whois_calls >= 1, "whois was asked before anything else");
}

#[test]
fn the_body_limit_and_bad_requests_are_answered_and_closed() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    let bob = net.node("bob@example.com", "bob-mbp");
    let _serving = alice.serve(echo()).unwrap();

    let big = http::Request::post("/mcp").body(Body::from(vec![b'x'; 2 * 1024 * 1024 + 1])).unwrap();
    assert_eq!(status_of(&bob, &net.addr("studio"), big), 413);
    let ok = http::Request::post("/mcp").body(Body::from(vec![b'x'; 2 * 1024 * 1024])).unwrap();
    assert_eq!(status_of(&bob, &net.addr("studio"), ok), 200);

    let mut s = raw_from(&net, "bob-mbp", "studio");
    s.write_all(b"NOT HTTP AT ALL\r\n\r\n").unwrap();
    assert!(String::from_utf8(read_to_end(s)).unwrap().starts_with("HTTP/1.1 400"));
}

#[test]
fn per_caller_limits_hold() {
    let net = FakeTailnet::new("example.ts.net");
    let limits = Limits { requests_per_sec: 1, burst: 3, streams: 2, ..Limits::default() };
    let alice = net.node_with("alice@example.com", "studio", Vec::new(), TailscaleConfig { limits, ..net.config() });
    let bob = net.node("bob@example.com", "bob-mbp");
    let carol = net.node("carol@example.com", "carol-mbp");
    let writers = Arc::new(Mutex::new(Vec::new()));
    let keep = writers.clone();
    let _serving = alice
        .serve(Arc::new(move |req: Request, _caller: Caller| {
            if req.uri().path() == "/stream" {
                let (tx, body) = Body::channel();
                keep.lock().unwrap().push(tx);
                return Response::new(body);
            }
            Response::new(Body::from("ok"))
        }))
        .unwrap();
    let to = net.addr("studio");
    let stream = || {
        let mut req = http::Request::get("/stream").body(Body::Empty).unwrap();
        req.extensions_mut().insert(Timeouts { connect: Duration::from_secs(2), idle: None });
        req
    };

    // Streams: two open, the third is refused.
    let client = bob.client(&to).unwrap();
    let open: Vec<_> = (0..2).map(|_| client.send(stream()).unwrap()).collect();
    assert!(open.iter().all(|r| r.status() == 200));
    assert_eq!(client.send(stream()).unwrap().status(), 429);

    // Requests: the burst of 3 is spent (2 streams + 1 refused).
    assert_eq!(status_of(&bob, &to, Request::new(Body::Empty)), 429);
    // Others aren't affected.
    assert_eq!(status_of(&carol, &to, Request::new(Body::Empty)), 200);

    drop(open);
}

#[test]
fn per_caller_connections_are_limited() {
    let net = FakeTailnet::new("example.ts.net");
    let limits = Limits { connections: 2, ..Limits::default() };
    let alice = net.node_with("alice@example.com", "studio", Vec::new(), TailscaleConfig { limits, ..net.config() });
    net.node("bob@example.com", "bob-mbp");
    net.node("carol@example.com", "carol-mbp");
    let _serving = alice.serve(Arc::new(|_req: Request, _caller: Caller| Response::new(Body::from("ok")))).unwrap();

    // A kept-alive connection that has been answered once.
    let open = |from: &str| -> Option<TcpStream> {
        let mut s = raw_from(&net, from, "studio");
        s.write_all(b"GET / HTTP/1.1\r\n\r\n").ok()?;
        let mut got = Vec::new();
        let mut buf = [0u8; 256];
        while !got.ends_with(b"ok") {
            match s.read(&mut buf) {
                Ok(0) | Err(_) => return None,
                Ok(n) => got.extend_from_slice(&buf[..n]),
            }
        }
        Some(s)
    };
    let first = open("bob-mbp").expect("first");
    let _second = open("bob-mbp").expect("second");
    assert!(open("bob-mbp").is_none(), "over the limit: closed without an answer");
    assert!(open("carol-mbp").is_some(), "others aren't affected");

    drop(first);
    let again = (0..100).find_map(|_| {
        std::thread::sleep(Duration::from_millis(20));
        open("bob-mbp")
    });
    assert!(again.is_some(), "a closed connection frees its place");
}

#[test]
fn groups_come_from_the_pitwall_capability() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    net.node("bob@example.com", "bob-mbp");
    net.grant("bob@example.com", serde_json::json!({"group": "platform", "admin": false}));
    net.grant("bob@example.com", serde_json::json!({"group": "platform", "admin": true, "logins": ["alice@example.com"]}));
    net.grant("bob@example.com", serde_json::json!({"group": "infra", "admin": true, "logins": ["bob@example.com"], "maxCapability": "read"}));
    net.grant("bob@example.com", serde_json::json!({"admin": true}));
    net.grant("bob@example.com", serde_json::json!("platform"));
    let ids: Vec<String> =
        alice.groups(&Identity::new(PROVIDER, "bob@example.com")).unwrap().into_iter().map(|g| g.id).collect();
    assert_eq!(ids, ["infra", "infra:admin", "platform"]);
    assert!(alice.groups(&Identity::new("fake", "bob@example.com")).unwrap().is_empty());
}

#[test]
fn tagged_devices_are_each_their_own_identity() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    let config = net.config();
    let ci = net.node_with("tagged-devices", "ci-1", vec!["tag:ci".into()], config.clone());
    let ci2 = net.node_with("tagged-devices", "ci-2", vec!["tag:ci".into()], config);
    let _serving = alice.serve(echo()).unwrap();
    assert_eq!(get(&ci, &net.addr("studio"), "/").unwrap(), "tailscale:node:n-ci-1|node:n-ci-1|n-ci-1|");
    assert_eq!(get(&ci2, &net.addr("studio"), "/").unwrap(), "tailscale:node:n-ci-2|node:n-ci-2|n-ci-2|");
    assert_eq!(ci.me().unwrap().0.name.as_deref(), Some("ci-1"));
}

#[test]
fn requests_reach_the_handler_whole() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    let bob = net.node("bob@example.com", "bob-mbp");
    let _serving = alice
        .serve(Arc::new(|req: Request, _caller: Caller| {
            let (parts, body) = req.into_parts();
            let body = String::from_utf8(body.read_all(1024).unwrap()).unwrap();
            let agent = parts.headers.get("pitwall-agent").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
            let mut res = Response::new(Body::from(format!("{} {} {agent} {body}", parts.method, parts.uri)));
            *res.status_mut() = http::StatusCode::CREATED;
            res
        }))
        .unwrap();

    let req = http::Request::post("/a2a/api-expert?x=1").header("Pitwall-Agent", "fix@bob-mbp").body(Body::from("hi")).unwrap();
    let res = bob.client(&net.addr("studio")).unwrap().send(req).unwrap();
    assert_eq!(res.status(), 201);
    assert_eq!(res.into_body().read_all(1024).unwrap(), b"POST /a2a/api-expert?x=1 fix@bob-mbp hi");

    // A streamed request body arrives whole too.
    let (tx, body) = Body::channel();
    tx.send("a");
    tx.send("b");
    drop(tx);
    let req = http::Request::put("/x").body(body).unwrap();
    let res = bob.client(&net.addr("studio")).unwrap().send(req).unwrap();
    assert_eq!(res.into_body().read_all(1024).unwrap(), b"PUT /x  ab");
}

#[test]
fn an_idle_timeout_gives_up_on_a_silent_device() {
    let net = FakeTailnet::new("example.ts.net");
    let alice = net.node("alice@example.com", "studio");
    let bob = net.node("bob@example.com", "bob-mbp");
    let writers = Arc::new(Mutex::new(Vec::new()));
    let keep = writers.clone();
    let _serving = alice
        .serve(Arc::new(move |_req: Request, _caller: Caller| {
            let (tx, body) = Body::channel();
            keep.lock().unwrap().push(tx);
            Response::new(body)
        }))
        .unwrap();
    let mut req = http::Request::get("/silent").body(Body::Empty).unwrap();
    req.extensions_mut().insert(Timeouts { connect: Duration::from_secs(2), idle: Some(Duration::from_millis(200)) });
    let res = bob.client(&net.addr("studio")).unwrap().send(req).unwrap();
    let started = std::time::Instant::now();
    let mut out = Vec::new();
    assert!(res.into_body().into_reader().read_to_end(&mut out).is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
}

// ------------------------------------------------- status, whois, the CLI

#[test]
fn the_cli_is_read_as_tailscale_writes_it() {
    let exec = FakeExec::new();
    let status = r#"{
        "Version": "1.90.0", "BackendState": "Running", "TailscaleIPs": ["100.101.102.103", "fd7a:115c:a1e0::1"],
        "Self": {"ID": "nSelf1CNTRL", "HostName": "studio", "DNSName": "studio.example.ts.net.", "OS": "macOS",
                 "UserID": 1, "TailscaleIPs": ["100.101.102.103", "fd7a:115c:a1e0::1"], "Online": true, "CapMap": null},
        "Peer": {"nodekey:ab": {"ID": "nBob1CNTRL", "HostName": "Bob's MacBook", "DNSName": "bob-mbp.example.ts.net.",
                 "UserID": 2, "TailscaleIPs": ["100.64.0.2"], "Tags": null, "Online": false},
                 "nodekey:cd": {"ID": "nCi1CNTRL", "HostName": "ci", "DNSName": "ci.example.ts.net.",
                 "UserID": 3, "TailscaleIPs": ["100.64.0.3"], "Tags": ["tag:pitwall"], "Online": true}},
        "User": {"1": {"ID": 1, "LoginName": "alice@example.com", "DisplayName": "Alice"},
                 "2": {"ID": 2, "LoginName": "bob@example.com", "DisplayName": "Bob"},
                 "3": {"ID": 3, "LoginName": "tagged-devices", "DisplayName": "Tagged Devices"}},
        "MagicDNSSuffix": "example.ts.net",
        "CurrentTailnet": {"Name": "example.com", "MagicDNSSuffix": "example.ts.net", "MagicDNSEnabled": true}
    }"#;
    let whois = r#"{"Node": {"ID": 12, "StableID": "nBob1CNTRL", "Name": "bob-mbp.example.ts.net.", "User": 2,
        "Addresses": ["100.64.0.2/32"]},
        "UserProfile": {"ID": 2, "LoginName": "bob@example.com", "DisplayName": "Bob"},
        "CapMap": {"aleksandrejavakhishvili.github.io/cap/pitwall": [{"group": "platform"}], "other/cap": [{}]}}"#;
    exec.on(&["tailscale", "status", "--json"], status);
    exec.on(&["tailscale", "whois", "--json", "100.64.0.2:51000"], whois);
    exec.on_exit(&["tailscale", "whois", "--json", "100.64.0.9:51000"], 1, "", "no match for IP:port");
    let api = CliApi::with(exec.clone(), "tailscale");

    let status = api.status().unwrap();
    assert_eq!(status.net_state(), NetState::Ready);
    assert_eq!(status.tailnet_name().as_deref(), Some("example.ts.net"));
    assert_eq!(status.peer("nBob1CNTRL").unwrap().label(), "bob-mbp");
    assert_eq!(status.peer("nBob1CNTRL").unwrap().tailscale_ips, vec![IpAddr::from([100, 64, 0, 2])]);
    assert_eq!(status.self_.as_ref().unwrap().tailscale_ips.len(), 2);
    let addr: SocketAddr = "100.64.0.2:51000".parse().unwrap();
    let who = api.whois(addr).unwrap().unwrap();
    assert_eq!(who.user_profile.login_name, "bob@example.com");
    assert_eq!(who.node.stable_id, "nBob1CNTRL");
    let network = NetworkId::new(PROVIDER, "example.ts.net");
    assert_eq!(groups_of(&network, &who), vec![GroupRef { network: network.clone(), id: "platform".into() }]);
    assert!(api.whois("100.64.0.9:51000".parse().unwrap()).unwrap().is_none());

    // The backend on top of it.
    let net = TailscaleNetwork::new(Arc::new(api), TailscaleConfig::default());
    assert_eq!(net.id(), network);
    let (me, _) = net.me().unwrap();
    assert_eq!(me.subject, "alice@example.com");
    assert_eq!(net.endpoint().base_url, "http://studio.example.ts.net:47421");
    let hints = net.discover().unwrap();
    assert_eq!(hints.iter().map(|h| h.label.as_str()).collect::<Vec<_>>(), ["ci"]);
    // Bob is offline: not even tried.
    assert!(net.client(&DeviceAddr { network, node: "nBob1CNTRL".into() }).unwrap().send(Request::new(Body::Empty)).unwrap_err().contains("offline"));
}

#[test]
fn states_follow_the_backend() {
    let cases = [
        ("Running", true, NetState::Ready),
        ("Running", false, NetState::Connecting),
        ("Starting", false, NetState::Connecting),
        ("NeedsLogin", false, NetState::SignedOut),
        ("NeedsMachineAuth", false, NetState::SignedOut),
        ("Stopped", false, NetState::Off),
        ("NoState", false, NetState::Off),
    ];
    for (backend, has_self, want) in cases {
        let status = Status { backend_state: backend.into(), self_: has_self.then(PeerStatus::default), ..Status::default() };
        assert_eq!(status.net_state(), want, "{backend}");
    }

    let missing = TailscaleNetwork::new(Arc::new(CliApi::with(FakeExec::new(), "tailscale")), TailscaleConfig::default());
    let exec = FakeExec::new();
    exec.on_exit(&["tailscale", "status", ".."], 1, "", "failed to connect to local tailscaled");
    let stopped = TailscaleNetwork::new(Arc::new(CliApi::with(exec, "tailscale")), TailscaleConfig::default());
    assert_eq!(stopped.state(), NetState::Off);
    assert!(matches!(missing.state(), NetState::Off | NetState::Error { .. }));
    assert!(stopped.me().is_none());
    assert!(stopped.serve(echo()).is_err());
    assert_eq!(stopped.id().as_str(), "tailscale:unknown");
    assert_eq!(stopped.endpoint().base_url, "");
}

#[test]
fn tailnet_ranges() {
    for ip in ["100.64.0.1", "100.127.255.254", "fd7a:115c:a1e0::1", "fd7a:115c:a1e0:ab12::99", "::ffff:100.100.1.1"] {
        assert!(is_tailnet_ip(ip.parse().unwrap()), "{ip}");
    }
    for ip in ["100.63.255.255", "100.128.0.1", "10.0.0.1", "127.0.0.1", "0.0.0.0", "::1", "fd7a:115c:a1e1::1", "192.168.1.2"] {
        assert!(!is_tailnet_ip(ip.parse().unwrap()), "{ip}");
    }
}

/// Against the real Tailscale on this machine:
/// `cargo test -p pitwall-core real_tailnet -- --ignored --nocapture`.
#[test]
#[ignore = "needs a signed-in Tailscale"]
fn real_tailnet() {
    let net = TailscaleNetwork::local(TailscaleConfig::default());
    println!("state: {:?}", net.state());
    println!("id: {}", net.id());
    let (me, addr) = net.me().expect("signed in");
    println!("me: {me:?} at {addr:?}");
    println!("endpoint: {:?}", net.endpoint());
    for hint in net.discover().unwrap() {
        println!("candidate: {} {} online={} owner={:?}", hint.label, hint.addr.node, hint.online, hint.owner.map(|o| o.subject));
    }
    // whois on our own address names us.
    let status = net.inner.status().unwrap();
    let ip = status.self_.unwrap().tailscale_ips[0];
    let who = net.inner.api.whois(SocketAddr::new(ip, 0)).unwrap().expect("whois for this node");
    assert_eq!(who.node.stable_id, addr.node);
    println!("groups: {:?}", groups_of(&net.id(), &who));

    // The listener binds only to tailnet addresses, and a request from this
    // node to itself arrives with this node's identity.
    let net = TailscaleNetwork::local(TailscaleConfig { port: 0, ..TailscaleConfig::default() });
    let _serving = net.serve(echo()).unwrap();
    let addrs = net.listening();
    println!("listening on {addrs:?}");
    assert!(addrs.iter().all(|a| is_tailnet_ip(a.ip())));
    let mut s = TcpStream::connect(addrs[0]).unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: me\r\nConnection: close\r\n\r\n").unwrap();
    let answer = String::from_utf8(read_to_end(s)).unwrap();
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(answer.contains(&format!("|{}|{}|", me.subject, addr.node)), "{answer}");
}
