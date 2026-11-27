//! The tile host end to end, with the committed hello tile
//! (tests/fixtures/hello/, rebuilt by tests/fixtures/build-hello-tile.sh):
//! load, render, events, storage, settings, views with content, limits and
//! stops. Each test has its own data folder in a temporary directory. The
//! tile is written with the guest SDK, which numbers handlers itself: the
//! tests click a node by its key, with the handler of the newest frame.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use pitwall_tile_host::ui::{self, Element, Event, Modifiers, MouseButton, Point};
use pitwall_tile_host::{
    Clock, Extension, FrameUpdate, Grants, HostConfig, LoadError, LogLevel, LogLine, Secrets, StartOptions, StopReason, TileHandle, TileHost, TileOutput, UiRequest, ViewSpec,
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello")
}

struct Run {
    handle: TileHandle,
    rx: Receiver<TileOutput>,
    /// The newest frame of each view seen so far.
    frames: RefCell<HashMap<String, FrameUpdate>>,
}

fn start(host: &TileHost, options: StartOptions) -> Run {
    let ext = host.load(&fixture()).expect("the fixture loads");
    let (tx, rx) = mpsc::channel();
    let handle = host
        .start(&ext, options, move |o| {
            let _ = tx.send(o);
        })
        .expect("starts");
    Run { handle, rx, frames: RefCell::default() }
}

fn host(data: &Path) -> TileHost {
    TileHost::new(HostConfig::new(data)).expect("the engine starts")
}

const WAIT: Duration = Duration::from_secs(20);

impl Run {
    /// The next output that `pick` accepts (others are skipped); panics
    /// after [`WAIT`] or when the tile stops first (unless `pick` takes
    /// the stop).
    fn next<T>(&self, what: &str, mut pick: impl FnMut(&TileOutput) -> Option<T>) -> T {
        let until = Instant::now() + WAIT;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(o) => {
                    if let TileOutput::Frame(f) = &o {
                        self.frames.borrow_mut().insert(f.view.clone(), f.clone());
                    }
                    if let Some(t) = pick(&o) {
                        return t;
                    }
                    if let TileOutput::Stopped(r) = o {
                        panic!("stopped while waiting for {what}: {r}");
                    }
                }
                Err(_) => panic!("no {what} within {WAIT:?}"),
            }
        }
    }

    /// The next frame of `view` that has a text node containing `text`.
    fn frame_with(&self, view: &str, text: &str) -> FrameUpdate {
        self.next(&format!("a frame of {view} with “{text}”"), |o| match o {
            TileOutput::Frame(f) if f.view == view && texts(f).iter().any(|t| t.contains(text)) => Some(f.clone()),
            _ => None,
        })
    }

    fn log_line(&self, text: &str) -> LogLine {
        self.next(&format!("a log line “{text}”"), |o| match o {
            TileOutput::Log(l) if l.message.contains(text) => Some(l.clone()),
            _ => None,
        })
    }

    fn stopped(&self) -> StopReason {
        self.next("a stop", |o| match o {
            TileOutput::Stopped(r) => Some(r.clone()),
            _ => None,
        })
    }

    /// The handler of the `on` listener of the node keyed `key` in
    /// `view`'s newest frame.
    fn handler(&self, view: &str, key: &str, on: ui::Listen) -> u32 {
        let frames = self.frames.borrow();
        let f = frames.get(view).unwrap_or_else(|| panic!("no frame of {view} yet"));
        let node = f.frame.nodes.iter().find(|n| n.key == key).unwrap_or_else(|| panic!("no node “{key}” in {view}"));
        node.on.iter().find(|l| l.on == on).map(|l| l.handler).unwrap_or_else(|| panic!("“{key}” has no {on:?} listener"))
    }

    /// Click the node keyed `key` in the main view.
    fn click(&self, key: &str) {
        let handler = self.handler("main", key, ui::Listen::Click);
        assert!(self.handle.send(Event::Click(ui::Click { handler, button: MouseButton::Left, modifiers: Modifiers::empty(), count: 1, at: Point { x: 4.0, y: 4.0 } })));
    }
}

fn texts(f: &FrameUpdate) -> Vec<String> {
    f.frame
        .nodes
        .iter()
        .filter_map(|n| match &n.element {
            Element::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_tile_loads_renders_and_handles_a_click() {
    let t = tempfile::tempdir().unwrap();
    let host = host(t.path());
    let ext = host.load(&fixture()).unwrap();
    assert_eq!((ext.id(), ext.tiles().len()), ("hello", 3));
    assert!(!host.is_compiled(&ext));
    let run = start(&host, StartOptions::view("main"));
    run.next("Preparing", |o| matches!(o, TileOutput::Preparing).then_some(()));
    let cached = run.next("Started", |o| match o {
        TileOutput::Started { cached, .. } => Some(*cached),
        _ => None,
    });
    assert!(!cached, "the first start compiles");
    run.log_line("hello hello 0.1.0 starts with 0 clicks");
    let first = run.frame_with("main", "Clicked 0 times");
    assert_eq!(first.tile, "main");
    assert!(texts(&first).iter().any(|t| t == "Hello from a tile (main)"), "the setting's default: {:?}", texts(&first));
    assert_eq!((first.diff.added, first.diff.removed), (first.frame.nodes.len(), 0));
    run.click("button");
    let second = run.frame_with("main", "Clicked 1 times");
    assert_eq!(second.diff.continues.iter().filter(|c| c.is_none()).count(), 0, "same key paths");
    assert_eq!(second.diff.changed.len(), 1, "only the count changed");
    run.handle.stop();
    assert_eq!(run.stopped(), StopReason::Stopped);
    // Compiled once, kept in Pitwall's own cache under the data folder.
    assert!(host.is_compiled(&ext));
    let cache = t.path().join("cache/tiles/hello");
    assert_eq!(std::fs::read_dir(cache).unwrap().count(), 1);
}

#[test]
fn storage_persists_across_starts() {
    let t = tempfile::tempdir().unwrap();
    let host = host(t.path());
    let run = start(&host, StartOptions::view("main"));
    run.frame_with("main", "Clicked 0 times");
    run.click("button");
    run.click("button");
    run.frame_with("main", "Clicked 2 times");
    run.handle.stop();
    run.stopped();
    run.handle.join();
    assert!(t.path().join("extension-data/hello/kv.bin").is_file());
    let again = start(&host, StartOptions::view("main"));
    let cached = again.next("Started", |o| match o {
        TileOutput::Started { cached, .. } => Some(*cached),
        _ => None,
    });
    assert!(cached, "the second start loads the compiled file");
    again.log_line("starts with 2 clicks");
    again.frame_with("main", "Clicked 2 times");
}

struct OneSecret;

impl Secrets for OneSecret {
    fn secret(&self, extension: &str, key: &str) -> Option<String> {
        (extension == "hello" && key == "token").then(|| "s3cret".to_string())
    }
}

#[test]
fn settings_are_read_and_changes_reach_the_tile() {
    let t = tempfile::tempdir().unwrap();
    let host = TileHost::new(HostConfig { secrets: std::sync::Arc::new(OneSecret), ..HostConfig::new(t.path()) }).unwrap();
    let data = host.extension_data("hello");
    data.set_setting("greeting", Some("Howdy")).unwrap();
    let run = start(&host, StartOptions::view("main"));
    run.frame_with("main", "Howdy from a tile");
    data.set_setting("greeting", Some("Hi")).unwrap();
    assert!(run.handle.settings_changed());
    run.frame_with("main", "Hi from a tile");
}

#[test]
fn text_input_timers_logs_and_toasts() {
    let t = tempfile::tempdir().unwrap();
    let run = start(&host(t.path()), StartOptions::view("main"));
    run.frame_with("main", "Typed: ");
    let handler = run.handler("main", "field", ui::Listen::Change);
    let change = |value: &str, revision| Event::InputChanged(ui::InputChange { handler, value: value.into(), revision, selection: ui::TextRange { start: 0, end: 0 } });
    // A burst of changes to one field is handled as its newest.
    run.handle.send(change("a", 1));
    run.handle.send(change("ab", 2));
    run.handle.send(change("abc", 3));
    run.frame_with("main", "Typed: abc");
    run.click("print");
    assert_eq!(run.log_line("hello from stdout").level, LogLevel::Info);
    assert_eq!(run.log_line("hello from stderr").level, LogLevel::Warn);
    run.click("timer");
    run.frame_with("main", "Ticks: 1");
    run.click("toast");
    let toast = run.next("a toast", |o| match o {
        TileOutput::Ui(r @ UiRequest::Toast { .. }) => Some(r.clone()),
        _ => None,
    });
    assert_eq!(toast, UiRequest::Toast { text: "Saved".into(), tone: ui::Tone::Good });
}

/// On a virtual clock a timer fires only when the test moves the clock,
/// at its time; a reserved key binding is dropped with a Log line; two
/// stored keys come back from one get-many.
#[test]
fn a_virtual_clock_reserved_keys_and_get_many() {
    let t = tempfile::tempdir().unwrap();
    let run = start(&host(t.path()), StartOptions { clock: Clock::manual(), ..StartOptions::view("main") });
    let first = run.frame_with("main", "Ticks: 0");
    let keys: Vec<&str> = first.frame.keymap.iter().map(|b| b.keys.as_str()).collect();
    assert_eq!(keys, ["j"], "cmd-w is Pitwall's");
    assert_eq!(run.log_line("“cmd-w” was dropped").level, LogLevel::Warn);
    run.click("timer");
    run.handle.advance(Duration::from_millis(60));
    run.click("button");
    let f = run.frame_with("main", "Clicked 1 times");
    assert!(texts(&f).iter().any(|t| t == "Ticks: 0"), "not due yet: {:?}", texts(&f));
    run.handle.advance(Duration::from_millis(60));
    run.frame_with("main", "Ticks: 1");
    // The tile's own key.
    let j = first.frame.keymap[0].handler;
    run.handle.send(Event::Key(ui::KeyEvent { handler: j, keys: "j".into() }));
    run.frame_with("main", "Clicked 2 times");
    run.click("many");
    run.log_line("get-many: Ok([true, false])");
}

#[test]
fn views_open_with_content_and_take_updates() {
    let t = tempfile::tempdir().unwrap();
    let run = start(&host(t.path()), StartOptions::view("main"));
    run.frame_with("main", "Clicked 0 times");
    let content = |json: &str| ui::Content { id: Some("note-1".into()), json: Some(json.into()), bytes: None, call: None, tool: None, caller: None, append: false, large: None };
    assert!(run.handle.open_view(ViewSpec { id: "agent-1".into(), tile: "second".into(), content: Some(content(r#"{"message":"first"}"#)), project: None }));
    let f = run.frame_with("agent-1", r#"Content: {"message":"first"}"#);
    assert_eq!(f.tile, "second");
    assert!(run.handle.set_content("agent-1", content(r#"{"message":"again"}"#)));
    run.frame_with("agent-1", r#"Content: {"message":"again"}"#);
    // The same view id again with content is an update, not a new view.
    run.handle.open_view(ViewSpec { id: "agent-1".into(), tile: "second".into(), content: Some(content(r#"{"message":"third"}"#)), project: None });
    run.frame_with("agent-1", r#"Content: {"message":"third"}"#);
    run.handle.close_view("agent-1");
    run.click("button");
    let f = run.frame_with("main", "Clicked 1 times");
    assert_eq!(f.view, "main");
}

#[test]
fn a_hidden_tile_renders_once_shown_and_saves_its_state() {
    let t = tempfile::tempdir().unwrap();
    let run = start(&host(t.path()), StartOptions { visible: false, grants: Grants::new(["memory"]), ..StartOptions::view("main") });
    run.next("Started", |o| matches!(o, TileOutput::Started { .. }).then_some(()));
    let saved = |run: &Run| {
        assert!(run.handle.save_state());
        run.next("saved state", |o| match o {
            TileOutput::StateSaved(b) => Some(b.clone()),
            TileOutput::Frame(_) => panic!("a hidden tile rendered"),
            _ => None,
        })
    };
    assert_eq!(saved(&run), 0u32.to_le_bytes());
    run.handle.set_visible(true);
    run.frame_with("main", "Clicked 0 times");
    // Hidden again: a click is handled, but nothing renders until shown.
    run.handle.set_visible(false);
    run.click("button");
    assert_eq!(saved(&run), 1u32.to_le_bytes());
    run.handle.set_visible(true);
    run.frame_with("main", "Clicked 1 times");
}

#[test]
fn a_call_past_its_deadline_stops_the_tile() {
    let t = tempfile::tempdir().unwrap();
    let run = start(&host(t.path()), StartOptions::view("main"));
    run.frame_with("main", "Clicked 0 times");
    let at = Instant::now();
    run.click("spin");
    let r = run.stopped();
    let took = at.elapsed();
    assert_eq!(r, StopReason::Deadline { call: "handle", limit: Duration::from_secs(1) });
    assert!(r.to_string().contains("didn't answer in time"));
    // Wall-clock deadlines: about one tick late, never ticks × 10 ms late.
    assert!(took >= Duration::from_secs(1) && took < Duration::from_millis(1600), "{took:?}");
}

#[test]
fn stop_ends_a_running_call_at_once() {
    let t = tempfile::tempdir().unwrap();
    let run = start(&host(t.path()), StartOptions::view("main"));
    run.frame_with("main", "Clicked 0 times");
    run.click("spin");
    let at = Instant::now();
    run.handle.stop();
    assert_eq!(run.stopped(), StopReason::Stopped);
    assert!(at.elapsed() < Duration::from_millis(500), "{:?}", at.elapsed());
}

#[test]
fn a_frame_over_the_tree_limit_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let run = start(&host(t.path()), StartOptions::view("main"));
    run.frame_with("main", "Clicked 0 times");
    run.click("huge");
    match run.stopped() {
        StopReason::FrameRefused(why) => assert!(why.contains("20001 nodes"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_panic_or_runaway_memory_ends_only_the_tile() {
    let t = tempfile::tempdir().unwrap();
    let host = host(t.path());
    let run = start(&host, StartOptions::view("main"));
    run.frame_with("main", "Clicked 0 times");
    run.click("panic");
    assert!(matches!(run.stopped(), StopReason::Crashed(_)));
    let run = start(&host, StartOptions::view("main"));
    run.frame_with("main", "Clicked 0 times");
    run.click("grow");
    assert!(matches!(run.stopped(), StopReason::Crashed(_)), "a grow past 64 MiB traps");
}

/// A copy of the fixture with its manifest replaced (and the wasm kept,
/// unless `wasm` says otherwise).
fn variant(dir: &Path, manifest: &str, wasm: Option<&[u8]>) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("extension.json"), manifest).unwrap();
    let bytes = match wasm {
        Some(b) => b.to_vec(),
        None => std::fs::read(fixture().join("tile.wasm")).unwrap(),
    };
    std::fs::write(dir.join("tile.wasm"), bytes).unwrap();
    dir.to_path_buf()
}

#[test]
fn bad_extensions_are_refused_with_a_reason() {
    let t = tempfile::tempdir().unwrap();
    let m = |tiles: &str| format!(r#"{{"v": 2, "id": "bad", "name": "Bad", "contributes": {{"tiles": [{tiles}]}}}}"#);
    let load = |name: &str, manifest: &str| Extension::load(&variant(&t.path().join(name), manifest, None));
    assert!(matches!(Extension::load(&t.path().join("missing")), Err(LoadError::Manifest(_))));
    assert!(matches!(load("json", "{ not json"), Err(LoadError::Manifest(_))));
    assert_eq!(load("none", r#"{"v": 2, "id": "bad", "name": "Bad"}"#).unwrap_err(), LoadError::NoTiles);
    assert!(matches!(load("escape", &m(r#"{"id": "t", "title": "T", "wasm": "../tile.wasm"}"#)), Err(LoadError::Manifest(e)) if e.contains("tile “t”")));
    assert!(matches!(load("absent", &m(r#"{"id": "t", "title": "T", "wasm": "other.wasm"}"#)), Err(LoadError::Wasm(e)) if e.contains("isn't there")));
    assert!(matches!(load("world", &m(r#"{"id": "t", "title": "T", "wasm": "tile.wasm", "world": "pitwall:tile@9.0.0"}"#)), Err(LoadError::World { .. })));
    assert_eq!(load("two", &m(r#"{"id": "a", "title": "A", "wasm": "tile.wasm"}, {"id": "b", "title": "B", "wasm": "b.wasm"}"#)).unwrap_err(), LoadError::SeveralComponents);
    assert!(matches!(load("schema", &m(r#"{"id": "t", "title": "T", "wasm": "tile.wasm", "input": "missing.json"}"#)), Err(LoadError::Manifest(e)) if e.contains("input schema")));
    // Not a component: refused when compiled, never run.
    let host = host(&t.path().join("data"));
    let junk = Extension::load(&variant(&t.path().join("junk"), &m(r#"{"id": "t", "title": "T", "wasm": "tile.wasm"}"#), Some(b"\0asm junk"))).unwrap();
    assert!(matches!(host.compile(&junk), Err(LoadError::Compile(_))));
    let (tx, rx) = mpsc::channel();
    let _h = host.start(&junk, StartOptions::view("t"), move |o| drop(tx.send(o))).unwrap();
    let stop = rx.iter().find_map(|o| match o {
        TileOutput::Stopped(r) => Some(r),
        _ => None,
    });
    assert!(matches!(stop, Some(StopReason::LoadFailed(_))), "{stop:?}");
    // A view the manifest doesn't declare.
    let ok = Extension::load(&fixture()).unwrap();
    assert_eq!(host.start(&ok, StartOptions::view("nope"), |_| ()).err(), Some(LoadError::UnknownTile("nope".into())));
}

fn start_from(host: &TileHost, dir: &Path, options: StartOptions) -> Run {
    let ext = host.load(dir).expect("the fixture loads");
    let (tx, rx) = mpsc::channel();
    let handle = host.start(&ext, options, move |o| drop(tx.send(o))).expect("starts");
    Run { handle, rx, frames: RefCell::default() }
}

/// Holds every request; the test answers them.
#[derive(Default)]
struct Held(std::sync::Mutex<Vec<pitwall_tile_host::net::Done>>);

impl pitwall_tile_host::net::Fetcher for Held {
    fn fetch(&self, _request: pitwall_tile_host::net::Outgoing, _reach: pitwall_tile_host::net::Reach, done: pitwall_tile_host::net::Done) {
        self.0.lock().unwrap().push(done);
    }
}

/// A grant taken back while the tile runs (permissions.md §5.4): its
/// next call is denied, and a request on its way arrives denied, its
/// late answer dropped; granted again, it reaches the host again. No
/// restart.
#[test]
fn a_revoke_applies_to_the_next_call() {
    let t = tempfile::tempdir().unwrap();
    let host = host(t.path());
    let held = std::sync::Arc::new(Held::default());
    host.set_fetcher(held.clone());
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/services");
    let m = host.load(&dir).unwrap().manifest().clone();
    let net = || Grants::for_extension(&m, ["net"]);
    let run = start_from(&host, &dir, StartOptions { grants: net(), ..StartOptions::view("main") });
    run.frame_with("main", "schema 2");
    run.click("fetch");
    run.frame_with("main", "fetching");
    assert!(run.handle.set_grants(Grants::for_extension(&m, Vec::<String>::new())));
    run.frame_with("main", "fetch failed: not allowed to reach api.example.com");
    let late = held.0.lock().unwrap().pop().expect("held");
    late(Ok(pitwall_tile_host::net::Response { status: 200, headers: vec![], body: b"late".to_vec(), url: "https://api.example.com/items?x=1".into() }));
    run.click("fetch");
    run.next("the next fetch refused", |o| match o {
        TileOutput::Frame(f) => {
            assert!(!texts(f).iter().any(|t| t.contains("late")), "the late answer was dropped");
            texts(f).iter().any(|t| t.starts_with("refused")).then_some(())
        }
        _ => None,
    });
    assert!(held.0.lock().unwrap().is_empty(), "nothing left");
    // Granted again: it reaches the host again, the same component.
    run.handle.set_grants(net());
    run.click("fetch");
    run.next("a request sent", |o| match o {
        TileOutput::Frame(f) => texts(f).iter().any(|t| t == "fetching").then_some(()),
        TileOutput::Started { .. } => panic!("restarted"),
        _ => None,
    });
    assert_eq!(held.0.lock().unwrap().len(), 1);
}

/// `files.read` taken back: the next read is denied and the watches on
/// the project end.
#[test]
fn a_revoke_ends_the_project_watches() {
    let t = tempfile::tempdir().unwrap();
    let project = t.path().join("shop");
    std::fs::create_dir_all(project.join("docs")).unwrap();
    std::fs::write(project.join("docs/a.md"), "# A\n").unwrap();
    let project = project.canonicalize().unwrap();
    let host = host(&t.path().join("data"));
    let watcher = pitwall_tile_host::files::FakeWatcher::new();
    host.set_watcher(watcher.clone());
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/files");
    let m = host.load(&dir).unwrap().manifest().clone();
    let view = ViewSpec { project: Some(ui::Project { name: "shop".into(), folder: project.to_string_lossy().into_owned() }), ..ViewSpec::tile("main") };
    let run = start_from(&host, &dir, StartOptions { views: vec![view], grants: Grants::for_extension(&m, ["files.read"]), ..StartOptions::view("main") });
    run.frame_with("main", "in shop");
    run.click("watch");
    run.frame_with("main", "watching");
    assert_eq!(watcher.watching(), 1);
    run.handle.set_grants(Grants::for_extension(&m, Vec::<String>::new()));
    run.click("read");
    run.frame_with("main", "read: not allowed");
    assert_eq!(watcher.watching(), 0, "the watch ended");
    watcher.changed(&[project.join("docs/a.md")]);
    run.click("stat");
    run.next("the next stat refused", |o| match o {
        TileOutput::Frame(f) => {
            assert!(!texts(f).iter().any(|t| t.starts_with("changed: ")), "no change heard");
            texts(f).iter().any(|t| t.starts_with("stat: not allowed")).then_some(())
        }
        _ => None,
    });
}
