//! The network contract (network.md §15.4): what every [`Network`] backend
//! must guarantee, checked the same way for each. A backend's tests
//! implement [`ContractWorld`] (a scripted provider: people, devices,
//! groups, removals) and expand [`network_contract_tests!`], which makes
//! one test per check. Run against the Fake backend (`net/tests.rs`) and
//! the Tailscale one (its fake LocalAPI and loopback sockets,
//! `net/tailscale/tests.rs`); a Pitwall Cloud backend joins the same way.
//!
//! Checked:
//! - **the caller is what the network proved**: identity, person, device
//!   and groups come from the backend; no header or body changes them;
//! - **only `serve` reaches the handler**, one serve at a time; stopping
//!   (or dropping the handle) closes it, and a stale handle can't stop a
//!   newer serve;
//! - **revocation**: a device or person removed at the provider is refused
//!   at once and announced on [`Network::revocations`]; their groups go;
//! - **unreachable**: offline devices, a signed-out device, other networks;
//! - **discovery** lists the other online devices, with their owners, and
//!   never this one;
//! - **stable identities**: `subject` and the device's node id stay the same
//!   whatever the display name; the endpoint names a base URL;
//! - **several networks**, one handler: linked identities arrive as one
//!   person, unlinked ones apart;
//! - **streaming** responses arrive as they're written.

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::error::ErrorCode;

/// A scripted provider for one backend: the world its devices live in.
pub trait ContractWorld: Sized {
    type Net: Network + 'static;

    /// A network (a tailnet, a Cloud org) called `name`.
    fn new(name: &str) -> Self;
    /// The provider name in identities and network ids (`tailscale`).
    fn provider(&self) -> &str;
    /// Device `host` signed in as `login`, online.
    fn device(&self, login: &str, host: &str) -> Self::Net;
    fn addr(&self, host: &str) -> DeviceAddr;
    /// `login` as this provider's identity (for [`Network::groups`]).
    fn identity(&self, login: &str) -> Identity {
        Identity::new(self.provider(), login)
    }
    /// The provider puts `login` in group `group`.
    fn grant_group(&self, login: &str, group: &str);
    fn set_online(&self, host: &str, online: bool);
    /// `host` signs out of the network (`false`) and back in (`true`).
    fn set_signed_in(&self, host: &str, net: &Self::Net, on: bool);
    /// The provider's admin removes a device, or a person with theirs.
    fn remove_device(&self, host: &str);
    fn remove_person(&self, login: &str);
    /// Let `net` notice what changed at the provider (a backend that polls).
    fn refresh(&self, _net: &Self::Net) {}
}

/// Answers with what the network proved: `person|subject|device|groups`.
pub fn echo() -> Arc<dyn Handler> {
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

pub fn get(net: &dyn Network, to: &DeviceAddr, path: &str) -> Result<String> {
    let mut req = http::Request::get(path).body(Body::Empty).unwrap();
    req.extensions_mut().insert(Timeouts { connect: Duration::from_secs(2), idle: Some(Duration::from_secs(10)) });
    let res = net.client(to)?.send(req)?;
    Ok(String::from_utf8(res.into_body().read_all(1 << 20)?).unwrap())
}

fn revocations_now(rx: &std::sync::mpsc::Receiver<Revocation>) -> Vec<Revocation> {
    let mut out = Vec::new();
    while let Ok(r) = rx.recv_timeout(Duration::from_millis(200)) {
        out.push(r);
    }
    out
}

pub fn the_caller_is_what_the_network_proved<W: ContractWorld>() {
    let w = W::new("example");
    let alice = w.device("alice@example.com", "studio");
    let bob = w.device("bob@example.com", "bob-mbp");
    w.grant_group("bob@example.com", "platform");
    let _serving = alice.serve(echo()).unwrap();
    let p = w.provider().to_string();
    let answer = get(&bob, &w.addr("studio"), "/pitwall/v1/info").unwrap();
    assert_eq!(answer, format!("{p}:bob@example.com|bob@example.com|{}|platform", w.addr("bob-mbp").node));
    assert_eq!(alice.groups(&w.identity("bob@example.com")).unwrap(), vec![GroupRef { network: alice.id(), id: "platform".into() }]);
    assert_eq!(alice.id().provider(), p);
    assert_eq!(alice.state(), NetState::Ready);
}

pub fn headers_and_bodies_cant_change_who_is_calling<W: ContractWorld>() {
    let w = W::new("example");
    let alice = w.device("alice@example.com", "studio");
    let mallory = w.device("mallory@example.com", "laptop");
    let _serving = alice.serve(echo()).unwrap();
    let p = w.provider().to_string();
    let req = http::Request::post("/mcp")
        .header("Tailscale-User-Login", "alice@example.com")
        .header("X-Pitwall-Identity", "alice@example.com")
        .header("X-Forwarded-For", "100.64.0.1")
        .header("Pitwall-Agent", "fix@studio")
        .body(Body::from(format!(r#"{{"identity":{{"provider":"{p}","subject":"alice@example.com"}}}}"#)))
        .unwrap();
    let res = mallory.client(&w.addr("studio")).unwrap().send(req).unwrap();
    let text = String::from_utf8(res.into_body().read_all(1024).unwrap()).unwrap();
    assert!(text.starts_with(&format!("{p}:mallory@example.com|mallory@example.com|{}", w.addr("laptop").node)), "{text}");
}

pub fn only_serve_reaches_a_handler_and_stopping_closes_it<W: ContractWorld>() {
    let w = W::new("example");
    let alice = w.device("alice@example.com", "studio");
    let bob = w.device("bob@example.com", "bob-mbp");
    assert!(get(&bob, &w.addr("studio"), "/").unwrap_err().is(ErrorCode::Unreachable), "not serving: unreachable");
    let serving = alice.serve(echo()).unwrap();
    assert!(alice.serve(echo()).unwrap_err().is(ErrorCode::Conflict), "one serve at a time");
    assert!(get(&bob, &w.addr("studio"), "/").is_ok());
    serving.stop();
    assert!(get(&bob, &w.addr("studio"), "/").unwrap_err().is(ErrorCode::Unreachable));
    // Dropping the handle stops too, and a stale handle can't stop a newer serve.
    let first = alice.serve(echo()).unwrap();
    first.stop();
    let second = alice.serve(echo()).unwrap();
    drop(first);
    assert!(get(&bob, &w.addr("studio"), "/").is_ok(), "a stale handle didn't stop it");
    drop(second);
    assert!(get(&bob, &w.addr("studio"), "/").is_err());
}

pub fn revocation_stops_requests_and_is_announced<W: ContractWorld>() {
    let w = W::new("example");
    let alice = w.device("alice@example.com", "studio");
    let bob = w.device("bob@example.com", "bob-mbp");
    let carol = w.device("carol@example.com", "carol-mbp");
    w.grant_group("carol@example.com", "platform");
    let _serving = alice.serve(echo()).unwrap();
    let revoked = alice.revocations();
    assert!(get(&bob, &w.addr("studio"), "/").is_ok());

    w.remove_device("bob-mbp");
    assert!(get(&bob, &w.addr("studio"), "/").unwrap_err().is(ErrorCode::Unreachable), "refused at once");
    w.refresh(&alice);
    let seen = revocations_now(&revoked);
    assert!(seen.contains(&Revocation::Device { device: w.addr("bob-mbp") }), "{seen:?}");

    assert!(get(&carol, &w.addr("studio"), "/").is_ok());
    w.remove_person("carol@example.com");
    assert!(get(&carol, &w.addr("studio"), "/").unwrap_err().is(ErrorCode::Unreachable));
    w.refresh(&alice);
    let seen = revocations_now(&revoked);
    assert!(seen.iter().any(|r| matches!(r, Revocation::Identity { identity } if identity.subject == "carol@example.com")), "{seen:?}");
    assert!(alice.groups(&w.identity("carol@example.com")).unwrap().is_empty(), "their groups go with them");
}

pub fn offline_signed_out_and_foreign_devices_cant_be_reached<W: ContractWorld>() {
    let w = W::new("example");
    let other = W::new("elsewhere");
    let alice = w.device("alice@example.com", "studio");
    let bob = w.device("bob@example.com", "bob-mbp");
    let _serving = alice.serve(echo()).unwrap();
    other.device("alice@example.com", "studio");

    w.set_online("studio", false);
    assert!(get(&bob, &w.addr("studio"), "/").is_err(), "offline");
    w.set_online("studio", true);
    assert!(get(&bob, &w.addr("studio"), "/").is_ok());

    w.set_signed_in("bob-mbp", &bob, false);
    assert!(get(&bob, &w.addr("studio"), "/").is_err(), "signed out");
    assert!(bob.me().is_none());
    assert_ne!(bob.state(), NetState::Ready);
    w.set_signed_in("bob-mbp", &bob, true);

    assert!(bob.client(&other.addr("studio")).err().expect("not on this network").is(ErrorCode::Unreachable));
}

pub fn discovery_lists_the_other_online_devices<W: ContractWorld>() {
    let w = W::new("example");
    let alice = w.device("alice@example.com", "studio");
    w.device("bob@example.com", "bob-mbp");
    w.device("erin@example.com", "erin-mbp");
    w.device("dave@example.com", "dave-mbp");
    w.set_online("erin-mbp", false);
    w.remove_device("dave-mbp");
    let hints = alice.discover().unwrap();
    assert!(!hints.iter().any(|h| h.addr == w.addr("studio")), "never this device");
    assert!(!hints.iter().any(|h| h.addr == w.addr("dave-mbp")), "removed devices aren't offered");
    let bob = hints.iter().find(|h| h.addr == w.addr("bob-mbp")).expect("bob's device is offered");
    assert!(bob.online);
    assert_eq!(bob.owner.as_ref().map(|o| o.subject.as_str()), Some("bob@example.com"));
    assert!(hints.iter().filter(|h| h.addr == w.addr("erin-mbp")).all(|h| !h.online), "offline, if listed at all");
}

pub fn identities_are_stable<W: ContractWorld>() {
    let w = W::new("example");
    let alice = w.device("alice@example.com", "studio");
    let (me, addr) = alice.me().expect("signed in");
    assert_eq!((me.provider.as_str(), me.subject.as_str()), (w.provider(), "alice@example.com"));
    assert_eq!(addr, w.addr("studio"));
    assert_eq!(addr.network, alice.id());
    // The same after a while and through another device's eyes.
    let bob = w.device("bob@example.com", "bob-mbp");
    let _serving = bob.serve(echo()).unwrap();
    let a = get(&alice, &w.addr("bob-mbp"), "/").unwrap();
    let b = get(&alice, &w.addr("bob-mbp"), "/").unwrap();
    assert_eq!(a, b);
    assert!(a.contains(&format!("|alice@example.com|{}|", addr.node)), "{a}");
    let e = alice.endpoint();
    assert!(e.base_url.starts_with("http") && !e.base_url.ends_with('/'), "{e:?}");
}

/// One handler on two networks; the owner links Bob's two identities
/// (§15.7), so both arrive as one person. Unlinked, they stay apart.
pub fn several_networks_one_handler_and_linked_identities<W: ContractWorld>() {
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
    let home = W::new("example");
    let work = W::new("acme");
    let handler: Arc<dyn Handler> = Arc::new(Linked(echo()));
    let _a = home.device("alice@example.com", "studio").serve(handler.clone()).unwrap();
    let _b = work.device("alice@acme.com", "studio").serve(handler).unwrap();
    let bob_home = home.device("bob@example.com", "bob-mbp");
    let bob_work = work.device("bob@acme.com", "bob-mbp");
    let carol = work.device("carol@acme.com", "carol-mbp");
    assert!(get(&bob_home, &home.addr("studio"), "/").unwrap().starts_with("bob|"));
    assert!(get(&bob_work, &work.addr("studio"), "/").unwrap().starts_with("bob|"));
    assert!(get(&carol, &work.addr("studio"), "/").unwrap().starts_with(&format!("{}:carol@acme.com|", work.provider())));
}

pub fn streaming_bodies_arrive_as_they_are_written<W: ContractWorld>() {
    let w = W::new("example");
    let alice = w.device("alice@example.com", "studio");
    let bob = w.device("bob@example.com", "bob-mbp");
    let (go, wait) = std::sync::mpsc::channel::<()>();
    let wait = std::sync::Mutex::new(Some(wait));
    let _serving = alice
        .serve(Arc::new(move |_req: Request, _caller: Caller| {
            let (tx, body) = Body::channel();
            let wait = wait.lock().unwrap().take();
            std::thread::spawn(move || {
                tx.send("data: 1\n\n");
                // The second part only once the caller has read the first.
                if let Some(w) = wait {
                    let _ = w.recv_timeout(Duration::from_secs(5));
                }
                tx.send("data: 2\n\n");
            });
            Response::new(body)
        }))
        .unwrap();
    let mut req = Request::new(Body::Empty);
    req.extensions_mut().insert(Timeouts { connect: Duration::from_secs(2), idle: Some(Duration::from_secs(10)) });
    let res = bob.client(&w.addr("studio")).unwrap().send(req).unwrap();
    assert!(res.body().is_stream());
    let mut reader = res.into_body().into_reader();
    let mut first = [0u8; 9];
    reader.read_exact(&mut first).unwrap();
    assert_eq!(&first, b"data: 1\n\n");
    go.send(()).unwrap();
    let mut rest = String::new();
    reader.read_to_string(&mut rest).unwrap();
    assert_eq!(rest, "data: 2\n\n");
}

/// One `#[test]` per contract check for backend world `$world`.
macro_rules! network_contract_tests {
    ($world:ty) => {
        mod contract {
            use super::*;
            use $crate::net::contract as c;
            #[test]
            fn the_caller_is_what_the_network_proved() {
                c::the_caller_is_what_the_network_proved::<$world>();
            }
            #[test]
            fn headers_and_bodies_cant_change_who_is_calling() {
                c::headers_and_bodies_cant_change_who_is_calling::<$world>();
            }
            #[test]
            fn only_serve_reaches_a_handler_and_stopping_closes_it() {
                c::only_serve_reaches_a_handler_and_stopping_closes_it::<$world>();
            }
            #[test]
            fn revocation_stops_requests_and_is_announced() {
                c::revocation_stops_requests_and_is_announced::<$world>();
            }
            #[test]
            fn offline_signed_out_and_foreign_devices_cant_be_reached() {
                c::offline_signed_out_and_foreign_devices_cant_be_reached::<$world>();
            }
            #[test]
            fn discovery_lists_the_other_online_devices() {
                c::discovery_lists_the_other_online_devices::<$world>();
            }
            #[test]
            fn identities_are_stable() {
                c::identities_are_stable::<$world>();
            }
            #[test]
            fn several_networks_one_handler_and_linked_identities() {
                c::several_networks_one_handler_and_linked_identities::<$world>();
            }
            #[test]
            fn streaming_bodies_arrive_as_they_are_written() {
                c::streaming_bodies_arrive_as_they_are_written::<$world>();
            }
        }
    };
}
pub(crate) use network_contract_tests;
