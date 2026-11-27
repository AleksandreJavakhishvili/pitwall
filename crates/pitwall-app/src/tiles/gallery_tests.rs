//! The components (docs/spec/tile-sdk/ui.md §3.3) drawn by the app with
//! the hello extension's `gallery` tile, headless: a 100 000-row list
//! whose rows come as they come into sight, a table with its header, a
//! tree, tabs, a select, a menu button and a row's context menu, Markdown
//! and images. `gallery_shots` takes the screenshots (dark and light) and
//! times a jump through the long list:
//!
//! `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app gallery_shots -- --ignored --nocapture`

use std::time::{Duration, Instant};

use gpui::{px, size, Entity, TestAppContext, VisualTestContext};
use pitwall_core::testing::TempDir;

use super::tests::copy_fixture;
use super::TileView;

const WAIT: Duration = Duration::from_secs(60);
const LIST: &str = "panes/left/list-box/rows";
const TABLE: &str = "panes/left/table-box/files";
const TREE: &str = "panes/right/tree-box/tree";

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

fn rows(tile: &Entity<TileView>, list: &str, cx: &mut VisualTestContext) -> Vec<(u32, String)> {
    tile.read_with(cx, |t, cx| t.row_keys(list, cx))
}

#[gpui::test]
fn the_gallery_draws_every_component(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-gallery");
    let root = t.path().join("data");
    copy_fixture(&root.join("extensions/hello"));
    super::tests::init(&root, cx);
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true)));
    let (tile, cx) = cx.add_window_view(|_, cx| TileView::new("hello", "gallery", cx));
    cx.simulate_resize(size(px(900.), px(640.)));
    until(cx, "the list's first rows", |cx| rows(&tile, LIST, cx).iter().any(|(i, k)| *i == 0 && k == "r0"));
    // Only the rows in sight (and a page either side) were asked for.
    let kept = rows(&tile, LIST, cx).len();
    assert!((1..=192).contains(&kept), "{kept} rows kept of 100 000");
    for sel in ["tile-row-panes/left/list-box/rows/r0", "tile-col-panes/left/table-box/files-name", "tile-node-toolbar/tabs", "tile-node-toolbar/size", "tile-node-toolbar/Actions", "tile-node-panes/right/notes/markdown", "tile-node-panes/right/images/logo", "tile-node-panes/right/images/dot"] {
        until(cx, sel, |cx| cx.debug_bounds(sel).is_some());
    }
    until(cx, "the table's rows", |cx| rows(&tile, TABLE, cx).len() == 4);
    until(cx, "the tree's chevrons", |cx| cx.debug_bounds("tile-chevron-panes/right/tree-box/tree/src").is_some());
    let _ = TREE;
    // A jump far down the list: the rows there are asked for and drawn.
    tile.update(cx, |t, _| t.scroll_list_to(LIST, 50_000));
    until(cx, "row 50 000", |cx| cx.debug_bounds("tile-row-panes/left/list-box/rows/r50000").is_some());
    // The menu button's menu, drawn by Pitwall.
    let opened = tile.update(cx, |t, cx| t.open_menu_of("toolbar/Actions", gpui::point(px(400.), px(40.)), cx));
    assert!(opened);
    until(cx, "the menu", |cx| cx.debug_bounds("tile-menu-item-delete").is_some());
    assert_eq!(tile.read_with(cx, |t, _| t.open_menu()), Some(vec!["Copy".into(), "Refresh".into(), "Delete".into()]));
}

/// Screenshots of the gallery, dark and light, and how long a jump to the
/// middle of the 100 000 rows takes to draw (rows asked for, sent, drawn).
#[cfg(target_os = "macos")]
#[test]
#[ignore]
fn gallery_shots() {
    use gpui::{AnyWindowHandle, AppContext, HeadlessAppContext};
    use std::sync::Arc;
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("gallery-shots");
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
    let settle = |cx: &mut HeadlessAppContext, what: &str, done: &mut dyn FnMut(&mut gpui::App) -> bool| {
        let end = Instant::now() + WAIT;
        loop {
            cx.run_until_parked();
            if cx.update(|cx| done(cx)) {
                return;
            }
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let refresh = |cx: &mut HeadlessAppContext, w: AnyWindowHandle| {
        let _ = cx.update_window(w, |_, window, _| window.refresh());
        cx.run_until_parked();
    };
    let shot = |cx: &mut HeadlessAppContext, w: AnyWindowHandle, name: &str| {
        for (theme, light) in [("dark", false), ("light", true)] {
            cx.update(|cx| cx.set_global(if light { crate::theme::Theme::light() } else { crate::theme::Theme::dark() }));
            refresh(cx, w);
            refresh(cx, w);
            let img = cx.capture_screenshot(w).expect("rendered");
            img.save(out.join(format!("components-{name}-{theme}.png"))).expect("saved");
        }
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
    };
    let vt = std::rc::Rc::new(std::cell::RefCell::new(None));
    let vt2 = vt.clone();
    let win = cx
        .open_window(size(px(960.), px(700.)), move |_, cx| {
            let v = cx.new(|cx| TileView::new("hello", "gallery", cx));
            *vt2.borrow_mut() = Some(v.clone());
            cx.new(|_| super::tests::Shell(v.into(), true))
        })
        .unwrap();
    let w: AnyWindowHandle = win.into();
    let tile: Entity<TileView> = vt.borrow().clone().unwrap();
    let kept = |cx: &mut HeadlessAppContext, list: &str, key: &str| {
        let (l, k) = (list.to_string(), key.to_string());
        cx.update(|cx| tile.read(cx).row_keys(&l, cx).iter().any(|(_, have)| *have == k))
    };
    let ready = |cx: &mut HeadlessAppContext, list: &str, key: &str| {
        let end = Instant::now() + WAIT;
        while !kept(cx, list, key) {
            refresh(cx, w);
            assert!(Instant::now() < end, "no {key} in {list}");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    settle(&mut cx, "the frame", &mut |cx| !tile.read(cx).texts().is_empty());
    refresh(&mut cx, w);
    ready(&mut cx, LIST, "r0");
    ready(&mut cx, TABLE, "README.md");
    ready(&mut cx, TREE, "README.md");
    shot(&mut cx, w, "gallery");

    // A jump to the middle: from the scroll to the rows drawn.
    let t0 = Instant::now();
    cx.update(|cx| tile.read(cx).scroll_list_to(LIST, 50_000));
    ready(&mut cx, LIST, "r50000");
    refresh(&mut cx, w);
    let jump = t0.elapsed();
    // Drawing a frame with the rows there.
    let t1 = Instant::now();
    let n = 20;
    for _ in 0..n {
        refresh(&mut cx, w);
    }
    let draw = t1.elapsed() / n;
    println!("[components] jump to row 50 000 drawn in {jump:?}; a frame of the gallery then {draw:?} (headless, debug build)");

    // The menu button's menu and a row's context menu.
    cx.update(|cx| tile.update(cx, |t, cx| t.open_menu_of("toolbar/Actions", gpui::point(px(760.), px(52.)), cx)));
    shot(&mut cx, w, "menu");
    cx.update(|cx| tile.update(cx, |t, cx| t.close_menu(cx)));
    // The select open.
    cx.update(|cx| tile.update(cx, |t, cx| t.open_menu_of("toolbar/size", gpui::point(px(0.), px(0.)), cx)));
    shot(&mut cx, w, "select");
}
