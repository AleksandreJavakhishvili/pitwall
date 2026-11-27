//! Extensions' tiles in the app, headless (GPUI's test platform), with the
//! tile host's hello fixture (crates/pitwall-tile-host/tests/fixtures/
//! hello): drawn in a tile of the layout, a click and typed text going
//! both ways, installed from a folder (and a bad folder refused), its
//! permission screen showing the memory it asks for, and a development
//! reload replacing the running component. Each test has its own data
//! folder; the component compiles in it on first start.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;

use super::{State, TileView, Tiles};
use crate::extensions::catalog;
use crate::settings::view::SettingsView;

/// How long the component may take to compile and answer.
const WAIT: Duration = Duration::from_secs(60);

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/hello")
}

/// A copy of the fixture at `to`.
fn copy_fixture(to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(fixture()).unwrap() {
        let e = e.unwrap();
        std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
    }
}

/// Pitwall's state for a test: data folder `root` (its extensions in
/// `root/extensions`), nothing on, Settings at Extensions.
fn init(root: &Path, cx: &mut TestAppContext) {
    let root = root.to_path_buf();
    // The component runs on its own thread: real time passes.
    cx.executor().allow_parking();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        crate::extensions::init(Some(root.clone()), cx);
        catalog::set_bundle(None, cx);
        cx.set_global(super::DataDir(root));
        crate::settings::data::set_page(cx, Page::Extensions);
    });
}

fn frame(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// Run until `done` holds (the component works on its own thread), or
/// panic saying `what`.
fn until(cx: &mut VisualTestContext, what: &str, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let end = Instant::now() + WAIT;
    loop {
        frame(cx);
        if done(cx) {
            return;
        }
        assert!(Instant::now() < end, "no {what} within {WAIT:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn texts(tile: &Entity<TileView>, cx: &mut VisualTestContext) -> Vec<String> {
    tile.read_with(cx, |t, _| t.texts())
}

fn has_text(tile: &Entity<TileView>, text: &str, cx: &mut VisualTestContext) -> bool {
    texts(tile, cx).iter().any(|t| t.contains(text))
}

fn tiles(cx: &mut VisualTestContext) -> Entity<Tiles> {
    cx.update(|_, cx| super::tiles(cx))
}

/// The hello tile in a window: "Starting…" while it compiles, then its
/// frame drawn with the kit (its button, its field); a click and typed
/// text reach the component and come back drawn.
#[gpui::test]
fn the_hello_tile_draws_and_answers(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-view");
    let root = t.path().join("data");
    copy_fixture(&root.join("extensions/hello"));
    init(&root, cx);
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true)));
    let (tile, cx) = cx.add_window_view(|_, cx| TileView::new("hello", "main", cx));
    cx.simulate_resize(gpui::size(gpui::px(600.), gpui::px(400.)));
    frame(cx);
    assert!(cx.debug_bounds("tile-state").is_some(), "Starting… while it compiles");
    until(cx, "first frame", |cx| has_text(&tile, "Clicked 0 times", cx));
    assert!(has_text(&tile, "Hello from a tile (main)", cx), "{:?}", texts(&tile, cx));
    assert!(cx.debug_bounds("tile-state").is_none());
    for node in ["tile-node-title", "tile-node-button", "tile-node-field"] {
        assert!(cx.debug_bounds(node).is_some(), "{node} is drawn");
    }
    let r = tiles(cx);
    assert_eq!(r.read_with(cx, |r, _| r.get("hello").map(|r| r.state.clone())), Some(State::Running));
    // A click on its button: the component counts and draws again.
    let sent = tile.read_with(cx, |t, cx| t.click("button", pitwall_tile_host::ui::MouseButton::Left, pitwall_tile_host::ui::Modifiers::empty(), 1, pitwall_tile_host::ui::Point { x: 3., y: 3. }, cx));
    assert!(sent, "the button has a click handler");
    until(cx, "the click drawn", |cx| has_text(&tile, "Clicked 1 times", cx));
    // Typed text: Pitwall's field, the component told, its echo drawn.
    let field = tile.read_with(cx, |t, _| t.field_of("field")).expect("a text field");
    field.update(cx, |i, cx| i.replace_text("hi there", cx));
    until(cx, "the typed text drawn", |cx| has_text(&tile, "Typed: hi there", cx));
    assert_eq!(field.read_with(cx, |i, _| i.text().to_string()), "hi there", "the field keeps what was typed");
    // Its Log in Settings has what it logged.
    let lines = cx.update(|_, cx| crate::settings::extensions_dev::log_lines("hello", cx));
    assert!(lines.iter().any(|(src, l)| src == "tile" && l.text.contains("starts with")), "{lines:?}");
    // Turned off: it stops at once and says so.
    cx.update(|_, cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", false)));
    frame(cx);
    assert!(r.read_with(cx, |r, _| r.get("hello").is_none()), "stopped when turned off");
    assert!(cx.debug_bounds("tile-settings").is_some(), "“Turn it on in Settings”");
}

/// "Open Hello": a tile in the layout like any other, the extension's
/// component drawing it, named once (its title is its extension's name).
#[gpui::test]
fn a_tile_opens_in_the_layout(cx: &mut TestAppContext) {
    use crate::agents::AgentStore;
    use crate::main_screen::MainScreen;
    let t = TempDir::new("tile-layout");
    let root = t.path().join("data");
    copy_fixture(&root.join("extensions/hello"));
    init(&root, cx);
    cx.update(|cx| {
        crate::main_screen::init(cx);
        crate::extensions::follow(cx);
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true));
    });
    let store = cx.new(|_| AgentStore::new(vec![crate::agents::tests::agent("web", "/work/alpha", "idle", 1, true)]));
    let (view, cx) = cx.add_window_view(move |window, cx| MainScreen::new(store.clone(), window, cx));
    cx.simulate_resize(gpui::size(gpui::px(1400.), gpui::px(900.)));
    view.update_in(cx, |s, window, cx| s.show_agent("web", window, cx));
    frame(cx);
    view.update_in(cx, |s, window, cx| s.open_view("hello", "main", None, window, cx));
    frame(cx);
    let tile = cx.update(|_, cx| crate::main_screen::view_tile::get("hello/main", cx)).and_then(|v| v.read_with(cx, |v, _| v.tile().cloned())).expect("the view tile draws the tile");
    until(cx, "its frame in the layout", |cx| has_text(&tile, "Clicked 0 times", cx));
    assert!(cx.debug_bounds("view-bar").is_some(), "the tile's own bar, as every view's");
    assert_eq!(cx.update(|_, cx| crate::main_screen::inbox_window::title("hello/main", cx)), "Hello", "not “Hello — Hello”");
    assert!(cx.debug_bounds("tile-node-button").is_some());
    assert!(tile.read_with(cx, |t, _| t.shown()), "shown: it runs");
    cx.update(|_, cx| crate::main_screen::view_tile::close("hello/main", cx));
    drop(tile);
    let r = tiles(cx);
    until(cx, "the stop", |cx| r.read_with(cx, |r, _| r.get("hello").is_none()));
}

/// Settings → Extensions → Install from folder…: a bad folder is refused
/// saying why; the hello folder lists its permissions with the memory it
/// asks for and the preview warning, installs as a preview (copied, off),
/// and turning it on shows the memory without a switch.
#[gpui::test]
fn a_folder_installs_as_a_preview(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-install");
    let root = t.path().join("data");
    std::fs::create_dir_all(root.join("extensions")).unwrap();
    init(&root, cx);
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(true, cx));
    cx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(2400.)));
    frame(cx);
    // A folder whose manifest is wrong.
    let bad = t.path().join("bad");
    std::fs::create_dir_all(&bad).unwrap();
    std::fs::write(bad.join("extension.json"), r#"{"v": 2, "id": "Bad Id", "name": "Bad"}"#).unwrap();
    view.update(cx, |v, cx| v.look_at_folder(bad.clone(), cx));
    until(cx, "the refusal", |cx| view.read_with(cx, |v, _| v.add_folder_state().is_some_and(|(busy, _, _)| !busy)));
    let (_, note, prepared) = view.read_with(cx, |v, _| v.add_folder_state()).unwrap();
    assert!(prepared.is_none() && note.is_some(), "refused, saying why: {note:?}");
    assert!(cx.debug_bounds("extension-add-folder-note").is_some());
    assert!(!root.join("extensions/Bad Id").exists() && !root.join("extensions/bad").exists(), "nothing installed");
    // The hello folder.
    let src = t.path().join("hello-src");
    copy_fixture(&src);
    view.update(cx, |v, cx| v.look_at_folder(src.clone(), cx));
    until(cx, "the review", |cx| view.read_with(cx, |v, _| v.add_folder_state().is_some_and(|(busy, _, _)| !busy)));
    let (_, note, prepared) = view.read_with(cx, |v, _| v.add_folder_state()).unwrap();
    assert_eq!((prepared.as_deref(), note), (Some("hello"), None));
    assert!(cx.debug_bounds("extension-preview-warning-hello").is_some(), "Unsigned, preview");
    assert!(cx.debug_bounds("extension-add-folder-amount-memory").is_some(), "the memory it asks for");
    view.update(cx, |v, cx| v.install_from_folder(cx));
    until(cx, "the install", |cx| view.read_with(cx, |v, _| v.add_folder_state().is_none()));
    let installed = root.join("extensions/hello");
    assert!(installed.join("tile.wasm").is_file(), "copied into the extensions folder");
    assert!(src.join("tile.wasm").is_file(), "the folder is left as it was");
    let origin = pitwall_core::extensions::install::origin(&installed);
    assert!(origin.as_ref().is_some_and(|o| o.is_preview()), "marked a preview: {origin:?}");
    assert!(cx.update(|_, cx| !crate::ui_state::get(cx).extension_on("hello")), "it stays off");
    // Turning it on: the permission screen shows the memory, no switch.
    view.update(cx, |v, cx| v.debug_permissions("hello", cx));
    frame(cx);
    assert!(cx.debug_bounds("extension-permissions-ask-hello").is_some());
    assert!(cx.debug_bounds("extension-permission-amount-hello-memory").is_some(), "memory shown with its size");
    assert!(cx.debug_bounds("extension-preview-warning-hello").is_some(), "the preview warning there too");
    view.update(cx, |v, cx| v.debug_permissions_on(cx));
    frame(cx);
    let granted = cx.update(|_, cx| {
        let m = crate::extensions::manifest_of("hello", cx).unwrap();
        super::grants_of(&m, cx).has("memory")
    });
    assert!(cx.update(|_, cx| crate::ui_state::get(cx).extension_on("hello")) && granted, "on, with the memory");
}

/// `pitwall extension dev` (`extension.devReload`): a folder loaded for
/// development and turned on; reloaded, its running component is
/// replaced in place and keeps drawing.
#[gpui::test]
fn a_dev_reload_replaces_the_running_tile(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-dev");
    let root = t.path().join("data");
    std::fs::create_dir_all(root.join("extensions")).unwrap();
    let src = t.path().join("hello-dev");
    copy_fixture(&src);
    init(&root, cx);
    let (id, loaded) = cx.update(|cx| crate::extensions::load_dev_or_reload(&src, cx)).unwrap();
    assert_eq!((id.as_str(), loaded), ("hello", true));
    assert!(cx.update(|cx| crate::ui_state::get(cx).extension_on("hello")), "turned on");
    let (tile, cx) = cx.add_window_view(|_, cx| TileView::new("hello", "main", cx));
    until(cx, "first frame", |cx| has_text(&tile, "Clicked 0 times", cx));
    tile.read_with(cx, |t, cx| t.click("button", pitwall_tile_host::ui::MouseButton::Left, pitwall_tile_host::ui::Modifiers::empty(), 1, pitwall_tile_host::ui::Point { x: 1., y: 1. }, cx));
    until(cx, "the click", |cx| has_text(&tile, "Clicked 1 times", cx));
    let r = tiles(cx);
    let before = r.read_with(cx, |r, _| r.generation("hello")).unwrap();
    let (_, loaded) = cx.update(|_, cx| crate::extensions::load_dev_or_reload(&src, cx)).unwrap();
    assert!(!loaded, "loaded before: reloaded");
    until(cx, "a new component", |cx| r.read_with(cx, |r, _| r.generation("hello").is_some_and(|g| g != before) && r.get("hello").is_some_and(|x| x.state == State::Running)));
    until(cx, "its frame again", |cx| has_text(&tile, "Clicked 1 times", cx));
    let log = r.read_with(cx, |r, _| r.log("hello"));
    assert!(log.iter().any(|l| l.from_host && l.message.contains("Reloaded")), "{log:?}");
}

/// `pitwall extension dev` end to end over a socket, as the CLI calls it
/// (`extension.devReload` with `{path}`, `message` in the answer): the
/// first load asks the user in Pitwall; a reload after it doesn't and
/// replaces the running tile; a folder without tiles is refused.
#[gpui::test]
fn the_cli_loads_and_reloads_a_dev_folder(cx: &mut TestAppContext) {
    use pitwall_core::testing::{ManualClock, MemStore, RecordingSink};
    use pitwall_proto::ApprovalAnswer;
    use serde_json::{json, Value};
    let t = TempDir::new("tile-cli-dev");
    let root = t.path().join("data");
    std::fs::create_dir_all(root.join("extensions")).unwrap();
    let src = t.path().join("my-tile");
    copy_fixture(&src);
    init(&root, cx);
    let (_, cx) = cx.add_window_view(|_, _| gpui::Empty);
    let engine = pitwall_core::Engine::open(pitwall_core::Deps { paths: pitwall_core::paths::Paths::new(t.path().join("engine")), events: RecordingSink::new(), clock: ManualClock::new(1), store: MemStore::with(vec![]), providers: vec![] });
    let approvals = pitwall_daemon::Approvals::new(Duration::from_secs(60));
    let backend = crate::extensions::agent_setup::AppExtensions::new();
    // A short socket path (Unix sockets have a small limit).
    let sock_dir = std::env::temp_dir().join(format!("pwtile-{}", std::process::id()));
    std::fs::create_dir_all(&sock_dir).unwrap();
    let socket = sock_dir.join("d.sock");
    let cfg = pitwall_daemon::Config { socket: socket.clone(), version: "test".into(), settings: None, workspace: None, viewer: None, events: None, extensions: None, ext_backend: Some(backend.clone()) };
    let identify = pitwall_daemon::ProcessIdentity::with_extensions(engine.clone(), pitwall_daemon::Registry::new());
    let server = pitwall_daemon::serve(engine, approvals.clone(), identify, cfg).unwrap();
    cx.update(|_, cx| crate::extensions::agent_setup::attach(backend, cx));
    // What `pitwall extension dev` does after a build (crates/pitwall-cli/src/tile.rs).
    let reload = |folder: &Path| {
        let (socket, folder) = (socket.clone(), folder.to_path_buf());
        std::thread::spawn(move || -> Result<Value, pitwall_client::Error> {
            let mut c = pitwall_client::Client::connect(&socket)?;
            c.call("extension.devReload", json!({ "path": folder }))
        })
    };
    let finish = |cx: &mut VisualTestContext, h: std::thread::JoinHandle<Result<Value, pitwall_client::Error>>| {
        until(cx, "the answer", |_| h.is_finished());
        h.join().unwrap()
    };
    // The first load asks.
    let h = reload(&src);
    until(cx, "the approval", |_| !approvals.pending().is_empty());
    let p = approvals.pending()[0].clone();
    assert!(p.summary.contains("load the extension Hello for development"), "{}", p.summary);
    assert!(p.details.iter().any(|d| d.contains("128 MiB")), "{:?}", p.details);
    approvals.answer(&ApprovalAnswer { id: p.id.clone(), allow: true, remember: false }).unwrap();
    let first = finish(cx, h).unwrap();
    assert_eq!((first["id"].as_str(), first["loaded"].as_bool()), (Some("hello"), Some(true)));
    assert!(first["message"].as_str().unwrap().starts_with("Loaded Hello"), "{first}");
    assert!(cx.update(|_, cx| crate::ui_state::get(cx).extension_on("hello")));
    // Its tile open (not drawn: suspended) and running.
    let _tile = cx.update(|_, cx| cx.new(|cx| TileView::new("hello", "main", cx)));
    let r = tiles(cx);
    until(cx, "it runs", |cx| r.read_with(cx, |r, _| r.get("hello").is_some_and(|x| x.state == State::Running)));
    let before = r.read_with(cx, |r, _| r.generation("hello")).unwrap();
    // A reload asks nothing and replaces it.
    let again = finish(cx, reload(&src)).unwrap();
    assert_eq!(again["loaded"].as_bool(), Some(false), "{again}");
    assert_eq!(again["reloaded"], json!(["main"]));
    assert!(again["message"].as_str().unwrap().starts_with("Reloaded Hello"), "{again}");
    assert!(approvals.pending().is_empty(), "nothing asked");
    until(cx, "a new component", |cx| r.read_with(cx, |r, _| r.generation("hello").is_some_and(|g| g != before)));
    // Not a folder of tiles: refused, saying why.
    let empty = t.path().join("not-tiles");
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::write(empty.join("extension.json"), r#"{"v": 2, "id": "plain", "name": "Plain", "version": "1.0.0"}"#).unwrap();
    match finish(cx, reload(&empty)) {
        Err(pitwall_client::Error::Server(e)) => assert_eq!(e.code, pitwall_proto::code::BAD_PARAMS, "{}", e.message),
        other => panic!("not refused: {other:?}"),
    }
    server.stop();
    let _ = std::fs::remove_dir_all(&sock_dir);
}

/// A tile whose title is its extension's name is named once.
#[test]
fn a_title_equal_to_the_name_is_said_once() {
    assert!(super::same_name("Hello", "hello "));
    assert!(!super::same_name("Runs", "CI"));
    assert_eq!(super::open_label("Hello", "Hello"), "Open Hello");
    assert_eq!(super::open_label("Runs", "CI"), "Open Runs (CI)");
}

/// Why a tile stopped, as the tile says it: a sentence, a crash by its
/// trap without the backtrace.
#[test]
fn stop_reasons_in_plain_words() {
    use pitwall_tile_host::StopReason;
    let crash = StopReason::Crashed("error while executing at wasm backtrace:\n    0: 0xca63 - <unknown>!<wasm function 184>\n: wasm trap: wasm `unreachable` instruction executed".into());
    assert_eq!(super::stop_words(&crash), "It crashed: wasm trap: wasm `unreachable` instruction executed.");
    let late = StopReason::Deadline { call: "handle", limit: Duration::from_millis(1000) };
    assert_eq!(super::stop_words(&late), "It didn't answer in time (handle took over 1000 ms).");
}

/// Screenshots for review, rendered off screen (no window is shown):
/// `PITWALL_TILE_SHOTS=<folder> cargo test -p pitwall-app --lib
/// tiles::tests::screenshots -- --ignored`. The hello tile in its view
/// tile (dark and light), starting and stopped, and Settings' install
/// review and permission screen.
#[cfg(target_os = "macos")]
#[test]
#[ignore]
fn screenshots() {
    use gpui::{px, size, AnyWindowHandle, HeadlessAppContext};
    use std::sync::Arc;
    let Ok(out) = std::env::var("PITWALL_TILE_SHOTS") else { return };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let t = TempDir::new("tile-shots");
    let root = t.path().join("data");
    std::fs::create_dir_all(root.join("extensions")).unwrap();
    let src = t.path().join("hello");
    copy_fixture(&src);
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
    let settle = |cx: &mut HeadlessAppContext, done: &mut dyn FnMut(&mut gpui::App) -> bool| {
        let end = Instant::now() + WAIT;
        loop {
            cx.run_until_parked();
            if cx.update(|cx| done(cx)) {
                return;
            }
            assert!(Instant::now() < end, "timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let shot = |cx: &mut HeadlessAppContext, w: AnyWindowHandle, name: &str, light: bool| {
        cx.update(|cx| cx.set_global(if light { crate::theme::Theme::light() } else { crate::theme::Theme::dark() }));
        cx.run_until_parked();
        let _ = cx.update_window(w, |_, window, _| window.refresh());
        cx.run_until_parked();
        let img = cx.capture_screenshot(w).expect("rendered");
        img.save(out.join(name)).expect("saved");
    };
    // Settings: the folder reviewed, then the permission screen.
    let sv = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sv2 = sv.clone();
    let shell = cx
        .open_window(size(px(900.), px(730.)), move |_, cx| {
            let v = cx.new(|cx| SettingsView::new(true, cx));
            *sv2.borrow_mut() = Some(v.clone());
            cx.new(|_| Shell(v.into(), false))
        })
        .unwrap();
    let settings: Entity<SettingsView> = sv.borrow().clone().unwrap();
    let s2 = src.clone();
    cx.update(|cx| settings.update(cx, |v, cx| v.look_at_folder(s2, cx)));
    settle(&mut cx, &mut |cx| settings.read(cx).add_folder_state().is_some_and(|(busy, _, _)| !busy));
    cx.update(|cx| settings.read(cx).scroll_to_end());
    shot(&mut cx, shell.into(), "install-from-folder.png", false);
    cx.update(|cx| settings.update(cx, |v, cx| v.install_from_folder(cx)));
    settle(&mut cx, &mut |cx| settings.read(cx).add_folder_state().is_none());
    cx.update(|cx| settings.update(cx, |v, cx| v.debug_permissions("hello", cx)));
    shot(&mut cx, shell.into(), "permission-screen.png", false);
    cx.update(|cx| settings.update(cx, |v, cx| v.debug_permissions_on(cx)));
    cx.run_until_parked();
    // The tile in its view tile.
    let vt = std::rc::Rc::new(std::cell::RefCell::new(None));
    let vt2 = vt.clone();
    let view = cx
        .open_window(size(px(640.), px(440.)), move |_, cx| {
            let v = crate::main_screen::view_tile::view("hello/main", cx);
            *vt2.borrow_mut() = Some(v.clone());
            cx.new(|_| Shell(v.into(), true))
        })
        .unwrap();
    let vtile: Entity<crate::main_screen::view_tile::ViewTile> = vt.borrow().clone().unwrap();
    shot(&mut cx, view.into(), "tile-starting.png", false);
    let tile = cx.update(|cx| vtile.read(cx).tile().cloned()).expect("a tile");
    settle(&mut cx, &mut |cx| tile.read(cx).texts().iter().any(|t| t.contains("Clicked 0")));
    cx.update(|cx| tile.read(cx).click("button", pitwall_tile_host::ui::MouseButton::Left, pitwall_tile_host::ui::Modifiers::empty(), 1, pitwall_tile_host::ui::Point { x: 1., y: 1. }, cx));
    settle(&mut cx, &mut |cx| tile.read(cx).texts().iter().any(|t| t.contains("Clicked 1")));
    let field = cx.update(|cx| tile.read(cx).field_of("field")).unwrap();
    cx.update(|cx| field.update(cx, |i, cx| i.replace_text("Native looking", cx)));
    settle(&mut cx, &mut |cx| tile.read(cx).texts().iter().any(|t| t.contains("Typed: Native")));
    shot(&mut cx, view.into(), "tile-dark.png", false);
    shot(&mut cx, view.into(), "tile-light.png", true);
    // It panics (the fixture's handler 7): stopped, with why and Restart.
    cx.update(|cx| {
        let r = super::tiles(cx);
        r.read(cx).send("hello", pitwall_tile_host::ui::Event::Click(pitwall_tile_host::ui::Click { handler: 7, button: pitwall_tile_host::ui::MouseButton::Left, modifiers: pitwall_tile_host::ui::Modifiers::empty(), count: 1, at: pitwall_tile_host::ui::Point { x: 0., y: 0. } }));
    });
    settle(&mut cx, &mut |cx| super::tiles(cx).read(cx).get("hello").is_some_and(|r| matches!(r.state, State::Stopped(_))));
    shot(&mut cx, view.into(), "tile-stopped.png", false);
}

/// A window's backdrop for screenshots: the page's background, and (a
/// tile) the pane frame the layout draws around a tile.
#[cfg(target_os = "macos")]
struct Shell(gpui::AnyView, bool);

#[cfg(target_os = "macos")]
impl gpui::Render for Shell {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{div, prelude::*, px};
        let t = crate::theme::theme(cx).clone();
        let body = div().size_full().flex().flex_col().child(self.0.clone());
        let body = if self.1 { body.rounded(px(8.)).border_1().border_color(t.line).bg(t.bg).overflow_hidden() } else { body };
        div().size_full().flex().flex_col().bg(if self.1 { t.surface } else { t.bg }).when(self.1, |d| d.p(px(10.))).child(body)
    }
}
