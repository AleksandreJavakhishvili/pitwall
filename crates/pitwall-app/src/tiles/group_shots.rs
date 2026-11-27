//! Screenshots of the agents' questions group, headless (GPUI's test
//! platform, no window shown), with the tile host's questions fixture
//! ("Release desk") and a daemon server: its badge on an agent's sidebar
//! row, its question in the floating control, and what it queued in the
//! agent's Next up, marked with its name. Dark and light. Run by hand:
//!
//! `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app group_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{px, size, AnyWindowHandle, AppContext, HeadlessAppContext};
use pitwall_core::testing::TempDir;
use pitwall_tile_host::ui;

use super::group_tests;

const WAIT: Duration = Duration::from_secs(120);

#[test]
#[ignore]
fn group_shots() {
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("group-shots");
    let root = t.path().join("data");
    let (engine, server) = group_tests::server(&t);
    let text: Arc<dyn gpui::PlatformTextSystem> = Arc::new(gpui_apple::AppleTextSystem::new());
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(crate::kit::Assets), gpui_platform::current_headless_renderer);
    cx.allow_parking();
    let (e2, r2) = (engine.clone(), root.clone());
    let store = cx.update(|cx| {
        let store = group_tests::init(&r2, &e2, server, cx);
        crate::main_screen::init(cx);
        crate::extensions::follow(cx);
        // The user is away from Pitwall: the floating control may come up.
        cx.set_global(crate::float::InFront(false));
        crate::float::init(&store, Arc::new(|_, _, _| Ok(())), Arc::new(|_, _| Ok(())), Arc::new(|_, _| {}), Arc::new(|_, _, _| {}), cx);
        store
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
            img.save(out.join(format!("agents-group-{name}-{theme}.png"))).expect("saved");
        }
    };
    // The main window: web shown, Release desk opened beside it.
    let sc = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sc2 = sc.clone();
    let s2 = store.clone();
    let main = cx
        .open_window(size(px(1280.), px(760.)), move |window, cx| {
            let v = cx.new(|cx| crate::main_screen::MainScreen::new(s2, window, cx));
            *sc2.borrow_mut() = Some(v.clone());
            v
        })
        .unwrap();
    let screen: gpui::Entity<crate::main_screen::MainScreen> = sc.borrow().clone().unwrap();
    let w: AnyWindowHandle = main.into();
    let proj = t.path().join("api").to_string_lossy().to_string();
    let _ = cx.update_window(w, |_, window, cx| {
        screen.update(cx, |s, cx| {
            s.show_agent("a1", window, cx);
            s.demo_project_menu(&proj, window, cx);
            s.run_menu_label("Open Release desk", window, cx);
        })
    });
    settle(&mut cx, "the tile", &mut |cx| crate::main_screen::view_tile::get("questions/main", cx).is_some_and(|v| v.read(cx).tile().is_some()));
    let tile = cx.update(|cx| crate::main_screen::view_tile::get("questions/main", cx).and_then(|v| v.read(cx).tile().cloned())).unwrap();
    settle(&mut cx, "the agents", &mut |cx| tile.read(cx).texts().iter().any(|t| t == "web"));
    let web = engine.views().into_iter().find(|a| a.id == "a1").unwrap();
    settle(&mut cx, "the badge", &mut |cx| {
        cx.background_executor().advance_clock(Duration::from_millis(50));
        !crate::extensions::marks_for_agent(&web, cx).is_empty()
    });
    // Queued for web, the store following the engine.
    let ok = cx.update(|cx| tile.read(cx).click("agents/a-a1/queue", ui::MouseButton::Left, ui::Modifiers::empty(), 1, ui::Point { x: 1., y: 1. }, cx));
    assert!(ok);
    settle(&mut cx, "the queued item", &mut |_| engine.views().iter().any(|a| a.id == "a1" && !a.queue.is_empty()));
    let e3 = engine.clone();
    cx.update(|cx| store.update(cx, |s, cx| {
        s.agents = e3.views();
        cx.notify();
    }));
    settle(&mut cx, "1 queued", &mut |cx| tile.read(cx).texts().iter().any(|t| t.contains("1 queued")));
    shot(&mut cx, w, "sidebar");
    // Its Next up popover: "From Release desk", waiting for Send.
    let _ = cx.update_window(w, |_, window, cx| screen.update(cx, |s, cx| s.demo_next_up("a1", window, cx)));
    shot(&mut cx, w, "next-up");
    // The floating control, with the tile's question.
    settle(&mut cx, "the floating control", &mut |cx| crate::float::window_of(cx).is_some());
    // (Its own window takes no resize here: drawn again at its size.)
    cx.update(|cx| crate::float::bench("expand", cx));
    // Drawn still: the control's arrival doesn't fade in.
    crate::kit::motion::set_on(false);
    let preview = cx.open_window(size(px(580.), px(310.)), |window, cx| cx.new(|cx| crate::float::Preview::new(window, cx))).unwrap();
    let fw: AnyWindowHandle = preview.into();
    // (Its card is measured in a frame, then the control sized to it.)
    for _ in 0..4 {
        let _ = cx.update_window(fw, |_, window, _| window.refresh());
        cx.run_until_parked();
    }
    shot(&mut cx, fw, "float");
}
