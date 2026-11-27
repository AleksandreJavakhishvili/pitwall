//! Screenshots of agents' tile tools drawn by the app, headless (GPUI's
//! test platform, no window shown): the hello tile with an agent's message
//! and its "from <agent>" mark, Notes with an agent's draft, the
//! extension's card with "Agents can use Hello", and Settings → Agents'
//! "Agents can use tiles", dark and light. Run by hand (Notes built first:
//! its `cargo test` in tiles/notes):
//!
//! `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app agents_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{px, size, AnyWindowHandle, AppContext, Entity, HeadlessAppContext};
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;
use pitwall_proto::tile_tools::TileShow;
use pitwall_tile_host::agent::{Caller, ShowCall};

use crate::extensions::catalog;
use crate::settings::view::SettingsView;

const WAIT: Duration = Duration::from_secs(120);

fn copy(from: &Path, to: &Path, files: &[&str]) {
    std::fs::create_dir_all(to).unwrap();
    for f in files {
        std::fs::copy(from.join(f), to.join(f)).unwrap();
    }
}

#[test]
#[ignore]
fn agents_shots() {
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let notes = here.join("../../tiles/notes");
    assert!(notes.join("tile.wasm").is_file(), "build tiles/notes first (its cargo test)");
    let t = TempDir::new("agents-shots");
    let root = t.path().join("data");
    copy(&here.join("../pitwall-tile-host/tests/fixtures/hello"), &root.join("extensions/hello"), &["extension.json", "input.schema.json", "tile.wasm"]);
    copy(&notes, &root.join("extensions/notes"), &["extension.json", "input.schema.json", "tile.wasm"]);

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
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true).set_extension("notes", true));
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
            img.save(out.join(format!("agent-open-{name}-{theme}.png"))).expect("saved");
        }
    };
    // A call as `agents::show` hands it over: checked, delivered, the
    // view's mark set once the tile answered.
    let call = |cx: &mut HeadlessAppContext, ext: &str, tool: Option<&str>, item: &str, input: serde_json::Value| {
        let (tx, rx) = std::sync::mpsc::channel();
        let (ext, tool, item) = (ext.to_string(), tool.map(String::from), item.to_string());
        let req = TileShow { agent_id: "api".into(), agent_name: "api".into(), extension: ext.clone(), tile: "main".into(), tool: tool.clone(), item: Some(item.clone()), input: Some(input.to_string()), ..Default::default() };
        let id = cx.update(|cx| {
            let tiles = super::tiles(cx);
            tiles.update(cx, |t, cx| {
                let c = ShowCall { tool, tile: "main".into(), item: Some(item), input: Some(input.to_string()), caller: Caller { agent_id: "api".into(), agent_name: "api".into(), project: None } };
                let (id, content) = t.calls(&ext).begin(&c, Instant::now()).unwrap();
                t.deliver(&ext, "main", id, content, tx, cx).unwrap();
                id
            })
        });
        let end = Instant::now() + WAIT;
        while rx.try_recv().is_err() {
            assert!(Instant::now() < end, "no answer");
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(20));
        }
        // Recorded in its Activity, the view marked.
        cx.update(|cx| super::agents::finished(&ext, "main", id, &req, &Ok(()), cx));
    };
    let open = |cx: &mut HeadlessAppContext, key: &str, w: f32, h: f32| {
        let vt = std::rc::Rc::new(std::cell::RefCell::new(None));
        let (vt2, key) = (vt.clone(), key.to_string());
        let win = cx
            .open_window(size(px(w), px(h)), move |_, cx| {
                let v = crate::main_screen::view_tile::view(&key, cx);
                *vt2.borrow_mut() = Some(v.clone());
                cx.new(|_| super::tests::Shell(v.into(), true))
            })
            .unwrap();
        let vtile: Entity<crate::main_screen::view_tile::ViewTile> = vt.borrow().clone().unwrap();
        (win, vtile)
    };

    // Hello, with api's messages and its mark.
    let (win, vtile) = open(&mut cx, "hello/main", 520., 420.);
    settle(&mut cx, "the tile", &mut |cx| vtile.read(cx).tile().is_some());
    let tile = cx.update(|cx| vtile.read(cx).tile().cloned()).unwrap();
    settle(&mut cx, "hello", &mut |cx| tile.read(cx).texts().iter().any(|t| t.contains("Clicked 0 times")));
    call(&mut cx, "hello", Some("show_message"), "build", serde_json::json!({ "message": "Build green on main, 0.9.0 tagged" }));
    call(&mut cx, "hello", Some("show_message"), "review", serde_json::json!({ "message": "Two comments left on #412" }));
    settle(&mut cx, "the messages", &mut |cx| tile.read(cx).texts().iter().any(|t| t.contains("api: Two comments")));
    shot(&mut cx, win.into(), "hello");
    let _ = cx.update_window(win.into(), |_, window, _| window.remove_window());

    // Notes, with api's draft.
    let (win, vtile) = open(&mut cx, "notes/main", 420., 480.);
    settle(&mut cx, "the tile", &mut |cx| vtile.read(cx).tile().is_some());
    let tile = cx.update(|cx| vtile.read(cx).tile().cloned()).unwrap();
    settle(&mut cx, "notes", &mut |cx| tile.read(cx).texts().iter().any(|t| t.contains("No notes yet")));
    call(&mut cx, "notes", None, "standup", serde_json::json!({ "title": "Standup", "text": "- api: 429 retries merged\n- web: Safari flicker, still open\n- next: release 0.9" }));
    settle(&mut cx, "the draft", &mut |cx| tile.read(cx).texts().iter().any(|t| t == "Draft, not saved"));
    shot(&mut cx, win.into(), "notes-draft");
    let _ = cx.update_window(win.into(), |_, window, _| window.remove_window());

    // Settings: Hello's card (Agents, Activity), then Agents.
    let sv = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sv2 = sv.clone();
    let shell = cx
        .open_window(size(px(900.), px(1100.)), move |_, cx| {
            let v = cx.new(|cx| SettingsView::new(true, cx));
            *sv2.borrow_mut() = Some(v.clone());
            cx.new(|_| super::tests::Shell(v.into(), false))
        })
        .unwrap();
    let settings: Entity<SettingsView> = sv.borrow().clone().unwrap();
    cx.update(|cx| settings.update(cx, |v, cx| v.debug_activity("hello", true, cx)));
    cx.run_until_parked();
    let _ = cx.update_window(shell.into(), |_, window, _| window.refresh());
    cx.run_until_parked();
    // Down to its Agents part and Activity.
    let down = |cx: &mut HeadlessAppContext, dy: f32| {
        cx.update(|cx| settings.read(cx).scroll_by(px(dy)));
        let _ = cx.update_window(shell.into(), |_, window, _| window.refresh());
        cx.run_until_parked();
    };
    down(&mut cx, card_dy());
    shot(&mut cx, shell.into(), "card");
    cx.update(|cx| crate::settings::data::set_page(cx, Page::Agents));
    cx.run_until_parked();
    let _ = cx.update_window(shell.into(), |_, window, _| window.refresh());
    cx.run_until_parked();
    down(&mut cx, agents_dy());
    shot(&mut cx, shell.into(), "settings-agents");
}

/// How far the Extensions page is scrolled for the card's Agents part
/// (`PITWALL_SHOTS_CARD_DY` to adjust).
fn card_dy() -> f32 {
    std::env::var("PITWALL_SHOTS_CARD_DY").ok().and_then(|v| v.parse().ok()).unwrap_or(640.)
}

/// How far Settings → Agents is scrolled for "Agents can use tiles".
fn agents_dy() -> f32 {
    std::env::var("PITWALL_SHOTS_AGENTS_DY").ok().and_then(|v| v.parse().ok()).unwrap_or(600.)
}
