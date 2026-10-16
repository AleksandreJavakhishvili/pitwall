//! The Fake backend against the guarantees every backend must meet (§15.4,
//! the shared contract suite in `net::contract`), and what is Fake's own.

use super::fake::{self, FakeNet};
use super::*;

use super::contract::ContractWorld;

/// The Fake backend as a scripted provider for the contract suite.
struct FakeWorld(FakeNet);

impl ContractWorld for FakeWorld {
    type Net = fake::FakeNetwork;

    fn new(name: &str) -> FakeWorld {
        FakeWorld(FakeNet::new(name))
    }
    fn provider(&self) -> &str {
        fake::PROVIDER
    }
    fn device(&self, login: &str, host: &str) -> fake::FakeNetwork {
        self.0.device(login, host)
    }
    fn addr(&self, host: &str) -> DeviceAddr {
        self.0.addr(host)
    }
    fn identity(&self, login: &str) -> Identity {
        self.0.person(login)
    }
    fn grant_group(&self, login: &str, group: &str) {
        self.0.join_group(login, group);
    }
    fn set_online(&self, host: &str, online: bool) {
        self.0.set_online(host, online);
    }
    fn set_signed_in(&self, _host: &str, net: &fake::FakeNetwork, on: bool) {
        net.set_state(if on { NetState::Ready } else { NetState::SignedOut });
    }
    fn remove_device(&self, host: &str) {
        self.0.revoke_device(host);
    }
    fn remove_person(&self, login: &str) {
        self.0.revoke_person(login);
    }
}

super::contract::network_contract_tests!(FakeWorld);

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
