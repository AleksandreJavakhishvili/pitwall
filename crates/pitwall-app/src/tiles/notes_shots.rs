//! Screenshots of the Notes seed tile (tiles/notes) drawn by the app,
//! headless (GPUI's test platform, no window shown): its list, a note open
//! and a draft, narrow and side by side, dark and light. Run by hand after
//! building the tile (`cargo build --release --target wasm32-wasip2` in
//! tiles/notes, or its `cargo test`):
//!
//! `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app notes_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{px, size, AnyWindowHandle, AppContext, Entity, HeadlessAppContext};
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;
use pitwall_tile_host::ui;

use super::State;
use crate::extensions::catalog;
use crate::settings::view::SettingsView;

const WAIT: Duration = Duration::from_secs(120);

fn notes_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tiles/notes")
}

#[test]
#[ignore]
fn notes_shots() {
    use std::sync::Arc;
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let wasm = notes_dir().join("target/wasm32-wasip2/release/notes_tile.wasm");
    let built = notes_dir().join("tile.wasm");
    assert!(built.is_file() || wasm.is_file(), "build tiles/notes first");
    let t = TempDir::new("notes-shots");
    let root = t.path().join("data");
    std::fs::create_dir_all(root.join("extensions")).unwrap();
    let src = t.path().join("notes");
    std::fs::create_dir_all(&src).unwrap();
    for f in ["extension.json", "input.schema.json"] {
        std::fs::copy(notes_dir().join(f), src.join(f)).unwrap();
    }
    std::fs::copy(if built.is_file() { &built } else { &wasm }, src.join("tile.wasm")).unwrap();
    // Notes it kept before.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    let mut kv = std::collections::BTreeMap::new();
    let notes = [
        ("web: the login page flickers\non reload, only in Safari\nask web to check the auth redirect", 4 * 60_000),
        ("Release checklist\n- tag 0.9.0\n- release notes\n- announce in #eng", 3 * 3_600_000),
        ("api\nhandle 429s from the billing service with a retry and a backoff", 2 * 86_400_000),
        ("Ideas\nA tile for the CI queue", 9 * 86_400_000),
    ];
    for (i, (text, ago)) in notes.iter().enumerate() {
        let v = serde_json::json!({ "text": text, "updated": now - ago });
        kv.insert(format!("p/_/{}", notes.len() - i), serde_json::to_vec(&v).unwrap());
    }
    pitwall_tile_host::ExtensionData::new(&root, "notes").set_storage(&kv).unwrap();

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
            img.save(out.join(format!("notes-{name}-{theme}.png"))).expect("saved");
        }
    };
    // Installed from its folder and turned on.
    let sv = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sv2 = sv.clone();
    let _shell = cx
        .open_window(size(px(900.), px(730.)), move |_, cx| {
            let v = cx.new(|cx| SettingsView::new(true, cx));
            *sv2.borrow_mut() = Some(v.clone());
            cx.new(|_| super::tests::Shell(v.into(), false))
        })
        .unwrap();
    let settings: Entity<SettingsView> = sv.borrow().clone().unwrap();
    let s2 = src.clone();
    cx.update(|cx| settings.update(cx, |v, cx| v.look_at_folder(s2, cx)));
    settle(&mut cx, "the review", &mut |cx| settings.read(cx).add_folder_state().is_some_and(|(busy, _, _)| !busy));
    cx.update(|cx| settings.update(cx, |v, cx| v.install_from_folder(cx)));
    settle(&mut cx, "the install", &mut |cx| settings.read(cx).add_folder_state().is_none());
    cx.update(|cx| settings.update(cx, |v, cx| v.debug_permissions("notes", cx)));
    cx.run_until_parked();
    cx.update(|cx| settings.update(cx, |v, cx| v.debug_permissions_on(cx)));
    cx.run_until_parked();
    settle(&mut cx, "Notes on", &mut |cx| crate::ui_state::get(cx).extension_on("notes"));

    // Narrow: the list, then a note.
    let open = |cx: &mut HeadlessAppContext, w: f32, h: f32| {
        let vt = std::rc::Rc::new(std::cell::RefCell::new(None));
        let vt2 = vt.clone();
        let win = cx
            .open_window(size(px(w), px(h)), move |_, cx| {
                let v = crate::main_screen::view_tile::view("notes/main", cx);
                *vt2.borrow_mut() = Some(v.clone());
                cx.new(|_| super::tests::Shell(v.into(), true))
            })
            .unwrap();
        let vtile: Entity<crate::main_screen::view_tile::ViewTile> = vt.borrow().clone().unwrap();
        (win, vtile)
    };
    let (narrow, vtile) = open(&mut cx, 380., 520.);
    settle(&mut cx, "the tile", &mut |cx| vtile.read(cx).tile().is_some());
    let tile = cx.update(|cx| vtile.read(cx).tile().cloned()).unwrap();
    settle(&mut cx, "the list", &mut |cx| tile.read(cx).texts().iter().any(|t| t == "Release checklist"));
    shot(&mut cx, narrow.into(), "list");
    let click = |cx: &mut HeadlessAppContext, tile: &Entity<super::TileView>, path: &str| {
        let p = path.to_string();
        let ok = cx.update(|cx| tile.read(cx).click(&p, ui::MouseButton::Left, ui::Modifiers::empty(), 1, ui::Point { x: 1., y: 1. }, cx));
        assert!(ok, "no node {path} to click: {:?}", cx.update(|cx| tile.read(cx).texts()));
    };
    click(&mut cx, &tile, "list/rows/n4");
    settle(&mut cx, "the editor", &mut |cx| tile.read(cx).field_of("editor/n4/text").is_some());
    shot(&mut cx, narrow.into(), "editor");
    // Asking to delete, in the note's bar.
    click(&mut cx, &tile, "editor/head/trash");
    settle(&mut cx, "the question", &mut |cx| tile.read(cx).texts().iter().any(|t| t.starts_with("Delete “")));
    shot(&mut cx, narrow.into(), "delete");
    click(&mut cx, &tile, "editor/head/cancel");
    settle(&mut cx, "no question", &mut |cx| !tile.read(cx).texts().iter().any(|t| t.starts_with("Delete “")));
    // A draft an agent opened it with.
    cx.update(|cx| {
        let r = super::tiles(cx);
        let r = r.read(cx);
        let running = r.get("notes").unwrap();
        assert!(matches!(running.state, State::Running));
        let draft = serde_json::json!({ "title": "Standup", "text": "- api: 429 retries merged\n- web: Safari flicker, still open\n- next: release 0.9" });
        running.handle.as_ref().unwrap().set_content("main", ui::Content { id: None, json: Some(draft.to_string()), bytes: None, call: None, tool: None, caller: None });
    });
    settle(&mut cx, "the draft", &mut |cx| tile.read(cx).texts().iter().any(|t| t == "Draft, not saved"));
    shot(&mut cx, narrow.into(), "draft");
    let _ = cx.update_window(narrow.into(), |_, window, _| window.remove_window());
    cx.run_until_parked();

    // Wide: the list and the note side by side.
    let (wide, vtile) = open(&mut cx, 780., 520.);
    settle(&mut cx, "the tile", &mut |cx| vtile.read(cx).tile().is_some());
    let tile = cx.update(|cx| vtile.read(cx).tile().cloned()).unwrap();
    settle(&mut cx, "side by side", &mut |cx| tile.read(cx).texts().iter().any(|t| t == "Release checklist") && tile.read(cx).texts().iter().any(|t| t == "Draft, not saved"));
    shot(&mut cx, wide.into(), "split");
}
