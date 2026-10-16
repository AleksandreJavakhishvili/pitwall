//! The fronts end to end on the Fake network: Bob's device calls Alice's
//! handler through `Network::client`, with the Caller the network proves.
//! The core is a stub ([`super::stub`]). Golden fixtures live in
//! `fixtures/`; `UPDATE_GOLDEN=1 cargo test -p pitwall-core front::` rewrites
//! them (review the diff: they are the wire format).

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::a2a::model::*;
use super::client::{self, Labels, Outgoing, PitwallClient, TeamEvent};
use super::stub::{OnAsk, StubConversations, StubTeams, AT};
use super::wire::{self, SseReader};
use super::{Fronts, Options};
use crate::collab::team::{TeamHead, TeamId};
use crate::collab::turn::{Changes, ChangesState, Front, ReplySource, ShareCard, TurnId, TurnStatus};
use crate::collab::{Capability, ConversationId, Left, Refusal, RefusalReason, Reply};
use crate::error::ErrorCode;
use crate::net::fake::{FakeNet, FakeNetwork};
use crate::net::http::{HeaderMap, StatusCode};
use crate::net::{Body, ClientAuth, HttpClient, Identity, Network, Request, ServeHandle};

// ------------------------------------------------------------- set-up

struct World {
    net: FakeNet,
    alice: FakeNetwork,
    bob: FakeNetwork,
    erin: FakeNetwork,
    conv: Arc<StubConversations>,
    teams: Arc<StubTeams>,
    _serving: ServeHandle,
}

fn options() -> Options {
    Options {
        heartbeat: Duration::from_secs(30),
        send_wait: Duration::from_secs(2),
        version: "0.0.0-test".into(),
        ..Options::default()
    }
}

fn card(owner: &Identity, slug: &str, name: &str, description: &str, capability: Capability) -> ShareCard {
    ShareCard {
        owner: owner.clone(),
        slug: slug.into(),
        name: name.into(),
        description: description.into(),
        capability,
        kind: "claude".into(),
        kind_name: "Claude Code".into(),
        version: 3,
        messages_per_day: 20,
        left: 17,
    }
}

fn world_with(options: Options) -> World {
    let net = FakeNet::new("example");
    let alice = net.device("alice@example.com", "studio");
    let bob = net.device("bob@example.com", "bob-mbp");
    let erin = net.device("erin@example.com", "erin-laptop");
    let carol = net.person("carol@example.com");
    let alice_id = net.person("alice@example.com");
    let bob_id = net.person("bob@example.com");
    let conv = Arc::new(StubConversations::default());
    conv.share(
        card(
            &alice_id,
            "api-expert",
            "API expert",
            "Questions about the API service: auth, billing, jobs",
            Capability::Read,
        ),
        &[&bob_id],
    );
    conv.share(
        card(&alice_id, "payments-expert", "Payments expert", "Refunds and invoices", Capability::WriteApproval),
        &[&bob_id],
    );
    conv.share(card(&alice_id, "secret", "Secret", "Not for Bob", Capability::Read), &[&carol]);
    let teams = Arc::new(StubTeams::new(alice_id, vec![bob_id]));
    let network: Arc<dyn Network> = Arc::new(alice.clone());
    let fronts = Fronts::for_networks(conv.clone(), teams.clone(), vec![network], options);
    let serving = alice.serve(Arc::new(fronts)).unwrap();
    World { net, alice, bob, erin, conv, teams, _serving: serving }
}

fn world() -> World {
    world_with(options())
}

impl World {
    fn http(&self, from: &FakeNetwork) -> Box<dyn HttpClient> {
        from.client(&self.alice.addr()).unwrap()
    }

    fn pitwall(&self, from: &FakeNetwork) -> PitwallClient {
        PitwallClient::new(self.http(from))
    }
}

struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: Body,
}

impl Answer {
    fn json(self) -> Value {
        let bytes = self.body.read_all(64 << 20).unwrap();
        serde_json::from_slice(&bytes).unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&bytes)))
    }

    fn text(self) -> String {
        String::from_utf8(self.body.read_all(64 << 20).unwrap()).unwrap()
    }

    fn header(&self, name: &str) -> Option<String> {
        self.headers.get(name).map(|v| v.to_str().unwrap().to_string())
    }
}

fn request(method: &str, path: &str, headers: &[(&str, &str)], body: Option<Value>) -> Request {
    let mut b = crate::net::http::Request::builder().method(method).uri(path);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    b.body(body.map_or(Body::Empty, |v| Body::json(&v))).unwrap()
}

fn call(http: &dyn HttpClient, method: &str, path: &str, headers: &[(&str, &str)], body: Option<Value>) -> Answer {
    let res = http.send(request(method, path, headers, body)).unwrap();
    let (parts, body) = res.into_parts();
    Answer { status: parts.status, headers: parts.headers, body }
}

const V: (&str, &str) = ("A2A-Version", "1.0");

fn a2a_message(text: &str) -> Value {
    json!({"message": {"messageId": "m1", "role": "ROLE_USER", "parts": [{"text": text}]}})
}

/// Wait until something follows the turn (a stream or a waiting call).
fn until_watched(conv: &StubConversations, turn: &TurnId) {
    let start = Instant::now();
    while conv.watchers(turn) == 0 {
        assert!(start.elapsed() < Duration::from_secs(5), "nobody watches {turn:?}");
        thread::sleep(Duration::from_millis(2));
    }
}

/// SSE events on a thread, so a test can wait with a timeout.
fn events(body: Body) -> mpsc::Receiver<wire::SseEvent> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for e in SseReader::new(body) {
            match e {
                Ok(e) => {
                    if tx.send(e).is_err() {
                        return;
                    }
                }
                Err(_) => return,
            }
        }
    });
    rx
}

fn next(rx: &mpsc::Receiver<wire::SseEvent>) -> Value {
    let e = rx.recv_timeout(Duration::from_secs(5)).expect("an event");
    serde_json::from_str(&e.data).unwrap()
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/collab/front/fixtures").join(name)
}

/// Compare with (or, with `UPDATE_GOLDEN`, write) a JSON fixture.
fn golden(name: &str, actual: &Value) {
    let path = golden_path(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(actual).unwrap() + "\n").unwrap();
        return;
    }
    let expected: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture")).unwrap();
    assert_eq!(actual, &expected, "{name} differs:\n{}", serde_json::to_string_pretty(actual).unwrap());
}

fn done_with_reply(text: &str) -> impl FnOnce(&mut crate::collab::Turn) {
    let text = text.to_string();
    move |t| {
        t.status = TurnStatus::Done;
        t.reply = Some(Reply { text, source: ReplySource::Hook });
        t.done_at = Some("2026-10-16T10:02:00Z".into());
        t.left = Some(Left { messages_today: 16, resets_at: "2026-10-17T00:00:00+02:00".into() });
    }
}

fn sent_changes(parts: u32) -> Changes {
    Changes {
        state: ChangesState::Sent,
        base_commit: "3f2a1c9d0e".into(),
        files: 2,
        added: 14,
        removed: 3,
        patches: 1,
        parts,
    }
}

const PATCH: &str =
    "From 7c1e Mon Sep 17 00:00:00 2001\nSubject: [PATCH] add dry-run\n\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";

// ---------------------------------------------------------------- A2A

#[test]
fn a2a_card_is_served_only_to_the_audience() {
    let w = world();
    let bob = w.http(&w.bob);
    let res = call(&*bob, "GET", "/a2a/api-expert/card", &[], None);
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("content-type").as_deref(), Some("application/a2a+json"));
    golden("a2a_card.json", &res.json());
    // Also where clients look relative to the base URL.
    let res = call(&*bob, "GET", "/a2a/api-expert/.well-known/agent-card.json", &[], None);
    assert_eq!(res.status, StatusCode::OK);

    // Not in the audience and no such share: the same 404.
    let not_mine = call(&*w.http(&w.erin), "GET", "/a2a/api-expert/card", &[], None);
    let unknown = call(&*bob, "GET", "/a2a/no-such-share/card", &[], None);
    let secret = call(&*bob, "GET", "/a2a/secret/card", &[], None);
    assert_eq!(
        (not_mine.status, unknown.status, secret.status),
        (StatusCode::NOT_FOUND, StatusCode::NOT_FOUND, StatusCode::NOT_FOUND)
    );
    let (a, b, c) = (not_mine.json(), unknown.json(), secret.json());
    assert_eq!(a, b);
    assert_eq!(a, c);
    golden("a2a_not_found.json", &a);
}

#[test]
fn a2a_card_carries_oauth_security_schemes_from_the_endpoint() {
    let w = world();
    w.alice.set_client_auth(ClientAuth::OAuth {
        issuer: "https://auth.pitwall.example".into(),
        resource: "https://studio.acme.pitwall.example/mcp".into(),
        scopes: vec!["collab".into()],
    });
    let res = call(&*w.http(&w.bob), "GET", "/a2a/api-expert/card", &[], None);
    let card = res.json();
    golden(
        "a2a_card_oauth.json",
        &json!({
            "securitySchemes": card["securitySchemes"],
            "securityRequirements": card["securityRequirements"],
        }),
    );
}

#[test]
fn a2a_requests_need_version_1_0() {
    let w = world();
    let bob = w.http(&w.bob);
    w.conv.on_ask(OnAsk::Reply("hi".into()));
    let res = call(&*bob, "POST", "/a2a/api-expert/message:send", &[], Some(a2a_message("hello")));
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    golden("a2a_version_error.json", &res.json());
    let res =
        call(&*bob, "POST", "/a2a/api-expert/message:send", &[("A2A-Version", "0.3")], Some(a2a_message("hello")));
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    let res = call(&*bob, "POST", "/a2a/api-expert/message:send?A2A-Version=1.0", &[], Some(a2a_message("hello")));
    assert_eq!(res.status, StatusCode::OK);
    assert!(w.conv.asks().len() == 1, "only the versioned request reached the core");
}

#[test]
fn a2a_send_returns_the_task_with_its_reply() {
    let w = world();
    w.conv.on_ask(OnAsk::Reply("Refunds older than 30 days go through the manual queue.".into()));
    let res = call(
        &*w.http(&w.bob),
        "POST",
        "/a2a/api-expert/message:send",
        &[V, ("Pitwall-Agent", "client-fix@bob-mbp")],
        Some(a2a_message("How are old refunds handled?")),
    );
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("a2a-version").as_deref(), Some("1.0"));
    golden("a2a_send.json", &res.json());

    let asks = w.conv.asks();
    let (asker, ask) = &asks[0];
    assert_eq!(ask.message, "How are old refunds handled?");
    assert_eq!(ask.share, "api-expert");
    assert_eq!(asker.front, Front::A2a);
    assert_eq!(asker.agent.as_deref(), Some("client-fix@bob-mbp"));
    assert!(ask.conversation.is_none() && !ask.new_conversation && ask.my_changes.is_none());
}

#[test]
fn a2a_send_maps_context_new_conversation_and_patch_parts() {
    let w = world();
    let bob = w.http(&w.bob);
    w.conv.on_ask(OnAsk::Reply("ok".into()));
    let first = call(&*bob, "POST", "/a2a/payments-expert/message:send", &[V], Some(a2a_message("one"))).json();
    let context = first["task"]["contextId"].as_str().unwrap().to_string();

    let mut again = a2a_message("two");
    again["message"]["contextId"] = json!(context);
    again["message"]["parts"].as_array_mut().unwrap().push(json!({
        "raw": wire::b64_encode(PATCH.as_bytes()), "mediaType": "text/x-patch", "filename": "mine.patch"
    }));
    let second = call(&*bob, "POST", "/a2a/payments-expert/message:send", &[V], Some(again)).json();
    assert_eq!(second["task"]["contextId"], json!(context));
    assert_eq!(second["task"]["id"], json!(format!("{context}/2")));

    let mut fresh = a2a_message("three");
    fresh["message"]["metadata"] = json!({"pitwall/newConversation": true});
    let third = call(&*bob, "POST", "/a2a/payments-expert/message:send", &[V], Some(fresh)).json();
    assert_ne!(third["task"]["contextId"], json!(context));

    let asks = w.conv.asks();
    assert_eq!(asks[1].1.conversation, Some(ConversationId(context.clone())));
    assert_eq!(asks[1].1.my_changes.as_deref(), Some(PATCH));
    assert!(asks[2].1.new_conversation);

    // Someone else's context is 404, like no context at all.
    let mut theirs = a2a_message("x");
    theirs["message"]["contextId"] = json!(context);
    w.conv.share(
        card(&w.net.person("alice@example.com"), "payments-expert", "Payments expert", "Refunds", Capability::Write),
        &[&w.net.person("bob@example.com"), &w.net.person("erin@example.com")],
    );
    let res = call(&*w.http(&w.erin), "POST", "/a2a/payments-expert/message:send", &[V], Some(theirs));
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    // Other part kinds are refused.
    let mut data = a2a_message("x");
    data["message"]["parts"] = json!([{"data": {"a": 1}}]);
    let res = call(&*bob, "POST", "/a2a/payments-expert/message:send", &[V], Some(data));
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.json()["error"]["details"][0]["reason"], json!("CONTENT_TYPE_NOT_SUPPORTED"));
}

#[test]
fn a2a_refusals_are_rejected_tasks_with_reason_and_reset() {
    let w = world();
    w.conv.on_ask(OnAsk::Refuse(Refusal {
        reason: RefusalReason::Quota,
        message: "Daily limit reached for alice/api-expert; resets at 00:00".into(),
        resets_at: Some("2026-10-17T00:00:00+02:00".into()),
        until: None,
    }));
    let res = call(&*w.http(&w.bob), "POST", "/a2a/api-expert/message:send", &[V], Some(a2a_message("again?")));
    assert_eq!(res.status, StatusCode::OK);
    golden("a2a_rejected.json", &res.json());
}

#[test]
fn a2a_owner_waits_and_held_changes_are_working() {
    let w = world();
    w.conv.on_ask(OnAsk::Work);
    let bob = w.http(&w.bob);
    let mut msg = a2a_message("change it");
    msg["configuration"] = json!({"returnImmediately": true});
    let task = call(&*bob, "POST", "/a2a/payments-expert/message:send", &[V], Some(msg)).json()["task"].clone();
    let id = TurnId(task["id"].as_str().unwrap().into());
    assert_eq!(task["status"]["state"], json!("TASK_STATE_WORKING"));

    w.conv.update(&id, |t| t.status = TurnStatus::WaitingForOwner);
    let path = format!("/a2a/payments-expert/tasks/{}", id.0); // the `/` as is
    let got = call(&*bob, "GET", &path, &[V], None).json();
    assert_eq!(got["status"]["state"], json!("TASK_STATE_WORKING"));
    assert_eq!(got["status"]["message"]["parts"][0]["text"], json!("waiting for alice"));

    w.conv.update(&id, |t| {
        t.status = TurnStatus::ChangesHeld;
        t.reply = Some(Reply { text: "Done; see the patch.".into(), source: ReplySource::Hook });
        t.changes = Some(Changes { state: ChangesState::Held, ..sent_changes(1) });
    });
    let path = format!("/a2a/payments-expert/tasks/{}", wire::percent_encode(&id.0)); // or as %2F
    let got = call(&*bob, "GET", &path, &[V], None).json();
    golden("a2a_held.json", &got);
}

#[test]
fn a2a_sent_changes_come_as_a_patch_file_inline_or_by_url() {
    let w = world();
    w.conv.on_ask(OnAsk::Work);
    let bob = w.http(&w.bob);
    let task = call(
        &*bob,
        "POST",
        "/a2a/payments-expert/message:send",
        &[V],
        Some(json!({
            "message": {"messageId": "m", "role": "ROLE_USER", "parts": [{"text": "x"}]},
            "configuration": {"returnImmediately": true}
        })),
    )
    .json()["task"]
        .clone();
    let id = TurnId(task["id"].as_str().unwrap().into());
    w.conv.set_patch(&id, PATCH);
    w.conv.update(&id, |t| {
        done_with_reply("Added the flag.")(t);
        t.changes = Some(sent_changes(1));
    });
    let got = call(&*bob, "GET", &format!("/a2a/payments-expert/tasks/{}", id.0), &[V], None).json();
    golden("a2a_done_with_changes.json", &got);
    let task: Task = serde_json::from_value(got).unwrap();
    assert_eq!(client::inline_patch(&task).as_deref(), Some(PATCH));

    // Over 2 MiB: a url to the results endpoint instead.
    let big = format!("{PATCH}{}", "+".repeat(crate::collab::MAX_BODY + 1));
    w.conv.set_patch(&id, &big);
    let task: Task =
        serde_json::from_value(call(&*bob, "GET", &format!("/a2a/payments-expert/tasks/{}", id.0), &[V], None).json())
            .unwrap();
    let url = client::patch_url(&task).unwrap().to_string();
    assert_eq!(url, "http://studio.example.fake/pitwall/v1/results/c_1%2F1");
    assert_eq!(w.pitwall(&w.bob).fetch(&url).unwrap(), big);
    // Same identity check: not Erin's.
    let res = call(&*w.http(&w.erin), "GET", "/pitwall/v1/results/c_1%2F1", &[], None);
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[test]
fn a2a_stream_sends_task_then_artifacts_and_status_updates() {
    let w = world();
    w.conv.on_ask(OnAsk::Queue(2));
    let res = call(&*w.http(&w.bob), "POST", "/a2a/payments-expert/message:stream", &[V], Some(a2a_message("go")));
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("content-type").as_deref(), Some("text/event-stream"));
    let id = TurnId("c_1/1".into());
    let (tx, rx) = mpsc::channel();
    let body = res.body;
    let reader = thread::spawn(move || {
        let text = String::from_utf8(body.read_all(64 << 20).unwrap()).unwrap();
        tx.send(()).unwrap();
        text
    });
    until_watched(&w.conv, &id);
    w.conv.update(&id, |t| t.status = TurnStatus::Working);
    w.conv.update(&id, |t| t.status = TurnStatus::Working); // no change: no event
    w.conv.update(&id, |t| t.status = TurnStatus::WaitingForOwner);
    w.conv.set_patch(&id, PATCH);
    w.conv.update(&id, |t| {
        done_with_reply("Added the flag.")(t);
        t.changes = Some(sent_changes(1));
    });
    rx.recv_timeout(Duration::from_secs(5)).expect("the stream ends after the final state");
    let text = reader.join().unwrap();
    // Framing: one `data:` line per event, a blank line after each.
    // (Key order inside the JSON depends on serde_json's features, so the
    // events are compared as values.)
    let frames: Vec<&str> = text.strip_suffix("\n\n").unwrap().split("\n\n").collect();
    assert!(frames.iter().all(|f| f.starts_with("data: {") && !f.contains('\n')), "{text}");
    let mut events = Vec::new();
    for e in SseReader::new(Body::from(text.clone())) {
        let v: Value = serde_json::from_str(&e.unwrap().data).unwrap();
        // Every event is one StreamResponse with exactly one field.
        assert_eq!(v.as_object().unwrap().len(), 1);
        let _: StreamResponse = serde_json::from_value(v.clone()).unwrap();
        events.push(v);
    }
    assert_eq!(events.len(), frames.len());
    golden("a2a_stream.json", &Value::Array(events));
}

#[test]
fn a2a_get_list_cancel_and_subscribe() {
    let w = world();
    w.conv.on_ask(OnAsk::Work);
    let bob = w.http(&w.bob);
    let mut msg = a2a_message("one");
    msg["configuration"] = json!({"returnImmediately": true});
    let one = call(&*bob, "POST", "/a2a/api-expert/message:send", &[V], Some(msg.clone())).json();
    let two = call(&*bob, "POST", "/a2a/api-expert/message:send", &[V], Some(msg.clone())).json();
    call(&*bob, "POST", "/a2a/payments-expert/message:send", &[V], Some(msg));
    let context = one["task"]["contextId"].as_str().unwrap().to_string();

    let list = call(&*bob, "GET", &format!("/a2a/api-expert/tasks?contextId={context}"), &[V], None).json();
    let ids: Vec<&str> = list["tasks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["c_1/2", "c_1/1"], "newest first, this share only");
    assert_eq!(list["totalSize"], json!(2));
    assert_eq!(list["nextPageToken"], json!(""));
    let page = call(&*bob, "GET", "/a2a/api-expert/tasks?pageSize=1", &[V], None).json();
    assert_eq!(page["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(page["nextPageToken"], json!("1"));
    let working = call(&*bob, "GET", "/a2a/api-expert/tasks?status=TASK_STATE_COMPLETED", &[V], None).json();
    assert_eq!(working["tasks"], json!([]));

    // Another share's base URL doesn't show this task; nor does Erin.
    let id2 = two["task"]["id"].as_str().unwrap().to_string();
    let res = call(&*bob, "GET", &format!("/a2a/payments-expert/tasks/{id2}"), &[V], None);
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = call(&*w.http(&w.erin), "GET", &format!("/a2a/api-expert/tasks/{id2}"), &[V], None);
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    golden("a2a_task_not_found.json", &res.json());
    let res = call(&*w.http(&w.erin), "GET", &format!("/a2a/api-expert/tasks?contextId={context}"), &[V], None);
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    // Subscribe, then cancel: the stream ends with CANCELED.
    let id1 = TurnId("c_1/1".into());
    let res = call(&*bob, "POST", "/a2a/api-expert/tasks/c_1%2F1:subscribe", &[V], None);
    let rx = events(res.body);
    assert_eq!(next(&rx)["task"]["id"], json!("c_1/1"));
    until_watched(&w.conv, &id1);
    let canceled = call(&*bob, "POST", "/a2a/api-expert/tasks/c_1/1:cancel", &[V], Some(json!({"id": "c_1/1"})));
    assert_eq!(canceled.status, StatusCode::OK);
    assert_eq!(canceled.json()["status"]["state"], json!("TASK_STATE_CANCELED"));
    assert_eq!(next(&rx)["statusUpdate"]["status"]["state"], json!("TASK_STATE_CANCELED"));
    assert_eq!(w.conv.canceled(), vec![id1]);
    let again = call(&*bob, "POST", "/a2a/api-expert/tasks/c_1/1:cancel", &[V], None);
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    assert_eq!(again.json()["error"]["details"][0]["reason"], json!("TASK_NOT_CANCELABLE"));
}

// ------------------------------------------------------- labels, loops

#[test]
fn labels_never_widen_access() {
    let w = world();
    w.conv.on_ask(OnAsk::Reply("x".into()));
    let erin = w.http(&w.erin);
    let pretend = [V, ("Pitwall-Agent", "client-fix@bob-mbp"), ("Pitwall-Chain", "c_1/1,bob@example.com/api-expert")];
    assert_eq!(call(&*erin, "GET", "/a2a/api-expert/card", &pretend, None).status, StatusCode::NOT_FOUND);
    let res = call(&*erin, "POST", "/a2a/api-expert/message:send", &pretend, Some(a2a_message("hi")));
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let tools = mcp(&*erin, &[("Pitwall-Agent", "bob")], json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
    assert_eq!(tools["result"]["tools"], json!([]));
    assert!(w.conv.asks().is_empty());

    // Bob's label is his to set, and it reaches the core as a label only.
    let bob = w.http(&w.bob);
    let long = "a".repeat(200);
    call(&*bob, "POST", "/a2a/api-expert/message:send", &[V, ("Pitwall-Agent", &long)], Some(a2a_message("hi")));
    let (asker, _) = &w.conv.asks()[0];
    assert_eq!(asker.agent, None, "too long: no label");
    match &asker.who {
        crate::collab::turn::Who::Network(c) => assert_eq!(c.identity().subject, "bob@example.com"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn loops_and_deep_chains_are_refused_on_both_fronts() {
    let w = world();
    w.conv.on_ask(OnAsk::Reply("x".into()));
    let bob = w.http(&w.bob);
    let send = |chain: &str| {
        call(&*bob, "POST", "/a2a/api-expert/message:send", &[V, ("Pitwall-Chain", chain)], Some(a2a_message("hi")))
            .json()
    };
    let cycle = send("c_9/3,alice/api-expert");
    assert_eq!(cycle["task"]["status"]["state"], json!("TASK_STATE_REJECTED"));
    assert_eq!(cycle["task"]["status"]["message"]["parts"][1]["data"]["reason"], json!("loop"));
    let by_email = send("alice@example.com/api-expert");
    assert_eq!(by_email["task"]["status"]["state"], json!("TASK_STATE_REJECTED"));
    let deep = send("c_1/1,bob/fixer,c_2/4,carol/reviewer,c_3/1,dave/ops");
    assert_eq!(deep["task"]["status"]["state"], json!("TASK_STATE_REJECTED"));
    assert!(w.conv.asks().is_empty(), "refused before the core");

    // Two shares deep and not this one: fine, and the chain reaches the core.
    let ok = send("c_1/1,bob/fixer,c_2/4,carol/reviewer");
    assert_eq!(ok["task"]["status"]["state"], json!("TASK_STATE_COMPLETED"));
    assert_eq!(w.conv.asks()[0].0.chain, ["c_1/1", "bob/fixer", "c_2/4", "carol/reviewer"]);
    // Another owner's share of the same name isn't this one.
    let other = send("bob/api-expert");
    assert_eq!(other["task"]["status"]["state"], json!("TASK_STATE_COMPLETED"));

    let res =
        mcp(&*bob, &[("Pitwall-Chain", "alice/api-expert")], call_tool(1, "ask_api_expert", json!({"message": "hi"})));
    assert_eq!(res["result"]["isError"], json!(true));
    assert_eq!(res["result"]["structuredContent"]["refused"], json!("loop"));
    assert_eq!(w.conv.asks().len(), 2);
}

// ---------------------------------------------------------------- MCP

fn mcp(http: &dyn HttpClient, headers: &[(&str, &str)], body: Value) -> Value {
    let mut h = vec![("Accept", "application/json, text/event-stream")];
    h.extend_from_slice(headers);
    let res = call(http, "POST", "/mcp", &h, Some(body));
    if res.header("content-type").as_deref() == Some("text/event-stream") {
        let text = res.text();
        let last = SseReader::new(Body::from(text)).last().unwrap().unwrap();
        return serde_json::from_str(&last.data).unwrap();
    }
    res.json()
}

fn call_tool(id: u64, name: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}})
}

fn initialize(http: &dyn HttpClient, version: &str) -> (Value, String) {
    let res = call(
        http,
        "POST",
        "/mcp",
        &[("Accept", "application/json, text/event-stream")],
        Some(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": version, "capabilities": {}, "clientInfo": {"name": "claude-code", "version": "2.1.301"}
        }})),
    );
    assert_eq!(res.status, StatusCode::OK);
    let session = res.header("mcp-session-id").expect("a session id");
    (res.json(), session)
}

#[test]
fn mcp_initialize_negotiates_the_version() {
    let w = world();
    let bob = w.http(&w.bob);
    let (init, _) = initialize(&*bob, "2025-06-18");
    golden("mcp_initialize.json", &init);
    let (init, _) = initialize(&*bob, "1999-01-01");
    assert_eq!(init["result"]["protocolVersion"], json!("2025-11-25"));
    let (init, _) = initialize(&*bob, "2025-03-26");
    assert_eq!(init["result"]["protocolVersion"], json!("2025-03-26"));
    let notified =
        call(&*bob, "POST", "/mcp", &[], Some(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})));
    assert_eq!(notified.status, StatusCode::ACCEPTED);
}

#[test]
fn mcp_tools_list_is_the_callers_directory() {
    let w = world();
    let (_, session) = initialize(&*w.http(&w.bob), "2025-11-25");
    let list = mcp(
        &*w.http(&w.bob),
        &[("Mcp-Session-Id", &session), ("MCP-Protocol-Version", "2025-11-25")],
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    );
    golden("mcp_tools_list.json", &list);
    let none = mcp(&*w.http(&w.erin), &[], json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    assert_eq!(none["result"]["tools"], json!([]), "outside every audience: nothing, not even the helpers");
}

#[test]
fn mcp_unknown_tool_and_not_in_audience_look_the_same() {
    let w = world();
    let bob = w.http(&w.bob);
    let secret = mcp(&*bob, &[], call_tool(5, "ask_secret", json!({"message": "hi"})));
    let ghost = mcp(&*bob, &[], call_tool(5, "ask_ghost", json!({"message": "hi"})));
    let unify = |v: &Value| v.to_string().replace("ask_secret", "X").replace("ask_ghost", "X");
    assert_eq!(unify(&secret), unify(&ghost));
    golden("mcp_unknown_tool.json", &ghost);
    // Erin has no shares: even the helpers are unknown to her.
    let helper = mcp(&*w.http(&w.erin), &[], call_tool(5, "check_turn", json!({"turn": "c_1/1"})));
    assert_eq!(helper["error"]["code"], json!(-32602));
    assert!(w.conv.asks().is_empty());
}

#[test]
fn mcp_ask_returns_the_reply_as_structured_content_and_text() {
    let w = world();
    w.conv.on_ask(OnAsk::Reply("Refunds older than 30 days go through the manual queue.".into()));
    let (_, session) = initialize(&*w.http(&w.bob), "2025-06-18");
    let res = mcp(
        &*w.http(&w.bob),
        &[("Mcp-Session-Id", &session)],
        call_tool(3, "ask_api_expert", json!({"message": "How are old refunds handled?"})),
    );
    golden("mcp_call_done.json", &res);
    let (asker, ask) = &w.conv.asks()[0];
    assert_eq!(asker.front, Front::Mcp);
    assert_eq!(asker.client.as_deref(), Some("claude-code"), "the client name from initialize");
    assert_eq!(ask.message, "How are old refunds handled?");
    assert!(ask.conversation.is_none());
}

#[test]
fn mcp_refusals_are_error_results_with_the_reason() {
    let w = world();
    w.conv.on_ask(OnAsk::Refuse(Refusal {
        reason: RefusalReason::Quota,
        message: "Daily limit reached for alice/api-expert; resets at 00:00".into(),
        resets_at: Some("2026-10-17T00:00:00+02:00".into()),
        until: None,
    }));
    let res = mcp(&*w.http(&w.bob), &[], call_tool(4, "ask_api_expert", json!({"message": "more?"})));
    golden("mcp_call_refused.json", &res);
}

#[test]
fn mcp_waits_then_check_turn_picks_it_up() {
    let w = world();
    w.conv.on_ask(OnAsk::Queue(2));
    let bob = w.http(&w.bob);
    let res = mcp(&*bob, &[], call_tool(1, "ask_payments_expert", json!({"message": "go", "wait_seconds": 0})));
    let s = &res["result"]["structuredContent"];
    assert_eq!((s["status"].clone(), s["turn"].clone()), (json!("working"), json!("c_1/1")));
    assert_eq!(s["statusMessage"], json!("2nd in line"));
    assert_eq!(res["result"]["isError"], json!(false));

    let id = TurnId("c_1/1".into());
    w.conv.update(&id, |t| t.status = TurnStatus::WaitingForOwner);
    let res = mcp(&*bob, &[], call_tool(2, "check_turn", json!({"turn": "c_1/1", "wait_seconds": 0})));
    assert_eq!(res["result"]["structuredContent"]["status"], json!("waiting_for_owner"));

    // A waiting check_turn returns as soon as the turn ends.
    let conv = w.conv.clone();
    let t = thread::spawn(move || {
        until_watched(&conv, &id);
        conv.update(&id, done_with_reply("All set."));
    });
    let res = mcp(&*bob, &[], call_tool(3, "check_turn", json!({"turn": "c_1/1", "wait_seconds": 240})));
    t.join().unwrap();
    assert_eq!(res["result"]["structuredContent"]["status"], json!("done"));
    assert_eq!(res["result"]["structuredContent"]["reply"], json!("All set."));

    let res = mcp(&*bob, &[], call_tool(4, "check_turn", json!({"turn": "c_99/1"})));
    assert_eq!(res["result"]["isError"], json!(true));
    let res = mcp(&*w.http(&w.erin), &[], call_tool(4, "check_turn", json!({"turn": "c_1/1"})));
    assert_eq!(res["error"]["code"], json!(-32602), "Erin sees no tools at all");
}

#[test]
fn mcp_progress_comes_over_sse_when_asked_for() {
    let w = world();
    w.conv.on_ask(OnAsk::Queue(2));
    let mut body = call_tool(7, "ask_payments_expert", json!({"message": "go"}));
    body["params"]["_meta"] = json!({"progressToken": "p-1"});
    let res = call(&*w.http(&w.bob), "POST", "/mcp", &[("Accept", "application/json, text/event-stream")], Some(body));
    assert_eq!(res.header("content-type").as_deref(), Some("text/event-stream"));
    let id = TurnId("c_1/1".into());
    let rx = events(res.body);
    let mut got = vec![next(&rx)];
    w.conv.update(&id, |t| t.status = TurnStatus::Working);
    got.push(next(&rx));
    w.conv.update(&id, |t| t.status = TurnStatus::WaitingForOwner);
    got.push(next(&rx));
    w.conv.update(&id, done_with_reply("Shipped."));
    got.push(next(&rx));
    assert!(rx.recv_timeout(Duration::from_secs(5)).is_err(), "the stream ends with the result");
    golden("mcp_call_progress.json", &Value::Array(got));
}

#[test]
fn mcp_patches_inline_up_to_40000_characters_then_in_parts() {
    let w = world();
    w.conv.on_ask(OnAsk::Work);
    let bob = w.http(&w.bob);
    mcp(&*bob, &[], call_tool(1, "ask_payments_expert", json!({"message": "go", "wait_seconds": 0})));
    let id = TurnId("c_1/1".into());
    w.conv.set_patch(&id, PATCH);
    w.conv.update(&id, |t| {
        done_with_reply("Added the flag.")(t);
        t.changes = Some(sent_changes(1));
    });
    let res = mcp(&*bob, &[], call_tool(2, "check_turn", json!({"turn": "c_1/1"})));
    golden("mcp_call_with_patch.json", &res);

    let big: String = format!("{PATCH}{}", "x".repeat(super::super::CHANGES_PART_CHARS + 10));
    w.conv.set_patch(&id, &big);
    let res = mcp(&*bob, &[], call_tool(3, "check_turn", json!({"turn": "c_1/1"})));
    let changes = &res["result"]["structuredContent"]["changes"];
    assert!(changes.get("patch").is_none());
    assert_eq!(changes["parts"], json!(2));
    let text = res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("get_changes") && !text.contains("xxxxxxxx"));
    let mut joined = String::new();
    for part in 1..=2 {
        let res = mcp(&*bob, &[], call_tool(4, "get_changes", json!({"turn": "c_1/1", "part": part})));
        let s = &res["result"]["structuredContent"];
        assert_eq!((s["part"].clone(), s["parts"].clone()), (json!(part), json!(2)));
        joined.push_str(s["patch"].as_str().unwrap());
    }
    assert_eq!(joined, big);
    let res = mcp(&*bob, &[], call_tool(5, "get_changes", json!({"turn": "c_1/1", "part": 3})));
    assert_eq!(res["result"]["isError"], json!(true));
}

#[test]
fn mcp_notifications_cancelled_stops_the_turn() {
    let w = world();
    w.conv.on_ask(OnAsk::Work);
    let (_, session) = initialize(&*w.http(&w.bob), "2025-11-25");
    let http = w.http(&w.bob);
    let s2 = session.clone();
    let waiting = thread::spawn(move || {
        mcp(
            &*http,
            &[("Mcp-Session-Id", &s2)],
            call_tool(42, "ask_api_expert", json!({"message": "slow", "wait_seconds": 240})),
        )
    });
    let id = TurnId("c_1/1".into());
    let start = Instant::now();
    while w.conv.asks().is_empty() {
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(2));
    }
    until_watched(&w.conv, &id);
    // Another session's cancel doesn't touch it.
    let other = call(
        &*w.http(&w.bob),
        "POST",
        "/mcp",
        &[],
        Some(json!({
            "jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 41}
        })),
    );
    assert_eq!(other.status, StatusCode::ACCEPTED);
    assert!(w.conv.canceled().is_empty());
    let res = call(
        &*w.http(&w.bob),
        "POST",
        "/mcp",
        &[("Mcp-Session-Id", &session)],
        Some(json!({
            "jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 42, "reason": "user"}
        })),
    );
    assert_eq!(res.status, StatusCode::ACCEPTED);
    let answer = waiting.join().unwrap();
    assert_eq!(w.conv.canceled(), vec![id]);
    assert_eq!(answer["result"]["structuredContent"]["failure"], json!("canceled"));
}

#[test]
fn mcp_sessions_are_transport_only_and_per_person() {
    let w = world();
    w.conv.on_ask(OnAsk::Reply("x".into()));
    let bob = w.http(&w.bob);
    let (_, s1) = initialize(&*bob, "2025-11-25");
    let unknown = call(
        &*bob,
        "POST",
        "/mcp",
        &[("Mcp-Session-Id", "nope")],
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})),
    );
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    // Bob's session id is nothing to Erin.
    let stolen = call(
        &*w.http(&w.erin),
        "POST",
        "/mcp",
        &[("Mcp-Session-Id", &s1)],
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})),
    );
    assert_eq!(stolen.status, StatusCode::NOT_FOUND);
    // A new session continues the same conversation.
    let a = mcp(&*bob, &[("Mcp-Session-Id", &s1)], call_tool(1, "ask_api_expert", json!({"message": "one"})));
    let (_, s2) = initialize(&*bob, "2025-11-25");
    let b = mcp(&*bob, &[("Mcp-Session-Id", &s2)], call_tool(1, "ask_api_expert", json!({"message": "two"})));
    assert_eq!(a["result"]["structuredContent"]["conversation"], b["result"]["structuredContent"]["conversation"]);
    let deleted = call(&*bob, "DELETE", "/mcp", &[("Mcp-Session-Id", &s1)], None);
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let after = call(
        &*bob,
        "POST",
        "/mcp",
        &[("Mcp-Session-Id", &s1)],
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})),
    );
    assert_eq!(after.status, StatusCode::NOT_FOUND);
}

fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "ExampleClient", "version": "1.0.0"},
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

#[test]
fn mcp_modern_requests_carry_their_version_and_matching_headers() {
    let w = world();
    let bob = w.http(&w.bob);
    let accept = ("Accept", "application/json, text/event-stream");
    let discover =
        json!({"jsonrpc": "2.0", "id": "d", "method": "server/discover", "params": {"_meta": modern_meta()}});
    let res = call(
        &*bob,
        "POST",
        "/mcp",
        &[accept, ("MCP-Protocol-Version", "2026-07-28"), ("Mcp-Method", "server/discover")],
        Some(discover.clone()),
    );
    assert_eq!(res.status, StatusCode::OK);
    golden("mcp_discover.json", &res.json());

    let res = call(&*bob, "POST", "/mcp", &[accept, ("MCP-Protocol-Version", "2026-07-28")], Some(discover.clone()));
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.json()["error"]["code"], json!(-32020));

    let mut old = discover.clone();
    old["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("2030-01-01");
    let res = call(
        &*bob,
        "POST",
        "/mcp",
        &[accept, ("MCP-Protocol-Version", "2030-01-01"), ("Mcp-Method", "server/discover")],
        Some(old),
    );
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    golden("mcp_unsupported_version.json", &res.json());

    let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {"_meta": modern_meta()}});
    let res = call(
        &*bob,
        "POST",
        "/mcp",
        &[accept, ("MCP-Protocol-Version", "2026-07-28"), ("Mcp-Method", "tools/list")],
        Some(list),
    );
    let res = res.json();
    assert_eq!(res["result"]["resultType"], json!("complete"));
    assert_eq!(
        (res["result"]["ttlMs"].clone(), res["result"]["cacheScope"].clone()),
        (json!(30_000), json!("private"))
    );
    assert_eq!(res["result"]["tools"].as_array().unwrap().len(), 4);

    // Mcp-Name must match, base64 sentinel allowed.
    w.conv.on_ask(OnAsk::Reply("hello".into()));
    let mut body = call_tool(3, "ask_api_expert", json!({"message": "hi"}));
    body["params"]["_meta"] = modern_meta();
    let h = [accept, ("MCP-Protocol-Version", "2026-07-28"), ("Mcp-Method", "tools/call")];
    let res = call(&*bob, "POST", "/mcp", &h, Some(body.clone()));
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "Mcp-Name missing");
    let encoded = format!("=?base64?{}?=", wire::b64_encode(b"ask_api_expert"));
    let res = mcp(&*bob, &[h[1], h[2], ("Mcp-Name", &encoded)], body);
    assert_eq!(res["result"]["structuredContent"]["reply"], json!("hello"));
    assert_eq!(res["result"]["resultType"], json!("complete"));
    assert_eq!(w.conv.asks()[0].0.client.as_deref(), Some("ExampleClient"));

    // Unknown method: 404 with a JSON-RPC error in the modern era.
    let other = json!({"jsonrpc": "2.0", "id": 9, "method": "prompts/list", "params": {"_meta": modern_meta()}});
    let res = call(
        &*bob,
        "POST",
        "/mcp",
        &[accept, ("MCP-Protocol-Version", "2026-07-28"), ("Mcp-Method", "prompts/list")],
        Some(other),
    );
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.json()["error"]["code"], json!(-32601));
}

#[test]
fn mcp_modern_closing_the_stream_cancels_the_turn() {
    let w = world_with(Options { heartbeat: Duration::from_millis(20), ..options() });
    w.conv.on_ask(OnAsk::Work);
    let mut body = call_tool(3, "ask_api_expert", json!({"message": "slow"}));
    body["params"]["_meta"] = modern_meta();
    let res = call(
        &*w.http(&w.bob),
        "POST",
        "/mcp",
        &[
            ("Accept", "application/json, text/event-stream"),
            ("MCP-Protocol-Version", "2026-07-28"),
            ("Mcp-Method", "tools/call"),
            ("Mcp-Name", "ask_api_expert"),
        ],
        Some(body),
    );
    assert_eq!(res.header("content-type").as_deref(), Some("text/event-stream"));
    let id = TurnId("c_1/1".into());
    until_watched(&w.conv, &id);
    drop(res.body); // the client goes away
    let start = Instant::now();
    while w.conv.canceled().is_empty() {
        assert!(start.elapsed() < Duration::from_secs(5), "not canceled");
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(w.conv.canceled(), vec![id]);
}

#[test]
fn mcp_list_changed_only_when_the_callers_list_changes() {
    let w = world_with(Options { heartbeat: Duration::from_millis(20), ..options() });
    let bob = w.http(&w.bob);
    // Legacy: the GET stream.
    let res = call(&*bob, "GET", "/mcp", &[("Accept", "text/event-stream")], None);
    assert_eq!(res.status, StatusCode::OK);
    let legacy = events(res.body);
    // Modern: subscriptions/listen.
    let listen = json!({"jsonrpc": "2.0", "id": 11, "method": "subscriptions/listen",
        "params": {"_meta": modern_meta(), "notifications": {"toolsListChanged": true}}});
    let res = call(
        &*bob,
        "POST",
        "/mcp",
        &[
            ("Accept", "application/json, text/event-stream"),
            ("MCP-Protocol-Version", "2026-07-28"),
            ("Mcp-Method", "subscriptions/listen"),
        ],
        Some(listen),
    );
    let modern = events(res.body);
    let ack = next(&modern);
    golden("mcp_listen_ack.json", &ack);
    thread::sleep(Duration::from_millis(50)); // both streams have their first look

    // Someone else's share changes: nothing for Bob.
    w.conv.share(
        card(&w.net.person("alice@example.com"), "ops", "Ops", "x", Capability::Read),
        &[&w.net.person("carol@example.com")],
    );
    w.conv.tick();
    assert!(legacy.recv_timeout(Duration::from_millis(100)).is_err());
    // A share for Bob goes live.
    w.conv.share(
        card(&w.net.person("alice@example.com"), "docs", "Docs", "x", Capability::Read),
        &[&w.net.person("bob@example.com")],
    );
    assert_eq!(next(&legacy), json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}));
    golden("mcp_listen_changed.json", &next(&modern));
}

#[test]
fn mcp_get_without_event_stream_and_protected_resource_metadata() {
    let w = world();
    let bob = w.http(&w.bob);
    assert_eq!(call(&*bob, "GET", "/mcp", &[], None).status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        call(&*bob, "GET", "/.well-known/oauth-protected-resource/mcp", &[], None).status,
        StatusCode::NOT_FOUND
    );
    w.alice.set_client_auth(ClientAuth::OAuth {
        issuer: "https://auth.pitwall.example".into(),
        resource: "https://studio.acme.pitwall.example/mcp".into(),
        scopes: vec!["collab".into()],
    });
    let res = call(&*bob, "GET", "/.well-known/oauth-protected-resource/mcp", &[], None);
    assert_eq!(res.status, StatusCode::OK);
    golden("mcp_protected_resource.json", &res.json());
    let header = super::mcp::www_authenticate(&w.alice.endpoint()).unwrap();
    assert_eq!(
        header,
        "Bearer resource_metadata=\"http://studio.example.fake/.well-known/oauth-protected-resource/mcp\", scope=\"collab\""
    );
}

// ------------------------------------------------- across both fronts

#[test]
fn one_conversation_across_both_fronts() {
    let w = world();
    w.conv.on_ask(OnAsk::Reply("ok".into()));
    let bob = w.http(&w.bob);
    let a2a = call(&*bob, "POST", "/a2a/api-expert/message:send", &[V], Some(a2a_message("from Pitwall"))).json();
    let context = a2a["task"]["contextId"].clone();
    let mcp = mcp(&*bob, &[], call_tool(1, "ask_api_expert", json!({"message": "from plain Claude"})));
    assert_eq!(mcp["result"]["structuredContent"]["conversation"], context);
    let turn = mcp["result"]["structuredContent"]["turn"].as_str().unwrap().to_string();
    // The MCP turn is an A2A task in the same context.
    let task = call(&*bob, "GET", &format!("/a2a/api-expert/tasks/{turn}"), &[V], None).json();
    assert_eq!(task["contextId"], context);
    let list =
        call(&*bob, "GET", &format!("/a2a/api-expert/tasks?contextId={}", context.as_str().unwrap()), &[V], None)
            .json();
    assert_eq!(list["totalSize"], json!(2));
}

// --------------------------------------------------------- the handler

#[test]
fn other_origins_are_refused() {
    let w = world();
    let bob = w.http(&w.bob);
    let res = call(&*bob, "GET", "/a2a/api-expert/card", &[("Origin", "https://evil.example")], None);
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = call(
        &*bob,
        "POST",
        "/mcp",
        &[("Origin", "http://localhost:3000")],
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})),
    );
    assert_eq!(res.status, StatusCode::FORBIDDEN);
    let res = call(&*bob, "GET", "/a2a/api-expert/card", &[("Origin", "http://studio.example.fake")], None);
    assert_eq!(res.status, StatusCode::OK);
}

#[test]
fn bodies_over_2_mib_are_refused() {
    let w = world();
    let bob = w.http(&w.bob);
    let big = json!({"message": {"messageId": "m", "role": "ROLE_USER", "parts": [{"text": "x".repeat(crate::collab::MAX_BODY)}]}});
    assert_eq!(
        call(&*bob, "POST", "/a2a/api-expert/message:send", &[V], Some(big)).status,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let big = call_tool(1, "ask_api_expert", json!({"message": "x".repeat(crate::collab::MAX_BODY)}));
    assert_eq!(call(&*bob, "POST", "/mcp", &[], Some(big)).status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(w.conv.asks().is_empty());
}

#[test]
fn requests_per_second_and_open_streams_are_limited_per_person() {
    let w = world_with(Options { requests_per_second: 1, burst: 3, ..options() });
    let bob = w.http(&w.bob);
    let statuses: Vec<u16> =
        (0..4).map(|_| call(&*bob, "GET", "/pitwall/v1/info", &[], None).status.as_u16()).collect();
    assert_eq!(statuses, [200, 200, 200, 429]);
    // Erin has her own budget.
    assert_eq!(call(&*w.http(&w.erin), "GET", "/pitwall/v1/info", &[], None).status, StatusCode::OK);

    let w = world();
    w.conv.on_ask(OnAsk::Work);
    let bob = w.http(&w.bob);
    let mut msg = a2a_message("x");
    msg["configuration"] = json!({"returnImmediately": true});
    call(&*bob, "POST", "/a2a/api-expert/message:send", &[V], Some(msg));
    let streams: Vec<Answer> = (0..crate::collab::MAX_STREAMS)
        .map(|_| call(&*bob, "POST", "/a2a/api-expert/tasks/c_1/1:subscribe", &[V], None))
        .collect();
    assert!(streams.iter().all(|s| s.status == StatusCode::OK));
    let over = call(&*bob, "POST", "/a2a/api-expert/tasks/c_1/1:subscribe", &[V], None);
    assert_eq!(over.status, StatusCode::TOO_MANY_REQUESTS);
    // Closing streams frees their places once their threads notice.
    w.conv.update(&TurnId("c_1/1".into()), done_with_reply("done"));
    drop(streams);
    let start = Instant::now();
    loop {
        let again = call(&*bob, "POST", "/a2a/api-expert/tasks/c_1/1:subscribe", &[V], None);
        if again.status == StatusCode::OK {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "streams never freed");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn unknown_routes_are_404() {
    let w = world();
    let bob = w.http(&w.bob);
    for path in ["/", "/a2a", "/a2a/api-expert/nope", "/pitwall/v1/nope", "/.well-known/agent-card.json"] {
        assert_eq!(call(&*bob, "GET", path, &[V], None).status, StatusCode::NOT_FOUND, "{path}");
    }
}

// ------------------------------------------------- Pitwall endpoints

#[test]
fn pitwall_endpoints() {
    let w = world();
    let bob = w.http(&w.bob);
    let erin = w.http(&w.erin);
    golden("pitwall_info.json", &call(&*bob, "GET", "/pitwall/v1/info", &[], None).json());
    assert_eq!(call(&*erin, "GET", "/pitwall/v1/info", &[], None).status, StatusCode::OK);
    assert_eq!(call(&*erin, "GET", "/pitwall/v1/teams:discover", &[], None).json(), json!({"records": []}));

    let dir = call(&*bob, "GET", "/pitwall/v1/directory", &[], None).json();
    let slugs: Vec<&str> = dir["agents"].as_array().unwrap().iter().map(|a| a["slug"].as_str().unwrap()).collect();
    assert_eq!(slugs, ["api-expert", "payments-expert"]);
    golden("pitwall_directory_entry.json", &dir["agents"][0]);
    assert_eq!(call(&*erin, "GET", "/pitwall/v1/directory", &[], None).json(), json!({"agents": []}));

    // Join: anyone; the identity is the network's.
    let join = json!({"note": "hi", "device": {"key": "ed25519:E1", "addr": []}});
    let res = call(&*erin, "POST", "/pitwall/v1/teams/t_9f2c/join", &[], Some(join));
    assert_eq!(res.status, StatusCode::ACCEPTED);
    assert_eq!(w.teams.joins.lock().unwrap()[0].0.subject, "erin@example.com");
    let bad = call(
        &*erin,
        "POST",
        "/pitwall/v1/teams/t_9f2c/invites:redeem",
        &[],
        Some(json!({"code": "bad", "device": {"key": "k", "addr": []}})),
    );
    assert_eq!(bad.status, StatusCode::NOT_FOUND);
    let good = call(
        &*erin,
        "POST",
        "/pitwall/v1/teams/t_9f2c/invites:redeem",
        &[],
        Some(json!({"code": "good", "device": {"key": "k", "addr": []}})),
    );
    assert_eq!(good.status, StatusCode::OK);

    // Member devices only.
    let head = json!({"team": "t_9f2c", "version": 7, "hash": "sha256:aa"});
    let res = call(&*bob, "POST", "/pitwall/v1/teams/t_9f2c/heads", &[], Some(head.clone()));
    assert_eq!(res.json(), json!({"heads": [{"team": "t_9f2c", "version": 8, "hash": "sha256:aa"}]}));
    assert_eq!(
        call(&*erin, "POST", "/pitwall/v1/teams/t_9f2c/heads", &[], Some(head.clone())).status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&*bob, "POST", "/pitwall/v1/teams/t_other/heads", &[], Some(head.clone())).status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(&*bob, "GET", "/pitwall/v1/teams/t_9f2c/records?after=3", &[], None).json(),
        json!({"records": []})
    );
    assert_eq!(call(&*erin, "GET", "/pitwall/v1/teams/t_9f2c/records", &[], None).status, StatusCode::NOT_FOUND);
    assert_eq!(call(&*bob, "POST", "/pitwall/v1/teams/t_9f2c/announce", &[], Some(head)).status, StatusCode::ACCEPTED);
    assert_eq!(call(&*bob, "GET", "/pitwall/v1/teams/t_9f2c/requests", &[], None).json(), json!({"requests": []}));

    // Sessions: mine only.
    w.conv.on_ask(OnAsk::Reply("x".into()));
    call(&*bob, "POST", "/a2a/api-expert/message:send", &[V], Some(a2a_message("hi")));
    let sessions = call(&*bob, "GET", "/pitwall/v1/sessions", &[], None).json();
    assert_eq!(
        sessions,
        json!({"sessions": [{"id": "c_1", "share": "api-expert", "turns": 1, "current": true, "lastAt": AT}]})
    );
    assert_eq!(call(&*erin, "GET", "/pitwall/v1/sessions", &[], None).json(), json!({"sessions": []}));
    assert_eq!(call(&*erin, "DELETE", "/pitwall/v1/sessions/c_1", &[], None).status, StatusCode::NOT_FOUND);
    assert_eq!(call(&*bob, "DELETE", "/pitwall/v1/sessions/c_1", &[], None).status, StatusCode::NO_CONTENT);
    assert_eq!(call(&*bob, "GET", "/pitwall/v1/sessions", &[], None).json(), json!({"sessions": []}));
}

#[test]
fn pitwall_events_stream_announcements_and_directory_changes() {
    let w = world_with(Options { heartbeat: Duration::from_millis(20), ..options() });
    assert_eq!(call(&*w.http(&w.erin), "GET", "/pitwall/v1/events", &[], None).status, StatusCode::NOT_FOUND);
    let res = call(&*w.http(&w.bob), "GET", "/pitwall/v1/events", &[], None);
    assert_eq!(res.header("content-type").as_deref(), Some("text/event-stream"));
    let (tx, rx) = mpsc::channel();
    let body = res.body;
    thread::spawn(move || {
        for e in SseReader::new(body).flatten() {
            if tx.send(e).is_err() {
                return;
            }
        }
    });
    thread::sleep(Duration::from_millis(50));
    w.teams.announce_head(TeamHead { team: TeamId("t_9f2c".into()), version: 8, hash: "sha256:bb".into() });
    let e = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(e.event, "team.announce");
    assert_eq!(
        serde_json::from_str::<Value>(&e.data).unwrap(),
        json!({"team": "t_9f2c", "version": 8, "hash": "sha256:bb"})
    );
    w.conv.tick(); // nothing for Bob
    w.conv.unshare("payments-expert");
    let e = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!((e.event.as_str(), e.data.as_str()), ("directory.changed", "{}"));
}

// ------------------------------------------------------------ client

#[test]
fn the_client_speaks_a2a_to_another_pitwall() {
    let w = world();
    let owner =
        w.pitwall(&w.bob).with_labels(Labels { agent: Some("client-fix@bob-mbp".into()), chain: vec!["c_5/2".into()] });
    let info = owner.info().unwrap();
    assert_eq!(info.person.subject, "alice@example.com");
    let dir = owner.directory().unwrap();
    assert_eq!(dir.len(), 2);
    assert_eq!(dir[0].card_url, "http://studio.example.fake/a2a/api-expert/card");
    assert_eq!(owner.agent("api-expert").card().unwrap(), dir[0].card);

    w.conv.on_ask(OnAsk::Reply("It rotates in auth/refresh.rs.".into()));
    let task = owner.agent("api-expert").send(&Outgoing::text("where are refresh tokens rotated?")).unwrap();
    let turn = client::turn_of(&task).unwrap();
    assert_eq!(turn, w.conv.turn_now(&TurnId("c_1/1".into())), "the owner's turn, without loss");
    let (asker, _) = &w.conv.asks()[0];
    assert_eq!(asker.agent.as_deref(), Some("client-fix@bob-mbp"));
    assert_eq!(asker.chain, ["c_5/2"]);

    // Redo on my code, in the same conversation.
    let msg = Outgoing {
        text: "redo on mine".into(),
        conversation: Some(turn.conversation.clone()),
        my_changes: Some(PATCH.into()),
        ..Outgoing::default()
    };
    let task = owner.agent("api-expert").send(&msg).unwrap();
    assert_eq!(task.context_id, "c_1");
    assert_eq!(w.conv.asks()[1].1.my_changes.as_deref(), Some(PATCH));

    // Streaming, get, list, cancel, subscribe.
    w.conv.on_ask(OnAsk::Work);
    let mut stream = owner.agent("api-expert").stream(&Outgoing::text("slow one")).unwrap();
    let first = stream.next().unwrap().unwrap();
    let id = TurnId(first.task.unwrap().id);
    until_watched(&w.conv, &id);
    w.conv.update(&id, done_with_reply("finally"));
    let rest: Vec<StreamResponse> = stream.map(Result::unwrap).collect();
    assert!(rest.iter().any(|e| e.artifact_update.as_ref().is_some_and(|a| a.artifact.artifact_id == "reply")));
    let last = rest.last().unwrap().status_update.as_ref().unwrap();
    assert_eq!(last.status.state, TaskState::Completed);
    assert_eq!(client::turn_in(last.metadata.as_ref()).unwrap().status, TurnStatus::Done);
    assert_eq!(owner.agent("api-expert").get(&id).unwrap().status.state, TaskState::Completed);
    assert_eq!(owner.agent("api-expert").list(Some(&ConversationId("c_1".into()))).unwrap().len(), 3);

    let task = owner.agent("api-expert").send(&Outgoing { return_immediately: true, ..Outgoing::text("x") }).unwrap();
    let id = TurnId(task.id.clone());
    let mut sub = owner.agent("api-expert").subscribe(&id).unwrap();
    assert_eq!(sub.next().unwrap().unwrap().task.unwrap().status.state, TaskState::Working);
    assert_eq!(owner.agent("api-expert").cancel(&id).unwrap().status.state, TaskState::Canceled);
    assert_eq!(sub.next().unwrap().unwrap().status_update.unwrap().status.state, TaskState::Canceled);
    assert!(owner.agent("api-expert").cancel(&id).is_err());

    // Sessions.
    assert_eq!(owner.sessions().unwrap().len(), 1);
    owner.forget(&ConversationId("c_1".into())).unwrap();
    assert!(owner.sessions().unwrap().is_empty());

    // Not in the audience is NotFound, like no such share.
    let erin = w.pitwall(&w.erin);
    assert!(erin.agent("api-expert").card().unwrap_err().is(ErrorCode::NotFound));
    assert!(erin.agent("api-expert").send(&Outgoing::text("x")).unwrap_err().is(ErrorCode::NotFound));
    assert!(owner.agent("nope").card().unwrap_err().is(ErrorCode::NotFound));
}

#[test]
fn the_client_reads_team_events() {
    let w = world_with(Options { heartbeat: Duration::from_millis(20), ..options() });
    let owner = w.pitwall(&w.bob);
    let mut events = owner.events().unwrap();
    let teams = w.teams.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        teams.announce_head(TeamHead { team: TeamId("t_9f2c".into()), version: 9, hash: "h".into() });
    });
    assert_eq!(
        events.next().unwrap().unwrap(),
        TeamEvent::Announce(TeamHead { team: TeamId("t_9f2c".into()), version: 9, hash: "h".into() })
    );
    assert!(w.pitwall(&w.erin).events().is_err());
}

// ------------------------------------------------------------- wire

#[test]
fn sse_framing() {
    assert_eq!(wire::frame(None, "{\"a\":1}"), b"data: {\"a\":1}\n\n");
    assert_eq!(wire::frame(Some("team.announce"), "a\nb"), b"event: team.announce\ndata: a\ndata: b\n\n");
    let stream = ": keep-alive\n\nevent: x\r\ndata: one\r\ndata:two\r\n\r\nid: 3\ndata: {}\n\ndata: dangling";
    let got: Vec<_> = SseReader::new(Body::from(stream)).map(Result::unwrap).collect();
    assert_eq!(
        got,
        [
            wire::SseEvent { event: "x".into(), data: "one\ntwo".into() },
            wire::SseEvent { event: "message".into(), data: "{}".into() },
        ]
    );
}

#[test]
fn base64_percent_and_queries() {
    for s in ["", "f", "fo", "foo", "foob", "fooba", "foobar", "Hello, 世界"] {
        assert_eq!(wire::b64_decode(&wire::b64_encode(s.as_bytes())).unwrap(), s.as_bytes());
    }
    assert_eq!(wire::b64_encode(b"foobar"), "Zm9vYmFy");
    assert_eq!(wire::b64_decode("Zm9vYg").unwrap(), b"foob", "padding optional");
    assert_eq!(wire::b64_decode("_-8").unwrap(), wire::b64_decode("/+8").unwrap(), "URL-safe too");
    assert!(wire::b64_decode("no!").is_none());
    assert_eq!(wire::percent_encode("c_8f2k/7"), "c_8f2k%2F7");
    assert_eq!(wire::percent_decode("c_8f2k%2f7").unwrap(), "c_8f2k/7");
    assert_eq!(wire::percent_decode("50%").unwrap(), "50%");
    assert_eq!(wire::query_get(Some("a=1&contextId=c%5F1&b"), "contextId").as_deref(), Some("c_1"));
    assert_eq!(wire::ordinal(2), "2nd");
    assert_eq!(wire::ordinal(12), "12th");
    assert_eq!(wire::ordinal(23), "23rd");
}

#[test]
fn unknown_task_states_read_as_unspecified() {
    let s: TaskState = serde_json::from_value(json!("TASK_STATE_SOMETHING_NEW")).unwrap();
    assert_eq!(s, TaskState::Unspecified);
    let r: Role = serde_json::from_value(json!("ROLE_AGENT")).unwrap();
    assert_eq!(r, Role::Agent);
    assert_eq!(super::mcp::tool_name("payments-expert"), "ask_payments_expert");
}

// ------------------------------------------------- interop (ignored)

/// A minimal HTTP/1.1 server for the interop tests only (the real one is
/// the network backend's): every connection is Bob, as the Fake network
/// proves him; streaming bodies go out chunked.
/// The black-box clients (`collab::interop`) use it against the integrated
/// core; the listener is bound already, so the fronts can name its address
/// in Agent Cards.
pub(crate) fn serve_locally_on(listener: std::net::TcpListener, fronts: Fronts, caller: crate::net::Caller) -> (u16, Arc<std::sync::Mutex<Vec<String>>>) {
    use std::io::{BufRead, BufReader, Read, Write};
    let port = listener.local_addr().unwrap().port();
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = log.clone();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (fronts, caller, log) = (fronts.clone(), caller.clone(), seen.clone());
            thread::spawn(move || {
                let mut out = stream.try_clone().unwrap();
                let mut reader = BufReader::new(stream);
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = line.split_whitespace();
                    let (method, path) =
                        (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("/").to_string());
                    let mut b = crate::net::http::Request::builder().method(method.as_str()).uri(path.as_str());
                    let mut length = 0usize;
                    loop {
                        let mut h = String::new();
                        reader.read_line(&mut h).unwrap();
                        let h = h.trim_end();
                        if h.is_empty() {
                            break;
                        }
                        if let Some((k, v)) = h.split_once(':') {
                            if k.eq_ignore_ascii_case("content-length") {
                                length = v.trim().parse().unwrap_or(0);
                            }
                            b = b.header(k.trim(), v.trim());
                        }
                    }
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    let req = b.body(Body::Bytes(body.clone())).unwrap();
                    let h = |n: &str| req.headers().get(n).and_then(|v| v.to_str().ok()).unwrap_or("-").to_string();
                    log.lock().unwrap().push(format!(
                        "{method} {path} version={} method={} session={} body={}",
                        h("mcp-protocol-version"),
                        h("mcp-method"),
                        h("mcp-session-id") != "-",
                        String::from_utf8_lossy(&body).chars().take(160).collect::<String>()
                    ));
                    let res = crate::net::Handler::handle(&fronts, req, caller.clone());
                    let (parts, body) = res.into_parts();
                    let mut head = format!("HTTP/1.1 {}\r\n", parts.status);
                    for (k, v) in &parts.headers {
                        head.push_str(&format!("{k}: {}\r\n", v.to_str().unwrap()));
                    }
                    match body {
                        Body::Stream(mut r) => {
                            head.push_str("transfer-encoding: chunked\r\n\r\n");
                            if out.write_all(head.as_bytes()).is_err() {
                                return;
                            }
                            let mut buf = [0u8; 8192];
                            loop {
                                let n = r.read(&mut buf).unwrap_or(0);
                                if n == 0 {
                                    let _ = out.write_all(b"0\r\n\r\n");
                                    break;
                                }
                                let chunk = [format!("{n:x}\r\n").as_bytes(), &buf[..n], b"\r\n"].concat();
                                if out.write_all(&chunk).and_then(|_| out.flush()).is_err() {
                                    return;
                                }
                            }
                        }
                        other => {
                            let bytes = other.read_all(usize::MAX).unwrap();
                            log.lock().unwrap().push(format!(
                                "  -> {} {}",
                                parts.status,
                                String::from_utf8_lossy(&bytes).chars().take(600).collect::<String>()
                            ));
                            head.push_str(&format!("content-length: {}\r\n\r\n", bytes.len()));
                            let _ = out.write_all(head.as_bytes());
                            let _ = out.write_all(&bytes);
                        }
                    }
                    let _ = out.flush();
                }
            });
        }
    });
    (port, log)
}
