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
