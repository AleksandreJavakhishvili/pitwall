//! The Agention tile in Pitwall's own tile runtime (the harness), against
//! a fake operator API (`fake`): set up, connected, the list by agent and
//! session, answering in the tile and in the floating control, replies,
//! Mark read and Archive, news through the wait, the sidebar's badges and
//! the toolbar's count, and what goes wrong (signed out, a missing scope,
//! not Agention, down and back). No request leaves the test.

mod fake;

use std::sync::Arc;
use std::time::Duration;

use fake::{AskSpec, Fake, SERVER};
use pitwall_extension_test::{agent, BadgeTone, ConnState, Harness, Session, ToolbarBadge, UiRequest};

/// How long a reply or an answer waits for Undo before it is sent.
const TAKE_BACK: Duration = Duration::from_secs(5);

fn dir() -> &'static str {
    env!("CARGO_MANIFEST_DIR")
}

/// The tile set up with `key`, granted, with Pitwall's agent `a1`
/// ("web", in /projects/web) and the fake as the server.
fn harness(fake: &Arc<Fake>, key: &str) -> Harness {
    Harness::build(dir()).setting("server", SERVER).secret("apiKey", key).grant("net").grant("agents.read").agent(agent("a1", "web")).serve(fake.server())
}

/// A thread from Pitwall's agent a1 (agent `web`, session `a1`) with a
/// blocking question.
fn asking(fake: &Fake) -> String {
    fake.add_thread("web", "a1", "Which database?", "I need to pick one for the cache.", vec![AskSpec::new("SQLite or Postgres?", &["Postgres", "SQLite"]).blocking()])
}

/// Whether the tile shows `words` anywhere: in a text, a button, a badge
/// or an empty state.
fn shows(t: &Session, words: &str) -> bool {
    t.text_snapshot().contains(words)
}

fn open(fake: &Arc<Fake>) -> Session {
    harness(fake, &fake::full()).open("inbox")
}

#[test]
fn not_set_up_says_what_to_do() {
    let mut t = Harness::build(dir()).open("inbox");
    assert!(shows(&t, "Set up Agention"), "{}", t.text_snapshot());
    assert!(shows(&t, "Set your Agention server's address in Settings."), "{}", t.text_snapshot());
    t.click_key("setup/0");
    assert!(t.ui_requests().iter().any(|r| matches!(r, UiRequest::OpenSettings)));
    assert!(t.requests().is_empty(), "nothing is sent");
    // An address, no key yet.
    t.set_setting("server", Some(SERVER));
    assert!(shows(&t, "Paste your Agention API key"), "{}", t.text_snapshot());
}

#[test]
fn the_network_permission_and_its_host_come_from_the_setting() {
    let fake = Fake::new();
    let mut t = Harness::build(dir()).setting("server", SERVER).secret("apiKey", &fake::full()).serve(fake.server()).open("inbox");
    assert!(shows(&t, "network permission"), "{}", t.text_snapshot());
    assert!(fake.requests().is_empty());
    // Granted: it reaches the host of Server address, and only it.
    t.grant("net");
    assert!(shows(&t, "Connected"), "{}", t.text_snapshot());
    assert!(t.requests().iter().all(|r| r.url.host_str() == Some("agention.example.ts.net")));
    // Another address: the grant follows it.
    t.set_setting("server", Some("https://inbox.example.com"));
    assert!(t.requests().iter().any(|r| r.url.host_str() == Some("inbox.example.com")), "the new host is reached");
}

#[test]
fn threads_by_agent_with_their_questions() {
    let fake = Fake::new();
    let id = asking(&fake);
    let notes = fake.add_thread("api", "s9", "Release notes", "Drafted the notes.", vec![]);
    fake.add_thread("api", "s9", "Read already", "Old news.", vec![]);
    fake.mark_read(&notes.replace("/1", "/2"));
    let mut t = open(&fake);
    assert!(shows(&t, "Connected"), "{}", t.text_snapshot());
    // Waiting on you: the question first, under Pitwall's agent's name,
    // then what is new; not what was read.
    let snap = t.text_snapshot();
    assert!(shows(&t, "web") && shows(&t, "1 question"), "{snap}");
    assert!(snap.find("Which database?") < snap.find("Release notes"), "{snap}");
    assert!(!shows(&t, "Read already"), "{snap}");
    t.choose("head/filter", "all");
    assert!(shows(&t, "Read already") && shows(&t, "api · s9"), "{}", t.text_snapshot());
    assert!(shows(&t, "Working in api"), "the session's status title");
    // In the floating control, from agent a1, blocking.
    let asks = t.asks();
    assert_eq!(asks.len(), 1, "{asks:?}");
    assert_eq!((asks[0].id.as_str(), asks[0].from.as_deref(), asks[0].blocking), (format!("{id}#1").as_str(), Some("a1"), true));
    assert_eq!(asks[0].suggestions.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["Postgres", "SQLite"]);
    // A badge on a1's row; the count on the toolbar.
    let badges = t.badges();
    assert_eq!(badges.len(), 1);
    assert_eq!(badges[0].count, Some(1));
    assert_eq!(t.toolbar().badge, Some((ToolbarBadge::Count(1), BadgeTone::Attention)));
}

#[test]
fn answering_in_the_tile() {
    let fake = Fake::new();
    let id = asking(&fake);
    let mut t = open(&fake);
    t.click_text("Which database?");
    assert!(shows(&t, "SQLite or Postgres?"), "{}", t.text_snapshot());
    assert!(shows(&t, "I need to pick one for the cache."));
    // One pick goes after its take-back time.
    t.click_key("main/thread/messages/m1/body/a1/picks/s1");
    assert!(fake.answer_of(&id, 1).is_none(), "held for Undo first");
    assert!(t.text_snapshot().contains("Your answer: sending in 5 s"), "{}", t.text_snapshot());
    t.advance(TAKE_BACK);
    let a = fake.answer_of(&id, 1).expect("answered");
    assert_eq!(a["labels"], serde_json::json!(["SQLite"]));
    assert!(shows(&t, "You answered: SQLite"), "{}", t.text_snapshot());
    assert!(t.asks().is_empty(), "gone from the floating control");
    assert!(t.badges().is_empty());
    assert_eq!(t.toolbar().badge.map(|b| b.0), Some(ToolbarBadge::None));
    // Each write carries the latest message seen and a request id.
    let sent = fake.sent();
    assert_eq!(sent[0]["seen_through"], 1);
    assert!(sent[0]["request_id"].as_str().unwrap().starts_with("pitwall-"));
}

#[test]
fn answering_in_words_and_declining() {
    let fake = Fake::new();
    let id = fake.add_thread("web", "a1", "Two things", "Two questions.", vec![AskSpec::new("Where do logs go?", &[]), AskSpec::new("Ship today?", &["Yes", "No"])]);
    let mut t = open(&fake);
    t.click_text("Two things");
    t.type_into("main/thread/messages/m1/body/a1/words", "in the data dir").click_key("main/thread/messages/m1/body/a1/actions/send").advance(TAKE_BACK);
    assert_eq!(fake.answer_of(&id, 1).unwrap()["text"], "in the data dir");
    t.click_key("main/thread/messages/m1/body/a2/actions/decline").advance(TAKE_BACK);
    assert_eq!(fake.answer_of(&id, 2).unwrap()["declined"], true);
    assert!(shows(&t, "You declined"), "{}", t.text_snapshot());
}

#[test]
fn answering_in_the_floating_control() {
    let fake = Fake::new();
    let id = asking(&fake);
    let mut t = open(&fake);
    let ask = format!("{id}#1");
    t.answer(&ask, &[1]);
    assert_eq!(t.reply(&ask), Some(None), "taken");
    assert_eq!(fake.answer_of(&id, 1).unwrap()["picks"], serde_json::json!([1]));
    assert!(t.asks().is_empty());
    // Another, answered in words; then one the agent withdrew meanwhile.
    fake.agent_post(&id, "And this?", vec![AskSpec::new("Which port?", &[])]);
    t.settle();
    let second = format!("{id}#2");
    assert!(t.asks().iter().any(|a| a.id == second), "{:?}", t.asks());
    fake.withdraw(&id, 2, "found it");
    t.answer_text(&second, "8787");
    assert!(matches!(t.reply(&second), Some(Some(why)) if why.contains("refused")), "{:?}", t.reply(&second));
}

#[test]
fn news_arrives_through_the_wait() {
    let fake = Fake::new();
    let mut t = open(&fake);
    assert!(shows(&t, "No threads yet"), "{}", t.text_snapshot());
    assert_eq!(fake.held(), 1, "a wait held for news");
    let id = asking(&fake);
    t.settle();
    assert!(shows(&t, "Which database?"), "{}", t.text_snapshot());
    assert_eq!(t.asks().len(), 1);
    assert_eq!(fake.held(), 1, "waiting again");
    let _ = id;
}

#[test]
fn replies_mark_read_and_archive() {
    let fake = Fake::new();
    let id = fake.add_thread("web", "a1", "Status", "Tests pass.", vec![]);
    let mut t = open(&fake);
    t.choose("head/filter", "all").click_text("Status");
    t.click_key("main/thread/head/read");
    assert_eq!(fake.marks(&id).0, 1);
    t.type_into("main/thread/reply-box/reply", "Thanks, merge it.").click_key("main/thread/reply-box/actions/send-reply").advance(TAKE_BACK);
    assert_eq!(fake.sent().last().unwrap()["text"], "Thanks, merge it.");
    assert!(shows(&t, "Thanks, merge it."), "{}", t.text_snapshot());
    assert_eq!(t.field_text("main/thread/reply-box/reply"), "", "cleared once sent");
    // The agent writes meanwhile: the reply is refused as stale and the
    // thread read again.
    fake.agent_post(&id, "One more thing.", vec![]);
    t.type_into("main/thread/reply-box/reply", "Ok").click_key("main/thread/reply-box/actions/send-reply").advance(TAKE_BACK);
    assert!(shows(&t, "One more thing."), "{}", t.text_snapshot());
    t.click_key("main/thread/head/archive");
    assert_eq!(fake.marks(&id).1, 4);
    assert!(!shows(&t, "Status"), "{}", t.text_snapshot());
    assert!(t.toasts().iter().any(|(s, _)| s.starts_with("Archived")));
}

#[test]
fn signed_out_and_missing_scopes() {
    let fake = Fake::new();
    let t = harness(&fake, &fake::revoked()).open("inbox");
    assert!(shows(&t, "Signed out"), "{}", t.text_snapshot());
    assert!(shows(&t, "refused the API key"), "{}", t.text_snapshot());
    let t = harness(&fake, &fake::read_only()).open("inbox");
    assert!(shows(&t, "threads:write scope"), "{}", t.text_snapshot());
    let t = harness(&fake, &fake::no_sessions()).open("inbox");
    assert!(shows(&t, "sessions:read scope"), "{}", t.text_snapshot());
    // Not a key at all: said before anything is sent.
    let quiet = Fake::new();
    let t = harness(&quiet, "not-a-key").open("inbox");
    assert!(shows(&t, "doesn't look like an Agention API key"), "{}", t.text_snapshot());
    assert!(quiet.requests().is_empty());
}

#[test]
fn not_agention_or_too_old() {
    let fake = Fake::new();
    fake.not_agention(true);
    let t = open(&fake);
    assert!(shows(&t, "doesn't answer like an Agention server"), "{}", t.text_snapshot());
    let old = Fake::new();
    old.legacy(true);
    let t = open(&old);
    assert!(shows(&t, "no operator API"), "{}", t.text_snapshot());
}

#[test]
fn down_then_back() {
    let fake = Fake::new();
    fake.fail_next(503, "unavailable");
    let mut t = open(&fake);
    assert!(shows(&t, "Not connected"), "{}", t.text_snapshot());
    assert!(shows(&t, "Trying again in 1 s"), "{}", t.text_snapshot());
    t.advance(Duration::from_secs(1));
    assert!(shows(&t, "Connected"), "{}", t.text_snapshot());
}

#[test]
fn its_status_is_reported_as_the_adapter_did() {
    let fake = Fake::new();
    let t = open(&fake);
    assert_eq!(t.status(), Some((ConnState::Ready, String::new())));
    let t = harness(&fake, &fake::revoked()).open("inbox");
    let (state, said) = t.status().unwrap();
    assert_eq!(state, ConnState::SignedOut);
    assert!(said.contains("refused the API key"), "{said}");
    fake.fail_next(503, "unavailable");
    let mut t = open(&fake);
    assert_eq!(t.status().map(|s| s.0), Some(ConnState::Error));
    t.advance(Duration::from_secs(1));
    assert_eq!(t.status().map(|s| s.0), Some(ConnState::Ready));
    let t = Harness::build(dir()).open("inbox");
    assert_eq!(t.status().map(|s| s.0), Some(ConnState::Error), "not set up");
}

#[test]
fn a_reply_can_be_taken_back() {
    let fake = Fake::new();
    let id = fake.add_thread("web", "a1", "Status", "Tests pass.", vec![]);
    let mut t = open(&fake);
    t.choose("head/filter", "all").click_text("Status");
    let before = fake.sent().len();
    t.type_into("main/thread/reply-box/reply", "Merge it.").click_key("main/thread/reply-box/actions/send-reply");
    assert!(shows(&t, "Your reply: sending in 5 s"), "{}", t.text_snapshot());
    t.advance(Duration::from_secs(2)).click_key("main/thread/take-back/take-back-1");
    assert_eq!(t.field_text("main/thread/reply-box/reply"), "Merge it.", "back in the box");
    t.advance(TAKE_BACK);
    assert_eq!(fake.sent().len(), before, "never sent");
    let _ = id;
}

#[test]
fn number_keys_answer_the_open_question() {
    let fake = Fake::new();
    let id = asking(&fake);
    let mut t = open(&fake);
    t.click_text("Which database?");
    // Typing in the reply box, 2 is a 2.
    t.focus_key("main/thread/reply-box/reply");
    assert!(t.try_press("2").is_err());
    t.blur().press("2").advance(TAKE_BACK);
    assert_eq!(fake.answer_of(&id, 1).expect("answered")["labels"], serde_json::json!(["SQLite"]));
    assert!(shows(&t, "You answered: SQLite"), "{}", t.text_snapshot());
}

#[test]
fn the_key_is_sent_only_to_the_server_and_never_shown() {
    let fake = Fake::new();
    asking(&fake);
    let key = fake::full();
    let mut t = open(&fake);
    t.click_text("Which database?");
    for (m, p, auth, _) in fake.requests() {
        if p == "/api/operator" {
            assert_eq!(auth, None, "the versions are read without the key");
        } else {
            assert_eq!(auth.as_deref(), Some(format!("Bearer {key}").as_str()), "{m} {p}");
        }
    }
    assert!(!t.text_snapshot().contains(&key) && !t.log_text().contains(&key));
    assert!(!t.text_snapshot().contains("agention_") && !t.log_text().contains("agention_"));
}
