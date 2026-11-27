//! Canvases and the code and diff views in the app, headless (GPUI's test
//! platform), with the tile host's blocks fixture
//! (crates/pitwall-tile-host/tests/fixtures/blocks): the canvas and its
//! regions drawn, a region's click and the wheel reaching the component,
//! the code view showing the tile's text with its marks, and line clicks
//! coming back.
//!
//! `blocks_shots` (ignored) writes screenshots, headless:
//! `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app blocks_shots -- --ignored`

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use pitwall_core::testing::TempDir;
use pitwall_tile_host::ui;

use super::TileView;

const WAIT: Duration = Duration::from_secs(60);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/blocks")
}

fn copy_fixture(to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(fixture()).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
    }
}

fn init(root: &Path, cx: &mut TestAppContext) {
    let root = root.to_path_buf();
    cx.executor().allow_parking();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::code_view::register(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::init(Some(root.clone()), cx);
        crate::extensions::catalog::set_bundle(None, cx);
        cx.set_global(super::DataDir(root));
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("blocks", true));
    });
}

fn until(cx: &mut VisualTestContext, what: &str, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let end = Instant::now() + WAIT;
    loop {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < end, "no {what} within {WAIT:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn has_text(tile: &Entity<TileView>, text: &str, cx: &mut VisualTestContext) -> bool {
    tile.read_with(cx, |t, _| t.texts()).iter().any(|t| t.contains(text))
}

/// The diagram: its canvas and a box over each region drawn; a click on a
/// region and the wheel reach the component, which zooms.
#[gpui::test]
fn a_canvas_draws_with_its_regions(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-canvas");
    let root = t.path().join("data");
    copy_fixture(&root.join("extensions/blocks"));
    init(&root, cx);
    let (tile, cx) = cx.add_window_view(|_, cx| TileView::new("blocks", "diagram", cx));
    cx.simulate_resize(gpui::size(gpui::px(640.), gpui::px(480.)));
    until(cx, "the diagram", |cx| has_text(&tile, "Click a box to zoom to it", cx));
    assert!(cx.debug_bounds("tile-node-diagram").is_some(), "the canvas is drawn");
    for r in ["tile-region-diagram/web", "tile-region-diagram/api", "tile-region-diagram/db"] {
        assert!(cx.debug_bounds(r).is_some(), "{r} is over it");
    }
    // The API box sits right of the web app's, as the tile placed them.
    let web = cx.debug_bounds("tile-region-diagram/web").unwrap();
    let api = cx.debug_bounds("tile-region-diagram/api").unwrap();
    assert!(api.left() > web.right(), "{web:?} {api:?}");
    let sent = tile.read_with(cx, |t, cx| t.click_region("diagram", "api", cx));
    assert!(sent, "the region has a click handler");
    until(cx, "the selection", |cx| has_text(&tile, "Selected: API", cx));
    let zoomed = cx.debug_bounds("tile-region-diagram/api").unwrap();
    assert!(zoomed.size.width > api.size.width, "zoomed to the box: {api:?} → {zoomed:?}");
    // The wheel pans it.
    let sent = tile.read_with(cx, |t, cx| t.pointer("diagram", ui::PointerKind::Wheel(ui::Delta { x: 0., y: 30. }), ui::Point { x: 10., y: 10. }, cx));
    assert!(sent, "the canvas takes the wheel");
    until(cx, "the pan", |cx| cx.debug_bounds("tile-region-diagram/api").is_some_and(|b| (b.top() - zoomed.top() + gpui::px(30.)).abs() < gpui::px(0.5)));
}

/// The code view shows the tile's text with its marks, in Pitwall's own
/// code view; a click on a line number comes back as the tile's event.
#[gpui::test]
fn a_code_view_shows_the_tiles_text(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-code");
    let root = t.path().join("data");
    copy_fixture(&root.join("extensions/blocks"));
    init(&root, cx);
    let (tile, cx) = cx.add_window_view(|_, cx| TileView::new("blocks", "code", cx));
    cx.simulate_resize(gpui::size(gpui::px(640.), gpui::px(400.)));
    until(cx, "the code", |cx| tile.read_with(cx, |t, cx| t.code_view_of("code").is_some_and(|v| v.read(cx).has_content())));
    let view = tile.read_with(cx, |t, _| t.code_view_of("code")).unwrap();
    assert_eq!(view.read_with(cx, |v, _| v.path().map(str::to_string)), Some("text.rs".into()), "highlighted as Rust");
    assert_eq!(view.read_with(cx, |v, _| v.line_marks().len()), 3);
    assert!(cx.debug_bounds("tile-node-code").is_some());
    view.update(cx, |_, cx| cx.emit(crate::code_view::CodeViewEvent::LineClicked { side: crate::code_view::Side::New, line: 4 }));
    until(cx, "the line click", |cx| has_text(&tile, "Line 5 clicked", cx));
}

/// The diff view diffs the tile's two texts, and a unified diff.
#[gpui::test]
fn a_diff_view_diffs_the_tiles_texts(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-diff");
    let root = t.path().join("data");
    copy_fixture(&root.join("extensions/blocks"));
    init(&root, cx);
    let (tile, cx) = cx.add_window_view(|_, cx| TileView::new("blocks", "diff", cx));
    cx.simulate_resize(gpui::size(gpui::px(800.), gpui::px(400.)));
    until(cx, "the diff", |cx| tile.read_with(cx, |t, cx| t.code_view_of("diff").is_some_and(|v| v.read(cx).change_count() > 0)));
    let view = tile.read_with(cx, |t, _| t.code_view_of("diff")).unwrap();
    assert_eq!(view.read_with(cx, |v, _| v.layout()), crate::code_view::Layout::Split);
    let sent = tile.read_with(cx, |t, cx| t.click("bar/source", ui::MouseButton::Left, ui::Modifiers::empty(), 1, ui::Point { x: 1., y: 1. }, cx));
    assert!(sent);
    until(cx, "its hunk", |cx| view.read_with(cx, |v, _| v.hunks().is_some_and(|h| h.len() == 1)));
}

/// Screenshots of the blocks fixture drawn by the app, headless: the
/// diagram fitted, zoomed to a box, and the code and diff views, dark and
/// light.
#[cfg(target_os = "macos")]
#[test]
#[ignore]
fn blocks_shots() {
    use std::sync::Arc;

    use gpui::{px, size, AppContext, HeadlessAppContext};
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    // `PITWALL_C4_TILE=<folder>`: instead, the C4 demo tile built from
    // tiles/c4/examples/shop_tile.rs, in a folder with its manifest (id
    // `c4-demo`, tile `main`).
    let c4 = std::env::var("PITWALL_C4_TILE").ok().map(PathBuf::from);
    let ext: &'static str = if c4.is_some() { "c4-demo" } else { "blocks" };
    let t = TempDir::new("blocks-shots");
    let root = t.path().join("data");
    match &c4 {
        Some(dir) => {
            let to = root.join("extensions/c4-demo");
            std::fs::create_dir_all(&to).unwrap();
            for f in ["extension.json", "tile.wasm"] {
                std::fs::copy(dir.join(f), to.join(f)).unwrap();
            }
        }
        None => copy_fixture(&root.join("extensions/blocks")),
    }
    let text: Arc<dyn gpui::PlatformTextSystem> = Arc::new(gpui_apple::AppleTextSystem::new());
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(crate::kit::Assets), gpui_platform::current_headless_renderer);
    cx.allow_parking();
    let r2 = root.clone();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::code_view::register(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::init(Some(r2.clone()), cx);
        crate::extensions::catalog::set_bundle(None, cx);
        cx.set_global(super::DataDir(r2));
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension(ext, true));
    });
    let settle = |cx: &mut HeadlessAppContext, what: &str, done: &mut dyn FnMut(&mut gpui::App) -> bool| {
        let end = Instant::now() + WAIT;
        loop {
            cx.run_until_parked();
            if cx.update(|cx| done(cx)) {
                return;
            }
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let shot = |cx: &mut HeadlessAppContext, w: gpui::AnyWindowHandle, name: &str| {
        for (theme, light) in [("dark", false), ("light", true)] {
            cx.update(|cx| cx.set_global(if light { crate::theme::Theme::light() } else { crate::theme::Theme::dark() }));
            // Let the tile redraw for the theme, then the window.
            for _ in 0..3 {
                cx.run_until_parked();
                let _ = cx.update_window(w, |_, window, _| window.refresh());
                std::thread::sleep(Duration::from_millis(30));
            }
            cx.run_until_parked();
            let img = cx.capture_screenshot(w).expect("rendered");
            img.save(out.join(format!("canvas-{name}-{theme}.png"))).expect("saved");
        }
    };
    let open = |cx: &mut HeadlessAppContext, tile: &'static str, w: f32, h: f32| {
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let s2 = slot.clone();
        let win = cx
            .open_window(size(px(w), px(h)), move |_, cx| {
                let v = cx.new(|cx| TileView::new(ext, tile, cx));
                *s2.borrow_mut() = Some(v.clone());
                cx.new(|_| super::tests::Shell(v.into(), true))
            })
            .unwrap();
        let v: Entity<TileView> = slot.borrow().clone().unwrap();
        (win, v)
    };
    let texts = |cx: &mut gpui::App, v: &Entity<TileView>, s: &str| v.read(cx).texts().iter().any(|t| t.contains(s));

    if c4.is_some() {
        let (w, v) = open(&mut cx, "main", 960., 640.);
        settle(&mut cx, "the context diagram", &mut |cx| texts(cx, &v, "System context"));
        shot(&mut cx, w.into(), "c4-context");
        // Click to zoom: the Shop box opens its containers.
        cx.update(|cx| v.read(cx).click_region("diagram", "shop", cx));
        settle(&mut cx, "the containers", &mut |cx| texts(cx, &v, "Shop containers"));
        shot(&mut cx, w.into(), "c4-containers");
        return;
    }

    let (w, v) = open(&mut cx, "diagram", 720., 520.);
    settle(&mut cx, "the diagram", &mut |cx| texts(cx, &v, "Click a box"));
    shot(&mut cx, w.into(), "diagram");
    cx.update(|cx| v.read(cx).click_region("diagram", "api", cx));
    settle(&mut cx, "the zoom", &mut |cx| texts(cx, &v, "Selected: API"));
    cx.update(|cx| v.read(cx).click_region("diagram", "db", cx));
    settle(&mut cx, "the zoom", &mut |cx| texts(cx, &v, "Selected: Database"));
    shot(&mut cx, w.into(), "zoomed");

    let (w, v) = open(&mut cx, "code", 720., 420.);
    settle(&mut cx, "the code", &mut |cx| v.read(cx).code_view_of("code").is_some_and(|c| c.read(cx).has_content()));
    std::thread::sleep(Duration::from_millis(300));
    shot(&mut cx, w.into(), "code");

    let (w, v) = open(&mut cx, "diff", 900., 420.);
    settle(&mut cx, "the diff", &mut |cx| v.read(cx).code_view_of("diff").is_some_and(|c| c.read(cx).change_count() > 0));
    std::thread::sleep(Duration::from_millis(300));
    shot(&mut cx, w.into(), "diff");
    cx.update(|cx| v.read(cx).click("bar/layout", ui::MouseButton::Left, ui::Modifiers::empty(), 1, ui::Point { x: 1., y: 1. }, cx));
    settle(&mut cx, "inline", &mut |cx| v.read(cx).code_view_of("diff").is_some_and(|c| c.read(cx).layout() == crate::code_view::Layout::Unified));
    std::thread::sleep(Duration::from_millis(300));
    shot(&mut cx, w.into(), "diff-inline");
}
