//! The harness on the tile host's committed hello tile (written with the
//! guest SDK; rebuilt by crates/pitwall-tile-host/tests/fixtures/
//! build-hello-tile.sh), so it runs without the wasm target.

use std::path::{Path, PathBuf};
use std::time::Duration;

use pitwall_extension_test::{Harness, Level, Stop, Tone};

fn hello() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/hello")
}

#[test]
fn clicks_render_and_are_stored() {
    let mut t = Harness::load(hello()).open("main");
    assert!(t.find_text("Clicked 0 times").is_some(), "{}", t.text_snapshot());
    assert!(t.logged(Level::Info, "starts with 0 clicks"));
    t.click_text("Click me").click_key("button");
    assert!(t.find_text("Clicked 2 times").is_some());
    assert_eq!(t.stored("clicks"), Some(2u32.to_le_bytes().to_vec()));
    let snap = t.text_snapshot();
    assert!(snap.starts_with("box\n  text \"Hello from a tile (main)\" #title\n"), "{snap}");
    assert!(snap.contains("button \"Click me\" #button [click]"), "{snap}");
    assert!(snap.contains("input placeholder=\"Type here\" #field [change]"), "{snap}");
}

/// Typed text, focus, keys, time, size, theme and a restart without
/// state, as the harness drives them.
#[test]
fn focus_keys_virtual_time_and_restart() {
    let mut t = Harness::load(hello()).open("main");
    assert!(!t.has_focus());
    t.type_into("field", "abc");
    assert!(t.has_focus() && t.focused() == Some("field"));
    let field = t.field("field");
    assert_eq!((field.text.as_str(), field.multiline, field.focused), ("abc", false, true));
    assert!(t.text_snapshot().contains("input placeholder=\"Type here\" text=\"abc\" focused #field [change]"), "{}", t.text_snapshot());
    // A plain key types into the focused field; Pitwall's keys are its own.
    assert!(t.try_press("j").err().unwrap().contains("types into the focused text field"));
    assert!(t.try_press("cmd-w").err().unwrap().contains("Pitwall keeps that key"));
    assert!(t.logged(Level::Warn, "“cmd-w” was dropped"));
    t.blur().press("j");
    assert!(t.find_text("Clicked 1 times").is_some(), "{}", t.text_snapshot());
    t.focus(false);
    assert!(!t.has_focus() && t.focused().is_none());
    // Time stands still until the test moves it.
    t.click_key("test/timer").advance(Duration::from_millis(99));
    assert!(t.find_text("Ticks: 0").is_some());
    t.advance(Duration::from_millis(1));
    assert!(t.find_text("Ticks: 1").is_some());
    t.resize("main", 800., 600.).light();
    assert!(t.find_text("Clicked 1 times").is_some());
    t.type_into("field", "gone");
    // Restarted without its state: what it stored is back, the typed text
    // isn't.
    t.click_text("Click me");
    t.restart();
    assert!(t.find_text("Clicked 2 times").is_some(), "{}", t.text_snapshot());
    assert_eq!(t.field_text("field"), "");
    assert!(t.unknown_icons().is_empty());
}

#[test]
fn settings_typing_timers_and_toasts() {
    let mut t = Harness::load(hello()).setting("greeting", "Howdy").secret("token", "s3cret").open("main");
    assert!(t.find_text("Howdy from a tile (main)").is_some());
    t.set_setting("greeting", Some("Hi"));
    assert!(t.find_text("Hi from a tile").is_some());
    t.type_into("field", "abc");
    assert!(t.find_text("Typed: abc").is_some());
    t.type_into("field", "abcd");
    assert!(t.find_text("Typed: abcd").is_some());
    t.click_key("test/timer").wait_for_text("Ticks: 1", Duration::from_secs(5));
    t.click_key("toast");
    assert_eq!(t.toasts(), vec![("Saved".to_string(), Tone::Good)]);
    t.click_key("print");
    assert!(t.logged(Level::Info, "hello from stdout") && t.logged(Level::Warn, "hello from stderr"));
}

/// Its Setup view, as its page shows it (`Harness::setup`), and beside a
/// tile's view (`Session::open_setup`).
#[test]
fn the_setup_view() {
    let mut t = Harness::load(hello()).setting("greeting", "Howdy").stored("clicks", 3u32.to_le_bytes()).setup();
    let snap = t.text_snapshot();
    assert!(snap.contains("Hello greets you in its tiles"), "{snap}");
    assert!(snap.contains("Howdy") && snap.contains("\"3\""), "{snap}");
    t.click_key("setup-reset");
    assert_eq!(t.stored("clicks"), Some(0u32.to_le_bytes().to_vec()));
    assert!(t.text_snapshot().contains("\"0\""), "{}", t.text_snapshot());
    let mut t = Harness::load(hello()).open("main");
    t.click_text("Click me");
    let view = t.open_setup();
    assert_eq!(view, "setup");
    assert!(t.text_snapshot_of("setup").contains("\"1\""), "{}", t.text_snapshot_of("setup"));
}

#[test]
fn views_with_content_and_a_reload_that_keeps_state() {
    let mut t = Harness::load(hello()).open_with("main", serde_json::json!({ "message": "first" }));
    assert!(t.find_text(r#"Content: {"message":"first"}"#).is_some(), "{}", t.text_snapshot());
    t.open_view("agent-1", "second", Some(serde_json::json!({ "message": "other" })));
    assert!(t.text_snapshot_of("agent-1").contains(r#"Content: {\"message\":\"other\"}"#), "{}", t.text_snapshot_of("agent-1"));
    t.set_content("agent-1", serde_json::json!({ "message": "again" }));
    assert!(t.text_snapshot_of("agent-1").contains("again"));
    t.close_view("agent-1");
    t.click_text("Click me");
    assert_eq!(t.save_state(), 1u32.to_le_bytes());
    t.reload();
    assert!(t.find_text("Clicked 1 times").is_some());
    assert!(t.logged(Level::Info, "starts with 1 clicks"));
}

#[test]
fn a_crash_is_reported_not_hidden() {
    let mut t = Harness::load(hello()).open("main");
    let f = t.find_key("test/panic").expect("the panic button");
    assert!(f.listens(pitwall_extension_test::Listen::Click));
    // click_key settles and so panics on a stop; send it by hand.
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        t.click_key("panic");
    }));
    assert!(r.is_err(), "the click reports the stop");
    assert!(matches!(t.stopped(), Some(Stop::Crashed(_))), "{:?}", t.stopped());
}

#[test]
fn a_hidden_tile_renders_once_shown() {
    let mut t = Harness::load(hello()).hidden().open("main");
    assert!(t.find_key("title").is_none(), "nothing rendered while hidden");
    t.show();
    assert!(t.find_text("Clicked 0 times").is_some());
}

/// Agents' calls (agent-tools.md §11.2–11.10): `hello__open` with content,
/// the declared `show_message` tool with items per agent, an input that
/// doesn't fit and a tile error that change nothing, an update in place,
/// and the refusal of an item the user edited.
#[test]
fn agents_open_it_and_show_messages() {
    use pitwall_extension_test::Called;
    use serde_json::json;
    let mut t = Harness::load(hello()).open("main");
    // hello__open: the view's content, no reply of the tile's own.
    assert_eq!(t.call("claude", None, None, json!({ "message": "hi" })), Called::Shown);
    assert!(t.find_text(r#"Content: {"message":"hi"}"#).is_some(), "{}", t.text_snapshot());
    // A declared tool: the tile's reply, its message shown.
    let r = t.call("claude", Some("show_message"), Some("a"), json!({ "message": "first" }));
    assert_eq!(r, Called::Replied { text: "Shown in Hello (1 messages)".into(), json: None, item: None });
    assert!(t.find_text("claude: first").is_some(), "{}", t.text_snapshot());
    // The same item again: replaced in place; another agent's item "a" is
    // its own.
    t.call("claude", Some("show_message"), Some("a"), json!({ "message": "second" }));
    t.call("codex", Some("show_message"), Some("a"), json!({ "message": "mine" }));
    assert!(t.find_text("claude: first").is_none() && t.find_text("claude: second").is_some() && t.find_text("codex: mine").is_some(), "{}", t.text_snapshot());
    // Input that doesn't fit the schema never reaches the tile.
    let before = t.text_snapshot();
    let r = t.call("claude", Some("show_message"), Some("b"), json!({ "message": 5 }));
    assert!(matches!(&r, Called::Refused(m) if m.contains("`message` must be a string")), "{r:?}");
    // The tile's own problems: nothing changes either.
    let r = t.call("claude", Some("show_message"), Some("b"), json!({ "message": "  " }));
    assert_eq!(r, Called::Failed("Nothing was shown: the message is empty.\n`message` has no text".into()));
    assert_eq!(t.text_snapshot(), before);
    // The user edits them: replacing an edited item is refused, a new item
    // is fine.
    t.click_key("test/edit");
    let r = t.call("claude", Some("show_message"), Some("a"), json!({ "message": "third" }));
    assert!(matches!(&r, Called::Refused(m) if m.contains("The user changed “a”")), "{r:?}");
    assert!(t.find_text("claude: second (edited)").is_some(), "{}", t.text_snapshot());
    assert!(t.call("claude", Some("show_message"), Some("c"), json!({ "message": "new" })).is_ok());
}

/// Phase 3 (agent-tools.md §11.2–11.7): a `read` tool asks the user until
/// they allow that agent always, and runs without the tile's acting
/// permissions; an `act` tool asks each time unless allowed always, and a
/// denial reaches nothing; an act answered later, or given up; the user's
/// own command; Undo of what a present call showed; appends; and a file
/// named instead of inline text.
#[test]
fn agents_read_act_and_the_user_undoes() {
    use pitwall_extension_test::{Approval, Called};
    use serde_json::json;
    let mut t = Harness::load(hello()).grant("notify").open("main");
    t.call("claude", Some("show_message"), Some("a"), json!({ "message": "first" }));
    // read: asked once, then always for that agent.
    t.approve(Approval::Always);
    let r = t.call("claude", Some("current_messages"), None, json!({}));
    assert_eq!(r.text(), "1 messages (notify: denied)", "a read can't notify");
    assert_eq!(r.json(), Some(&json!({ "claude/a": "first" })));
    t.approve(Approval::Deny);
    assert!(t.call("claude", Some("current_messages"), None, json!({})).is_ok(), "allowed always: no second ask");
    assert_eq!(t.approvals_asked().len(), 1, "{:?}", t.approvals_asked());
    assert!(matches!(t.call("codex", Some("current_messages"), None, json!({})), Called::Refused(m) if m.contains("denied")), "another agent asks for itself");
    // act: denied reaches nothing; once runs with the tile's grants.
    let r = t.call("claude", Some("send_message"), None, json!({ "text": "hi" }));
    assert!(matches!(&r, Called::Refused(m) if m.contains("denied")), "{r:?}");
    assert!(t.find_text("Sent: ; cancelled: 0").is_some(), "{}", t.text_snapshot());
    t.approve(Approval::Once);
    let r = t.call("claude", Some("send_message"), None, json!({ "text": "hi" }));
    assert_eq!(r.text(), "Sent “hi” for claude (notify: ok)");
    assert!(t.approvals_asked().last().unwrap().contains(r#"claude wants to run send_message of Hello: {"text":"hi"}"#), "{:?}", t.approvals_asked());
    let asked = t.approvals_asked().len();
    t.call("claude", Some("send_message"), None, json!({ "text": "again" }));
    assert_eq!(t.approvals_asked().len(), asked + 1, "once is once");
    // Answered later: waiting until the tile replies.
    let Called::Waiting(id) = t.call("claude", Some("send_message"), None, json!({ "text": "slow", "wait_ms": 500 })) else { panic!() };
    assert_eq!(t.answer_of(id), None);
    t.advance(Duration::from_millis(500));
    assert_eq!(t.answer_of(id).map(|c| c.text().to_string()), Some("Sent “slow” later".into()));
    // Given up: the tile hears it, and its late reply is dropped.
    let Called::Waiting(id) = t.call("claude", Some("send_message"), None, json!({ "text": "slower", "wait_ms": 500 })) else { panic!() };
    t.cancel_call(id);
    assert!(t.find_text("Sent: hi, again, slow; cancelled: 1").is_some(), "{}", t.text_snapshot());
    t.advance(Duration::from_millis(500));
    assert_eq!(t.answer_of(id), None);
    // The user's own command: an act still asks.
    let asked = t.approvals_asked().len();
    // (Its notification is over the few a minute the tile may send.)
    assert!(t.run_command("send_message", json!({ "text": "mine" })).text().starts_with("Sent “mine” for the user"));
    assert!(t.approvals_asked()[asked].starts_with("You wants to run"), "{:?}", t.approvals_asked());
    // Undo: the value before the call comes back; the call that made an
    // item takes it away.
    t.call("claude", Some("show_message"), Some("a"), json!({ "message": "second" }));
    let second = t.last_call().unwrap();
    assert!(t.find_text("claude: second").is_some());
    t.undo(second);
    assert!(t.find_text("claude: first").is_some(), "{}", t.text_snapshot());
    t.call("claude", Some("show_message"), Some("new"), json!({ "message": "made" }));
    let made = t.last_call().unwrap();
    t.undo(made);
    assert!(t.find_text("claude: made").is_none(), "{}", t.text_snapshot());
    // Appends add to the item; a file's text arrives in parts (past
    // the rate limit first: 10 calls in 10 seconds).
    t.advance(Duration::from_secs(10));
    t.call("claude", Some("show_message"), Some("log"), json!({ "message": "one" }));
    let r = t.call_append("claude", "show_message", "log", json!({ "message": "+two" }));
    assert!(r.is_ok(), "{r:?}");
    assert!(t.find_text("claude: one+two").is_some(), "{}", t.text_snapshot());
    // Appends close together: drawn at most 4 times a second.
    t.call_append("claude", "show_message", "log", json!({ "message": "+3" }));
    assert!(t.find_text("claude: one+two").is_some(), "held back: {}", t.text_snapshot());
    t.advance(Duration::from_millis(250));
    assert!(t.find_text("claude: one+two+3").is_some(), "{}", t.text_snapshot());
    // Over the 1 MiB one part carries, cut between characters.
    let big = "é".repeat(600_001);
    assert!(t.call_from_file("claude", "show_message", Some("file"), json!({}), &big).is_ok());
    assert!(t.find_text(&format!("claude: {big}")).is_some());
}
