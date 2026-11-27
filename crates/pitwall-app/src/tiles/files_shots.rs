//! Screenshots of `files` in the app, headless (GPUI's test platform, no
//! window shown): the permission screens of the C4 tile (files.read and
//! show) and of the host's files fixture (files.read and files.write, the
//! project folder its view is open in), and the C4 tile drawing a
//! project's own docs diagram, dark and light. Run by hand:
//!
//! `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app --lib files_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{px, size, AnyWindowHandle, AppContext, Entity, HeadlessAppContext};
use pitwall_core::extensions::permissions::Decision;
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;

use crate::extensions::catalog;
use crate::settings::view::SettingsView;

const WAIT: Duration = Duration::from_secs(120);

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_file() {
            std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

#[test]
#[ignore]
fn files_shots() {
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("files-shots");
    let root = t.path().join("data");
    super::c4_tests::copy_c4(&root.join("extensions/c4"));
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/files"), &root.join("extensions/files"));
    // A project with the C4 spec's own architecture doc.
    let project = t.path().join("c4-tile");
    std::fs::create_dir_all(project.join("docs")).unwrap();
    std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tiles/c4/tests/fixtures/c4-tile/hla.md"), project.join("docs/hla.md")).unwrap();
    let project = project.canonicalize().unwrap().to_string_lossy().into_owned();

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
            img.save(out.join(format!("files-{name}-{theme}.png"))).expect("saved");
        }
    };

    // The permission screens, as on turning each on.
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
    settle(&mut cx, "they were found", &mut |cx| crate::extensions::manifest_of("c4", cx).is_some() && crate::extensions::manifest_of("files", cx).is_some());
    cx.update(|cx| settings.update(cx, |v, cx| v.debug_permissions("c4", cx)));
    cx.run_until_parked();
    cx.update(|cx| settings.read(cx).scroll_to_end());
    shot(&mut cx, shell.into(), "c4-permissions");
    cx.update(|cx| settings.update(cx, |v, cx| v.debug_permissions_on(cx)));
    cx.run_until_parked();

    // The C4 tile opened for the project, files.read granted: its docs'
    // context diagram drawn, the docs in the outline.
    cx.update(|cx| {
        crate::ui_state::update(cx, |u| {
            *u = std::mem::take(u).with_extension("c4", |e| {
                e.enabled = true;
                e.grants.insert("files.read".into(), Decision { granted: true, at: 1 });
            })
        });
        super::tiles(cx).update(cx, |t, cx| t.hint_project("c4", "main", &project, cx));
    });
    let vt = std::rc::Rc::new(std::cell::RefCell::new(None));
    let vt2 = vt.clone();
    let win = cx
        .open_window(size(px(980.), px(600.)), move |_, cx| {
            let v = crate::main_screen::view_tile::view("c4/main", cx);
            *vt2.borrow_mut() = Some(v.clone());
            cx.new(|_| super::tests::Shell(v.into(), true))
        })
        .unwrap();
    let vtile: Entity<crate::main_screen::view_tile::ViewTile> = vt.borrow().clone().unwrap();
    settle(&mut cx, "the tile", &mut |cx| vtile.read(cx).tile().is_some());
    let tile = cx.update(|cx| vtile.read(cx).tile().cloned()).unwrap();
    let has = |cx: &mut gpui::App, s: &str| {
        let t = tile.read(cx);
        t.texts().iter().chain(t.canvas_texts().iter()).any(|t| t.contains(s))
    };
    settle(&mut cx, "the docs diagram", &mut |cx| has(cx, "C4 tile") && has(cx, "docs/hla.md"));
    shot(&mut cx, win.into(), "c4-docs");
    // Click into "C4 tile": the containers from the same doc.
    let ok = cx.update(|cx| tile.read(cx).click_region("body/main/diagram", "c4", cx));
    assert!(ok, "no region c4");
    settle(&mut cx, "the containers", &mut |cx| has(cx, "tile.wasm"));
    shot(&mut cx, win.into(), "c4-docs-zoomed");

    // The files fixture's permission screen, now that a view is open in
    // the project: the folder it reaches named.
    cx.update(|cx| super::tiles(cx).update(cx, |t, cx| t.hint_project("files", "main", &project, cx)));
    let vf = std::rc::Rc::new(std::cell::RefCell::new(None));
    let vf2 = vf.clone();
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("files", true)));
    let _files = cx
        .open_window(size(px(600.), px(300.)), move |_, cx| {
            let v = crate::main_screen::view_tile::view("files/main", cx);
            *vf2.borrow_mut() = Some(v.clone());
            cx.new(|_| super::tests::Shell(v.into(), true))
        })
        .unwrap();
    settle(&mut cx, "its view in the project", &mut |cx| super::scoped_folders("files", cx).contains(&project));
    // Off again, so Settings asks as on turning it on.
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("files", false)));
    cx.run_until_parked();
    cx.update(|cx| settings.update(cx, |v, cx| v.debug_permissions("files", cx)));
    cx.run_until_parked();
    cx.update(|cx| settings.read(cx).scroll_to_end());
    shot(&mut cx, shell.into(), "permissions");
}
