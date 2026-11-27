//! Screenshots of the tile host's services fixture drawn by the app,
//! headless (GPUI's test platform, no window shown): the permission screen
//! with `net` and its hosts, `notify`, `show` and the `sql` size; its card
//! with Show in toolbar; and its toolbar button with a badge and its
//! popover in the top bar, dark and light. Run by hand:
//!
//! `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app --lib services_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{px, size, AnyWindowHandle, AppContext, Entity, HeadlessAppContext};
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;
use pitwall_tile_host::ui::{BadgeTone, ToolbarBadge as Badge};
use pitwall_tile_host::ToolbarRequest;

use super::State;
use crate::extensions::catalog;
use crate::main_screen::MainScreen;
use crate::settings::view::SettingsView;

const WAIT: Duration = Duration::from_secs(120);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/services")
}

#[test]
#[ignore]
fn services_shots() {
    use std::sync::Arc;
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("services-shots");
    let root = t.path().join("data");
    std::fs::create_dir_all(root.join("extensions")).unwrap();
    // In the extensions folder, as a Git or npm install leaves it (a folder
    // install refuses an extension that runs a program).
    let src = root.join("extensions").join("services");
    std::fs::create_dir_all(&src).unwrap();
    for e in std::fs::read_dir(fixture()).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), src.join(e.file_name())).unwrap();
    }
    let text: Arc<dyn gpui::PlatformTextSystem> = Arc::new(gpui_apple::AppleTextSystem::new());
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(crate::kit::Assets), gpui_platform::current_headless_renderer);
    cx.allow_parking();
    let r2 = root.clone();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::init(Some(r2), cx);
        catalog::set_bundle(None, cx);
        cx.set_global(super::DataDir(root.clone()));
        crate::settings::data::set_page(cx, Page::Extensions);
        crate::main_screen::init(cx);
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
    let shot = |cx: &mut HeadlessAppContext, w: AnyWindowHandle, name: &str| {
        for (theme, light) in [("dark", false), ("light", true)] {
            cx.update(|cx| cx.set_global(if light { crate::theme::Theme::light() } else { crate::theme::Theme::dark() }));
            cx.run_until_parked();
            let _ = cx.update_window(w, |_, window, _| window.refresh());
            cx.run_until_parked();
            let img = cx.capture_screenshot(w).expect("rendered");
            img.save(out.join(format!("services-{name}-{theme}.png"))).expect("saved");
        }
    };

    // The permission screen; turned on.
    let sv = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sv2 = sv.clone();
    let shell = cx
        .open_window(size(px(900.), px(900.)), move |_, cx| {
            let v = cx.new(|cx| SettingsView::new(true, cx));
            *sv2.borrow_mut() = Some(v.clone());
            cx.new(|_| super::tests::Shell(v.into(), false))
        })
        .unwrap();
    let settings: Entity<SettingsView> = sv.borrow().clone().unwrap();
    settle(&mut cx, "it found", &mut |cx| crate::extensions::manifest_of("services", cx).is_some());
    cx.update(|cx| settings.update(cx, |v, cx| v.debug_permissions("services", cx)));
    cx.run_until_parked();
    cx.update(|cx| settings.read(cx).scroll_to_end());
    shot(&mut cx, shell.into(), "permissions");
    cx.update(|cx| settings.update(cx, |v, cx| v.debug_permissions_on(cx)));
    settle(&mut cx, "it on", &mut |cx| crate::ui_state::get(cx).extension_on("services"));
    cx.update(|cx| settings.read(cx).scroll_to_end());
    shot(&mut cx, shell.into(), "card");

    // The top bar: its button (shown by default), kept running for its
    // badge, with a count; then its popover.
    let store = cx.update(|cx| {
        cx.new(|_| {
            crate::agents::AgentStore::new(vec![
                crate::agents::tests::agent("api", "/work/alpha", "working", 1, true),
                crate::agents::tests::agent("web", "/work/alpha", "blocked", 2, true),
            ])
        })
    });
    let ms = std::rc::Rc::new(std::cell::RefCell::new(None));
    let ms2 = ms.clone();
    let main = cx
        .open_window(size(px(1100.), px(560.)), move |window, cx| {
            let s = cx.new(|cx| MainScreen::new(store.clone(), window, cx));
            *ms2.borrow_mut() = Some(s.clone());
            s
        })
        .unwrap();
    let screen: Entity<MainScreen> = ms.borrow().clone().unwrap();
    settle(&mut cx, "kept running", &mut |cx| super::tiles(cx).read(cx).get("services").is_some_and(|r| r.state == State::Running));
    cx.update(|cx| {
        crate::main_screen::toolbar_items::apply("services", ToolbarRequest::Badge { project: None, badge: Badge::Count(3), tone: BadgeTone::Attention }, cx);
        crate::main_screen::toolbar_items::apply("services", ToolbarRequest::Tooltip("3 builds failed".into()), cx);
    });
    shot(&mut cx, main.into(), "toolbar");
    let _ = cx.update_window(main.into(), |_, window, cx| screen.update(cx, |s, cx| s.press_toolbar("services", window, cx)));
    settle(&mut cx, "the popover", &mut |cx| super::tiles(cx).read(cx).get("services").is_some_and(|r| r.frames.contains_key(pitwall_tile_host::POPOVER_VIEW)));
    // Its opening animation done (it runs on the clock).
    std::thread::sleep(Duration::from_millis(400));
    shot(&mut cx, main.into(), "popover");
}
