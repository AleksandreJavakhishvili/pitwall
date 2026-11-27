//! The C4 seed tile (tiles/c4, its committed tile.wasm) end to end,
//! headless (GPUI's test platform): an agent calls `c4__show` through
//! Pitwall's MCP server, as `pitwall-cli mcp` passes it on. The tools list
//! has it; a call with `where: right` draws the diagram right of the
//! agent's terminal; the same item again updates it in place, where it
//! is; bad Mermaid changes nothing and comes back with its line.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext, Entity, Global, TestAppContext, WindowHandle};
use pitwall_core::testing::{record, FakeProvider, ManualClock, MemStore, RecordingSink, TempDir};
use pitwall_daemon::{Approvals, Caller, Server};
use pitwall_core::extensions::activity::Via;
use pitwall_proto::method;
use serde_json::{json, Value};

use crate::agents::tests::agent;
use crate::agents::AgentStore;
use crate::main_screen::tree::{Side, Surface};
use crate::main_screen::workspace::Whereabouts;
use crate::ui::{Content, MainView};
use crate::windows::MAIN;

const WAIT: Duration = Duration::from_secs(90);

struct Main(Option<WindowHandle<MainView>>, Entity<AgentStore>);

impl Global for Main {}

fn show_main(cx: &mut gpui::App) -> WindowHandle<MainView> {
    if let Some(h) = cx.global::<Main>().0 {
        if cx.windows().iter().any(|w| w.window_id() == h.window_id()) {
            return h;
        }
    }
    let store = cx.global::<Main>().1.clone();
    let h = cx.open_window(Default::default(), |window, cx| cx.new(|cx| MainView::new(Content::Live { store }, None, window, cx))).unwrap();
    cx.global_mut::<Main>().0 = Some(h);
    h
}

fn frames(cx: &mut TestAppContext) {
    for _ in 0..2 {
        cx.update(|cx| {
            for w in cx.windows() {
                let _ = w.update(cx, |_, window, _| window.refresh());
            }
        });
        cx.run_until_parked();
    }
}

fn until(cx: &mut TestAppContext, what: &str, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    let end = Instant::now() + WAIT;
    loop {
        frames(cx);
        if done(cx) {
            return;
        }
        assert!(Instant::now() < end, "no {what} within {WAIT:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn api() -> Caller {
    Caller { agent: Some(("api".into(), "api".into())), ..Caller::outside() }
}

fn ask(s: &Arc<Server>, m: &'static str, p: Value, cx: &mut TestAppContext) -> Value {
    let h = {
        let s = s.clone();
        std::thread::spawn(move || s.call_as(&api(), m, p, Via::Socket))
    };
    until(cx, "answer", |_| h.is_finished());
    h.join().unwrap().unwrap_or_else(|e| panic!("{m}: {}", e.message))
}

fn call(s: &Arc<Server>, name: &str, arguments: Value, cx: &mut TestAppContext) -> (String, bool) {
    let r = ask(s, method::MCP_CALL, json!({ "name": name, "arguments": arguments }), cx);
    let text = r["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string();
    (text, r["result"]["isError"].as_bool().unwrap_or(false))
}

/// The C4 tile as committed: its manifest, input schema and tile.wasm.
pub(super) fn copy_c4(to: &Path) {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tiles/c4");
    std::fs::create_dir_all(to).unwrap();
    for f in ["extension.json", "input.schema.json", "tile.wasm"] {
        std::fs::copy(from.join(f), to.join(f)).unwrap_or_else(|e| panic!("tiles/c4/{f}: {e}"));
    }
}

/// What the C4 tile's view shows now: its texts, the drawing's labels
/// among them.
fn shown(cx: &mut TestAppContext) -> Vec<String> {
    let key = Surface::View("c4".into(), "main".into()).key();
    cx.update(|cx| crate::main_screen::view_tile::get(&key, cx).and_then(|v| v.read(cx).tile().cloned()).map(|t| { let t = t.read(cx); let mut all = t.texts(); all.extend(t.canvas_texts()); all }).unwrap_or_default())
}

fn has(text: &str, cx: &mut TestAppContext) -> bool {
    shown(cx).iter().any(|t| t.contains(text))
}

const CONTAINERS: &str = "flowchart LR\n  buyer[\"Buyer [Person]\"]\n  subgraph shop[\"Shop [Software System]\"]\n    web[\"Web app [Container]<br/>pages\"]\n    api[\"Orders API [Container]<br/>takes orders\"]\n  end\n  buyer -->|uses| web\n  web -->|calls| api\n";

#[gpui::test]
fn an_agent_draws_a_diagram_on_the_right_with_c4_show(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-c4");
    let root = t.path().join("data");
    copy_c4(&root.join("extensions/c4"));
    cx.executor().allow_parking();
    let workspace = crate::cli_spaces::AppWorkspace::new();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::main_screen::init(cx);
        crate::ui_state::init(None, cx);
        crate::preview::fake::install(cx);
        crate::extensions::init(Some(root.clone()), cx);
        crate::extensions::catalog::set_bundle(None, cx);
        cx.set_global(super::DataDir(root.clone()));
        let store = cx.new(|_| AgentStore::new(vec![agent("api", "/work/alpha", "idle", 1, true), agent("web", "/work/alpha", "idle", 2, true)]));
        cx.set_global(Main(None, store.clone()));
        cx.set_global(crate::AppState { host: None, content: Content::Live { store: store.clone() }, root: None, main: None });
        crate::windows::init(Content::Live { store }, None, show_main, cx);
        show_main(cx);
        crate::windows::restore(cx);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("c4", true));
        crate::cli_spaces::attach(workspace.clone(), cx);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let (ui, id) = crate::ui_state::get(cx).create_custom_space(MAIN);
        let ui = ui.place_agent(&id, "api", None, None).place_agent(&id, "web", None, Some(Side::Right));
        crate::ui_state::update(cx, |s| *s = ui);
        let h = crate::windows::main_window(MAIN, cx).unwrap();
        h.update(cx, |v, _, cx| v.screen().unwrap().update(cx, |s, cx| s.show_space(&id, cx))).unwrap();
    });
    frames(cx);
    let engine = pitwall_core::Engine::open(pitwall_core::Deps {
        paths: pitwall_core::paths::Paths::new(t.path().join("engine")),
        events: RecordingSink::new(),
        clock: ManualClock::new(1),
        store: MemStore::with(vec![record("api", "/work/alpha"), record("web", "/work/alpha")]),
        providers: vec![FakeProvider::named("local", "this-mac", Arc::new(pitwall_core::exec::LocalExec))],
    });
    let cfg = pitwall_daemon::Config { socket: t.path().join("unused.sock"), version: "test".into(), settings: None, workspace: None, viewer: Some(workspace), events: None, extensions: None, ext_backend: None };
    let identify = pitwall_daemon::ProcessIdentity::with_extensions(engine.clone(), pitwall_daemon::Registry::new());
    let server = pitwall_daemon::build(engine, Approvals::new(Duration::from_secs(60)), identify, cfg);

    // The tools list: c4__show, with Pitwall's item and where.
    let tools = ask(&server, method::MCP_TOOLS, json!({}), cx);
    let show = tools["tools"].as_array().unwrap().iter().find(|t| t["name"] == "c4__show").unwrap_or_else(|| panic!("no c4__show: {tools}")).clone();
    assert!(show["inputSchema"]["properties"]["mermaid"].is_object(), "{show}");
    assert!(show["inputSchema"]["properties"]["item"].is_object() && show["inputSchema"]["properties"]["where"].is_object(), "{show}");

    // "Draw me the containers, on the right."
    let (text, error) = call(&server, "c4__show", json!({ "mermaid": CONTAINERS, "title": "Shop containers", "item": "containers", "where": "right" }), cx);
    assert!(!error, "{text}");
    assert!(text.contains("Showing “Shop containers” in the C4 tile"), "{text}");
    let what = Surface::View("c4".into(), "main".into()).content();
    let at = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let ui = crate::ui_state::get(cx);
            let (space, pane) = ui.tile_of(&what)?;
            Some(crate::main_screen::workspace::whereabouts(&ui.space(&space)?.layout, &pane, Some("api")))
        })
    };
    assert_eq!(at(cx), Some(Whereabouts::Next(Side::Right, "api".into())));
    until(cx, "the diagram drawn", |cx| has("Orders API", cx) && has("Web app", cx));

    // The same item, changed, without `where`: updated where it is.
    let changed = CONTAINERS.replace("    api[", "    cache[\"Price cache [Container]\"]\n    api[").replace("  web -->|calls| api\n", "  web -->|calls| api\n  api -->|reads| cache\n");
    let (text, error) = call(&server, "c4__show", json!({ "mermaid": changed, "item": "containers" }), cx);
    assert!(!error && text.contains("Updated “Shop containers” in place") && text.contains("1 boxes added"), "{text}");
    until(cx, "the update", |cx| has("Price cache", cx));
    assert_eq!(at(cx), Some(Whereabouts::Next(Side::Right, "api".into())), "not moved");

    // Its Changes, chosen in the bar as a click would; B goes back to Now.
    let key = Surface::View("c4".into(), "main".into()).key();
    let tile = cx.update(|cx| crate::main_screen::view_tile::get(&key, cx).and_then(|v| v.read(cx).tile().cloned())).unwrap();
    assert!(cx.update(|cx| tile.read(cx).choose("bar/mode", "changes", cx)), "{:?}", shown(cx));
    let chosen = |cx: &mut TestAppContext| cx.update(|cx| tile.read(cx).chosen("bar/mode"));
    until(cx, "the changes", |cx| chosen(cx).as_deref() == Some("changes"));
    assert!(!cx.update(|cx| tile.read(cx).choose("bar/mode", "nowhere", cx)), "no such option");
    assert!(cx.update(|cx| tile.read(cx).press_key("b", cx)));
    until(cx, "Now again", |cx| chosen(cx).as_deref() == Some("now"));
    assert!(!cx.update(|cx| tile.read(cx).press_key("cmd-q", cx)), "not bound");

    // Bad Mermaid: an error with the line; the drawing stays.
    let (text, error) = call(&server, "c4__show", json!({ "mermaid": "flowchart LR\n  a[\"A\"]\n  a -->|calls b\n", "item": "containers" }), cx);
    assert!(error && text.contains("nothing was changed") && text.contains("line 3, column"), "{text}");
    frames(cx);
    assert!(has("Price cache", cx), "{:?}", shown(cx));
}
