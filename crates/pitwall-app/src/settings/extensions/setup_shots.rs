//! Screenshots of an extension's page with its Setup view (docs/spec/
//! tile-sdk/ui.md §3.12), headless (GPUI's test platform, no window
//! shown), dark and light, in the Extensions tile: the hello fixture's
//! page with its Setup view drawn, its skeleton while it is off, "Setup
//! stopped", and the same extension without a Setup view (header and
//! permissions, with the form drawn from its manifest until 8c4). Run by
//! hand:
//!
//! `PITWALL_SETUP_SHOTS=<folder> cargo test -p pitwall-app --lib setup_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{px, size, AnyWindowHandle, AppContext, Entity, HeadlessAppContext};
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;

use super::super::view::SettingsView;
use crate::extensions::catalog;
use crate::tiles::{State, TileView};

const WAIT: Duration = Duration::from_secs(120);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/hello")
}

/// The hello fixture in `exts` as `id` (named `name`); without its Setup
/// view unless `setup`.
fn put(exts: &Path, id: &str, name: &str, setup: bool) {
    let to = exts.join(id);
    std::fs::create_dir_all(to.join("assets")).unwrap();
    for f in ["tile.wasm", "input.schema.json", "assets/logo.svg"] {
        std::fs::copy(fixture().join(f), to.join(f)).unwrap();
    }
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(fixture().join("extension.json")).unwrap()).unwrap();
    if !setup {
        m["contributes"].as_object_mut().unwrap().remove("setup");
    }
    m["id"] = id.into();
    m["name"] = name.into();
    m["description"] = "A counter, a greeting and a text field".into();
    std::fs::write(to.join("extension.json"), serde_json::to_vec_pretty(&m).unwrap()).unwrap();
}

struct Shell(gpui::AnyView);

impl gpui::Render for Shell {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{div, prelude::*};
        let t = crate::theme::theme(cx).clone();
        div().size_full().flex().flex_col().bg(t.bg).text_color(t.text).child(self.0.clone())
    }
}

#[test]
#[ignore]
fn setup_shots() {
    let Ok(out) = std::env::var("PITWALL_SETUP_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("setup-shots");
    let root = t.path().join("data");
    put(&root.join("extensions"), "hello", "Hello", true);
    put(&root.join("extensions"), "plain", "Plain hello", false);
    let text: Arc<dyn gpui::PlatformTextSystem> = Arc::new(gpui_apple::AppleTextSystem::new());
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(crate::kit::Assets), gpui_platform::current_headless_renderer);
    cx.allow_parking();
    let r2 = root.clone();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::init(Some(r2.clone()), cx);
        catalog::set_bundle(None, cx);
        cx.set_global(crate::tiles::DataDir(r2));
        crate::settings::data::set_page(cx, Page::Extensions);
        crate::ui_state::update(cx, |s| *s = s.clone().set_extension("hello", true).set_extension("plain", true));
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
    let sv = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sv2 = sv.clone();
    let win = cx
        .open_window(size(px(1280.), px(860.)), move |_, cx| {
            let v = cx.new(SettingsView::extensions_surface);
            *sv2.borrow_mut() = Some(v.clone());
            cx.new(|_| Shell(v.into()))
        })
        .unwrap();
    let settings: Entity<SettingsView> = sv.borrow().clone().unwrap();
    let w: AnyWindowHandle = win.into();
    let shoot = |cx: &mut HeadlessAppContext, name: &str| {
        for (theme, light) in [("dark", false), ("light", true)] {
            cx.update(|cx| cx.set_global(if light { crate::theme::Theme::light() } else { crate::theme::Theme::dark() }));
            for _ in 0..3 {
                cx.run_until_parked();
                let _ = cx.update_window(w, |_, window, _| window.refresh());
            }
            cx.run_until_parked();
            let img = cx.capture_screenshot(w).expect("rendered");
            img.save(out.join(format!("setup-{name}-{theme}.png"))).expect("saved");
        }
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
    };
    settle(&mut cx, "both found", &mut |cx| crate::extensions::manifest_of("hello", cx).is_some() && crate::extensions::manifest_of("plain", cx).is_some());
    let drawn = |cx: &mut gpui::App, s: &Entity<SettingsView>| s.read(cx).setup_view_shown().is_some_and(|(_, v)| v.read(cx).texts().iter().any(|t| t.contains("Hello greets you"))) && crate::tiles::tiles(cx).read(cx).get("hello").is_some_and(|r| r.frames.contains_key("setup"));

    // Its page, its Setup view drawn.
    cx.update(|cx| settings.update(cx, |v, cx| v.show_extension_detail(Some("hello"), cx)));
    settle(&mut cx, "its Setup view", &mut |cx| drawn(cx, &settings));
    shoot(&mut cx, "page");

    // Its component stopped.
    let main = cx.update(|cx| cx.new(|cx| TileView::new("hello", "main", cx)));
    settle(&mut cx, "its tile's frame", &mut |cx| main.read(cx).texts().iter().any(|t| t.contains("Clicked")));
    cx.update(|cx| main.read(cx).click("test/panic", pitwall_tile_host::ui::MouseButton::Left, pitwall_tile_host::ui::Modifiers::empty(), 1, pitwall_tile_host::ui::Point { x: 0., y: 0. }, cx));
    settle(&mut cx, "it stopped", &mut |cx| crate::tiles::tiles(cx).read(cx).get("hello").is_some_and(|r| matches!(r.state, State::Stopped(_))));
    shoot(&mut cx, "stopped");
    drop(main);
    cx.update(|cx| crate::tiles::tiles(cx).update(cx, |t, cx| t.restart("hello", cx)));
    settle(&mut cx, "its Setup view again", &mut |cx| drawn(cx, &settings));

    // Off: the skeleton.
    cx.update(|cx| crate::ui_state::update(cx, |s| *s = s.clone().set_extension("hello", false)));
    settle(&mut cx, "let go", &mut |cx| settings.read(cx).setup_view_shown().is_none());
    shoot(&mut cx, "off");

    // Without a Setup view.
    cx.update(|cx| settings.update(cx, |v, cx| v.show_extension_detail(Some("plain"), cx)));
    cx.run_until_parked();
    shoot(&mut cx, "none");
}
