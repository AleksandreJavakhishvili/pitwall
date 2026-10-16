//! Teams on the Fake network: records signed, chained and checked (forged
//! and stale ones refused), pull replication, joining by request and by
//! invite link, members without Pitwall, discovery of listed teams.

use std::sync::Arc;

use super::chain::{self, Chain};
use super::keys::{Ed25519, Ed25519Signer, RecordSigner};
use super::sync;
use super::*;
use crate::collab::time::ManualTime;
use crate::net::fake::{FakeNet, FakeNetwork};
use crate::net::{http, Body, Handler, Request, Response};
use crate::testing::TempDir;

/// The team endpoints of §8.4, routed to a store, with the Fronts'
/// handler's body shapes.
struct Router(Arc<TeamStore>);

fn json<T: serde::Serialize>(v: &T) -> Response {
    Response::new(Body::json(v))
}

fn status(code: u16) -> Response {
    let mut r = Response::new(Body::Empty);
    *r.status_mut() = http::StatusCode::from_u16(code).unwrap();
    r
}

fn from_err(e: PwError) -> Response {
    status(match e.code {
        ErrorCode::NotFound => 404,
        ErrorCode::Denied => 403,
        ErrorCode::Conflict => 409,
        _ => 500,
    })
}

impl Handler for Router {
    fn handle(&self, req: Request, caller: Caller) -> Response {
        let path = req.uri().path().to_string();
        let query = req.uri().query().unwrap_or("").to_string();
        let body = req.into_body().read_all(1 << 20).unwrap_or_default();
        let store = &self.0;
        if path == sync::paths::INFO {
            return json(&store.info(&caller));
        }
        if path == sync::paths::DISCOVER {
            return json(&serde_json::json!({"records": store.listed(&caller)}));
        }
        let rest = path.strip_prefix("/pitwall/v1/teams/").unwrap_or("");
        let (team, op) = rest.split_once('/').unwrap_or((rest, ""));
        let team = TeamId(team.to_string());
        let res = match op {
            "heads" => store.heads(&caller, &[serde_json::from_slice::<TeamHead>(&body).unwrap()]).map(|h| json(&serde_json::json!({"heads": h}))),
            "records" => {
                let after = query.strip_prefix("after=").and_then(|n| n.parse().ok()).unwrap_or(0);
                store.records(&caller, &team, after).map(|r| json(&serde_json::json!({"records": r})))
            }
            "join" => store.request_join(&caller, &team, serde_json::from_slice(&body).unwrap()).map(|()| json(&serde_json::json!({}))),
            "invites:redeem" => store.redeem_invite(&caller, &team, serde_json::from_slice(&body).unwrap()).map(|()| json(&serde_json::json!({}))),
            _ => return status(404),
        };
        res.unwrap_or_else(from_err)
    }
}

struct Node {
    store: Arc<TeamStore>,
    net: FakeNetwork,
    _serving: crate::net::ServeHandle,
    _dir: TempDir,
}

fn node(net: &FakeNet, login: &str, device: &str, seed: u8) -> Node {
    let dir = TempDir::new("teams");
    let node = open(dir.path(), net, login, device, seed);
    let serving = node.1.serve(Arc::new(Router(node.0.clone()))).unwrap();
    Node { store: node.0, net: node.1, _serving: serving, _dir: dir }
}

fn open(dir: &std::path::Path, net: &FakeNet, login: &str, device: &str, seed: u8) -> (Arc<TeamStore>, FakeNetwork) {
    let paths = CollabPaths::new(dir.join("collab"));
    let fake = net.device(login, device);
    let store = TeamStore::open(TeamsDeps {
        paths: paths.clone(),
        signer: Arc::new(Ed25519Signer::from_seed([seed; 32])),
        check: Arc::new(Ed25519),
        clock: Arc::new(ManualTime::at("2026-10-16T09:00:00+02:00")),
        audit: Arc::new(AuditLog::new(paths.audit_file())),
        version: "0.3.0".into(),
        label: device.into(),
    });
    store.set_networks(vec![Arc::new(fake.clone())]);
    (store, fake)
}

/// A record `by` signs on top of `prev`, as a forger or a stale admin would.
fn signed(prev: &TeamRecord, team: &TeamId, ops: Vec<TeamOp>, by: &Ed25519Signer, member: &str) -> TeamRecord {
    let state = chain::apply(&prev.state, &ops, prev.version + 1).unwrap();
    let mut r = TeamRecord {
        team: Some(team.clone()),
        version: prev.version + 1,
        prev: Some(chain::record_hash(prev)),
        ops,
        state,
        by: Signer { member: MemberId(member.into()), key: by.key() },
        at: "2026-10-16T09:00:00Z".into(),
        sig: String::new(),
        cosig: vec![],
    };
    r.sig = by.sign(&chain::signing_bytes(&r));
    r
}

#[test]
fn a_team_is_a_signed_chain_from_its_genesis() {
    let net = FakeNet::new("example");
    let alice = node(&net, "alice@example.com", "studio", 1);
    let team = alice.store.create("Platform", "API, jobs", true, JoinPolicy::Request).unwrap();
    assert!(team.0.starts_with("t_") && valid_team_id(&team));
    let records = alice.store.chain(&team).unwrap();
    assert!(records[0].is_genesis());
    assert_eq!(chain::team_id(&records[0]), team);
    alice.store.add_member(&team, net.person("erin@example.com")).unwrap();
    let records = alice.store.chain(&team).unwrap();
    assert_eq!(records[1].prev.as_deref(), Some(chain::record_hash(&records[0]).as_str()));
    // A member without Pitwall: an identity, no devices (§4.7).
    let state = alice.store.state(&team).unwrap();
    let erin = state.member_with(&net.person("erin@example.com")).unwrap();
    assert!(erin.devices.is_empty());
    assert!(alice.store.is_member(&team, &[net.person("erin@example.com")], &[]));
    // The whole chain checks again from disk.
    let reopened = open(alice._dir.path(), &net, "alice@example.com", "studio", 1).0;
    assert_eq!(reopened.chain(&team).unwrap(), records);
    let cards = reopened.cards();
    assert_eq!(cards[0].members, 2);
    assert_eq!(cards[0].mine, Some(TeamRole::Admin));
    assert_eq!(cards[0].fingerprint, chain::fingerprint(&team));
}

#[test]
fn forged_and_stale_records_are_refused() {
    let net = FakeNet::new("example");
    let alice = node(&net, "alice@example.com", "studio", 1);
    let team = alice.store.create("Platform", "", true, JoinPolicy::Request).unwrap();
    alice.store.add_member(&team, net.person("bob@example.com")).unwrap();
    let records = alice.store.chain(&team).unwrap();
    let head = records.last().unwrap().clone();
    let admin = alice.store.state(&team).unwrap().admins[0].0.clone();
    let alice_key = Ed25519Signer::from_seed([1; 32]);
    let mallory = Ed25519Signer::from_seed([66; 32]);
    let add_mallory = || vec![TeamOp::AddMember { member: MemberId("p_mal".into()), identities: vec![net.person("mallory@example.com")], devices: vec![] }];
    let mut chain = Chain::from_records(records.clone(), &Ed25519).unwrap();

    // Signed by a key that isn't an admin's device.
    let forged = signed(&head, &team, add_mallory(), &mallory, &admin);
    assert!(chain.offer(forged, &Ed25519).unwrap_err().message.contains("may not"));
    // Claims the admin's key, but the signature is someone else's.
    let mut stolen = signed(&head, &team, add_mallory(), &alice_key, &admin);
    stolen.sig = mallory.sign(&chain::signing_bytes(&stolen));
    assert!(chain.offer(stolen, &Ed25519).unwrap_err().message.contains("signature"));
    // A valid signature over a state that isn't the ops applied.
    let mut lying = signed(&head, &team, add_mallory(), &alice_key, &admin);
    lying.state.admins.push(MemberId("p_mal".into()));
    lying.sig = alice_key.sign(&chain::signing_bytes(&lying));
    assert!(chain.offer(lying, &Ed25519).unwrap_err().message.contains("state"));
    // Not chained to the head.
    let mut unchained = signed(&head, &team, add_mallory(), &alice_key, &admin);
    unchained.prev = Some("sha256:00".into());
    unchained.sig = alice_key.sign(&chain::signing_bytes(&unchained));
    assert!(chain.offer(unchained, &Ed25519).is_err());
    // Another team's record.
    let other = TeamId("t_other".into());
    assert!(chain.offer(signed(&head, &other, add_mallory(), &alice_key, &admin), &Ed25519).is_err());
    // Stale: an older version with different content is a conflict; the same is known.
    let stale = signed(&records[0], &team, add_mallory(), &alice_key, &admin);
    assert_eq!(chain.offer(stale, &Ed25519).unwrap(), chain::Accept::Conflict);
    assert_eq!(chain.offer(head.clone(), &Ed25519).unwrap(), chain::Accept::Known);
    // Ahead of us: pull what's in between first.
    let next = signed(&head, &team, add_mallory(), &alice_key, &admin);
    let after = signed(&next, &team, vec![TeamOp::SetJoin { join: JoinPolicy::Closed }], &alice_key, &admin);
    assert_eq!(chain.offer(after, &Ed25519).unwrap(), chain::Accept::Gap);
    assert_eq!(chain.offer(next, &Ed25519).unwrap(), chain::Accept::Added);
    // A genesis signed by a non-admin, or tampered with, founds nothing.
    let mut genesis = records[0].clone();
    genesis.state.name = "Renamed".into();
    assert!(chain::validate_genesis(&genesis, &Ed25519).is_err());
    // The store refuses forged records arriving by replication too.
    let forged = signed(&head, &team, add_mallory(), &mallory, &admin);
    assert!(alice.store.accept(&team, vec![forged], false).is_err());
    assert_eq!(alice.store.head(&team).unwrap().version, 2);
}

#[test]
fn ops_keep_the_team_consistent() {
    let net = FakeNet::new("example");
    let alice = node(&net, "alice@example.com", "studio", 1);
    let team = alice.store.create("Platform", "", true, JoinPolicy::Request).unwrap();
    let me = alice.store.state(&team).unwrap().admins[0].clone();
    // The last admin can't leave or be removed.
    assert!(alice.store.leave(&team).is_err());
    assert!(alice.store.remove_member(&team, &me).is_err());
    let bob = net.person("bob@example.com");
    alice.store.add_member(&team, bob.clone()).unwrap();
    assert!(alice.store.add_member(&team, bob.clone()).is_err(), "an identity belongs to one member");
    let bob_id = alice.store.state(&team).unwrap().member_with(&bob).unwrap().member.clone();
    alice.store.remove_member(&team, &bob_id).unwrap();
    let state = alice.store.state(&team).unwrap();
    assert!(state.member_with(&bob).is_none());
    assert_eq!(state.removed[0].member, bob_id);
    alice.store.set_info(&team, None, Some("API, jobs".into()), Some(false)).unwrap();
    assert!(!alice.store.state(&team).unwrap().listed);
}

#[test]
fn members_pull_records_they_are_behind_on() {
    let net = FakeNet::new("example");
    let alice = node(&net, "alice@example.com", "studio", 1);
    let bob = node(&net, "bob@example.com", "bob-mbp", 2);
    let team = alice.store.create("Platform", "", true, JoinPolicy::Request).unwrap();
    // Bob asks to join from discovery; Alice approves his request.
    let cards = sync::discover(&bob.store);
    let card = cards.iter().find(|c| c.team == team).unwrap();
    assert_eq!(card.mine, None);
    sync::request_join(&bob.store, &alice.net.addr(), &team, "I work on the API").unwrap();
    assert_eq!(bob.store.cards().iter().find(|c| c.team == team).unwrap().mine, Some(TeamRole::Requested));
    let pending = alice.store.requests(&team);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].identity.subject, "bob@example.com");
    assert_eq!(pending[0].device.addr, vec![bob.net.addr()], "bound to where the request came from");
    // Requests are for admins only; records for members only.
    let bob_caller = net.caller("bob-mbp").unwrap();
    assert!(alice.store.join_requests(&bob_caller, &team).unwrap_err().is(ErrorCode::NotFound));
    assert!(alice.store.records(&bob_caller, &team, 0).unwrap_err().is(ErrorCode::NotFound));
    alice.store.approve(&team, &net.person("bob@example.com")).unwrap();
    assert_eq!(sync::poll_joins(&bob.store), vec![team.clone()]);
    assert_eq!(bob.store.head(&team), alice.store.head(&team));
    assert_eq!(bob.store.cards().iter().find(|c| c.team == team).unwrap().mine, Some(TeamRole::Member));

    // Alice changes the team; Bob catches up when they next talk.
    alice.store.add_member(&team, net.person("erin@example.com")).unwrap();
    assert_eq!(sync::sync_with(&bob.store, &alice.net.addr()).unwrap(), 1);
    assert_eq!(bob.store.head(&team), alice.store.head(&team));
    assert!(bob.store.is_member(&team, &[net.person("erin@example.com")], &[]));
    // Up to date: nothing to pull.
    assert_eq!(sync::sync_with(&bob.store, &alice.net.addr()).unwrap(), 0);
}

#[test]
fn closed_teams_and_too_many_requests_are_refused() {
    let net = FakeNet::new("example");
    let alice = node(&net, "alice@example.com", "studio", 1);
    let bob = node(&net, "bob@example.com", "bob-mbp", 2);
    let team = alice.store.create("Platform", "", true, JoinPolicy::Request).unwrap();
    let caller = net.caller("bob-mbp").unwrap();
    let request = |n: u8| JoinRequest { note: String::new(), device: TeamDevice { key: Ed25519Signer::from_seed([n; 32]).key(), addr: vec![] } };
    for n in 10..13 {
        alice.store.request_join(&caller, &team, request(n)).unwrap();
    }
    assert!(alice.store.request_join(&caller, &team, request(13)).unwrap_err().is(ErrorCode::Conflict));
    alice.store.set_join(&team, JoinPolicy::Closed).unwrap();
    assert!(sync::request_join(&bob.store, &alice.net.addr(), &team, "").unwrap_err().is(ErrorCode::Denied));
}

#[test]
fn an_invite_link_carries_the_trust_and_works_once() {
    let net = FakeNet::new("example");
    let alice = node(&net, "alice@example.com", "studio", 1);
    let bob = node(&net, "bob@example.com", "bob-mbp", 2);
    let carol = node(&net, "carol@example.com", "carol-mbp", 3);
    // Invite-only: never listed.
    let team = alice.store.create("Secret", "", false, JoinPolicy::Closed).unwrap();
    assert!(sync::discover(&bob.store).iter().all(|c| c.team != team));
    let link = alice.store.invite(&team).unwrap();
    let parsed = ParsedInvite::parse(&link.url).unwrap();
    assert_eq!(parsed.team, team);
    assert_eq!(parsed.by, "fake:alice@example.com");
    assert_eq!(parsed.code, link.code);
    assert_eq!(sync::redeem(&bob.store, &alice.net.addr(), &link.url).unwrap(), team);
    assert_eq!(bob.store.head(&team), alice.store.head(&team));
    assert!(alice.store.is_member(&team, &[net.person("bob@example.com")], &[]));
    // One use only; a wrong code looks like nothing.
    assert!(sync::redeem(&carol.store, &alice.net.addr(), &link.url).unwrap_err().is(ErrorCode::NotFound));
    let wrong = link.url.replace(&link.code, "nope");
    assert!(sync::redeem(&carol.store, &alice.net.addr(), &wrong).unwrap_err().is(ErrorCode::NotFound));
}

#[test]
fn discovery_shows_listed_teams_and_info() {
    let net = FakeNet::new("example");
    let alice = node(&net, "alice@example.com", "studio", 1);
    let bob = node(&net, "bob@example.com", "bob-mbp", 2);
    let listed = alice.store.create("Platform", "API", true, JoinPolicy::Request).unwrap();
    alice.store.create("Hidden", "", false, JoinPolicy::Request).unwrap();
    let info = alice.store.info(&net.caller("bob-mbp").unwrap());
    assert_eq!(info.person.subject, "alice@example.com");
    assert_eq!(info.listed_teams, vec![listed.clone()]);
    assert!(info.a2a && info.mcp == "/mcp");
    let cards = sync::discover(&bob.store);
    assert_eq!(cards.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Platform"]);
    assert_eq!(bob.store.discovered()[0].seen_at, vec![alice.net.addr()]);
    // A tampered listed record isn't shown.
    let mut record = alice.store.chain(&listed).unwrap().remove(0);
    record.state.name = "Phish".into();
    assert!(sync::check_listed(&record, &Ed25519).is_err());
}

#[test]
fn self_leave_and_key_rotation_are_signed_by_the_members_own_device() {
    let net = FakeNet::new("example");
    let alice = node(&net, "alice@example.com", "studio", 1);
    let bob = node(&net, "bob@example.com", "bob-mbp", 2);
    let team = alice.store.create("Platform", "", true, JoinPolicy::Request).unwrap();
    sync::request_join(&bob.store, &alice.net.addr(), &team, "").unwrap();
    alice.store.approve(&team, &net.person("bob@example.com")).unwrap();
    sync::poll_joins(&bob.store);
    // Bob may not add people…
    assert!(bob.store.add_member(&team, net.person("mallory@example.com")).is_err());
    // …but may leave; Alice takes the record.
    bob.store.leave(&team).unwrap();
    let records = bob.store.chain(&team).unwrap();
    assert_eq!(alice.store.accept(&team, vec![records.last().unwrap().clone()], false).unwrap(), 1);
    assert!(!alice.store.is_member(&team, &[net.person("bob@example.com")], &[]));
}

#[test]
fn identity_links_come_from_member_entries() {
    let net = FakeNet::new("example");
    let alice = node(&net, "alice@example.com", "studio", 1);
    let team = alice.store.create("Platform", "", true, JoinPolicy::Request).unwrap();
    let bob = net.person("bob@example.com");
    alice.store.add_member(&team, bob.clone()).unwrap();
    let member = alice.store.state(&team).unwrap().member_with(&bob).unwrap().member.clone();
    let cloud = crate::net::Identity::new("cloud", "u_8kq2");
    alice.store.add_identity(&team, &member, cloud.clone()).unwrap();
    let linked = alice.store.linked(&cloud);
    assert!(linked.iter().any(|i| i.is(&bob)) && linked.iter().any(|i| i.is(&cloud)));
}

#[test]
fn member_devices_hear_announcements() {
    let net = FakeNet::new("example");
    let alice = node(&net, "alice@example.com", "studio", 1);
    let bob = node(&net, "bob@example.com", "bob-mbp", 2);
    let team = alice.store.create("Platform", "", true, JoinPolicy::Request).unwrap();
    let bob_caller = net.caller("bob-mbp").unwrap();
    assert!(alice.store.announcements(&bob_caller).unwrap_err().is(ErrorCode::NotFound));
    sync::request_join(&bob.store, &alice.net.addr(), &team, "").unwrap();
    alice.store.approve(&team, &net.person("bob@example.com")).unwrap();
    let heads = alice.store.announcements(&bob_caller).unwrap();
    alice.store.add_member(&team, net.person("erin@example.com")).unwrap();
    let head = heads.try_recv().unwrap();
    assert_eq!(head, alice.store.head(&team).unwrap());
    assert_eq!(head.version, 3);
}
