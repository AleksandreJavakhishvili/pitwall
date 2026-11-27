//! Agents open tiles end to end, headless (GPUI's test platform): an
//! agent's MCP calls through Pitwall's server (`mcp.tools`, `mcp.call`, as
//! `pitwall-cli mcp` passes them on) with the app's workspace behind it,
//! and the tile host's hello fixture. The tools list has `hello__open`
//! and the declared `hello__show_message`; a call with `where: right`
//! opens the tile right of the agent's terminal with the content; a second
//! call with the same item updates it in place; the tile's "Agents can
//! use" turned off takes its tools out of the list (the stamp changes, so
//! the agent's server sends `notifications/tools/list_changed`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext, Entity, Global, TestAppContext, WindowHandle};
use pitwall_core::extensions::activity::Via;
use pitwall_core::testing::{record, FakeProvider, ManualClock, MemStore, RecordingSink, TempDir};
use pitwall_daemon::{Approvals, Caller, Server};
use pitwall_proto::method;
use serde_json::{json, Value};

use crate::agents::tests::agent;
use crate::agents::AgentStore;
use crate::main_screen::tree::{Side, Surface};
use crate::main_screen::workspace::Whereabouts;
use crate::ui::{Content, MainView};
use crate::windows::MAIN;

/// How long the component may take to compile and answer.
const WAIT: Duration = Duration::from_secs(60);

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

/// Every window draws a frame, settled.
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

/// Run until `done` holds (the component and the server work on threads
/// of their own), or panic saying `what`.
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

/// The agent's server: "api", an agent Pitwall started.
fn api() -> Caller {
    Caller { agent: Some(("api".into(), "api".into())), ..Caller::outside() }
}

/// `method` from the agent's server, on a thread of its own, answered.
fn ask(s: &Arc<Server>, m: &'static str, p: Value, cx: &mut TestAppContext) -> Value {
    let h = {
        let s = s.clone();
        std::thread::spawn(move || s.call_as(&api(), m, p, Via::Socket))
    };
    until(cx, "answer", |_| h.is_finished());
    h.join().unwrap().unwrap_or_else(|e| panic!("{m}: {}", e.message))
}

fn names(tools: &Value) -> Vec<String> {
    tools["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str().map(String::from)).collect()
}

/// `mcp.call` of `name` with `arguments`: its result's text, and whether
/// it is an error.
fn call(s: &Arc<Server>, name: &str, arguments: Value, cx: &mut TestAppContext) -> (String, bool) {
    let r = ask(s, method::MCP_CALL, json!({ "name": name, "arguments": arguments }), cx);
    let text = r["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string();
    (text, r["result"]["isError"].as_bool().unwrap_or(false))
}

/// What the hello tile's view shows now (nothing while it isn't drawn).
fn shown(cx: &mut TestAppContext) -> Vec<String> {
    let key = Surface::View("hello".into(), "main".into()).key();
    cx.update(|cx| crate::main_screen::view_tile::get(&key, cx).and_then(|v| v.read(cx).tile().cloned()).map(|t| t.read(cx).texts()).unwrap_or_default())
}

fn has(text: &str, cx: &mut TestAppContext) -> bool {
    shown(cx).iter().any(|t| t.contains(text))
}

#[gpui::test]
fn an_agent_opens_a_tile_with_content_through_the_mcp_server(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-agents");
    let root = t.path().join("data");
    super::tests::copy_fixture(&root.join("extensions/hello"));
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
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true));
        crate::cli_spaces::attach(workspace.clone(), cx);
    });
    cx.run_until_parked();
    // api and web side by side in a space of main.
    cx.update(|cx| {
        let (ui, id) = crate::ui_state::get(cx).create_custom_space(MAIN);
        let ui = ui.place_agent(&id, "api", None, None).place_agent(&id, "web", None, Some(Side::Right));
        crate::ui_state::update(cx, |s| *s = ui);
        let h = crate::windows::main_window(MAIN, cx).unwrap();
        h.update(cx, |v, _, cx| v.screen().unwrap().update(cx, |s, cx| s.show_space(&id, cx))).unwrap();
    });
    frames(cx);
    // Pitwall's server, with the app's workspace and the same agents.
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

    // The list: every tile's open and the declared tool, read-only, and a
    // line in the instructions.
    let tools = ask(&server, method::MCP_TOOLS, json!({}), cx);
    let list = names(&tools);
    assert!(list.contains(&"hello__open".to_string()) && list.contains(&"hello__show_message".to_string()), "{list:?}");
    let open = tools["tools"].as_array().unwrap().iter().find(|t| t["name"] == "hello__open").unwrap().clone();
    assert_eq!(open["annotations"]["readOnlyHint"], json!(true), "{open}");
    assert!(open["inputSchema"]["properties"]["message"].is_object(), "the tile's input at the top: {open}");
    assert!(open["inputSchema"]["properties"]["where"].is_object(), "{open}");
    assert!(tools["instructions"].as_str().unwrap().contains("hello__"), "{}", tools["instructions"]);
    let stamp = tools["stamp"].as_str().unwrap().to_string();

    // hello__open, right of api's terminal: the tile opens with it.
    let (text, error) = call(&server, "hello__open", json!({ "message": "hi", "where": "right" }), cx);
    assert!(!error, "{text}");
    assert!(text.contains("Hello"), "{text}");
    let what = Surface::View("hello".into(), "main".into()).content();
    let at = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let ui = crate::ui_state::get(cx);
            let (space, pane) = ui.tile_of(&what)?;
            Some(crate::main_screen::workspace::whereabouts(&ui.space(&space)?.layout, &pane, Some("api")))
        })
    };
    assert_eq!(at(cx), Some(Whereabouts::Next(Side::Right, "api".into())));
    until(cx, "the content drawn", |cx| has(r#"Content: {"message":"hi"}"#, cx));
    // The view says who showed it.
    assert_eq!(cx.update(|cx| super::tiles(cx).read(cx).origin("hello", "main").cloned()), Some(("api".to_string(), "api".to_string())));

    // The declared tool, item "a": the tile's reply comes back.
    let (text, error) = call(&server, "hello__show_message", json!({ "item": "a", "message": "one" }), cx);
    assert!(!error && text.contains("Shown in Hello (1 messages)"), "{text}");
    until(cx, "the message", |cx| has("api: one", cx));
    // The same item again: updated in place, in the same tile.
    let (text, error) = call(&server, "hello__show_message", json!({ "item": "a", "message": "two" }), cx);
    assert!(!error && text.contains("(1 messages)"), "{text}");
    until(cx, "the update", |cx| has("api: two", cx) && !has("api: one", cx));
    assert_eq!(at(cx), Some(Whereabouts::Next(Side::Right, "api".into())), "not moved");
    // Input that doesn't fit: nothing shown, the problem named.
    let (text, error) = call(&server, "hello__show_message", json!({ "item": "b", "message": 5 }), cx);
    assert!(error && text.contains("`message` must be a string"), "{text}");
    // The calls are in its activity log, as the agent's.
    let log = cx.update(|cx| crate::extensions::calls::activity("hello", cx).lock().unwrap().entries());
    assert!(log.iter().filter(|e| e.via == Via::Agent).count() >= 3, "{log:?}");

    // "Agents can use Hello" off: its tools are gone, the stamp changes.
    cx.update(|cx| super::agents::set_tile_on("hello", "main", false, cx));
    let tools = ask(&server, method::MCP_TOOLS, json!({}), cx);
    let list = names(&tools);
    assert!(!list.iter().any(|n| n.starts_with("hello__")), "{list:?}");
    assert_ne!(tools["stamp"].as_str().unwrap(), stamp, "the agents' servers hear the list changed");
    let (text, error) = call(&server, "hello__open", json!({ "message": "again" }), cx);
    assert!(error, "{text}");
    // On again: back in the list.
    cx.update(|cx| super::agents::set_tile_on("hello", "main", true, cx));
    assert!(names(&ask(&server, method::MCP_TOOLS, json!({}), cx)).contains(&"hello__open".to_string()));
}
