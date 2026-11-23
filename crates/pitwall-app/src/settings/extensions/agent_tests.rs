//! An agent setting up an extension (`extension.*` through the app's
//! backend, `crate::extensions::agent_setup`): each act raises an approval
//! saying exactly what will run, asked by that agent; approved, it runs
//! what the card's button runs and the caller hears how it went; declined,
//! nothing runs. A first run's link reaches only the browser hook; a
//! secret is only marked on the card. Only the fake shell programs of
//! `service_tests` run, in a temporary folder.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gpui::{TestAppContext, VisualTestContext};
use pitwall_core::extensions::activity::Via;
use pitwall_core::paths::Paths;
use pitwall_core::testing::{FakeProvider, ManualClock, MemStore, RecordingSink, TempDir};
use pitwall_daemon::{Approvals, Caller, Server};
use pitwall_proto::{method, ApprovalAnswer, ApprovalView, ErrorBody, Risk};
use serde_json::{json, Value};

use super::service_tests::{gone, kit, open, running, starts, state, wait, SECRET};
use crate::extensions::agent_setup::{self, AppExtensions};
use crate::extensions::service::{OpenLink, State};

type Res = Result<Value, ErrorBody>;

/// The agent asking: an agent Pitwall started, named "api".
fn agent() -> Caller {
    Caller { agent: Some(("a1".into(), "api".into())), ..Caller::outside() }
}

/// A server with the app's extensions backend, attached to this app.
fn api(t: &TempDir, cx: &mut VisualTestContext) -> (Arc<Server>, Arc<Approvals>) {
    let engine = pitwall_core::Engine::open(pitwall_core::Deps {
        paths: Paths::new(t.path().join("engine")),
        events: RecordingSink::new(),
        clock: ManualClock::new(1),
        store: MemStore::with(vec![]),
        providers: vec![FakeProvider::named("local", "this-mac", Arc::new(pitwall_core::exec::LocalExec))],
    });
    let approvals = Approvals::new(Duration::from_secs(60));
    let backend = AppExtensions::new();
    let cfg = pitwall_daemon::Config { socket: t.path().join("unused.sock"), version: "test".into(), settings: None, workspace: None, viewer: None, events: None, extensions: None, ext_backend: Some(backend.clone()) };
    let identify = pitwall_daemon::ProcessIdentity::with_extensions(engine.clone(), pitwall_daemon::Registry::new());
    let server = pitwall_daemon::build(engine, approvals.clone(), identify, cfg);
    cx.executor().allow_parking();
    cx.update(|_, cx| agent_setup::attach(backend, cx));
    (server, approvals)
}

/// Real threads run and Pitwall's clock moves on.
fn pump(cx: &mut VisualTestContext) {
    std::thread::sleep(Duration::from_millis(20));
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
}

/// `method` called by the agent, from a thread of its own (as the socket
/// server does).
fn start(s: &Arc<Server>, m: &'static str, p: Value) -> JoinHandle<Res> {
    let s = s.clone();
    std::thread::spawn(move || s.call_as(&agent(), m, p, Via::Socket))
}

fn finish(cx: &mut VisualTestContext, h: JoinHandle<Res>) -> Res {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !h.is_finished() {
        assert!(Instant::now() < deadline, "no answer");
        pump(cx);
    }
    h.join().unwrap()
}

fn call(cx: &mut VisualTestContext, s: &Arc<Server>, m: &'static str, p: Value) -> Res {
    let h = start(s, m, p);
    finish(cx, h)
}

fn status(cx: &mut VisualTestContext, s: &Arc<Server>) -> Value {
    call(cx, s, method::EXTENSION_STATUS, json!({"id": "kit"})).unwrap()
}

/// The approval the user is asked now.
fn asked(cx: &mut VisualTestContext, a: &Approvals) -> ApprovalView {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(p) = a.pending().first().cloned() {
            return p;
        }
        assert!(Instant::now() < deadline, "nothing was asked");
        pump(cx);
    }
}

fn answer(a: &Approvals, p: &ApprovalView, allow: bool) {
    a.answer(&ApprovalAnswer { id: p.id.clone(), allow, remember: false }).unwrap();
}

/// Not installed: setup asks with its command line, declined runs nothing,
/// approved installs as the button does; then confirm lists its exact
/// commands and starts it; the first run opens its link through the hook
/// and the agent hears only that it did. Status moves at each step, and
/// a request while one waits is refused.
#[gpui::test]
fn an_agent_sets_up_a_service_step_by_step(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-agent-svc");
    let opened: Rc<RefCell<Vec<String>>> = Rc::default();
    let (_view, cx) = open(cx, &t, &kit(None, "a"), false);
    let o = opened.clone();
    cx.update(|_, cx| cx.set_global(OpenLink(Rc::new(move |url: &str, _| o.borrow_mut().push(url.to_string())))));
    let (s, approvals) = api(&t, cx);

    // Nothing installed: its install comes first.
    let st = status(cx, &s);
    assert_eq!((st["steps"][0]["kind"].as_str(), st["steps"][0]["state"].as_str()), (Some("serviceInstall"), Some("todo")));
    assert_eq!(st["next"]["command"], "pitwall extension setup kit --action install");

    // Declined: nothing runs, and the agent is told not to ask again.
    let h = start(&s, method::EXTENSION_ACT, json!({"id": "kit", "op": "setup", "action": "install"}));
    let p = asked(cx, &approvals);
    assert_eq!((p.summary.as_str(), p.requester.name.as_str(), p.risk), ("run “Install the server” of Kit", "api", Risk::High));
    assert!(p.details.iter().any(|d| d == "$ fake-install"), "{:?}", p.details);
    answer(&approvals, &p, false);
    let e = finish(cx, h).unwrap_err();
    assert_eq!(e.code, "denied");
    assert!(e.message.contains("Nothing ran"), "{}", e.message);
    for _ in 0..10 {
        pump(cx);
    }
    assert!(!t.path().join("bin/path/fake-srv").exists(), "declined: nothing installed");

    // Approved: the button's run, and its outcome comes back.
    let h = start(&s, method::EXTENSION_ACT, json!({"id": "kit", "op": "setup", "action": "install"}));
    let p = asked(cx, &approvals);
    answer(&approvals, &p, true);
    let done = finish(cx, h).unwrap();
    assert_eq!(done["approved"], true);
    assert!(done["text"].as_str().unwrap().contains("$ fake-install — done"), "{done}");
    assert!(t.path().join("bin/path/fake-srv").exists());
    // Looked at again after its install action, as after the button.
    let deadline = Instant::now() + Duration::from_secs(20);
    while cx.update(|_, cx| crate::extensions::service::try_services(cx).and_then(|v| v.read(cx).get("kit").and_then(|v| v.installed))) != Some(true) {
        assert!(Instant::now() < deadline, "never looked at again: {:?}", state(cx));
        pump(cx);
    }
    wait(cx, Duration::from_millis(100), "asks to confirm", |s| matches!(s, State::NeedsConfirm { .. }));
    let st = status(cx, &s);
    assert_eq!((st["steps"][0]["state"].as_str(), st["service"]["state"].as_str()), (Some("done"), Some("needsConfirmation")));
    assert_eq!(st["next"]["command"], "pitwall extension service kit confirm");

    // Confirm: its exact commands, data folder; then running.
    let h = start(&s, method::EXTENSION_ACT, json!({"id": "kit", "op": "service", "what": "confirm"}));
    let p = asked(cx, &approvals);
    assert!(p.details.iter().any(|d| d.starts_with("$ fake-srv --data ") && d.ends_with("extension-data/kit --tag a")), "{:?}", p.details);
    // Another request while it waits: refused, not asked.
    let e = call(cx, &s, method::EXTENSION_ACT, json!({"id": "kit", "op": "service", "what": "stop"})).unwrap_err();
    assert_eq!(e.code, "busy");
    assert_eq!(approvals.pending().len(), 1);
    assert_eq!(starts(&t), 0, "nothing starts before it is approved");
    answer(&approvals, &p, true);
    let done = finish(cx, h).unwrap();
    assert_eq!(done["text"], "It is running.");
    assert!(running(&state(cx)));
    let st = status(cx, &s);
    assert_eq!(st["service"]["state"], "running");
    assert_eq!(st["next"]["command"], "pitwall extension first-run kit");

    // The first run: the link goes to the hook only.
    let h = start(&s, method::EXTENSION_ACT, json!({"id": "kit", "op": "firstRun"}));
    let p = asked(cx, &approvals);
    let shown = format!("{p:?}");
    assert!(shown.contains("$ fake-srv link --data ") && !shown.contains(SECRET) && !shown.contains("https://"), "{shown}");
    answer(&approvals, &p, true);
    let done = finish(cx, h).unwrap();
    assert_eq!(done["text"], "Opened a one-time sign-in link in your browser.");
    assert!(!done.to_string().contains(SECRET) && !done.to_string().contains("example.test"), "{done}");
    assert_eq!(*opened.borrow(), [format!("https://example.test/first/{SECRET}")]);
    let log = call(cx, &s, method::EXTENSION_LOG, json!({"id": "kit"})).unwrap();
    assert!(!log.to_string().contains(SECRET), "{log}");
    let st = status(cx, &s);
    assert_eq!((st["service"]["firstRun"]["pending"].as_bool(), st["next"]["who"].as_str()), (Some(false), Some("nobody")));

    cx.update(|_, cx| crate::ui_state::update(cx, |s| *s = s.clone().set_extension("kit", false)));
    cx.run_until_parked();
    gone(&t, cx);
}

/// Turning on: the approval lists what it runs; approved, it is on, as
/// the card's switch does. Its service then waits for its own confirm.
#[gpui::test]
fn turning_on_shows_what_it_runs(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-agent-on");
    let (_view, cx) = open(cx, &t, &kit(None, "a"), true);
    cx.update(|_, cx| crate::ui_state::update(cx, |s| *s = s.clone().set_extension("kit", false)));
    let (s, approvals) = api(&t, cx);
    assert_eq!(status(cx, &s)["next"]["command"], "pitwall extension on kit");
    let h = start(&s, method::EXTENSION_ACT, json!({"id": "kit", "op": "on"}));
    let p = asked(cx, &approvals);
    assert_eq!(p.summary, "turn on the extension Kit 1.0.0");
    assert!(p.details.iter().any(|d| d.contains("keeps a local program running") && d.contains("Kit's server")), "{:?}", p.details);
    assert!(p.details.iter().any(|d| d.contains("“Install the server” runs only when asked and confirmed: fake-install")), "{:?}", p.details);
    answer(&approvals, &p, true);
    assert_eq!(finish(cx, h).unwrap()["text"], "It is on.");
    assert!(cx.update(|_, cx| crate::ui_state::get(cx).extension_on("kit")));
    wait(cx, Duration::from_millis(100), "asks to confirm", |s| matches!(s, State::NeedsConfirm { .. }));
    assert_eq!(starts(&t), 0, "turning on doesn't confirm its service");
    // Already on: refused before asking.
    let e = call(cx, &s, method::EXTENSION_ACT, json!({"id": "kit", "op": "on"})).unwrap_err();
    assert!(e.message.contains("on already"), "{}", e.message);
    assert!(approvals.pending().is_empty());
}

/// A plain setting through an approval; a secret only marked on the card
/// (its value never in any answer); `set` refuses a secret.
#[gpui::test]
fn settings_and_secrets(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-agent-secret");
    let manifest = r#"{"v": 2, "pitwall": ">=0.8.0", "id": "kit", "name": "Kit", "version": "1.0.0",
      "settings": [{"key": "server", "type": "url", "label": "Server address", "required": true},
                   {"key": "apiKey", "type": "secret", "label": "API key", "required": true}]}"#;
    let (view, cx) = open(cx, &t, manifest, true);
    let (s, approvals) = api(&t, cx);
    let st = status(cx, &s);
    assert_eq!(st["next"]["command"], "pitwall extension set kit server <value>");

    let h = start(&s, method::EXTENSION_ACT, json!({"id": "kit", "op": "set", "key": "server", "value": "https://inbox.example.com"}));
    let p = asked(cx, &approvals);
    assert_eq!((p.summary.as_str(), p.risk), ("set “Server address” of Kit", Risk::Low));
    assert!(p.details[0].ends_with("(not set) → https://inbox.example.com"), "{:?}", p.details);
    answer(&approvals, &p, true);
    finish(cx, h).unwrap();
    let st = status(cx, &s);
    assert_eq!(st["connections"][0]["values"]["server"], "https://inbox.example.com");
    assert_eq!((st["steps"][0]["state"].as_str(), st["steps"][1]["state"].as_str()), (Some("done"), Some("todo")));
    assert_eq!(st["next"]["who"], "user");
    assert_eq!(st["next"]["command"], "pitwall extension secret kit apiKey");

    let e = call(cx, &s, method::EXTENSION_ACT, json!({"id": "kit", "op": "set", "key": "apiKey", "value": "sk-123"})).unwrap_err();
    assert!(e.message.contains("only the user enters it"), "{}", e.message);
    assert!(approvals.pending().is_empty(), "never asked");

    // What `extension secret` has the tile do: the field marked.
    view.update(cx, |v, cx| v.ask_secret("kit", "apiKey", None, cx));
    assert!(view.read_with(cx, |v, _| v.secret_asked("kit", "apiKey")));
    // The user enters it in Pitwall: only "set" comes back.
    let (tx, rx) = std::sync::mpsc::channel();
    cx.update(|_, cx| crate::extensions::connections::set_secret_in("kit", None, "apiKey", Some("sk-SECRET-42".into()), cx, move |e, _| tx.send(e).unwrap()));
    let deadline = Instant::now() + Duration::from_secs(10);
    let set = loop {
        if let Ok(e) = rx.try_recv() {
            break e;
        }
        assert!(Instant::now() < deadline);
        pump(cx);
    };
    assert_eq!(set, None);
    super::accounts_tests::frame(cx);
    assert!(!view.read_with(cx, |v, _| v.secret_asked("kit", "apiKey")), "set: no longer marked");
    let st = status(cx, &s);
    assert_eq!(st["connections"][0]["secrets"], json!({"apiKey": true}));
    assert_eq!(st["steps"][1]["state"], "done");
    assert!(!st.to_string().contains("sk-SECRET-42"));
    assert!(!call(cx, &s, method::EXTENSION_LIST, Value::Null).unwrap().to_string().contains("sk-SECRET-42"));
}
