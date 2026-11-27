//! A grant changed on an extension's card reaches its running tile at
//! once, headless (GPUI's test platform), on the tile host's services and
//! files fixtures: `net` granted, a fetch answered; refused on the card,
//! the next fetch is refused and a fetch still on its way isn't
//! delivered, with no restart; the same for `files.read`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use pitwall_core::extensions::permissions::{self, Decision};
use pitwall_core::testing::TempDir;
use pitwall_tile_host::net::{Done, Fetcher, Outgoing, Reach, Response};
use pitwall_tile_host::ui::{Modifiers, MouseButton, Point};

use super::tests::{copy_dir, init};
use super::TileView;

const WAIT: Duration = Duration::from_secs(60);

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures").join(name)
}

fn until(cx: &mut VisualTestContext, what: &str, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let end = Instant::now() + WAIT;
    loop {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
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

fn has(tile: &Entity<TileView>, text: &str, cx: &mut VisualTestContext) -> bool {
    texts(tile, cx).iter().any(|t| t.starts_with(text))
}

fn click(tile: &Entity<TileView>, path: &str, cx: &mut VisualTestContext) {
    let sent = tile.read_with(cx, |t, cx| t.click(path, MouseButton::Left, Modifiers::empty(), 1, Point { x: 3., y: 3. }, cx));
    assert!(sent, "{path} has a click handler");
}

/// Which start of `id`'s component runs now (a restart moves it).
fn gen(id: &str, cx: &mut VisualTestContext) -> u64 {
    cx.update(|_, cx| super::tiles(cx).read(cx).get(id).map(|r| r.gen)).expect("it runs")
}

/// The card's switch for `name`, as a click on it does.
fn card(id: &str, name: &str, granted: bool, cx: &mut VisualTestContext) {
    cx.update(|_, cx| crate::extensions::calls::decide(id, name, granted, cx));
}

/// Answers each request with "ok" at once, or holds it while `hold` is
/// set (the test answers it later).
#[derive(Default)]
struct Fake {
    hold: Mutex<bool>,
    held: Mutex<Vec<Done>>,
}

impl Fetcher for Fake {
    fn fetch(&self, request: Outgoing, _reach: Reach, done: Done) {
        if *self.hold.lock().unwrap() {
            self.held.lock().unwrap().push(done);
            return;
        }
        done(Ok(Response { status: 200, headers: vec![], body: b"ok".to_vec(), url: request.url.to_string() }));
    }
}

#[gpui::test]
fn refusing_net_on_the_card_refuses_the_next_fetch_without_a_restart(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-grants-net");
    let root = t.path().join("data");
    copy_dir(&fixture("services"), &root.join("extensions/services"));
    init(&root, cx);
    let fake = Arc::new(Fake::default());
    let f = fake.clone();
    cx.update(|cx| {
        super::tiles(cx).update(cx, |t, cx| t.host(cx).expect("the host").set_fetcher(f));
        crate::ui_state::update(cx, |u| {
            *u = std::mem::take(u).with_extension("services", |e| {
                e.enabled = true;
                e.grants.insert(permissions::NET.into(), Decision::now(true));
            })
        });
    });
    let (tile, cx) = cx.add_window_view(|_, cx| TileView::new("services", "main", cx));
    until(cx, "its first frame", |cx| has(&tile, "schema 2", cx));
    let started = gen("services", cx);
    click(&tile, "do/fetch", cx);
    until(cx, "the answer", |cx| has(&tile, "fetched 200: ok", cx));
    // Held on its way when the user refuses `net` on the card.
    *fake.hold.lock().unwrap() = true;
    click(&tile, "do/fetch", cx);
    until(cx, "fetching", |cx| has(&tile, "fetching", cx));
    card("services", permissions::NET, false, cx);
    until(cx, "the fetch on its way refused", |cx| has(&tile, "fetch failed", cx));
    let late = fake.held.lock().unwrap().pop().expect("the fake holds it");
    late(Ok(Response { status: 200, headers: vec![], body: b"late".to_vec(), url: "https://api.example.com/items?x=1".into() }));
    // The next one is refused before anything leaves.
    *fake.hold.lock().unwrap() = false;
    click(&tile, "do/fetch", cx);
    until(cx, "the next fetch refused", |cx| has(&tile, "refused", cx));
    assert!(!has(&tile, "fetched 200: late", cx), "{:?}", texts(&tile, cx));
    assert_eq!(gen("services", cx), started, "no restart");
    // Granted again on the card: it fetches again.
    card("services", permissions::NET, true, cx);
    cx.run_until_parked();
    click(&tile, "do/fetch", cx);
    until(cx, "the answer again", |cx| has(&tile, "fetched 200: ok", cx));
    assert_eq!(gen("services", cx), started, "no restart");
}

#[gpui::test]
fn refusing_files_read_on_the_card_refuses_the_next_read_without_a_restart(cx: &mut TestAppContext) {
    let t = TempDir::new("tile-grants-files");
    let root = t.path().join("data");
    copy_dir(&fixture("files"), &root.join("extensions/files"));
    let project = t.path().join("shop");
    std::fs::create_dir_all(project.join("docs")).unwrap();
    std::fs::write(project.join("docs/a.md"), "# A\n").unwrap();
    let project = project.canonicalize().unwrap().to_string_lossy().into_owned();
    init(&root, cx);
    cx.update(|cx| {
        crate::ui_state::update(cx, |u| {
            *u = std::mem::take(u).with_extension("files", |e| {
                e.enabled = true;
                e.grants.insert(permissions::FILES_READ.into(), Decision::now(true));
            })
        });
        super::tiles(cx).update(cx, |t, cx| t.hint_project("files", "main", &project, cx));
    });
    let (tile, cx) = cx.add_window_view(|_, cx| TileView::new("files", "main", cx));
    until(cx, "its project", |cx| has(&tile, "in shop", cx));
    let started = gen("files", cx);
    click(&tile, "do/read", cx);
    until(cx, "the file read", |cx| has(&tile, "read: # A", cx));
    card("files", permissions::FILES_READ, false, cx);
    cx.run_until_parked();
    click(&tile, "do/read", cx);
    until(cx, "the next read refused", |cx| has(&tile, "read: not allowed", cx));
    assert_eq!(gen("files", cx), started, "no restart");
}
