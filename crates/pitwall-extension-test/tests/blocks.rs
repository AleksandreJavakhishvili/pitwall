//! The harness on the committed blocks tile (crates/pitwall-tile-host/
//! tests/fixtures/blocks-tile, rebuilt by build-hello-tile.sh): a canvas
//! with clickable regions, zoom and pan; drag and drop inside a view; the
//! code and diff views with line clicks and the selection.

use std::path::{Path, PathBuf};

use pitwall_extension_test::{DrawOp, Harness, Paint, Side, Token};

fn blocks() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/blocks")
}

#[test]
fn a_diagram_zooms_pans_and_its_boxes_click() {
    let mut t = Harness::load(blocks()).open("diagram");
    t.resize("diagram", 640., 480.);
    let snap = t.text_snapshot();
    assert!(snap.contains("region \"API\" #api [click, hover]"), "{snap}");
    assert!(snap.contains("canvas ops="), "{snap}");
    assert!(snap.contains("label=\"Diagram of the shop: Web app, API and Database\""), "{snap}");
    assert!(snap.contains("[pointer, wheel]"), "{snap}");
    let c = t.canvas("diagram");
    // Theme colours, a dashed line, text in pixels.
    assert!(c.ops.iter().any(|o| matches!(o, DrawOp::Rect(r) if r.fill == Some(Paint::Token(Token::Surface2)))));
    assert!(c.ops.iter().any(|o| matches!(o, DrawOp::Path(p) if p.stroke.as_ref().is_some_and(|s| s.dash == [5., 4.]))));
    assert!(c.ops.iter().any(|o| matches!(o, DrawOp::Text(x) if x.text == "API" && x.size == 14.)));
    let fit = c.transform;
    assert!(fit.scale > 0.5 && fit.scale <= 1., "{fit:?}");

    // A click on a box selects it and zooms to it.
    t.click_region("API");
    assert!(t.find_text("Selected: API").is_some(), "{}", t.text_snapshot());
    let zoomed = t.canvas("diagram").transform;
    assert!(zoomed.scale > fit.scale, "{zoomed:?}");
    // Hover marks it.
    t.hover_region("Database", true);
    assert!(t.canvas("diagram").ops.iter().any(|o| matches!(o, DrawOp::Rect(r) if r.fill == Some(Paint::Token(Token::Surface3)))));

    // The wheel pans; ⌘-wheel and pinch zoom; dragging pans.
    let before = t.canvas("diagram").transform;
    t.wheel("diagram", 0., 40., 100., 100.);
    let after = t.canvas("diagram").transform;
    assert_eq!((after.dy, after.scale), (before.dy - 40., before.scale));
    t.pinch("diagram", 0.5, 100., 100.);
    assert!((t.canvas("diagram").transform.scale - after.scale * 1.5).abs() < 1e-3);
    let before = t.canvas("diagram").transform;
    t.drag_pointer("diagram", (50., 50.), (90., 70.));
    let after = t.canvas("diagram").transform;
    assert_eq!((after.dx - before.dx, after.dy - before.dy), (40., 20.));
    // The zoom buttons and Fit.
    t.click_label("Zoom in");
    assert!(t.canvas("diagram").transform.scale > after.scale);
    t.click_label("Fit");
    assert_eq!(t.canvas("diagram").transform, fit);
    assert!(t.find_text("Click a box to zoom to it").is_some());
}

#[test]
fn names_drag_onto_the_pinned_list() {
    let mut t = Harness::load(blocks()).open("diagram");
    let snap = t.text_snapshot();
    assert!(snap.contains("text \"API\" #api drag=box"), "{snap}");
    assert!(snap.contains("#pinned drop=box"), "{snap}");
    t.drag("foot/names/api", "foot/pinned");
    assert!(t.find_key("foot/pinned/api").is_some(), "{}", t.text_snapshot());
    t.drag("foot/names/db", "foot/pinned");
    assert!(t.find_key("foot/pinned/db").is_some());
    // Only what a target accepts, only onto a target.
    assert!(t.try_drag("foot/names/web", "bar/status").err().unwrap().contains("take no drops"));
    assert!(t.try_drag("bar/status", "foot/pinned").err().unwrap().contains("can't be dragged"));
}

#[test]
fn the_code_view_tells_line_clicks_and_the_selection() {
    let mut t = Harness::load(blocks()).open("code");
    let snap = t.text_snapshot();
    assert!(snap.contains("code-view language=rust lines=8 marks=3 #code [line-click, select]"), "{snap}");
    assert_eq!(t.code_view("code").marks.len(), 3);
    t.click_line("code", Side::New, 2);
    assert!(t.find_text("Line 3 clicked").is_some(), "{}", t.text_snapshot());
    t.select_code("code", Side::New, (1, 3), (1, 8));
    assert!(t.find_text("Selected: total").is_some(), "{}", t.text_snapshot());
}

#[test]
fn the_diff_view_takes_two_texts_or_a_unified_diff() {
    let mut t = Harness::load(blocks()).open("diff");
    assert!(t.text_snapshot().contains("diff-view texts side-by-side language=rust #diff [line-click]"), "{}", t.text_snapshot());
    t.click_line("diff", Side::Old, 0);
    assert!(t.find_text("Line 1 clicked").is_some());
    t.click_text("Inline").click_text("Unified diff");
    assert!(t.text_snapshot().contains("diff-view unified inline language=rust"), "{}", t.text_snapshot());
}
