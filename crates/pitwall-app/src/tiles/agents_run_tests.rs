//! Agents' `read` and `act` calls end to end, headless (GPUI's test
//! platform): through Pitwall's server (`mcp.tools`, `mcp.call`) with the
//! app's workspace and approvals behind it, and the hello fixture's
//! `current_messages` (read) and `send_message` (act). The list keeps
//! their hints; a read asks, Deny refuses it, "Always for api" lets the
//! next one through without asking; an act asks each time; the grant is
//! on the card and revocable; Undo beside the mark takes back a present
//! call; a present call appends and names a file.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext, Entity, Global, TestAppContext, WindowHandle};
use pitwall_core::extensions::activity::Via;
use pitwall_core::testing::{record, FakeProvider, ManualClock, MemStore, RecordingSink, TempDir};
use pitwall_daemon::{Approvals, Caller, Server};
use pitwall_proto::{method, ApprovalAnswer};
use serde_json::{json, Value};

use crate::agents::tests::agent;
use crate::agents::AgentStore;
use crate::main_screen::tree::Surface;
use crate::ui::{Content, MainView};

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

/// A plain Pitwall terminal the user types in.
fn term() -> Caller {
    Caller { agent: Some(("term".into(), "term".into())), ..Caller::outside() }
}

/// A Claude Code agent's record.
fn claude(id: &str, cwd: &str) -> pitwall_core::model::AgentRecord {
    let mut v = serde_json::to_value(record(id, cwd)).unwrap();
    v["kind"] = json!("claude");
    v["kindName"] = json!("Claude Code");
    serde_json::from_value(v).unwrap()
}

/// What the user does when asked.
#[derive(Clone, Copy)]
enum User {
    /// Isn't asked (the call goes through, or is refused, by itself).
    NotAsked,
    Deny,
    Once,
    Always,
}

/// `mcp.call` of `name` from api, the user answering `user` in the
/// approval (whose "always" label must be `always`): its result's text,
/// whether it is an error, and its JSON.
fn call(s: &Arc<Server>, a: &Arc<Approvals>, name: &str, arguments: Value, user: User, always: Option<&str>, cx: &mut TestAppContext) -> (String, bool, Value) {
    call_by(api(), s, a, name, arguments, user, always, cx)
}

/// [`call`] by `who`.
#[allow(clippy::too_many_arguments)]
fn call_by(who: Caller, s: &Arc<Server>, a: &Arc<Approvals>, name: &str, arguments: Value, user: User, always: Option<&str>, cx: &mut TestAppContext) -> (String, bool, Value) {
    let h = {
        let s = s.clone();
        let p = json!({ "name": name, "arguments": arguments });
        std::thread::spawn(move || s.call_as(&who, method::MCP_CALL, p, Via::Socket))
    };
    if !matches!(user, User::NotAsked) {
        until(cx, "the approval", |_| !a.pending().is_empty() || h.is_finished());
        let Some(p) = a.pending().first().cloned() else { panic!("{name} wasn't asked: {:?}", h.join().unwrap().map(|r| r["result"].clone()).map_err(|e| e.message)) };
        assert_eq!(p.always.as_deref(), always, "{p:?}");
        assert!(p.summary.contains(name.split("__").nth(1).unwrap()), "names the tool: {}", p.summary);
        let (allow, remember) = match user {
            User::Deny => (false, false),
            User::Once => (true, false),
            _ => (true, true),
        };
        a.answer(&ApprovalAnswer { id: p.id, allow, remember }).unwrap();
    }
    until(cx, "the answer", |_| h.is_finished());
    assert!(a.pending().is_empty(), "nothing else asked");
    let r = h.join().unwrap().unwrap_or_else(|e| panic!("{name}: {}", e.message));
    let text = r["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string();
    (text, r["result"]["isError"].as_bool().unwrap_or(false), r["result"]["structuredContent"].clone())
}

fn shown(cx: &mut TestAppContext) -> Vec<String> {
    let key = Surface::View("hello".into(), "main".into()).key();
    cx.update(|cx| crate::main_screen::view_tile::get(&key, cx).and_then(|v| v.read(cx).tile().cloned()).map(|t| t.read(cx).texts()).unwrap_or_default())
}

fn has(text: &str, cx: &mut TestAppContext) -> bool {
    shown(cx).iter().any(|t| t.contains(text))
}

#[gpui::test]
fn an_agent_reads_and_acts_with_the_users_approval(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-agents-run");
    let root = t.path().join("data");
    super::tests::copy_fixture(&root.join("extensions/hello"));
    // A file in api's project for `from_file`.
    let project = t.path().join("alpha");
    std::fs::create_dir_all(project.join("docs")).unwrap();
    std::fs::write(project.join("docs/long.txt"), "x".repeat(300_000)).unwrap();
    let project = project.to_string_lossy().to_string();
    cx.executor().allow_parking();
    let workspace = crate::cli_spaces::AppWorkspace::new();
    let approvals = Approvals::new(Duration::from_secs(60));
    let a2 = approvals.clone();
    let p2 = project.clone();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::main_screen::init(cx);
        crate::ui_state::init(None, cx);
        crate::preview::fake::install(cx);
        crate::extensions::init(Some(root.clone()), cx);
        crate::extensions::catalog::set_bundle(None, cx);
        cx.set_global(super::DataDir(root.clone()));
        let store = cx.new(|_| AgentStore::new(vec![agent("api", &p2, "idle", 1, true)]));
        crate::approvals::init(&store, a2, cx);
        cx.set_global(Main(None, store.clone()));
        cx.set_global(crate::AppState { host: None, content: Content::Live { store: store.clone() }, root: None, main: None });
        crate::windows::init(Content::Live { store }, None, show_main, cx);
        show_main(cx);
        crate::windows::restore(cx);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true));
        crate::cli_spaces::attach(workspace.clone(), cx);
    });
    cx.run_until_parked();
    let engine = pitwall_core::Engine::open(pitwall_core::Deps {
        paths: pitwall_core::paths::Paths::new(t.path().join("engine")),
        events: RecordingSink::new(),
        clock: ManualClock::new(1),
        store: MemStore::with(vec![claude("api", &project), record("term", &project)]),
        providers: vec![FakeProvider::named("local", "this-mac", Arc::new(pitwall_core::exec::LocalExec))],
    });
    let cfg = pitwall_daemon::Config { socket: t.path().join("unused.sock"), version: "test".into(), settings: None, workspace: None, viewer: Some(workspace), events: None, extensions: None, ext_backend: None };
    let identify = pitwall_daemon::ProcessIdentity::with_extensions(engine.clone(), pitwall_daemon::Registry::new());
    let server = pitwall_daemon::build(engine, approvals.clone(), identify, cfg);
    let s = &server;
    let a = &approvals;

    // The list: read is read-only, act carries its own hints; the launch
    // allow-lists get neither.
    let tools = {
        let h = {
            let s = s.clone();
            std::thread::spawn(move || s.call_as(&api(), method::MCP_TOOLS, json!({}), Via::Socket))
        };
        until(cx, "the tools", |_| h.is_finished());
        h.join().unwrap().unwrap()
    };
    let tool = |n: &str| tools["tools"].as_array().unwrap().iter().find(|t| t["name"] == n).cloned().unwrap_or_else(|| panic!("no {n}: {tools}"));
    assert_eq!(tool("hello__current_messages")["annotations"]["readOnlyHint"], json!(true));
    let act = tool("hello__send_message");
    assert_eq!((act["annotations"]["readOnlyHint"].clone(), act["annotations"]["destructiveHint"].clone()), (json!(false), json!(false)), "{act}");
    assert!(act["inputSchema"]["properties"].get("where").is_none(), "{act}");
    let allowed = cx.update(|cx| super::agents::tool_names(cx));
    assert!(allowed.contains(&"hello__show_message".to_string()) && !allowed.iter().any(|n| n == "hello__current_messages" || n == "hello__send_message"), "{allowed:?}");

    // Something to read: api presents a message (no approval).
    let (text, error, _) = call(s, a, "hello__show_message", json!({ "item": "a", "message": "first", "where": "right" }), User::NotAsked, None, cx);
    assert!(!error, "{text}");
    until(cx, "the message", |cx| has("api: first", cx));

    // A read asks; denied, nothing comes back.
    let (text, error, _) = call(s, a, "hello__current_messages", json!({}), User::Deny, Some("Always for api"), cx);
    assert!(error && text.contains("denied"), "{text}");
    // Allowed always: the data, and no question the next time.
    let (text, error, data) = call(s, a, "hello__current_messages", json!({}), User::Always, Some("Always for api"), cx);
    assert!(!error && text.contains("1 messages"), "{text}");
    assert_eq!(data["api/a"], json!("first"), "{data}");
    let (_, error, _) = call(s, a, "hello__current_messages", json!({}), User::NotAsked, None, cx);
    assert!(!error);
    assert!(cx.update(|cx| super::agents::granted("hello", "api", "current_messages", cx)), "kept for the card");

    // An act asks each time; once, then denied.
    let (text, error, _) = call(s, a, "hello__send_message", json!({ "text": "hi" }), User::Once, Some("Always for api"), cx);
    assert!(!error && text.contains("Sent “hi” for api"), "{text}");
    let (text, error, _) = call(s, a, "hello__send_message", json!({ "text": "no" }), User::Deny, Some("Always for api"), cx);
    assert!(error && text.contains("Nothing was done"), "{text}");
    until(cx, "what was sent", |cx| has("Sent: hi;", cx));

    // The user in a plain terminal (`pitwall extension run`): their read
    // runs, their act asks, with no "always".
    let (text, error, _) = call_by(term(), s, a, "hello__current_messages", json!({}), User::NotAsked, None, cx);
    assert!(!error, "{text}");
    let (text, error, _) = call_by(term(), s, a, "hello__send_message", json!({ "text": "mine" }), User::Once, None, cx);
    assert!(!error && text.contains("for the user"), "{text}");

    // Revoked on the card: the read asks again.
    cx.update(|cx| super::agents::revoke("hello", "api", Some("current_messages"), cx));
    let (_, error, _) = call(s, a, "hello__current_messages", json!({}), User::Once, Some("Always for api"), cx);
    assert!(!error);

    // Undo beside the mark: the item before api's last present call.
    let (_, error, _) = call(s, a, "hello__show_message", json!({ "item": "a", "message": "second" }), User::NotAsked, None, cx);
    assert!(!error);
    until(cx, "the change", |cx| has("api: second", cx));
    let undo = cx.update(|cx| super::tiles(cx).read(cx).last_present("hello", "main")).expect("Undo offered");
    cx.update(|cx| super::tiles(cx).update(cx, |t, cx| t.undo_present("hello", "main", undo, cx)));
    until(cx, "the undo", |cx| has("api: first", cx) && !has("api: second", cx));
    assert!(cx.update(|cx| super::tiles(cx).read(cx).last_present("hello", "main")).is_none(), "the mark goes with it");

    // Appends add to the item; a file in the project is read in parts.
    let (text, error, _) = call(s, a, "hello__show_message", json!({ "item": "a", "message": "+more", "mode": "append" }), User::NotAsked, None, cx);
    assert!(!error, "{text}");
    until(cx, "the append", |cx| has("api: first+more", cx));
    let (text, error, _) = call(s, a, "hello__show_message", json!({ "item": "f", "from_file": "docs/long.txt" }), User::NotAsked, None, cx);
    assert!(!error, "{text}");
    until(cx, "the file", |cx| shown(cx).iter().any(|t| t.starts_with("api: xxxx") && t.len() > 300_000));
    let (text, error, _) = call(s, a, "hello__show_message", json!({ "item": "g", "from_file": "../outside.txt" }), User::NotAsked, None, cx);
    assert!(error && text.contains("from_file"), "{text}");

    // Each call is in the tile's Activity.
    let log = cx.update(|cx| crate::extensions::calls::activity("hello", cx).lock().unwrap().entries());
    assert!(log.iter().any(|e| e.via == Via::Agent && e.method.contains("send_message")), "{log:?}");
}
