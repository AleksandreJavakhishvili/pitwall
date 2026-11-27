//! Screenshots of crash-loop protection ([`super::guard`]), headless
//! (GPUI's test platform, no window shown), with the hello fixture: its
//! tile paused after a launch that crashed while it was starting, and its
//! card in Settings saying so, with Go back to the version before the
//! update. Dark and light. Run by hand:
//!
//! `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app guard_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{px, size, AnyWindowHandle, AppContext, Entity, HeadlessAppContext};
use pitwall_core::testing::TempDir;

use super::guard::{self, Record};
use super::tests::copy_fixture;
use super::TileView;
use crate::settings::view::SettingsView;

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
fn guard_shots() {
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("guard-shots");
    let root = t.path().join("data");
    let exts = root.join("extensions");
    copy_fixture(&exts.join(".hello.previous"));
    copy_fixture(&exts.join("hello"));
    let m = std::fs::read_to_string(exts.join("hello/extension.json")).unwrap();
    std::fs::write(exts.join("hello/extension.json"), m.replacen(r#""version": "0.1.0""#, r#""version": "0.2.0""#, 1)).unwrap();
    let text: Arc<dyn gpui::PlatformTextSystem> = Arc::new(gpui_apple::AppleTextSystem::new());
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(crate::kit::Assets), gpui_platform::current_headless_renderer);
    let r2 = root.clone();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::init(Some(r2.clone()), cx);
        crate::extensions::catalog::set_bundle(None, cx);
        cx.set_global(super::DataDir(r2.clone()));
        crate::settings::data::set_page(cx, pitwall_proto::settings::Page::Extensions);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true));
        let rec = Record { starting: BTreeMap::from([("hello".to_string(), Some("0.2.0".to_string()))]), ..Default::default() };
        guard::init_for_test(&r2, &rec, guard::HEALTHY_AFTER, cx);
    });
    let shot = |cx: &mut HeadlessAppContext, w: AnyWindowHandle, name: &str| {
        for (theme, light) in [("dark", false), ("light", true)] {
            cx.update(|cx| cx.set_global(if light { crate::theme::Theme::light() } else { crate::theme::Theme::dark() }));
            for _ in 0..2 {
                cx.run_until_parked();
                let _ = cx.update_window(w, |_, window, _| window.refresh());
            }
            cx.run_until_parked();
            let img = cx.capture_screenshot(w).expect("rendered");
            img.save(out.join(format!("tile-guard-{name}-{theme}.png"))).expect("saved");
        }
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
    };

    let win = cx.open_window(size(px(520.), px(360.)), |_, cx| {
        let v = cx.new(|cx| TileView::new("hello", "main", cx));
        cx.new(|_| Shell(v.into()))
    });
    shot(&mut cx, win.unwrap().into(), "paused-tile");

    let sv = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sv2 = sv.clone();
    let page = cx
        .open_window(size(px(1100.), px(640.)), move |_, cx| {
            let v = cx.new(SettingsView::extensions_surface);
            *sv2.borrow_mut() = Some(v.clone());
            cx.new(|_| Shell(v.into()))
        })
        .unwrap();
    let settings: Entity<SettingsView> = sv.borrow().clone().unwrap();
    cx.update(|cx| settings.update(cx, |v, cx| v.show_extension_detail(Some("hello"), cx)));
    shot(&mut cx, page.into(), "paused-card");
}
