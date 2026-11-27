//! Screenshots of extensions' listing media in Settings → Extensions,
//! headless (GPUI's test platform, no window shown), dark and light: the
//! seed tiles (tiles/c4, tiles/notes) installed with their own icons,
//! screenshots and READMEs, their detail pages (a screenshot shown large
//! too), and Browse with them listed as the index's build lists them
//! (README, icon, screenshots and README images copied out). Run by hand:
//!
//! `PITWALL_MEDIA_SHOTS=<folder> cargo test -p pitwall-app media_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{px, size, AnyWindowHandle, AppContext, Entity, HeadlessAppContext};
use pitwall_core::extensions::index::repo;
use pitwall_core::extensions::package::zip;
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;

use super::super::view::SettingsView;
use super::page::Tab;
use super::page_tests::{fixture_index, publish, recent};
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

fn tiles() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tiles")
}

/// The files of seed tile `id` a package would hold for its listing.
fn listing_files(id: &str) -> Vec<zip::Entry> {
    let dir = tiles().join(id);
    let m = pitwall_core::extensions::read_in_place(&dir).unwrap();
    let mut names = vec!["README.md".to_string(), "extension.json".to_string()];
    names.extend(pitwall_core::extensions::media::files(&dir, &m).unwrap());
    names.into_iter().map(|n| zip::Entry { bytes: std::fs::read(dir.join(&n)).unwrap(), name: n }).collect()
}

#[test]
#[ignore]
fn media_shots() {
    let Ok(out) = std::env::var("PITWALL_MEDIA_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("media-shots");
    let root = t.path().join("data");
    let exts = root.join("extensions");
    // The seed tiles installed by hand, as their packages hold them.
    for id in ["c4", "notes"] {
        let to = exts.join(id);
        for f in listing_files(id) {
            let p = to.join(&f.name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, &f.bytes).unwrap();
        }
        if tiles().join(id).join("tile.wasm").is_file() {
            std::fs::copy(tiles().join(id).join("tile.wasm"), to.join("tile.wasm")).unwrap();
        }
    }
    // The index: the fixture's, and the seed tiles listed as its build
    // lists them.
    let f = fixture_index();
    publish(&f, &recent());
    let got = {
        use pitwall_core::extensions::index::IndexSource as _;
        f.source().fetch().unwrap()
    };
    let mut idx = got.index.clone();
    for id in ["notes", "c4"] {
        let files = listing_files(id);
        let m = pitwall_core::extensions::parse(std::str::from_utf8(&files.iter().find(|f| f.name == "extension.json").unwrap().bytes).unwrap()).unwrap();
        let c = repo::Checked { id: id.into(), version: "0.1.0".into(), manifest: m.clone(), files, size: 1_400_000, signer: f.author.signer() };
        let (screens, readme, media, written) = repo::listing_media(id, &c).unwrap();
        for (p, bytes) in written {
            f.http.put(&format!("{}{p}", pitwall_core::testing::index::BASE), bytes);
        }
        let mut l = idx.listing("ci-status").unwrap().clone();
        l.id = id.into();
        l.name = m.name.clone();
        l.author = "pitwall".into();
        l.description = m.description.clone().unwrap_or_default();
        l.tagline = m.tagline.clone();
        l.categories = m.categories.clone();
        l.license = "Apache-2.0".into();
        l.repository = "https://github.com/pitwalls/pitwall".into();
        l.homepage = m.homepage.clone();
        l.permissions = repo::listed_permissions(&m);
        l.screens = screens;
        l.readme = readme;
        l.media = media;
        idx.extensions.insert(0, l);
    }
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
        .open_window(size(px(1280.), px(820.)), move |_, cx| {
            let v = cx.new(SettingsView::extensions_surface);
            *sv2.borrow_mut() = Some(v.clone());
            cx.new(|_| Shell(v.into()))
        })
        .unwrap();
    let settings: Entity<SettingsView> = sv.borrow().clone().unwrap();
    let shoot = |cx: &mut HeadlessAppContext, w: AnyWindowHandle, name: &str| {
        for (theme, light) in [("dark", false), ("light", true)] {
            cx.update(|cx| cx.set_global(if light { crate::theme::Theme::light() } else { crate::theme::Theme::dark() }));
            for _ in 0..4 {
                cx.run_until_parked();
                let _ = cx.update_window(w, |_, window, _| window.refresh());
                std::thread::sleep(Duration::from_millis(30));
            }
            cx.run_until_parked();
            let img = cx.capture_screenshot(w).expect("rendered");
            img.save(out.join(format!("media-{name}-{theme}.png"))).expect("saved");
        }
        cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
    };
    let w: AnyWindowHandle = win.into();
    let fetched = |f: &pitwall_core::testing::index::IndexFixture, n: usize| f.http.requests().iter().filter(|(u, _)| u.contains("/media/")).count() >= n;

    // Browse: each card with its own icon and first screenshot.
    cx.update(|cx| settings.update(cx, |v, cx| v.set_extensions_tab(Tab::Browse, cx)));
    settle(&mut cx, "the index", &mut |cx| settings.update(cx, |v, cx| v.browse_state_now(cx).0 == "ready"));
    cx.update(|cx| settings.update(cx, |v, cx| v.set_browse_query(None, false, false, cx)));
    settle(&mut cx, "the cards' media", &mut |_| fetched(&f, 6));
    shoot(&mut cx, w, "browse");

    // A listing's page: screenshots, then its README with its images.
    cx.update(|cx| settings.update(cx, |v, cx| v.show_listing(Some("c4"), cx)));
    settle(&mut cx, "its media", &mut |_| fetched(&f, 14));
    shoot(&mut cx, w, "browse-detail");
    cx.update(|cx| settings.read(cx).scroll_by(px(700.)));
    shoot(&mut cx, w, "browse-detail-readme");

    // Installed: the detail page, a screenshot large, the README.
    cx.update(|cx| settings.update(cx, |v, cx| v.show_extension_detail(Some("notes"), cx)));
    shoot(&mut cx, w, "detail");
    cx.update(|cx| settings.read(cx).scroll_by(px(520.)));
    shoot(&mut cx, w, "detail-readme");
    cx.update(|cx| settings.update(cx, |v, cx| v.show_extension_detail(Some("c4"), cx)));
    cx.update(|cx| settings.update(cx, |v, _| v.exts.page.enlarged = Some(("installed/c4".into(), 1))));
    shoot(&mut cx, w, "detail-large");
    cx.update(|cx| settings.update(cx, |v, cx| {
        v.exts.page.enlarged = None;
        v.show_extension_detail(None, cx);
        v.set_extensions_tab(Tab::Installed, cx);
    }));
    shoot(&mut cx, w, "installed");
}
