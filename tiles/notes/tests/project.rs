//! Project notes (notes-files-lld.md) in Pitwall's tile runtime: the
//! `*.md` files in a test project's `notes/` folder, read with
//! `files.read`, saved with `files.write` after the version check,
//! followed when they change on disk, and Send with the note's project's
//! agents first.

use std::time::Duration;

use pitwall_extension_test::{agent, Harness, Session};

const SAVE_AFTER: Duration = Duration::from_secs(1);

fn shop() -> Harness {
    Harness::build(env!("CARGO_MANIFEST_DIR"))
        .project("shop")
        .project_file("notes/login-flicker.md", "Login flickers\non reload, only in Safari")
        .project_file("notes/old/archived.md", "In a sub-folder: not shown")
        .project_file("notes/todo.txt", "not Markdown")
        .project_file("README.md", "# Shop")
}

fn shows(tile: &Session, text: &str) -> bool {
    tile.find_text(text).is_some()
}

#[test]
fn without_files_read_the_project_group_explains_and_private_notes_work() {
    let mut tile = shop().open("main");
    assert!(shows(&tile, "Keep notes in this project's notes/ folder"), "{}", tile.text_snapshot());
    assert!(tile.find_key("allow").is_some());
    assert!(!shows(&tile, "Login flickers"));
    tile.click_label("New note").type_into("text", "Private thought");
    tile.advance(SAVE_AFTER);
    assert!(tile.stored_json::<serde_json::Value>("p/_/1").is_some(), "a private note");
}

#[test]
fn the_notes_folder_is_listed_and_a_note_opens() {
    let mut tile = shop().grant("files.read").open("main");
    let snap = tile.text_snapshot();
    assert!(snap.contains("In shop") && snap.contains("Login flickers") && snap.contains("on reload, only in Safari"), "{snap}");
    // Only the *.md files directly in notes/ (FR-3).
    assert!(!snap.contains("sub-folder") && !snap.contains("not Markdown") && !snap.contains("# Shop"), "{snap}");
    assert!(snap.contains("Private"), "{snap}");
    tile.click_text("Login flickers");
    assert!(tile.text_snapshot().contains("notes/login-flicker.md"), "{}", tile.text_snapshot());
    // Without files.write it reads, read only.
    assert!(shows(&tile, "Read only: allow “Change files in your projects” for Notes in Settings → Extensions to edit project notes."), "{}", tile.text_snapshot());
    assert_eq!(tile.watches(), 1);
}

#[test]
fn a_new_project_note_gets_a_file_named_from_its_title() {
    let mut tile = shop().grant("files.read").grant("files.write").open("main");
    tile.click_label("New note").type_into("text", "Checkout: 429s on /api\nretry with backoff");
    tile.advance(SAVE_AFTER);
    assert_eq!(tile.project_file("notes/checkout-429s-on-api.md").as_deref(), Some("Checkout: 429s on /api\nretry with backoff"));
    assert!(tile.stored_json::<serde_json::Value>("p/_/1").is_none(), "not private");
    // The name is kept when the title changes; Rename takes the new one.
    tile.type_into("text", "Checkout rate limits\nretry with backoff");
    tile.advance(SAVE_AFTER);
    assert_eq!(tile.project_file("notes/checkout-429s-on-api.md").as_deref(), Some("Checkout rate limits\nretry with backoff"));
    tile.click_key("rename");
    assert!(tile.project_file("notes/checkout-429s-on-api.md").is_none());
    assert_eq!(tile.project_file("notes/checkout-rate-limits.md").as_deref(), Some("Checkout rate limits\nretry with backoff"));
    // A second note of the same title is numbered.
    tile.click_label("All notes").click_label("New note").type_into("text", "Checkout rate limits\nagain");
    tile.advance(SAVE_AFTER);
    assert_eq!(tile.project_file("notes/checkout-rate-limits-2.md").as_deref(), Some("Checkout rate limits\nagain"));
}

#[test]
fn delete_moves_the_file_to_the_trash() {
    let mut tile = shop().grant("files.read").grant("files.write").open("main");
    tile.click_text("Login flickers");
    tile.click_key("trash").click_key("delete");
    assert!(tile.project_file("notes/login-flicker.md").is_none());
    assert_eq!(tile.trashed(), ["login-flicker.md"]);
    assert!(!shows(&tile, "Login flickers"));
}

#[test]
fn a_change_on_disk_reloads_a_note_without_edits() {
    let mut tile = shop().grant("files.read").grant("files.write").open("main");
    tile.click_text("Login flickers");
    tile.write_project_file("notes/login-flicker.md", "Login flickers\nfixed by the agent");
    assert_eq!(tile.field("text").text, "Login flickers\nfixed by the agent");
    // A new file shows up in the list, and a deleted one leaves it.
    tile.write_project_file("notes/new.md", "From an agent");
    tile.click_label("All notes");
    assert!(shows(&tile, "From an agent"), "{}", tile.text_snapshot());
    tile.remove_project_file("notes/new.md");
    assert!(!shows(&tile, "From an agent"), "{}", tile.text_snapshot());
}

#[test]
fn a_change_under_an_edit_asks_reload_keep_mine_or_a_copy() {
    let mut tile = shop().grant("files.read").grant("files.write").open("main");
    tile.click_text("Login flickers");
    tile.type_into("text", "Login flickers\nmy edit");
    // The agent writes the file before the save.
    tile.write_project_file("notes/login-flicker.md", "Login flickers\nthe agent's edit");
    tile.advance(SAVE_AFTER);
    assert!(shows(&tile, "Changed on disk while you were editing. Not saved until you choose."), "{}", tile.text_snapshot());
    assert_eq!(tile.project_file("notes/login-flicker.md").as_deref(), Some("Login flickers\nthe agent's edit"), "not written over");
    // Save as a copy: mine in a new file, the agent's in the original.
    tile.click_key("copy");
    assert_eq!(tile.project_file("notes/login-flickers-copy.md").as_deref(), Some("Login flickers\nmy edit"));
    assert_eq!(tile.project_file("notes/login-flicker.md").as_deref(), Some("Login flickers\nthe agent's edit"));
    // Keep mine writes over it.
    tile.click_label("All notes");
    let row = tile_row(&tile, "the agent's edit");
    tile.click_key(&row);
    tile.type_into("text", "Login flickers\nmine again");
    tile.write_project_file("notes/login-flicker.md", "Login flickers\nthe agent, again");
    tile.advance(SAVE_AFTER);
    tile.click_key("keep-mine");
    assert_eq!(tile.project_file("notes/login-flicker.md").as_deref(), Some("Login flickers\nmine again"));
    // Reload drops the edit.
    tile.type_into("text", "Login flickers\nthrown away");
    tile.write_project_file("notes/login-flicker.md", "Login flickers\nthe agent wins");
    tile.advance(SAVE_AFTER);
    tile.click_key("reload");
    assert_eq!(tile.field("text").text, "Login flickers\nthe agent wins");
    assert_eq!(tile.project_file("notes/login-flicker.md").as_deref(), Some("Login flickers\nthe agent wins"));
}

#[test]
fn a_file_deleted_under_an_edit_can_be_saved_again() {
    let mut tile = shop().grant("files.read").grant("files.write").open("main");
    tile.click_text("Login flickers");
    tile.type_into("text", "Login flickers\nkeep this");
    tile.remove_project_file("notes/login-flicker.md");
    assert!(shows(&tile, "Deleted on disk while you were editing."), "{}", tile.text_snapshot());
    tile.click_key("save-again");
    assert_eq!(tile.project_file("notes/login-flicker.md").as_deref(), Some("Login flickers\nkeep this"));
}

#[test]
fn send_lists_the_notes_project_agents_first() {
    let mut tile = shop().grant("files.read").grant("agents.read").grant("agents.queue").agent(agent("a1", "elsewhere")).agent(agent("a2", "here")).open("main");
    let home = tile.project_dir().to_string_lossy().into_owned();
    let mut here = agent("a2", "here");
    here.project = home;
    tile.set_agents(vec![agent("a1", "elsewhere"), here]);
    tile.click_text("Login flickers").click_label("Send to an agent");
    let snap = tile.text_snapshot();
    let (a, b) = (snap.find("#a-a2").expect("here"), snap.find("#a-a1").expect("elsewhere"));
    assert!(a < b, "the project's agent first: {snap}");
}

/// The key of the list row whose preview is `text`.
fn tile_row(tile: &Session, text: &str) -> String {
    let snap = tile.text_snapshot();
    let at = snap.find(text).expect("the row");
    let row = snap[..at].rfind("box #n").expect("its row");
    snap[row + 5..].split([' ', '\n']).next().unwrap().to_string()
}
