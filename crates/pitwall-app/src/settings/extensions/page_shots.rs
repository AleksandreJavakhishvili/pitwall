//! Screenshots of Settings → Extensions as a page, headless (GPUI's test
//! platform, no window shown), dark and light, in the Extensions tile
//! (wide: the list beside the page; narrow: one column) and the dialog's
//! compact page: Installed (cards and
//! rows), an installed extension's detail page (one put there by hand,
//! one installed from a signed package the index lists as Verified),
//! Browse with a fixture index, and a listing's detail page. Run by hand:
//!
//! `PITWALL_PAGE_SHOTS=<folder> cargo test -p pitwall-app page_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{px, size, AnyWindowHandle, AppContext, Entity, HeadlessAppContext};
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;

use super::super::view::SettingsView;
use super::page::{Filters, Sort, Tab};
use super::page_tests::{fixture_index, installed, publish, recent};
use crate::extensions::catalog;
use crate::extensions::index;

const WAIT: Duration = Duration::from_secs(120);

struct Shell(gpui::AnyView);

impl gpui::Render for Shell {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{div, prelude::*};
        let t = crate::theme::theme(cx).clone();
        div().size_full().flex().flex_col().bg(t.bg).text_color(t.text).child(self.0.clone())
    }
}

fn shots_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/extensions/images/seed-tiles")
}

#[test]
#[ignore]
fn page_shots() {
    let Ok(out) = std::env::var("PITWALL_PAGE_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("page-shots");
    let root = t.path().join("data");
    let exts = root.join("extensions");
    installed(&exts, None);
    // Hello comes from a signed package instead, with a README, a licence
    // and a changelog.
    std::fs::remove_dir_all(exts.join("hello")).unwrap();
    // C4 gets its screenshots and a licence.
    std::fs::create_dir_all(exts.join("c4/screenshots")).unwrap();
    for (i, f) in ["c4-drawn-dark.png", "c4-changes-dark.png"].iter().enumerate() {
        std::fs::copy(shots_dir().join(f), exts.join(format!("c4/screenshots/{i}.png"))).unwrap();
    }
    std::fs::write(exts.join("c4/LICENSE"), "Apache License\nVersion 2.0, January 2004\n").unwrap();
    // The index: the fixture's, built today, and C4 listed too with its
    // real screenshots.
    let f = fixture_index();
    publish(&f, &recent());
    let got = {
        use pitwall_core::extensions::index::IndexSource as _;
        f.source().fetch().unwrap()
    };
    let mut idx = got.index.clone();
    let mut c4 = idx.listing("ci-status").unwrap().clone();
    c4.id = "c4".into();
    c4.name = "C4 diagrams".into();
    c4.author = "pitwall".into();
    c4.description = "C4 diagrams your agents draw and your docs hold, shown next to their terminal, with click to zoom".into();
    c4.icon = Some("grid".into());
    c4.categories = vec!["docs".into()];
    c4.license = "Apache-2.0".into();
    c4.repository = "https://github.com/pitwalls/pitwall".into();
    c4.permissions = vec![index::ListedPermission { name: "files.read".into(), reason: "Find and follow the Mermaid diagrams in your project's docs".into(), hosts: vec![], amount: None }];
    c4.screens = vec!["screens/c4/drawn.png".into(), "screens/c4/changes.png".into()];
    for (v, ch) in c4.versions.iter_mut().zip(["c4__show: an agent's Mermaid drawn next to its terminal.", "Diagrams in your project's docs, followed as they change; before and after pairs."]) {
        v.changelog = Some(ch.into());
        v.size = 1_834_000;
    }
    idx.extensions.insert(0, c4);
    f.http.put(&format!("{}screens/c4/drawn.png", pitwall_core::testing::index::BASE), std::fs::read(shots_dir().join("c4-drawn-dark.png")).unwrap());
    f.http.put(&format!("{}screens/c4/changes.png", pitwall_core::testing::index::BASE), std::fs::read(shots_dir().join("c4-changes-dark.png")).unwrap());
    let setup = index::Setup::new(Arc::new(index::FixtureSource::new(idx, got.blocked.clone())), f.config(), f.http.clone(), "the test index");

    let text: Arc<dyn gpui::PlatformTextSystem> = Arc::new(gpui_apple::AppleTextSystem::new());
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(crate::kit::Assets), gpui_platform::current_headless_renderer);
    cx.allow_parking();
    let r2 = root.clone();
    let mut setup = Some(setup);
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::init(Some(r2.clone()), cx);
        catalog::set_bundle(None, cx);
        index::set_setup(setup.take(), cx);
        cx.set_global(crate::tiles::DataDir(r2));
        crate::settings::data::set_page(cx, Page::Extensions);
        crate::ui_state::update(cx, |s| *s = s.clone().set_extension("demo-inbox", true));
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
        .open_window(size(px(1280.), px(800.)), move |_, cx| {
            let v = cx.new(SettingsView::extensions_surface);
            *sv2.borrow_mut() = Some(v.clone());
            cx.new(|_| Shell(v.into()))
        })
        .unwrap();
    let settings: Entity<SettingsView> = sv.borrow().clone().unwrap();
    let shoot = |cx: &mut HeadlessAppContext, w: AnyWindowHandle, name: &str| {
        for (theme, light) in [("dark", false), ("light", true)] {
            cx.update(|cx| cx.set_global(if light { crate::theme::Theme::light() } else { crate::theme::Theme::dark() }));
            cx.run_until_parked();
            let _ = cx.update_window(w, |_, window, _| window.refresh());
            cx.run_until_parked();
            let _ = cx.update_window(w, |_, window, _| window.refresh());
            cx.run_until_parked();
            let img = cx.capture_screenshot(w).expect("rendered");
            img.save(out.join(format!("extensions-page-{name}-{theme}.png"))).expect("saved");
        }
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
    };
    let w: AnyWindowHandle = win.into();
    // The index read first: Installed's detail pages name who wrote it.
    cx.update(|cx| settings.update(cx, |v, cx| v.set_extensions_tab(Tab::Browse, cx)));
    settle(&mut cx, "the index", &mut |cx| settings.update(cx, |v, cx| v.browse_state_now(cx).0 == "ready"));
    cx.update(|cx| settings.update(cx, |v, cx| v.install_listing("ci-status", cx)));
    settle(&mut cx, "the trust screen", &mut |cx| settings.read(cx).listing_trust_shown("ci-status") && settings.read(cx).add_file_state().is_some_and(|(b, _, id)| !b && id.is_some()));
    shoot(&mut cx, w, "trust");
    cx.update(|cx| settings.update(cx, |v, cx| v.install_from_package(cx)));
    settle(&mut cx, "the install", &mut |cx| settings.read(cx).add_file_state().is_none());

    // Installed: the list beside what extensions are.
    cx.update(|cx| settings.update(cx, |v, cx| {
        v.set_extensions_tab(Tab::Installed, cx);
        v.set_extension_filters(Filters::default(), Sort::Name, true, cx);
        v.show_extension_detail(None, cx);
        v.move_selection(1, cx);
    }));
    shoot(&mut cx, w, "installed");

    // Detail pages.
    cx.update(|cx| settings.update(cx, |v, cx| v.show_extension_detail(Some("c4"), cx)));
    shoot(&mut cx, w, "detail");
    cx.update(|cx| settings.read(cx).scroll_by(px(560.)));
    shoot(&mut cx, w, "detail-more");
    cx.update(|cx| settings.update(cx, |v, cx| v.show_extension_detail(Some("ci-status"), cx)));
    shoot(&mut cx, w, "detail-signed");

    // Browse, and a listing's page.
    cx.update(|cx| settings.update(cx, |v, cx| {
        v.set_extensions_tab(Tab::Browse, cx);
        // Every listing, the Verified and the not yet reviewed.
        v.set_browse_query(None, false, false, cx);
    }));
    shoot(&mut cx, w, "browse");
    cx.update(|cx| settings.update(cx, |v, cx| v.show_listing(Some("c4"), cx)));
    settle(&mut cx, "its files", &mut |cx| {
        let _ = cx;
        f.http.requests().iter().any(|(u, _)| u.ends_with("screens/c4/changes.png"))
    });
    for _ in 0..5 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(50));
    }
    shoot(&mut cx, w, "browse-detail");
    cx.update(|cx| settings.read(cx).scroll_by(px(640.)));
    shoot(&mut cx, w, "browse-detail-more");

    // Narrow: one column (the cards, then a detail page alone).
    let nv = std::rc::Rc::new(std::cell::RefCell::new(None));
    let nv2 = nv.clone();
    let narrow = cx
        .open_window(size(px(720.), px(800.)), move |_, cx| {
            let v = cx.new(SettingsView::extensions_surface);
            *nv2.borrow_mut() = Some(v.clone());
            cx.new(|_| Shell(v.into()))
        })
        .unwrap();
    let nview: Entity<SettingsView> = nv.borrow().clone().unwrap();
    let n: AnyWindowHandle = narrow.into();
    shoot(&mut cx, n, "narrow");
    cx.update(|cx| nview.update(cx, |v, cx| v.show_extension_detail(Some("c4"), cx)));
    shoot(&mut cx, n, "narrow-detail");

    // The Settings dialog's page: compact, with Open Extensions.
    let dialog = cx
        .open_window(size(px(1000.), px(700.)), move |_, cx| {
            let v = cx.new(|cx| {
                let mut v = SettingsView::new(true, cx);
                v.set_compact_extensions(true, cx);
                v
            });
            cx.new(|_| Shell(v.into()))
        })
        .unwrap();
    shoot(&mut cx, dialog.into(), "dialog");
}
