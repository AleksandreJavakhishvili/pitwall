//! Screenshots of Install from file… (docs/spec/tile-sdk/packaging.md
//! §6.3, §6.4), headless (GPUI's test platform, no window shown): the
//! trust screen of a signed package, of an unsigned one, an update by the
//! same key with its permission changes, and an update signed by another
//! key; dark and light. The hello fixture is packed and signed by
//! throwaway keys made here. Run by hand:
//!
//! `PITWALL_PKG_SHOTS=<folder> cargo test -p pitwall-app package_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{px, size, AnyWindowHandle, AppContext, Entity, HeadlessAppContext};
use pitwall_core::testing::{Signing, TempDir};
use pitwall_proto::settings::Page;

use super::super::view::SettingsView;
use super::package_tests::hello_package;
use crate::extensions::catalog;

const WAIT: Duration = Duration::from_secs(120);

/// The settings view on the theme's background.
struct Shell(gpui::AnyView);

impl gpui::Render for Shell {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{div, prelude::*};
        let t = crate::theme::theme(cx).clone();
        div().size_full().flex().flex_col().bg(t.bg).child(self.0.clone())
    }
}

#[test]
#[ignore]
fn package_shots() {
    use std::sync::Arc;
    let Ok(out) = std::env::var("PITWALL_PKG_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("pkg-shots");
    let root = t.path().join("data");
    std::fs::create_dir_all(root.join("extensions")).unwrap();
    let author = Signing::new();
    let v1 = hello_package(t.path(), "0.1.0", "", Some(&author));
    let bare = t.path().join("bare");
    std::fs::create_dir_all(&bare).unwrap();
    let unsigned = hello_package(&bare, "0.1.0", "", None);
    let v2 = hello_package(t.path(), "0.2.0", r#", "notify": "Tell you when a long job is done", "net": {"reason": "Fetch the weather it greets you with", "hosts": ["api.weather.example"]}"#, Some(&author));
    let v3 = hello_package(t.path(), "0.3.0", "", Some(&Signing::new()));

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
    let shoot = |cx: &mut HeadlessAppContext, w: AnyWindowHandle, name: &str| {
        for (theme, light) in [("dark", false), ("light", true)] {
            cx.update(|cx| cx.set_global(if light { crate::theme::Theme::light() } else { crate::theme::Theme::dark() }));
            cx.run_until_parked();
            let _ = cx.update_window(w, |_, window, _| window.refresh());
            cx.run_until_parked();
            let img = cx.capture_screenshot(w).expect("rendered");
            img.save(out.join(format!("packages-{name}-{theme}.png"))).expect("saved");
        }
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
    };
    let sv = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sv2 = sv.clone();
    let win = cx
        .open_window(size(px(900.), px(540.)), move |_, cx| {
            let v = cx.new(|cx| SettingsView::new(true, cx));
            *sv2.borrow_mut() = Some(v.clone());
            cx.new(|_| Shell(v.into()))
        })
        .unwrap();
    let settings: Entity<SettingsView> = sv.borrow().clone().unwrap();
    // Settings is 540 px high: the card scrolled to its top, then further
    // for its second half.
    let shot = |cx: &mut HeadlessAppContext, w: AnyWindowHandle, name: &str| {
        for (part, dy) in [("", 380.), ("-more", 400.)] {
            cx.update(|cx| settings.read(cx).scroll_by(px(dy)));
            shoot(cx, w, &format!("{name}{part}"));
        }
        cx.update(|cx| settings.read(cx).scroll_by(px(-780.)));
    };
    let look = |cx: &mut HeadlessAppContext, file: &PathBuf| {
        let f = file.clone();
        cx.update(|cx| settings.update(cx, |v, cx| v.look_at_package(f, cx)));
        settle(cx, "the package read", &mut |cx| settings.read(cx).add_file_state().is_some_and(|(busy, _, _)| !busy));
        let (_, note, _) = cx.update(|cx| settings.read(cx).add_file_state()).unwrap();
        assert_eq!(note, None);
    };
    let install = |cx: &mut HeadlessAppContext| {
        cx.update(|cx| settings.update(cx, |v, cx| v.install_from_package(cx)));
        settle(cx, "the install", &mut |cx| settings.read(cx).add_file_state().is_none());
    };
    look(&mut cx, &unsigned);
    shot(&mut cx, win.into(), "unsigned");
    look(&mut cx, &v1);
    shot(&mut cx, win.into(), "trust");
    install(&mut cx);
    look(&mut cx, &v2);
    shot(&mut cx, win.into(), "update");
    install(&mut cx);
    look(&mut cx, &v3);
    shot(&mut cx, win.into(), "new-key");
}
