//! Runs the built tile in Pitwall's tile runtime: `cargo test` builds
//! tile.wasm first (it needs `rustup target add wasm32-wasip2`). Timers run
//! on the wall clock, so the save tests wait a little over a second.

use std::time::{Duration, Instant};

use pitwall_extension_test::{Harness, Session};
use serde_json::{json, Value};

/// Long enough for the save a second after typing stops.
const AFTER_SAVE: Duration = Duration::from_millis(1400);

fn notes() -> Session {
    Harness::build(env!("CARGO_MANIFEST_DIR")).open("main")
}

/// The text kept in storage for note `id`.
fn stored_text(tile: &Session, id: u64) -> Option<String> {
    tile.stored_json::<Value>(&format!("p/_/{id}")).and_then(|v| v["text"].as_str().map(str::to_string))
}

fn shows(tile: &Session, text: &str) -> bool {
    tile.find_text(text).is_some()
}

#[test]
fn a_new_note_is_typed_on_several_lines_and_saved_after_typing_stops() {
    let mut tile = notes();
    assert!(shows(&tile, "No notes yet"), "{}", tile.text_snapshot());
    tile.click_label("New note");
    assert!(shows(&tile, "New note"), "{}", tile.text_snapshot());
    // The new note's field asks for focus, so typing can start at once.
    assert_eq!(tile.frame().focus.as_deref(), Some("editor/n1/text"));
    tile.type_into("text", "web: the login page flickers\non reload\nask web to check");
    // Not saved while typing...
    assert_eq!(stored_text(&tile, 1), None);
    assert!(shows(&tile, "Editing…"));
    assert!(shows(&tile, "web: the login page flickers"), "the title is the first line:\n{}", tile.text_snapshot());
    // ...but a second after it stops.
    tile.wait(AFTER_SAVE);
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("web: the login page flickers\non reload\nask web to check"));
    assert!(shows(&tile, "Saved"), "{}", tile.text_snapshot());
    // Back in the list: title, when, and the next line.
    tile.click_label("All notes");
    assert!(shows(&tile, "web: the login page flickers"));
    assert!(shows(&tile, "on reload"), "{}", tile.text_snapshot());
    assert!(shows(&tile, "now"));
}

#[test]
fn editing_a_note_saves_the_new_text() {
    let mut tile = notes();
    tile.press("cmd-n").type_into("text", "Plan\n- one").click_label("All notes");
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("Plan\n- one"), "leaving a note saves it");
    tile.click_text("Plan");
    tile.type_into("text", "Plan\n- one\n- two");
    tile.wait(AFTER_SAVE);
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("Plan\n- one\n- two"));
}

#[test]
fn hiding_the_tile_saves_at_once() {
    let mut tile = notes();
    tile.click_label("New note").type_into("text", "catch this");
    tile.hide();
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("catch this"));
    tile.show();
    assert!(shows(&tile, "Saved"));
}

#[test]
fn unsaved_text_survives_a_reload() {
    let mut tile = notes();
    tile.click_label("New note").type_into("text", "half a thought");
    assert_eq!(stored_text(&tile, 1), None);
    // Dropped or reloaded before the save: save-state keeps the text and
    // the open note.
    tile.reload();
    assert!(shows(&tile, "half a thought"), "{}", tile.text_snapshot());
    assert!(shows(&tile, "Editing…"));
    tile.wait(AFTER_SAVE);
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("half a thought"));
}

#[test]
fn a_reload_keeps_the_notes() {
    let mut tile = notes();
    for t in ["first", "second", "third"] {
        tile.press("cmd-n").type_into("text", t);
    }
    tile.click_label("All notes");
    tile.reload();
    // Newest change first.
    let titles: Vec<String> = tile.texts().into_iter().filter(|t| ["first", "second", "third"].contains(&t.as_str())).collect();
    assert_eq!(titles, ["third", "second", "first"], "{}", tile.text_snapshot());
    assert_eq!(tile.storage().keys().filter(|k| k.starts_with("p/_/")).count(), 3);
}

#[test]
fn an_empty_note_is_dropped_when_left() {
    let mut tile = notes();
    tile.click_label("New note").click_label("All notes");
    assert!(shows(&tile, "No notes yet"), "{}", tile.text_snapshot());
    assert!(tile.storage().is_empty());
}

#[test]
fn delete_asks_inline_then_removes_the_note() {
    let mut tile = notes();
    tile.press("cmd-n").type_into("text", "keep me").press("cmd-n").type_into("text", "drop me");
    tile.click_label("Delete note");
    assert!(shows(&tile, "Delete “drop me”?"), "{}", tile.text_snapshot());
    // Cancel leaves it.
    tile.click_text("Cancel");
    assert!(shows(&tile, "Saved") || shows(&tile, "Editing…"));
    tile.click_label("Delete note").click_key("delete");
    assert!(!shows(&tile, "drop me"), "{}", tile.text_snapshot());
    assert!(shows(&tile, "keep me"));
    assert_eq!(stored_text(&tile, 2), None);
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("keep me"));
}

#[test]
fn search_filters_by_title_and_text() {
    let mut tile = notes();
    tile.press("cmd-n").type_into("text", "api\nhandle 429s");
    tile.press("cmd-n").type_into("text", "web\ndrop the old route");
    tile.click_label("All notes");
    tile.type_into("search", "429");
    assert!(shows(&tile, "api") && !shows(&tile, "web"), "{}", tile.text_snapshot());
    tile.type_into("search", "WEB route");
    assert!(shows(&tile, "web") && !shows(&tile, "api"), "{}", tile.text_snapshot());
    tile.type_into("search", "nothing like it");
    assert!(shows(&tile, "No notes match “nothing like it”"));
    tile.type_into("search", "");
    assert!(shows(&tile, "api") && shows(&tile, "web"));
}

#[test]
fn opened_with_a_draft_it_stays_unsaved_until_kept() {
    let draft = json!({ "title": "Release checklist", "text": "- tag\n- notes" });
    let mut tile = Harness::build(env!("CARGO_MANIFEST_DIR")).open_with("main", draft);
    assert!(shows(&tile, "Release checklist"), "{}", tile.text_snapshot());
    assert!(shows(&tile, "Draft, not saved"));
    tile.type_into("text", "Release checklist\n- tag\n- notes\n- announce");
    tile.wait(AFTER_SAVE);
    assert!(tile.storage().is_empty(), "a draft isn't saved by itself");
    // The same view given a new draft (an agent updating it in place).
    tile.set_content("main", json!({ "text": "Release checklist v2\n- tag" }));
    assert!(shows(&tile, "Release checklist v2"), "{}", tile.text_snapshot());
    // Back in the list the draft waits at the top.
    tile.click_label("All notes");
    assert!(shows(&tile, "Draft, not saved"));
    tile.click_text("Release checklist v2");
    tile.click_text("Keep");
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("Release checklist v2\n- tag"));
    assert!(shows(&tile, "Saved") && !shows(&tile, "Draft, not saved"), "{}", tile.text_snapshot());
}

#[test]
fn a_discarded_draft_leaves_nothing() {
    let mut tile = Harness::build(env!("CARGO_MANIFEST_DIR")).open_with("main", json!({ "text": "scratch" }));
    tile.click_text("Discard");
    assert!(shows(&tile, "No notes yet"), "{}", tile.text_snapshot());
    assert!(tile.storage().is_empty());
}

#[test]
fn a_note_of_256_kib_is_typed_and_saved() {
    // G1c (frd.md §7): the whole value crosses the boundary on each change.
    let mut tile = notes();
    tile.click_label("New note");
    let big = format!("Big\n{}", "0123456789abcdef".repeat(16 * 1024));
    let start = Instant::now();
    for i in 0..10 {
        tile.type_into("text", &format!("{big}{i}"));
    }
    let typed = start.elapsed() / 10;
    tile.hide();
    assert_eq!(stored_text(&tile, 1).map(|t| t.len()), Some(big.len() + 1));
    println!("a 256 KiB change: {typed:?} per round trip (event, handle, render)");
    assert!(typed < Duration::from_millis(500), "{typed:?}");
}

#[test]
fn the_list_and_the_editor_as_frames() {
    let mut tile = notes();
    tile.press("cmd-n").type_into("text", "api\nhandle 429s");
    tile.press("cmd-n").type_into("text", "web: login flickers\non reload");
    let editor = "\
box
  box #editor
    box #head
      button \"\" #back label=\"All notes\" [click]
      text \"web: login flickers\"
      text \"Editing…\"
      button \"\" #trash label=\"Delete note\" [click]
    box #n2
      input placeholder=\"Title on the first line, then anything\" #text [change, submit]
";
    assert_eq!(tile.text_snapshot(), editor);
    tile.click_label("All notes");
    let list = "\
box
  box #list
    box #head
      input placeholder=\"Search notes\" #search [change]
      button \"\" #new label=\"New note\" [click]
    scroll #rows
      box #n2 [click]
        box
          text \"web: login flickers\"
          text \"now\"
        text \"on reload\"
      box #n1 [click]
        box
          text \"api\"
          text \"now\"
        text \"handle 429s\"
";
    assert_eq!(tile.text_snapshot(), list);
}
