//! Runs the built tile in Pitwall's tile runtime: `cargo test` builds
//! tile.wasm first (it needs `rustup target add wasm32-wasip2`). Time is
//! the harness's virtual clock: the save tests move it on, they don't wait.

use std::time::{Duration, Instant};

use pitwall_extension_test::{Harness, Session};
use serde_json::{json, Value};

/// The save a second after typing stops.
const SAVE_AFTER: Duration = Duration::from_secs(1);

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

/// A new note with `text`, from the list's + button.
fn write(tile: &mut Session, text: &str) {
    if tile.find_label("New note").is_none() {
        tile.click_label("All notes");
    }
    tile.click_label("New note").type_into("text", text);
}

#[test]
fn a_new_note_is_typed_on_several_lines_and_saved_after_typing_stops() {
    let mut tile = notes();
    assert!(shows(&tile, "No notes yet"), "{}", tile.text_snapshot());
    // N makes one, while no field has the keyboard.
    tile.focus(true).press("n");
    assert!(shows(&tile, "New note"), "{}", tile.text_snapshot());
    // The new note's field takes the keyboard, so typing can start at once.
    assert_eq!(tile.focused(), Some("editor/n1/text"));
    tile.type_into("text", "web: the login page flickers\non reload\nask web to check");
    // Not saved while typing...
    tile.advance(SAVE_AFTER / 2);
    assert_eq!(stored_text(&tile, 1), None);
    assert!(shows(&tile, "Editing…"));
    assert!(shows(&tile, "web: the login page flickers"), "the title is the first line:\n{}", tile.text_snapshot());
    // ...but a second after it stops.
    tile.advance(SAVE_AFTER / 2);
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("web: the login page flickers\non reload\nask web to check"));
    assert!(shows(&tile, "Saved"), "{}", tile.text_snapshot());
    // Back in the list: title, when, and the next line.
    tile.click_label("All notes");
    assert!(shows(&tile, "web: the login page flickers"));
    assert!(shows(&tile, "on reload"), "{}", tile.text_snapshot());
    assert!(shows(&tile, "now"));
}

#[test]
fn typing_on_keeps_putting_the_save_off() {
    let mut tile = notes();
    write(&mut tile, "a");
    for t in ["ab", "abc", "abcd"] {
        tile.advance(SAVE_AFTER * 3 / 4).type_into("text", t);
    }
    assert_eq!(stored_text(&tile, 1), None);
    tile.advance(SAVE_AFTER);
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("abcd"));
}

#[test]
fn editing_a_note_saves_the_new_text() {
    let mut tile = notes();
    write(&mut tile, "Plan\n- one");
    tile.click_label("All notes");
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("Plan\n- one"), "leaving a note saves it");
    tile.click_text("Plan");
    tile.type_into("text", "Plan\n- one\n- two");
    tile.advance(SAVE_AFTER);
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("Plan\n- one\n- two"));
}

#[test]
fn leaving_the_field_saves_at_once() {
    let mut tile = notes();
    // Wide: the list and the note side by side.
    tile.resize("main", 800., 500.);
    write(&mut tile, "to the search");
    assert!(shows(&tile, "Search notes") || tile.find_key("search").is_some(), "{}", tile.text_snapshot());
    // Into the search field: the note's field blurs, and it is saved.
    tile.focus_key("list/head/search");
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("to the search"));
    tile.click_text("to the search").type_into("text", "out of the tile");
    // Out of the tile.
    tile.focus(false);
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("out of the tile"));
}

#[test]
fn hiding_the_tile_saves_at_once() {
    let mut tile = notes();
    write(&mut tile, "catch this");
    tile.hide();
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("catch this"));
    tile.show();
    assert!(shows(&tile, "Saved"));
}

#[test]
fn unsaved_text_survives_a_reload() {
    let mut tile = notes();
    write(&mut tile, "half a thought");
    assert_eq!(stored_text(&tile, 1), None);
    // Dropped or reloaded before the save: save-state keeps the text and
    // the open note.
    tile.reload();
    assert!(shows(&tile, "half a thought"), "{}", tile.text_snapshot());
    assert!(shows(&tile, "Editing…"));
    tile.advance(SAVE_AFTER);
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("half a thought"));
}

#[test]
fn a_restart_reads_the_notes_back() {
    let mut tile = notes();
    for t in ["first", "second", "third"] {
        write(&mut tile, t);
        tile.advance(SAVE_AFTER);
    }
    // Started again with no saved state: the list, newest change first.
    tile.restart();
    let titles: Vec<String> = tile.texts().into_iter().filter(|t| ["first", "second", "third"].contains(&t.as_str())).collect();
    assert_eq!(titles, ["third", "second", "first"], "{}", tile.text_snapshot());
    assert_eq!(tile.storage().keys().filter(|k| k.starts_with("p/_/")).count(), 3);
    // A new note after them gets the next id.
    write(&mut tile, "fourth");
    tile.hide();
    assert_eq!(stored_text(&tile, 4).as_deref(), Some("fourth"));
}

#[test]
fn many_notes_are_read_at_once() {
    let mut h = Harness::build(env!("CARGO_MANIFEST_DIR"));
    for i in 1..=400u64 {
        h = h.stored_json(&format!("p/_/{i}"), &json!({ "text": format!("note {i}"), "updated": i }));
    }
    let tile = h.open("main");
    assert!(shows(&tile, "note 400") && shows(&tile, "note 1"), "{}", tile.text_snapshot());
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
    write(&mut tile, "keep me");
    write(&mut tile, "drop me");
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
fn a_row_deletes_its_note_after_asking() {
    let mut tile = notes();
    write(&mut tile, "keep me");
    write(&mut tile, "drop me");
    tile.click_label("All notes");
    // The row's Delete (shown while the pointer is over the row) asks in
    // the row.
    tile.click_key("list/rows/n2/delete");
    assert!(shows(&tile, "Delete “drop me”?"), "{}", tile.text_snapshot());
    tile.click_key("list/rows/n2/cancel");
    assert!(shows(&tile, "drop me") && !shows(&tile, "Delete “drop me”?"));
    tile.click_key("list/rows/n2/delete").click_key("list/rows/n2/delete");
    assert!(!shows(&tile, "drop me") && shows(&tile, "keep me"), "{}", tile.text_snapshot());
    assert_eq!(stored_text(&tile, 2), None);
}

#[test]
fn search_filters_by_title_and_text() {
    let mut tile = notes();
    write(&mut tile, "api\nhandle 429s");
    write(&mut tile, "web\ndrop the old route");
    tile.click_label("All notes");
    tile.type_into("search", "429");
    assert!(shows(&tile, "api") && !shows(&tile, "web"), "{}", tile.text_snapshot());
    tile.type_into("search", "WEB route");
    assert!(shows(&tile, "web") && !shows(&tile, "api"), "{}", tile.text_snapshot());
    tile.type_into("search", "nothing like it");
    assert!(shows(&tile, "No notes match “nothing like it”"));
    tile.type_into("search", "");
    assert!(shows(&tile, "api") && shows(&tile, "web"));
    // "/" goes to the search, from the list.
    tile.blur().press("/");
    assert_eq!(tile.focused(), Some("list/head/search"));
}

#[test]
fn opened_with_a_draft_it_stays_unsaved_until_kept() {
    let draft = json!({ "title": "Release checklist", "text": "- tag\n- notes" });
    let mut tile = Harness::build(env!("CARGO_MANIFEST_DIR")).open_with("main", draft);
    assert!(shows(&tile, "Release checklist"), "{}", tile.text_snapshot());
    assert!(shows(&tile, "Draft, not saved"));
    tile.type_into("text", "Release checklist\n- tag\n- notes\n- announce");
    tile.advance(SAVE_AFTER * 2);
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

/// `notes__open` from an agent (agent-tools.md §11.2): the draft shown,
/// replaced in place under the same item, refused once the user typed in
/// it; an input that doesn't fit the schema never reaches the tile.
#[test]
fn an_agent_drafts_a_note_until_the_user_edits_it() {
    use pitwall_extension_test::Called;
    let mut tile = notes();
    let r = tile.call("claude", None, Some("plan"), json!({ "title": "Plan", "text": "- a" }));
    assert_eq!(r, Called::Shown);
    assert!(shows(&tile, "Draft, not saved") && tile.field_text("editor/draft/text") == "Plan\n- a", "{}", tile.text_snapshot());
    // The same item again: the draft replaced in place.
    assert!(tile.call("claude", None, Some("plan"), json!({ "title": "Plan", "text": "- a\n- b" })).is_ok());
    assert_eq!(tile.field_text("editor/draft/text"), "Plan\n- a\n- b");
    // No text: refused before the tile sees it.
    let r = tile.call("claude", None, Some("plan"), json!({ "title": "Plan" }));
    assert!(matches!(&r, Called::Refused(m) if m.contains("`text` is required")), "{r:?}");
    // The user types in it: the agent can't replace it any more.
    tile.type_into("editor/draft/text", "Plan\n- a\n- b\n- mine");
    let r = tile.call("claude", None, Some("plan"), json!({ "text": "Plan\n- c" }));
    assert!(matches!(&r, Called::Refused(m) if m.contains("The user changed “plan”")), "{r:?}");
    assert_eq!(tile.field_text("editor/draft/text"), "Plan\n- a\n- b\n- mine");
    assert!(tile.storage().is_empty(), "still a draft, not saved");
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
    write(&mut tile, "api\nhandle 429s");
    write(&mut tile, "web: login flickers\non reload");
    let editor = "\
box
  box #editor
    box #head
      button \"\" #back label=\"All notes\" [click]
      text \"web: login flickers\"
      text \"Editing…\"
      tabs [*Edit, Preview] #mode [select]
      box #send-at
        button \"\" #send label=\"Send to an agent\" [click]
      button \"\" #trash label=\"Delete note\" [click]
    box #n2
      input placeholder=\"Title on the first line, then anything\" text=\"web: login flickers\\non reload\" multiline focused #text [change, submit]
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
          box
            text \"web: login flickers\"
            text \"now\"
          text \"on reload\"
        button \"\" #delete label=\"Delete note\" [click]
      box #n1 [click]
        box
          box
            text \"api\"
            text \"now\"
          text \"handle 429s\"
        button \"\" #delete label=\"Delete note\" [click]
";
    assert_eq!(tile.text_snapshot(), list);
}

#[test]
fn preview_draws_the_note_and_ticks_its_tasks() {
    let mut tile = notes();
    write(&mut tile, "Plan\n- [ ] one\n- [x] two");
    tile.select_tab("editor/head/mode", "Preview");
    let snap = tile.text_snapshot();
    assert!(snap.contains("markdown \"Plan\" #markdown [toggle]"), "{snap}");
    assert!(tile.find_key("editor/n1/text").is_none(), "no editor in Preview");
    // Its box ticked: the text changes and is saved at once.
    tile.toggle_task("editor/p1/markdown", 1);
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("Plan\n- [x] one\n- [x] two"));
    tile.select_tab("editor/head/mode", "Edit");
    assert_eq!(tile.field_text("editor/n1/text"), "Plan\n- [x] one\n- [x] two");
}

#[test]
fn cmd_slash_switches_edit_and_preview_while_typing() {
    let mut tile = notes();
    write(&mut tile, "Plan\n- [ ] one");
    assert_eq!(tile.focused(), Some("editor/n1/text"));
    tile.press("cmd-/");
    assert!(tile.find_key("editor/n1/text").is_none(), "Preview: {}", tile.text_snapshot());
    assert!(tile.text_snapshot().contains("markdown \"Plan\""), "{}", tile.text_snapshot());
    // Saved before it is shown.
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("Plan\n- [ ] one"));
    tile.press("cmd-/");
    assert_eq!(tile.focused(), Some("editor/n1/text"), "back in the editor, typing");
}

/// Notes with web (Claude Code) and api (Codex) and a plain terminal, and
/// the grants Send needs.
fn notes_with_agents() -> Session {
    let mut codex = pitwall_extension_test::agent("a2", "api");
    codex.kind = "codex".into();
    codex.kind_name = "Codex".into();
    let mut shell = pitwall_extension_test::agent("t1", "zsh");
    shell.kind = "shell".into();
    Harness::build(env!("CARGO_MANIFEST_DIR")).grant("agents.read").grant("agents.queue").agent(pitwall_extension_test::agent("a1", "web")).agent(codex).agent(shell).open("main")
}

#[test]
fn send_puts_the_note_in_an_agents_next_up() {
    let mut tile = notes_with_agents();
    write(&mut tile, "web: the login page flickers\non reload, only in Safari");
    tile.click_label("Send to an agent");
    let snap = tile.text_snapshot();
    assert!(snap.contains("Send to an agent") && snap.contains("\"web\"") && snap.contains("Claude Code · idle"), "{snap}");
    assert!(snap.contains("Codex · idle"), "{snap}");
    assert!(!snap.contains("zsh"), "plain terminals take no notes: {snap}");
    tile.click_key("a-a1");
    assert_eq!(tile.queued(), [pitwall_extension_test::Queued { agent: "a1".into(), text: "web: the login page flickers\non reload, only in Safari".into() }]);
    assert!(tile.toasts().iter().any(|(t, _)| t == "In web's Next up, waiting for your Send"), "{:?}", tile.toasts());
    assert!(tile.find_key("picker").is_none(), "closed after sending");
}

#[test]
fn a_section_can_be_sent_alone() {
    let mut tile = notes_with_agents();
    write(&mut tile, "Plan\n## Today\n- rebase api\n## Tomorrow\n- release 0.9");
    tile.click_label("Send to an agent");
    tile.select_option("what", "§ Tomorrow");
    tile.click_key("a-a2");
    assert_eq!(tile.queued()[0].text, "## Tomorrow\n- release 0.9");
}

#[test]
fn too_long_a_note_is_refused_before_sending() {
    let mut tile = notes_with_agents();
    write(&mut tile, &format!("Log\n{}", "x".repeat(5205)));
    tile.click_label("Send to an agent");
    assert!(tile.text_snapshot().contains("Too long to send: 5 209 of 4 000 characters. Make it shorter, or split it under headings."), "{}", tile.text_snapshot());
    assert!(!tile.find_key("a-a1").unwrap().listens(pitwall_extension_test::Listen::Click), "can't be clicked");
    assert!(tile.queued().is_empty());
}

#[test]
fn pitwalls_refusal_shows_in_the_picker() {
    let mut tile = notes_with_agents();
    write(&mut tile, "Ping");
    for _ in 0..3 {
        tile.click_label("Send to an agent").click_key("a-a1");
    }
    tile.click_label("Send to an agent").click_key("a-a1");
    assert_eq!(tile.queued().len(), 3);
    assert!(tile.text_snapshot().contains("web already has Notes' items waiting"), "{}", tile.text_snapshot());
}

#[test]
fn without_agents_queue_send_says_so_and_notes_still_work() {
    let mut tile = Harness::build(env!("CARGO_MANIFEST_DIR")).grant("agents.read").agent(pitwall_extension_test::agent("a1", "web")).open("main");
    write(&mut tile, "Ping");
    tile.click_label("Send to an agent").click_key("a-a1");
    assert!(tile.queued().is_empty());
    assert!(tile.text_snapshot().contains("Notes may not put text in an agent's Next up"), "{}", tile.text_snapshot());
    assert_eq!(stored_text(&tile, 1).as_deref(), Some("Ping"));
}

#[test]
fn without_the_grants_send_says_what_to_allow() {
    let mut tile = notes();
    write(&mut tile, "Ping");
    tile.click_label("Send to an agent");
    assert!(tile.text_snapshot().contains("Notes can't see your agents yet."), "{}", tile.text_snapshot());
    assert!(tile.find_key("settings").is_some());
}
