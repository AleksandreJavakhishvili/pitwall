//! The agents' questions group in the app, headless, with the tile host's
//! questions fixture ("Release desk") and a daemon server answering its
//! calls: it reads the agents through the method table, its badge is on
//! the agent's sidebar row, its question is a floating control card whose
//! answer comes back to it, and what it queues lands in the agent's Next
//! up marked with its name.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use pitwall_core::extensions::permissions::Decision;
use pitwall_core::testing::TempDir;
use pitwall_proto::AgentView;

use super::{group, TileView};
use crate::extensions::catalog;

const WAIT: Duration = Duration::from_secs(60);

pub(crate) fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/questions")
}

/// An engine with agents `web` and `api`, and a daemon server for
/// extensions' calls on it.
pub(crate) fn server(t: &TempDir) -> (pitwall_core::engine::Shared, Arc<pitwall_daemon::Server>) {
    use pitwall_core::testing::{record, FakeProvider, ManualClock, MemStore, RecordingSink};
    let proj = t.path().join("api").to_string_lossy().to_string();
    let mut web = record("a1", &proj);
    web.name = "web".into();
    let mut api = record("a2", &proj);
    api.name = "api".into();
    let events = pitwall_daemon::Hub::new(RecordingSink::new());
    let engine = pitwall_core::Engine::open(pitwall_core::Deps {
        paths: pitwall_core::paths::Paths::new(t.path().join("engine")),
        events: events.clone(),
        clock: ManualClock::new(1),
        store: MemStore::with(vec![web, api]),
        providers: vec![FakeProvider::named("local", "this-mac", Arc::new(pitwall_core::exec::LocalExec))],
    });
    let reg = pitwall_daemon::Registry::new();
    let identify = pitwall_daemon::ProcessIdentity::with_extensions(engine.clone(), reg.clone());
    let cfg = pitwall_daemon::Config { socket: t.path().join("unused.sock"), version: "test".into(), settings: None, workspace: None, viewer: None, events: Some(events), extensions: Some(reg), ext_backend: None };
    let server = pitwall_daemon::build(engine.clone(), pitwall_daemon::Approvals::new(Duration::from_secs(5)), identify, cfg);
    (engine, server)
}

/// Pitwall for a test: data folder `root` with the fixture installed and
/// on, granted `agents.read` and `agents.queue`; the agents' store and the
/// server from [`server`].
pub(crate) fn init(root: &Path, engine: &pitwall_core::engine::Shared, server: Arc<pitwall_daemon::Server>, cx: &mut gpui::App) -> Entity<crate::agents::AgentStore> {
    let dst = root.join("extensions/questions");
    std::fs::create_dir_all(&dst).unwrap();
    for f in ["extension.json", "tile.wasm"] {
        std::fs::copy(fixture().join(f), dst.join(f)).unwrap();
    }
    cx.set_global(crate::theme::Theme::dark());
    crate::kit::init(cx);
    crate::ui_state::init(None, cx);
    crate::extensions::init(Some(root.to_path_buf()), cx);
    catalog::set_bundle(None, cx);
    cx.set_global(super::DataDir(root.to_path_buf()));
    crate::extensions::calls::set_api(server, cx);
    let store = cx.new(|_| crate::agents::AgentStore::new(engine.views()));
    cx.set_global(crate::agents::StoreHandle(store.clone()));
    crate::ui_state::update(cx, |u| {
        *u = std::mem::take(u).with_extension("questions", |e| {
            e.enabled = true;
            e.grants.insert("agents.read".into(), Decision { granted: true, at: 1 });
            e.grants.insert("agents.queue".into(), Decision { granted: true, at: 1 });
        })
    });
    crate::extensions::calls::sync(cx);
    store
}

fn until(cx: &mut VisualTestContext, what: &str, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let end = Instant::now() + WAIT;
    loop {
        // Badges wait their turn on the test clock (four sets a second).
        cx.executor().advance_clock(Duration::from_millis(50));
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < end, "no {what} within {WAIT:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn has_text(tile: &Entity<TileView>, text: &str, cx: &mut VisualTestContext) -> bool {
    tile.read_with(cx, |t, _| t.texts()).iter().any(|t| t.contains(text))
}

fn agent(engine: &pitwall_core::engine::Shared, id: &str) -> AgentView {
    engine.views().into_iter().find(|a| a.id == id).expect("the agent")
}

#[gpui::test]
fn a_tile_reads_agents_badges_asks_and_queues(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-group");
    let root = t.path().join("data");
    let (engine, server) = server(&t);
    cx.executor().allow_parking();
    let store = cx.update(|cx| init(&root, &engine, server, cx));
    let (tile, cx) = cx.add_window_view(|_, cx| TileView::new("questions", "main", cx));
    cx.simulate_resize(gpui::size(gpui::px(500.), gpui::px(400.)));
    // The agents, read through the method table (in its Activity, as a tile's).
    until(cx, "the agents", |cx| has_text(&tile, "web", cx) && has_text(&tile, "api", cx));
    let log = cx.update(|_, cx| crate::extensions::calls::entries("questions", cx));
    assert!(log.iter().any(|e| e.method == "agent.list" && e.via == pitwall_core::extensions::activity::Via::Tile), "{log:?}");
    // Its badge on the first agent's row (none waits on the user).
    let web = agent(&engine, "a1");
    until(cx, "the badge", |cx| cx.update(|_, cx| !crate::extensions::marks_for_agent(&web, cx).is_empty()));
    let marks = cx.update(|_, cx| crate::extensions::marks_for_agent(&web, cx));
    assert_eq!((marks[0].ext.as_str(), marks[0].name.as_str(), marks[0].badge.count), ("questions", "Release desk", Some(1)));
    assert!(cx.update(|_, cx| crate::extensions::marks_for_agent(&agent(&engine, "a2"), cx)).is_empty());
    // Its question, as a card of the floating control, from that agent.
    let agents = engine.views();
    let asks = cx.update(|_, cx| group::float_asks(&agents, cx));
    assert_eq!(asks.len(), 1);
    let a = &asks[0];
    assert_eq!((a.from.as_deref(), a.who(), a.title.as_str(), a.ask.blocking, a.ask.suggestions.len()), (Some("web"), "web", "Release 0.9", true, 2));
    assert_eq!(a.tile, Some(("main".to_string(), "Release desk".to_string())));
    // Answered in its own words "later": refused, the card says why.
    let req = cx.update(|_, cx| group::answer("questions", &a.thread, vec![], Some("later".into()), false, cx)).expect("sent");
    until(cx, "the refusal", |cx| cx.update(|_, cx| group::ask_result("questions", req, cx)) == Some(false));
    let again = cx.update(|_, cx| group::float_asks(&agents, cx));
    assert_eq!((again.len(), again[0].note.as_deref(), again[0].busy), (1, Some("The release isn't ready yet"), false));
    // Picked "Deploy": taken, the card and the badge go.
    let req = cx.update(|_, cx| group::answer("questions", &a.thread, vec![0], None, false, cx)).expect("sent");
    until(cx, "the answer taken", |cx| cx.update(|_, cx| group::ask_result("questions", req, cx) == Some(true) && group::float_asks(&agents, cx).is_empty()));
    until(cx, "the answer drawn", |cx| has_text(&tile, "Answered: Deploy", cx));
    until(cx, "no badge", |cx| cx.update(|_, cx| crate::extensions::marks_for_agent(&web, cx).is_empty()));
    // Queue: in web's Next up, marked with its name, waiting for Send.
    let sent = tile.read_with(cx, |t, cx| t.click("agents/a-a1/queue", pitwall_tile_host::ui::MouseButton::Left, pitwall_tile_host::ui::Modifiers::empty(), 1, pitwall_tile_host::ui::Point { x: 1., y: 1. }, cx));
    assert!(sent);
    until(cx, "the queued item", |_| !agent(&engine, "a1").queue.is_empty());
    let q = agent(&engine, "a1").queue;
    assert!(q[0].text.starts_with("Rebase on main"), "{q:?}");
    assert_eq!(q[0].origin.as_ref().map(|o| (o.extension.as_str(), o.name.as_str())), Some(("questions", "Release desk")));
    until(cx, "Queued for web", |cx| has_text(&tile, "Queued for web", cx));
    // The store follows the engine: the tile hears agents-changed.
    cx.update(|_, cx| store.update(cx, |s, cx| {
        s.agents = engine.views();
        cx.notify();
    }));
    until(cx, "1 queued", |cx| has_text(&tile, "1 queued", cx));
    // Turned off: its badges and questions go with it.
    let sent = tile.read_with(cx, |t, cx| t.click("head/ask", pitwall_tile_host::ui::MouseButton::Left, pitwall_tile_host::ui::Modifiers::empty(), 1, pitwall_tile_host::ui::Point { x: 1., y: 1. }, cx));
    assert!(sent);
    until(cx, "the question again", |cx| cx.update(|_, cx| group::float_asks(&agents, cx).len() == 1));
    cx.update(|_, cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("questions", false)));
    cx.run_until_parked();
    assert!(cx.update(|_, cx| group::float_asks(&agents, cx)).is_empty());
}
