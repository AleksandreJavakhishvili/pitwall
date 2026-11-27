//! Screenshots of what a tile reports and asks for beyond its views,
//! headless (GPUI's test platform, no window shown), with the tile host's
//! questions fixture ("Release desk"): its card with the status it
//! reports ("Connected") and its shortcut granted, with the field to
//! change its keys; and the tile with a note held for take-back. Dark and
//! light. Run by hand:
//!
//! `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app report_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{px, size, AnyWindowHandle, AppContext, Entity, HeadlessAppContext};
use pitwall_core::extensions::permissions;
use pitwall_core::testing::TempDir;
use pitwall_tile_host::ui;

use super::{group_tests, TileView};
use crate::settings::view::SettingsView;

const WAIT: Duration = Duration::from_secs(120);

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
fn report_shots() {
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("report-shots");
    let root = t.path().join("data");
    let (engine, server) = group_tests::server(&t);
    let text: Arc<dyn gpui::PlatformTextSystem> = Arc::new(gpui_apple::AppleTextSystem::new());
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(crate::kit::Assets), gpui_platform::current_headless_renderer);
    cx.allow_parking();
    let (e2, r2) = (engine.clone(), root.clone());
    cx.update(|cx| {
        group_tests::init(&r2, &e2, server, cx);
        crate::extensions::follow(cx);
        crate::settings::data::set_page(cx, pitwall_proto::settings::Page::Extensions);
        crate::extensions::calls::decide("questions", permissions::SHORTCUT, true, cx);
    });
    let settle = |cx: &mut HeadlessAppContext, what: &str, done: &mut dyn FnMut(&mut gpui::App) -> bool| {
        let end = Instant::now() + WAIT;
        loop {
            cx.background_executor().advance_clock(Duration::from_millis(50));
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
            for _ in 0..2 {
                cx.run_until_parked();
                let _ = cx.update_window(w, |_, window, _| window.refresh());
            }
            cx.run_until_parked();
            let img = cx.capture_screenshot(w).expect("rendered");
            img.save(out.join(format!("tile-report-{name}-{theme}.png"))).expect("saved");
        }
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
    };

    // The tile, with a note held for take-back.
    let tv = std::rc::Rc::new(std::cell::RefCell::new(None));
    let tv2 = tv.clone();
    let win = cx
        .open_window(size(px(520.), px(460.)), move |_, cx| {
            let v = cx.new(|cx| TileView::new("questions", "main", cx));
            *tv2.borrow_mut() = Some(v.clone());
            cx.new(|_| Shell(v.into()))
        })
        .unwrap();
    let tile: Entity<TileView> = tv.borrow().clone().unwrap();
    let w: AnyWindowHandle = win.into();
    settle(&mut cx, "the agents", &mut |cx| tile.read(cx).texts().iter().any(|t| t == "web"));
    settle(&mut cx, "the note field", &mut |cx| tile.read(cx).field_of("note/text").is_some());
    let field = cx.update(|cx| tile.read(cx).field_of("note/text")).unwrap();
    cx.update(|cx| field.update(cx, |i, cx| i.replace_text("Ship 0.9 on Friday", cx)));
    // Send, once the tile has the words (its Send is enabled then): the
    // field empties and the banner shows.
    let f2 = field.clone();
    settle(&mut cx, "the note held", &mut |cx| {
        let _ = tile.read(cx).click("note/send", ui::MouseButton::Left, ui::Modifiers::empty(), 1, ui::Point { x: 1., y: 1. }, cx);
        f2.read(cx).text().is_empty()
    });
    for _ in 0..5 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(30));
    }
    shot(&mut cx, w, "take-back");

    // Its card: Connected, from its status; the shortcut granted, its keys.
    cx.update(|cx| crate::extensions::hub::hub(cx).update(cx, |h, cx| h.reconcile(cx)));
    settle(&mut cx, "Connected", &mut |cx| {
        let m = crate::extensions::manifest_of("questions", cx).unwrap();
        crate::settings::extensions::card_status(&m, None, cx).0 == "Connected"
    });
    let sv = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sv2 = sv.clone();
    let page = cx
        .open_window(size(px(1100.), px(820.)), move |_, cx| {
            let v = cx.new(SettingsView::extensions_surface);
            *sv2.borrow_mut() = Some(v.clone());
            cx.new(|_| Shell(v.into()))
        })
        .unwrap();
    let settings: Entity<SettingsView> = sv.borrow().clone().unwrap();
    cx.update(|cx| settings.update(cx, |v, cx| v.show_extension_detail(Some("questions"), cx)));
    let p: AnyWindowHandle = page.into();
    shot(&mut cx, p, "card");
    cx.update(|cx| settings.read(cx).scroll_by(px(420.)));
    shot(&mut cx, p, "card-shortcut");
}
