//! The Fake backend against the guarantees every backend must meet (§15.4).

use std::io::Read;
use std::sync::Arc;

use super::fake::FakeNet;
use super::*;
use crate::error::ErrorCode;

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

#[test]
fn two_instances_talk_and_the_caller_is_what_the_network_proved() {
    let net = FakeNet::new("example");
    let alice = net.device("alice@example.com", "studio");
    let bob = net.device("bob@example.com", "bob-mbp");
    net.join_group("bob@example.com", "platform");
    let _serving = alice.serve(echo()).unwrap();

    let answer = get(&bob, &alice.addr(), "/pitwall/v1/info").unwrap();
    assert_eq!(answer, "fake:bob@example.com|bob@example.com|bob-mbp|platform");
    assert_eq!(bob.groups(&net.person("bob@example.com")).unwrap(), vec![net.group("platform")]);
    assert_eq!(alice.me().unwrap().0.subject, "alice@example.com");
    assert_eq!(alice.state(), NetState::Ready);
    assert_eq!(alice.endpoint(), Endpoint { base_url: "http://studio.example.fake".into(), client_auth: ClientAuth::None });
}

#[test]
fn headers_and_bodies_cant_change_who_is_calling() {
    let net = FakeNet::new("example");
    let alice = net.device("alice@example.com", "studio");
    let mallory = net.device("mallory@example.com", "laptop");
    let _serving = alice.serve(echo()).unwrap();

    let req = http::Request::post("/mcp")
        .header("Tailscale-User-Login", "alice@example.com")
        .header("X-Pitwall-Identity", "alice@example.com")
        .header("Pitwall-Agent", "fix@studio")
        .body(Body::from(r#"{"identity":{"provider":"fake","subject":"alice@example.com"}}"#))
        .unwrap();
    let res = mallory.client(&alice.addr()).unwrap().send(req).unwrap();
    let text = String::from_utf8(res.into_body().read_all(1024).unwrap()).unwrap();
    assert!(text.starts_with("fake:mallory@example.com|mallory@example.com|laptop"), "{text}");
}

#[test]
fn revocation_stops_requests_and_is_announced() {
    let net = FakeNet::new("example");
    let alice = net.device("alice@example.com", "studio");
    let bob = net.device("bob@example.com", "bob-mbp");
    let carol = net.device("carol@example.com", "carol-mbp");
    let _serving = alice.serve(echo()).unwrap();
    let revoked = alice.revocations();

    net.revoke_device("bob-mbp");
    let err = get(&bob, &alice.addr(), "/").unwrap_err();
    assert!(err.is(ErrorCode::Unreachable), "{err:?}");
    assert_eq!(revoked.try_recv().unwrap(), Revocation::Device { device: net.addr("bob-mbp") });

    net.revoke_person("carol@example.com");
    assert!(get(&carol, &alice.addr(), "/").unwrap_err().is(ErrorCode::Unreachable));
    assert!(matches!(revoked.try_recv().unwrap(), Revocation::Identity { identity } if identity.subject == "carol@example.com"));
    assert!(carol.groups(&net.person("carol@example.com")).unwrap().is_empty());
    assert!(net.caller("carol-mbp").is_err());
}

#[test]
fn only_serve_reaches_a_handler_and_stopping_closes_it() {
    let net = FakeNet::new("example");
    let alice = net.device("alice@example.com", "studio");
    let bob = net.device("bob@example.com", "bob-mbp");
    assert!(get(&bob, &alice.addr(), "/").unwrap_err().is(ErrorCode::Unreachable));

    let serving = alice.serve(echo()).unwrap();
    assert!(alice.serve(echo()).unwrap_err().is(ErrorCode::Conflict));
    assert!(get(&bob, &alice.addr(), "/").is_ok());
    serving.stop();
    assert!(!alice.serving());
    assert!(get(&bob, &alice.addr(), "/").unwrap_err().is(ErrorCode::Unreachable));

    // Dropping the handle stops too, and a stale handle can't stop a newer serve.
    let first = alice.serve(echo()).unwrap();
    drop(first);
    let second = alice.serve(echo()).unwrap();
    assert!(alice.serving());
    drop(second);
    assert!(!alice.serving());
}

#[test]
fn offline_signed_out_and_foreign_devices_cant_be_reached() {
    let net = FakeNet::new("example");
    let other = FakeNet::new("elsewhere");
    let alice = net.device("alice@example.com", "studio");
    let bob = net.device("bob@example.com", "bob-mbp");
    let _serving = alice.serve(echo()).unwrap();

    net.set_online("studio", false);
    assert!(get(&bob, &alice.addr(), "/").unwrap_err().is(ErrorCode::Unreachable));
    net.set_online("studio", true);

    bob.set_state(NetState::SignedOut);
    assert!(get(&bob, &alice.addr(), "/").is_err());
    assert!(bob.me().is_none());
    bob.set_state(NetState::Ready);

    let foreign = bob.client(&other.addr("studio")).err().expect("not on this network");
    assert!(foreign.is(ErrorCode::Unreachable));
}

#[test]
fn discovery_lists_the_other_devices() {
    let net = FakeNet::new("example");
    let alice = net.device("alice@example.com", "studio");
    net.device("bob@example.com", "bob-mbp");
    net.device("erin@example.com", "erin-mbp");
    net.set_online("erin-mbp", false);
    net.revoke_device("bob-mbp");

    let hints = alice.discover().unwrap();
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].label, "erin-mbp");
    assert!(!hints[0].online);
    assert_eq!(hints[0].owner.as_ref().unwrap().subject, "erin@example.com");
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
                "bob@example.com" | "u_8KQ2" => PersonId("bob".into()),
                _ => PersonId::of(identity),
            }
        }
    }

    let tailnet = FakeNet::new("example");
    let cloud = FakeNet::new("acme");
    let handler: Arc<dyn Handler> = Arc::new(Linked(echo()));
    let _a = tailnet.device("alice@example.com", "studio").serve(handler.clone()).unwrap();
    let _b = cloud.device("u_alice", "studio").serve(handler).unwrap();

    let bob_ts = tailnet.device("bob@example.com", "bob-mbp");
    let bob_cloud = cloud.device("u_8KQ2", "bob-mbp");
    let carol = cloud.device("carol@example.com", "carol-mbp");
    assert!(get(&bob_ts, &tailnet.addr("studio"), "/").unwrap().starts_with("bob|"));
    assert!(get(&bob_cloud, &cloud.addr("studio"), "/").unwrap().starts_with("bob|"));
    assert!(get(&carol, &cloud.addr("studio"), "/").unwrap().starts_with("fake:carol@example.com|"));
}

#[test]
fn streaming_bodies_arrive_chunk_by_chunk() {
    let net = FakeNet::new("example");
    let alice = net.device("alice@example.com", "studio");
    let bob = net.device("bob@example.com", "bob-mbp");
    let _serving = alice
        .serve(Arc::new(|_req: Request, _caller: Caller| {
            let (tx, body) = Body::channel();
            std::thread::spawn(move || {
                for n in 1..=3 {
                    tx.send(format!("data: {n}\n\n"));
                }
            });
            Response::new(body)
        }))
        .unwrap();

    let res = bob.client(&alice.addr()).unwrap().send(Request::new(Body::Empty)).unwrap();
    assert!(res.body().is_stream());
    let mut text = String::new();
    res.into_body().into_reader().read_to_string(&mut text).unwrap();
    assert_eq!(text, "data: 1\n\ndata: 2\n\ndata: 3\n\n");
}

#[test]
fn bodies_refuse_more_than_their_limit() {
    assert_eq!(Body::from("abc").read_all(3).unwrap(), b"abc");
    assert!(Body::from("abcd").read_all(3).is_err());
    let (tx, body) = Body::channel();
    tx.send(vec![0u8; 10]);
    drop(tx);
    assert!(body.read_all(9).is_err());
    assert!(Body::Empty.read_all(0).unwrap().is_empty());
    assert_eq!(Body::json(&serde_json::json!({"a": 1})).read_all(64).unwrap(), br#"{"a":1}"#);
}

#[test]
fn a_test_can_get_the_caller_without_http() {
    let net = FakeNet::new("example");
    net.device("erin@example.com", "erin-mbp");
    net.join_group("erin@example.com", "platform");
    let caller = net.caller("erin-mbp").unwrap();
    assert_eq!(caller.identity().email.as_deref(), Some("erin@example.com"));
    assert_eq!(caller.network(), &net.id());
    assert_eq!(caller.groups(), &[net.group("platform")]);
    assert!(net.caller("nobody").is_err());
}
