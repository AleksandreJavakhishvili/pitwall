//! The C4 tile's diagrams in the docs (plan.md phase C), in Pitwall's tile
//! runtime: a project of the test's with this spec's own hla.md and the
//! shop fixture, read with `files.read`, followed with `files.watch`, and
//! linked to the diagrams agents show in the same project.

use pitwall_extension_test::{DrawOp, Harness, Session, UiRequest};
use serde_json::json;

const HLA: &str = include_str!("fixtures/c4-tile/hla.md");
const CACHE_LLD: &str = include_str!("fixtures/shop/docs/orders/cache-lld.md");

fn project() -> Harness {
    Harness::build(env!("CARGO_MANIFEST_DIR"))
        .project("c4")
        .project_file("docs/hla.md", HLA)
        .project_file("docs/orders/cache-lld.md", CACHE_LLD)
        .project_file("docs/notes.txt", "```mermaid\nflowchart LR\n  a --> b\n```\n")
        .project_file("node_modules/pkg/docs/hla.md", HLA)
        .project_file("README.md", "# Not in the path list\n\n```mermaid\nflowchart LR\n  x --> y\n```\n")
}

fn open(h: Harness) -> Session {
    let mut tile = h.open("main");
    tile.resize("main", 900., 600.);
    tile
}

fn drawn(tile: &Session) -> Vec<String> {
    tile.canvas("diagram")
        .ops
        .iter()
        .filter_map(|o| match o {
            DrawOp::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

fn draws(tile: &Session, text: &str) -> bool {
    drawn(tile).iter().any(|t| t == text)
}

fn shows(tile: &Session, text: &str) -> bool {
    tile.find_text(text).is_some() || tile.text_snapshot().contains(&format!("\"{text}\""))
}

#[test]
fn without_files_read_it_shows_no_docs() {
    let tile = open(project());
    assert!(shows(&tile, "No diagrams yet"), "{}", tile.text_snapshot());
    assert!(tile.text_snapshot().contains("to see the diagrams in its docs"), "{}", tile.text_snapshot());
    assert_eq!(tile.watches(), 0);
}

#[test]
fn it_lists_the_docs_diagrams_and_draws_the_first() {
    let tile = open(project().grant("files.read"));
    let snap = tile.text_snapshot();
    assert!(snap.contains("In c4's docs"), "{snap}");
    // The path list's files only: not README.md, a .txt or node_modules.
    assert!(snap.contains("docs/hla.md") && snap.contains("docs/orders/cache-lld.md"), "{snap}");
    assert!(!snap.contains("README.md") && !snap.contains("notes.txt") && !snap.contains("node_modules"), "{snap}");
    // hla.md's context diagram first, drawn; its line in the outline.
    assert!(draws(&tile, "C4 tile"), "{:?}", drawn(&tile));
    assert!(snap.contains("Context · line 22"), "{snap}");
    assert_eq!(tile.watches(), 1, "it follows the project");
}

#[test]
fn a_box_zooms_into_the_docs_container_diagram() {
    let mut tile = open(project().grant("files.read"));
    let c4 = tile.canvas("diagram").regions.iter().find(|r| r.label == "C4 tile").cloned().expect("the C4 tile box");
    assert_eq!(c4.tooltip.as_deref(), Some("C4 tile: click to open its diagram"));
    tile.click_region("C4 tile");
    assert!(draws(&tile, "tile.wasm"), "the containers: {:?}", drawn(&tile));
    tile.click_region("tile.wasm");
    assert!(drawn(&tile).iter().any(|t| t.contains("Scanner") || t.contains("Parser")), "the components: {:?}", drawn(&tile));
    tile.focus(true).press("backspace").press("backspace");
    assert!(draws(&tile, "Engineer"), "back to the context: {:?}", drawn(&tile));
}

#[test]
fn an_edit_on_disk_redraws_and_a_deleted_file_goes() {
    let mut tile = open(project().grant("files.read"));
    assert!(draws(&tile, "C4 tile"));
    let edited = HLA.replace("C4 tile [Software System]<br/>draws C4", "Diagram tile [Software System]<br/>draws C4");
    tile.write_project_file("docs/hla.md", edited);
    assert!(draws(&tile, "Diagram tile"), "redrawn: {:?}", drawn(&tile));
    // A new doc joins the list; a deleted one leaves it.
    tile.write_project_file("docs/new-lld.md", "# New\n\n## Flow\n\n```mermaid\nflowchart LR\n  a[\"Alpha [Container]\"] --> b[\"Beta [Container]\"]\n```\n");
    assert!(tile.text_snapshot().contains("docs/new-lld.md"), "{}", tile.text_snapshot());
    tile.remove_project_file("docs/new-lld.md");
    assert!(!tile.text_snapshot().contains("docs/new-lld.md"), "{}", tile.text_snapshot());
    // The open file deleted: the next docs diagram is drawn.
    tile.remove_project_file("docs/hla.md");
    assert!(!tile.text_snapshot().contains("docs/hla.md"), "{}", tile.text_snapshot());
    assert!(draws(&tile, "Orders API"), "cache-lld.md's: {:?}", drawn(&tile));
    // A renamed folder: read again.
    tile.rename_project_file("docs/orders", "docs/shop");
    assert!(tile.text_snapshot().contains("docs/shop/cache-lld.md"), "{}", tile.text_snapshot());
}

#[test]
fn before_and_after_in_a_doc_offer_the_changes() {
    let mut tile = open(project().grant("files.read").setting("paths", "docs/orders/*.md"));
    assert!(!tile.text_snapshot().contains("docs/hla.md"), "the path list is the setting's");
    // The Before diagram is first; the After has the Changes view.
    tile.click_key("doc1");
    assert!(tile.find_key("mode").is_some(), "{}", tile.text_snapshot());
    tile.choose("mode", "changes");
    assert!(shows(&tile, "Added"), "{}", tile.text_snapshot());
}

#[test]
fn a_shown_diagram_zooms_into_the_docs_of_its_project() {
    let mut tile = open(project().grant("files.read"));
    // The agent works in the project: its context diagram's "C4 tile"
    // box is detailed by the containers in hla.md.
    let context = "flowchart TB\n  me[\"Engineer [Person]\"]\n  c4[\"C4 tile [Software System]<br/>draws diagrams\"]\n  me -->|reads| c4\n";
    let c = tile.call("api", Some("show"), Some("mine"), json!({ "mermaid": context, "title": "Mine" }));
    assert!(c.is_ok() && c.text().contains("1 of its boxes open another diagram"), "{c:?}");
    tile.click_region("C4 tile");
    assert!(draws(&tile, "tile.wasm"), "hla.md's containers: {:?}", drawn(&tile));
}

#[test]
fn its_source_opens_at_its_line() {
    let mut tile = open(project().grant("files.read").grant("show"));
    tile.choose_menu_item("more", "Show the source");
    assert!(tile.text_snapshot().contains("docs/hla.md, line 22"), "{}", tile.text_snapshot());
    tile.choose_menu_item("more", "Open in the editor");
    let asked: Vec<_> = tile.ui_requests().iter().filter(|r| matches!(r, UiRequest::OpenFile { .. })).cloned().collect();
    let want = tile.project_dir().join("docs/hla.md");
    assert!(matches!(&asked[..], [UiRequest::OpenFile { path, line: Some(22) }] if std::path::Path::new(path) == want), "{asked:?} {want:?}");
}

#[test]
fn the_open_input_takes_a_docs_diagram() {
    let mut tile = open(project().grant("files.read"));
    let c = tile.call("api", None, None, json!({ "diagram": "docs/hla.md#2-containers" }));
    assert!(c.is_ok(), "{c:?}");
    assert!(draws(&tile, "tile.wasm"), "{:?}", drawn(&tile));
}

#[test]
fn a_doc_over_the_size_limit_is_left_out_and_said() {
    let big = format!("# Big\n\n```mermaid\nflowchart LR\n  a --> b\n```\n\n{}", "x".repeat(1 << 20));
    let tile = open(project().project_file("docs/big.md", big).grant("files.read"));
    let snap = tile.text_snapshot();
    assert!(snap.contains("A Markdown file over 1 MiB is left out."), "{snap}");
    assert!(!snap.contains("docs/big.md"), "{snap}");
    assert!(snap.contains("docs/hla.md"), "the rest is read: {snap}");
}
