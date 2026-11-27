//! Screenshots of the C4 seed tile (tiles/c4, its committed tile.wasm)
//! drawn by the app after an agent's `c4__show` calls, headless (GPUI's
//! test platform, no window shown): a diagram drawn, the same after an
//! update and its Changes (chosen in the bar, then B back to Now), the
//! menu of a box several diagrams detail (under the box), and a call with
//! bad Mermaid with its line marked as an error, dark and light. Run by
//! hand:
//!
//! `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app c4_shots -- --ignored`
#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{px, size, AnyWindowHandle, AppContext, Entity, HeadlessAppContext};
use pitwall_core::testing::TempDir;
use pitwall_proto::tile_tools::TileShow;
use pitwall_tile_host::agent::{Caller, ShowCall};
use pitwall_tile_host::ui;

use crate::extensions::catalog;

const WAIT: Duration = Duration::from_secs(120);

const CONTEXT: &str = "flowchart TB\n  buyer[\"Buyer [Person]<br/>orders things\"]\n  shop[\"Shop [Software System]<br/>sells things\"]\n  pay[\"Payments [External System]<br/>takes cards\"]\n  buyer -->|orders with| shop\n  shop -->|charges with| pay\n  class pay external\n";

const CONTAINERS: &str = "flowchart LR\n  buyer[\"Buyer [Person]<br/>orders things\"]\n  subgraph shop[\"Shop [Software System]\"]\n    web[\"Web app [Container]<br/>pages, in React\"]\n    api[\"Orders API [Container]<br/>takes orders\"]\n    db[(\"Orders DB [Database]<br/>orders, in Postgres\")]\n  end\n  pay[\"Payments [External System]\"]\n  buyer -->|uses| web\n  web -->|calls, JSON| api\n  api -->|reads and writes| db\n  api -->|charges with| pay\n  class pay external\n";

const CONTAINERS_2: &str = "flowchart LR\n  buyer[\"Buyer [Person]<br/>orders things\"]\n  subgraph shop[\"Shop [Software System]\"]\n    web[\"Web app [Container]<br/>pages, in React\"]\n    api[\"Orders API [Container]<br/>takes orders, caches prices\"]\n    cache[\"Price cache [Container]<br/>Redis\"]\n    db[(\"Orders DB [Database]<br/>orders, in Postgres\")]\n  end\n  pay[\"Payments [External System]\"]\n  buyer -->|uses| web\n  web -->|calls, JSON| api\n  api -->|reads| cache\n  api -->|reads and writes| db\n  api -->|charges with| pay\n  class pay external\n";

const BAD: &str = "flowchart LR\n  web[\"Web app [Container]\"]\n  api[\"Orders API [Container]\"]\n  web -->|calls api\n";

#[test]
#[ignore]
fn c4_shots() {
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("c4-shots");
    let root = t.path().join("data");
    super::c4_tests::copy_c4(&root.join("extensions/c4"));

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
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("c4", true));
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
            img.save(out.join(format!("c4-{name}-{theme}.png"))).expect("saved");
        }
    };
    // An agent's `c4__show`, as Pitwall's MCP server hands it over.
    let call = |cx: &mut HeadlessAppContext, item: &str, input: serde_json::Value| {
        let (tx, rx) = std::sync::mpsc::channel();
        let req = TileShow { agent_id: "api".into(), agent_name: "api".into(), extension: "c4".into(), tile: "main".into(), tool: Some("show".into()), item: Some(item.into()), input: Some(input.to_string()), ..Default::default() };
        let id = cx.update(|cx| {
            super::tiles(cx).update(cx, |t, cx| {
                let c = ShowCall { tool: Some("show".into()), tile: "main".into(), item: Some(item.into()), input: Some(input.to_string()), caller: Caller { agent_id: "api".into(), agent_name: "api".into(), project: None }, append: false, large: None };
                let (id, content) = t.calls("c4").begin(&c, Instant::now()).unwrap();
                t.deliver("c4", "main", id, content, tx, cx).unwrap();
                id
            })
        });
        let end = Instant::now() + WAIT;
        while rx.try_recv().is_err() {
            assert!(Instant::now() < end, "no answer");
            cx.run_until_parked();
            std::thread::sleep(Duration::from_millis(20));
        }
        cx.update(|cx| super::agents::finished("c4", "main", id, &req, &Ok(()), cx));
    };
    let vt = std::rc::Rc::new(std::cell::RefCell::new(None));
    let vt2 = vt.clone();
    let win = cx
        .open_window(size(px(920.), px(560.)), move |_, cx| {
            let v = crate::main_screen::view_tile::view("c4/main", cx);
            *vt2.borrow_mut() = Some(v.clone());
            cx.new(|_| super::tests::Shell(v.into(), true))
        })
        .unwrap();
    let vtile: Entity<crate::main_screen::view_tile::ViewTile> = vt.borrow().clone().unwrap();
    settle(&mut cx, "the tile", &mut |cx| vtile.read(cx).tile().is_some());
    let tile = cx.update(|cx| vtile.read(cx).tile().cloned()).unwrap();
    for _ in 0..5 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(50));
    }
    shot(&mut cx, win.into(), "empty");
    let has = |cx: &mut gpui::App, s: &str| {
        let t = tile.read(cx);
        t.texts().iter().chain(t.canvas_texts().iter()).any(|t| t.contains(s))
    };

    // Drawn after a call, then its changes after an update.
    call(&mut cx, "containers", serde_json::json!({ "mermaid": CONTAINERS, "title": "Shop containers" }));
    settle(&mut cx, "the diagram", &mut |cx| has(cx, "Orders API"));
    shot(&mut cx, win.into(), "drawn");
    call(&mut cx, "containers", serde_json::json!({ "mermaid": CONTAINERS_2 }));
    settle(&mut cx, "the update", &mut |cx| has(cx, "Price cache"));
    let click = |cx: &mut HeadlessAppContext, path: &str| {
        let p = path.to_string();
        let ok = cx.update(|cx| tile.read(cx).click(&p, ui::MouseButton::Left, ui::Modifiers::empty(), 1, ui::Point { x: 1., y: 1. }, cx));
        assert!(ok, "no node {path} to click: {:?}", cx.update(|cx| tile.read(cx).texts()));
    };
    shot(&mut cx, win.into(), "updated");
    // Its Changes, chosen in the bar's segmented control; B goes back.
    let ok = cx.update(|cx| tile.read(cx).choose("bar/mode", "changes", cx));
    assert!(ok, "no Changes to choose: {:?}", cx.update(|cx| tile.read(cx).texts()));
    for _ in 0..5 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(50));
    }
    shot(&mut cx, win.into(), "changes");
    let ok = cx.update(|cx| tile.read(cx).press_key("b", cx));
    assert!(ok, "B isn't bound");
    for _ in 0..5 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(50));
    }

    // A box two diagrams detail: which one?
    call(&mut cx, "containers-old", serde_json::json!({ "mermaid": CONTAINERS, "title": "Shop containers, before the cache" }));
    call(&mut cx, "context", serde_json::json!({ "mermaid": CONTEXT, "title": "Shop" }));
    settle(&mut cx, "the context", &mut |cx| has(cx, "Payments") && !has(cx, "Orders API"));
    let ok = cx.update(|cx| tile.read(cx).click_region("body/main/diagram", "shop", cx));
    assert!(ok, "no region shop");
    settle(&mut cx, "the menu", &mut |cx| has(cx, "Open which diagram?"));
    shot(&mut cx, win.into(), "box-menu");
    click(&mut cx, "body/main/diagram/menu/targets/t0");
    settle(&mut cx, "the detail", &mut |cx| has(cx, "Orders API"));

    // Bad Mermaid: nothing changes; the notice, then its line.
    call(&mut cx, "containers", serde_json::json!({ "mermaid": BAD }));
    for _ in 0..5 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(50));
    }
    shot(&mut cx, win.into(), "error");
    click(&mut cx, "notice0");
    settle(&mut cx, "the source", &mut |cx| has(cx, "Line 4"));
    shot(&mut cx, win.into(), "error-line");
}
