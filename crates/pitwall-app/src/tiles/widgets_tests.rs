//! The form controls, feedback, layout, data display and charts (ui.md
//! §3.10) drawn by the app with the hello extension's `gallery` tile,
//! headless: each shown alone (the content `{"component": "<name>"}`),
//! drawn, and what the user does sent back. `widgets_shots` takes each
//! one's screenshot, dark and light, for the docs:
//!
//! `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app widgets_shots -- --ignored --nocapture`
//!
//! It saves `components-<name>-<theme>.png`; the docs show those from
//! `docs/reference/extensions/images/components/`.

use std::time::{Duration, Instant};

use gpui::{px, size, App, Entity, TestAppContext, VisualTestContext};
use pitwall_core::testing::TempDir;
use pitwall_tile_host::ui;

use super::tests::copy_fixture;
use super::TileView;

const WAIT: Duration = Duration::from_secs(60);

/// Every component, as the gallery names them.
const ALL: &[&str] = &[
    "checkbox", "switch", "radio-group", "segmented", "slider", "number-field", "search-field", "form-row", "disabled", "progress", "spinner", "skeleton", "empty-state", "banner", "badge", "chip", "kbd", "tooltip", "split", "disclosure", "card", "divider", "breadcrumbs", "toolbar-row", "popover", "confirm-sheet", "key-values", "avatar", "relative-time", "copyable", "link", "sparkline", "bar-chart", "line-chart", "status-timeline",
];

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

/// The gallery's view shows component `name` alone.
fn show(name: &str, cx: &mut App) {
    let tiles = super::tiles(cx);
    let r = tiles.read(cx);
    let json = serde_json::json!({ "component": name }).to_string();
    r.get("hello").and_then(|r| r.handle.as_ref()).expect("running").set_content("gallery", ui::Content { id: None, json: Some(json), bytes: None, call: None, tool: None, caller: None });
}

/// A selector for `debug_bounds` (which keeps it).
fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn texts(tile: &Entity<TileView>, cx: &mut VisualTestContext) -> Vec<String> {
    tile.read_with(cx, |t, _| t.texts())
}

#[gpui::test]
fn each_component_is_drawn_and_answers(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-widgets");
    let root = t.path().join("data");
    copy_fixture(&root.join("extensions/hello"));
    super::tests::init(&root, cx);
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true)));
    let (tile, cx) = cx.add_window_view(|_, cx| TileView::new("hello", "gallery", cx));
    cx.simulate_resize(size(px(720.), px(480.)));
    until(cx, "the gallery", |cx| cx.debug_bounds("tile-node-pages").is_some());
    for name in ALL {
        cx.update(|_, cx| show(name, cx));
        let sel = leak(format!("tile-node-{name}"));
        until(cx, name, |cx| cx.debug_bounds(sel).is_some());
    }
    // Each kind is drawn by its own selector.
    for sel in ["tile-node-status-timeline/history", "tile-segment-status-timeline/history/404"] {
        until(cx, sel, |cx| cx.debug_bounds(sel).is_some());
    }

    // A confirm sheet over the tile: Enter confirms.
    cx.update(|_, cx| show("confirm-sheet", cx));
    let ask = "confirm-sheet/0/notes/ask";
    until(cx, "the sheet", |cx| cx.debug_bounds(leak(format!("tile-sheet-confirm-{ask}"))).is_some());
    assert!(tile.update(cx, |t, cx| t.component_key(ask, "enter", cx)));
    until(cx, "the notes deleted", |cx| texts(&tile, cx).iter().any(|t| t == "3 notes deleted"));
    assert!(cx.debug_bounds(leak(format!("tile-sheet-confirm-{ask}"))).is_none(), "the sheet closed");

    // A copy button writes the clipboard when used.
    cx.update(|_, cx| show("copyable", cx));
    let sha = "copyable/0/sha";
    until(cx, "the copy button", |cx| cx.debug_bounds(leak(format!("tile-copy-{sha}"))).is_some());
    assert!(tile.update(cx, |t, cx| t.component_click(sha, cx)));
    assert_eq!(cx.update(|_, cx| cx.read_from_clipboard()).and_then(|c| c.text()), Some("4f2a9c1e7b".into()));

    // A link without `show`: the tile hears the click.
    cx.update(|_, cx| show("link", cx));
    until(cx, "the links", |cx| cx.debug_bounds("tile-node-link/0/all").is_some());
    assert!(tile.update(cx, |t, cx| t.component_click("link/0/web", cx)));
    assert!(tile.update(cx, |t, cx| t.component_click("link/0/all", cx)));

    // A disclosure closes with Space; a popover goes with Esc.
    cx.update(|_, cx| show("disclosure", cx));
    until(cx, "the disclosure", |cx| cx.debug_bounds("tile-node-disclosure/env").is_some());
    assert!(tile.update(cx, |t, cx| t.component_key("disclosure/env", "space", cx)));
    until(cx, "it closed", |cx| !texts(&tile, cx).iter().any(|t| t == "NODE_ENV") && cx.debug_bounds("tile-node-disclosure/env").is_some());
    cx.update(|_, cx| show("popover", cx));
    until(cx, "the popover", |cx| cx.debug_bounds("tile-node-popover/0/filters/pop").is_some());
    assert!(tile.update(cx, |t, cx| t.component_key("popover/0/filters/pop", "escape", cx)));
    until(cx, "the popover gone", |cx| cx.debug_bounds("tile-node-popover/0/filters/pop").is_none());
}

/// Each component's screenshot, dark and light.
#[cfg(target_os = "macos")]
#[test]
#[ignore]
fn widgets_shots() {
    use gpui::{AnyWindowHandle, AppContext, HeadlessAppContext};
    use std::sync::Arc;
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("widgets-shots");
    let root = t.path().join("data");
    copy_fixture(&root.join("extensions/hello"));
    let text: Arc<dyn gpui::PlatformTextSystem> = Arc::new(gpui_apple::AppleTextSystem::new());
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(crate::kit::Assets), gpui_platform::current_headless_renderer);
    cx.allow_parking();
    let r2 = root.clone();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::init(Some(r2.clone()), cx);
        crate::extensions::catalog::set_bundle(None, cx);
        cx.set_global(super::DataDir(r2));
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true));
    });
    let (w_px, h_px) = (640., 420.);
    let vt = std::rc::Rc::new(std::cell::RefCell::new(None));
    let vt2 = vt.clone();
    let win = cx
        .open_window(size(px(w_px), px(h_px)), move |_, cx| {
            let v = cx.new(|cx| TileView::new("hello", "gallery", cx));
            *vt2.borrow_mut() = Some(v.clone());
            cx.new(|_| super::tests::Shell(v.into(), true))
        })
        .unwrap();
    let w: AnyWindowHandle = win.into();
    let tile: Entity<TileView> = vt.borrow().clone().unwrap();
    let paths: Vec<String> = ALL.iter().map(|n| n.to_string()).collect();
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    cx.update(|cx| tile.update(cx, |t, _| t.probe(&refs)));
    let refresh = |cx: &mut HeadlessAppContext| {
        let _ = cx.update_window(w, |_, window, _| window.refresh());
        cx.run_until_parked();
    };
    let settle = |cx: &mut HeadlessAppContext, what: &str, done: &mut dyn FnMut(&mut App) -> bool| {
        let end = Instant::now() + WAIT;
        loop {
            refresh(cx);
            if cx.update(|cx| done(cx)) {
                return;
            }
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    settle(&mut cx, "the gallery", &mut |cx| !tile.read(cx).texts().is_empty());
    for name in ALL {
        cx.update(|cx| show(name, cx));
        let path = name.to_string();
        settle(&mut cx, name, &mut |cx| tile.read(cx).node_bounds(&path).is_some());
        // Let the frame's own size settle (fields, charts).
        refresh(&mut cx);
        refresh(&mut cx);
        for (theme, light) in [("dark", false), ("light", true)] {
            cx.update(|cx| cx.set_global(if light { crate::theme::Theme::light() } else { crate::theme::Theme::dark() }));
            refresh(&mut cx);
            refresh(&mut cx);
            let img = cx.capture_screenshot(w).expect("rendered");
            let scale = img.width() as f32 / w_px;
            // The sheet covers the tile: all of it.
            let b = match *name {
                "confirm-sheet" => gpui::Bounds::new(gpui::point(px(8.), px(8.)), gpui::size(px(w_px - 16.), px(h_px - 16.))),
                _ => cx.update(|cx| tile.read(cx).node_bounds(&path)).expect("drawn"),
            };
            let pad = 12.;
            let x = ((f32::from(b.origin.x) - pad).max(0.) * scale) as u32;
            let y = ((f32::from(b.origin.y) - pad).max(0.) * scale) as u32;
            let cw = (((f32::from(b.size.width) + 2. * pad) * scale) as u32).min(img.width() - x);
            let ch = (((f32::from(b.size.height) + 2. * pad) * scale) as u32).min(img.height() - y);
            image::imageops::crop_imm(&img, x, y, cw, ch).to_image().save(out.join(format!("components-{name}-{theme}.png"))).expect("saved");
        }
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
    }
}
