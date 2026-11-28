//! The Setup view on an extension's page (docs/spec/tile-sdk/ui.md
//! §3.12), headless (GPUI's test platform), with the tile host's hello
//! fixture, whose manifest names one: drawn by its component between the
//! header and the permissions, in place of the form drawn from the
//! manifest; a skeleton while it is off; "Setup stopped" with Restart
//! after its component stops; let go when the page closes. The same
//! extension without a Setup view keeps its generated form.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;

use super::super::view::SettingsView;
use crate::extensions::catalog;
use crate::tiles::TileView;

const WAIT: Duration = Duration::from_secs(60);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/hello")
}

/// The hello fixture in `exts` as `id`; `setup: false` without its Setup
/// view (and named after `id`).
fn put(exts: &Path, id: &str, setup: bool) {
    let to = exts.join(id);
    std::fs::create_dir_all(to.join("assets")).unwrap();
    for f in ["tile.wasm", "input.schema.json"] {
        std::fs::copy(fixture().join(f), to.join(f)).unwrap();
    }
    for e in std::fs::read_dir(fixture().join("assets")).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_file() {
            std::fs::copy(e.path(), to.join("assets").join(e.file_name())).unwrap();
        }
    }
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(fixture().join("extension.json")).unwrap()).unwrap();
    if !setup {
        m["contributes"].as_object_mut().unwrap().remove("setup");
    }
    m["id"] = id.into();
    std::fs::write(to.join("extension.json"), serde_json::to_vec_pretty(&m).unwrap()).unwrap();
}

/// Pitwall with data folder `root`, `on` turned on, and the Extensions
/// tile's page in a window.
fn open<'a>(root: &Path, on: &str, cx: &'a mut TestAppContext) -> (Entity<SettingsView>, &'a mut VisualTestContext) {
    let root = root.to_path_buf();
    // The component runs on its own thread: real time passes.
    cx.executor().allow_parking();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::init(Some(root.clone()), cx);
        catalog::set_bundle(None, cx);
        cx.set_global(crate::tiles::DataDir(root));
        crate::settings::data::set_page(cx, Page::Extensions);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension(on, true));
    });
    let (v, cx) = cx.add_window_view(|_, cx| SettingsView::extensions_surface(cx));
    cx.simulate_resize(gpui::size(gpui::px(1280.), gpui::px(900.)));
    until(cx, "it found", |cx| cx.update(|_, cx| crate::extensions::manifest_of(on, cx).is_some()));
    (v, cx)
}

fn frame(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn until(cx: &mut VisualTestContext, what: &str, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let end = Instant::now() + WAIT;
    loop {
        frame(cx);
        if done(cx) {
            return;
        }
        assert!(Instant::now() < end, "no {what} within {WAIT:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn held(v: &Entity<SettingsView>, cx: &mut VisualTestContext) -> Option<(String, Entity<TileView>)> {
    v.read_with(cx, |v, _| v.setup_view_shown())
}

fn drawn(v: &Entity<SettingsView>, text: &str, cx: &mut VisualTestContext) -> bool {
    cx.debug_bounds("extension-setup-view").is_some() && held(v, cx).is_some_and(|(_, t)| t.read_with(cx, |t, _| t.texts().iter().any(|x| x.contains(text))))
}

/// Its page: the header, its Setup view drawn by its component, then the
/// permissions; no form drawn from the manifest. Off: a skeleton, and
/// nothing held; back to the list: let go.
#[gpui::test]
fn the_setup_view_shows_on_its_page(cx: &mut TestAppContext) {
    let t = TempDir::new("setup-view");
    let root = t.path().join("data");
    put(&root.join("extensions"), "hello", true);
    let (v, cx) = open(&root, "hello", cx);
    v.update(cx, |v, cx| v.show_extension_detail(Some("hello"), cx));
    frame(cx);
    assert!(cx.debug_bounds("extension-setup-hello").is_some(), "its frame is drawn at once");
    until(cx, "its Setup view drawn", |cx| drawn(&v, "Hello greets you", cx));
    assert_eq!(held(&v, cx).map(|(id, t)| (id, t.read_with(cx, |t, _| t.tile().to_string()))), Some(("hello".into(), "setup".into())));
    assert!(cx.debug_bounds("extension-setup-skeleton").is_none());
    assert!(cx.debug_bounds("extension-checklist").is_none(), "no form drawn from the manifest");
    let setup = cx.debug_bounds("extension-setup-hello").unwrap();
    let perms = cx.debug_bounds("extension-permissions").expect("its permissions");
    let card = cx.debug_bounds("extension-card-hello").unwrap();
    assert!(card.top() < setup.top() && setup.bottom() <= perms.top(), "header, Setup view, permissions: {card:?} {setup:?} {perms:?}");
    // Its height is its content's.
    assert!(f32::from(setup.size.height) < super::setup_ui::SETUP_MAX_HEIGHT, "{setup:?}");
    assert!(f32::from(setup.size.height) > 60., "{setup:?}");
    // Off: a skeleton; its component let go.
    cx.update(|_, cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", false)));
    until(cx, "the skeleton", |cx| cx.debug_bounds("extension-setup-skeleton").is_some());
    assert!(held(&v, cx).is_none());
    // On again: drawn again; back to the list, let go.
    cx.update(|_, cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true)));
    until(cx, "its Setup view drawn again", |cx| drawn(&v, "Hello greets you", cx));
    v.update(cx, |v, cx| v.show_extension_detail(None, cx));
    frame(cx);
    assert!(held(&v, cx).is_none());
    until(cx, "its component let go", |cx| cx.update(|_, cx| crate::tiles::tiles(cx).read(cx).get("hello").is_none()));
}

/// Its component stopped (a tile of it crashed): "Setup stopped" with
/// Restart, which brings the Setup view back.
#[gpui::test]
fn a_stopped_setup_view_offers_restart(cx: &mut TestAppContext) {
    let t = TempDir::new("setup-stopped");
    let root = t.path().join("data");
    put(&root.join("extensions"), "hello", true);
    let (v, cx) = open(&root, "hello", cx);
    v.update(cx, |v, cx| v.show_extension_detail(Some("hello"), cx));
    until(cx, "its Setup view drawn", |cx| drawn(&v, "Hello greets you", cx));
    let main = cx.update(|_, cx| cx.new(|cx| TileView::new("hello", "main", cx)));
    until(cx, "its tile's frame", |cx| main.read_with(cx, |t, _| t.texts().iter().any(|x| x.contains("Clicked 0 times"))));
    main.read_with(cx, |t, cx| t.click("test/panic", pitwall_tile_host::ui::MouseButton::Left, pitwall_tile_host::ui::Modifiers::empty(), 1, pitwall_tile_host::ui::Point { x: 0., y: 0. }, cx));
    until(cx, "Setup stopped", |cx| cx.debug_bounds("extension-setup-stopped").is_some());
    assert!(cx.debug_bounds("extension-setup-view").is_none());
    cx.update(|_, cx| crate::tiles::tiles(cx).update(cx, |t, cx| t.restart("hello", cx)));
    until(cx, "its Setup view drawn again", |cx| drawn(&v, "Hello greets you", cx));
    drop(main);
}

/// An extension without a Setup view: its page has no Setup frame, and
/// keeps the form drawn from its manifest (until 8c4 removes it).
#[gpui::test]
fn without_a_setup_view_the_page_keeps_its_form(cx: &mut TestAppContext) {
    let t = TempDir::new("setup-none");
    let root = t.path().join("data");
    put(&root.join("extensions"), "plain", false);
    let (v, cx) = open(&root, "plain", cx);
    v.update(cx, |v, cx| v.show_extension_detail(Some("plain"), cx));
    frame(cx);
    assert!(cx.debug_bounds("extension-card-plain").is_some());
    assert!(cx.debug_bounds("extension-setup-plain").is_none());
    assert!(cx.debug_bounds("extension-checklist").is_some(), "its generated form");
    assert!(cx.debug_bounds("extension-permissions").is_some());
    assert!(held(&v, cx).is_none());
}
