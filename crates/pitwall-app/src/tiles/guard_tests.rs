//! Crash-loop protection in the app, headless ([`super::guard`]): the
//! record the last launch left in the data folder (a launch that crashed
//! while the hello tile was starting) is written by hand, then the app's
//! launch is played: its tiles paused, with the paused screen and Resume;
//! its card in Settings saying so; twice in a row, turned off with a note;
//! a component that ran a while, or a clean quit, crossed out.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use pitwall_core::testing::TempDir;

use super::guard::{self, Note, Record, FILE};
use super::tests::{copy_fixture, init};
use super::{State, TileView, Tiles};
use crate::settings::view::SettingsView;

const WAIT: Duration = Duration::from_secs(60);
/// How long a component runs before it is crossed out, in these tests.
const HEALTHY: Duration = Duration::from_millis(500);

fn frame(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

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

fn tiles(cx: &mut VisualTestContext) -> Entity<Tiles> {
    cx.update(|_, cx| super::tiles(cx))
}

fn state(cx: &mut VisualTestContext) -> Option<State> {
    let t = tiles(cx);
    t.read_with(cx, |t, _| t.get("hello").map(|r| r.state.clone()))
}

/// A data folder with the hello tile installed (unless it is already)
/// and on.
fn setup(t: &TempDir, cx: &mut TestAppContext) -> std::path::PathBuf {
    let root = t.path().join("data");
    if !root.join("extensions/hello").is_dir() {
        copy_fixture(&root.join("extensions/hello"));
    }
    init(&root, cx);
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true)));
    root
}

/// What the last launch left: hello was starting (version 0.1.0), after
/// `crashes` launches in a row that crashed with it.
fn left(crashes: u32) -> Record {
    Record {
        starting: BTreeMap::from([("hello".to_string(), Some("0.1.0".to_string()))]),
        crashes: if crashes > 0 { BTreeMap::from([("hello".to_string(), crashes)]) } else { BTreeMap::new() },
        turned_off: BTreeMap::new(),
    }
}

/// The last launch ended while hello's tile was starting, just after it
/// was updated to 0.2.0: in this one its tile isn't started. Its view
/// says it is paused (Resume, Turn off), agents' calls are refused, and
/// its card in Settings says so, with Go back to 0.1.0. Resume
/// starts it; it is written down before it starts, and crossed out once
/// it has run a while, with its count started again.
#[gpui::test]
fn a_crash_while_a_tile_starts_pauses_it_next_launch(cx: &mut TestAppContext) {
    let t = TempDir::new("tiles-guard-paused");
    // Updated from 0.1.0, which the update kept for Go back.
    let exts = t.path().join("data/extensions");
    copy_fixture(&exts.join(".hello.previous"));
    copy_fixture(&exts.join("hello"));
    let m = std::fs::read_to_string(exts.join("hello/extension.json")).unwrap();
    std::fs::write(exts.join("hello/extension.json"), m.replacen(r#""version": "0.1.0""#, r#""version": "0.2.0""#, 1)).unwrap();
    let root = setup(&t, cx);
    let mut rec = left(0);
    rec.starting.insert("hello".into(), Some("0.2.0".into()));
    cx.update(|cx| guard::init_for_test(&root, &rec, HEALTHY, cx));
    cx.update(|cx| {
        assert!(guard::paused("hello", cx));
        assert_eq!(guard::note("hello", cx), Some(Note::Paused { version: Some("0.2.0".into()) }));
        assert!(crate::ui_state::get(cx).extension_on("hello"), "still on");
    });
    // On disk: one crash, nothing starting.
    let rec = guard::read(&root);
    assert_eq!(rec.crashes.get("hello"), Some(&1));
    assert!(rec.starting.is_empty());

    // Its card in Settings, in a window of its own.
    let sw = cx.add_window(|_, cx| SettingsView::new(true, cx));
    let mut scx = VisualTestContext::from_window(sw.into(), cx);
    let scx = &mut scx;
    scx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(2400.)));
    let (tile, cx) = cx.add_window_view(|_, cx| TileView::new("hello", "main", cx));
    cx.simulate_resize(gpui::size(gpui::px(600.), gpui::px(400.)));
    frame(cx);
    assert_eq!(state(cx), Some(State::Paused), "not started");
    let tl = tiles(cx);
    assert!(tl.read_with(cx, |t, _| t.get("hello").is_some_and(|r| r.handle.is_none())), "no component");
    assert!(cx.debug_bounds("tile-resume").is_some() && cx.debug_bounds("tile-turn-off").is_some(), "the paused screen, with Resume and Turn off");
    let (tx, _rx) = std::sync::mpsc::channel();
    let refused = tl.update(cx, |t, cx| t.deliver("hello", "main", 1, pitwall_tile_host::ui::Content { id: None, json: None, bytes: None, call: Some(1), tool: None, caller: None, append: false, large: None }, tx, cx));
    assert!(refused.unwrap_err().contains("paused"), "an agent's call is told");
    assert!(!guard::read(&root).starting.contains_key("hello"), "nothing was started");

    frame(scx);
    assert!(scx.debug_bounds("extension-crash-hello").is_some(), "the card says so");
    assert!(scx.debug_bounds("extension-crash-resume-hello").is_some());
    assert!(scx.debug_bounds("extension-crash-back-hello").is_some(), "with Go back to the version before the update");

    // Resume: started, written down first.
    cx.update(|_, cx| guard::resume("hello", cx));
    frame(cx);
    assert!(guard::read(&root).starting.contains_key("hello"), "written down before it starts");
    until(cx, "its first frame", |cx| tile.read_with(cx, |t, _| t.texts().iter().any(|s| s.contains("Clicked 0 times"))));
    assert_eq!(state(cx), Some(State::Running));
    cx.update(|_, cx| assert_eq!(guard::note("hello", cx), None));
    // Run a while: crossed out, the count gone with it (no file left).
    cx.executor().advance_clock(HEALTHY * 2);
    frame(cx);
    assert_eq!(guard::read(&root), Record::default());
    assert!(!root.join(FILE).exists());
    frame(scx);
    assert!(scx.debug_bounds("extension-crash-hello").is_none(), "the card's note is gone");
}

/// The second launch in a row that ended while it was starting: it is
/// turned off, its card says why, and turning it on again clears it.
#[gpui::test]
fn two_crashes_in_a_row_turn_the_extension_off(cx: &mut TestAppContext) {
    let t = TempDir::new("tiles-guard-off");
    let root = setup(&t, cx);
    cx.update(|cx| guard::init_for_test(&root, &left(1), HEALTHY, cx));
    cx.update(|cx| {
        assert!(!crate::ui_state::get(cx).extension_on("hello"), "turned off");
        assert!(!guard::paused("hello", cx));
        assert_eq!(guard::note("hello", cx), Some(Note::TurnedOff { version: Some("0.1.0".into()) }));
    });
    assert_eq!(guard::read(&root).turned_off.get("hello"), Some(&Some("0.1.0".to_string())), "kept for the next launch");
    let sw = cx.add_window(|_, cx| SettingsView::new(true, cx));
    let mut scx = VisualTestContext::from_window(sw.into(), cx);
    let scx = &mut scx;
    scx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(2400.)));
    frame(scx);
    assert!(scx.debug_bounds("extension-crash-hello").is_some(), "the card says why it is off");
    assert!(scx.debug_bounds("extension-crash-resume-hello").is_none(), "nothing to resume");
    // On again: the note goes and it gets a fresh count.
    scx.update(|_, cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).set_extension("hello", true)));
    frame(scx);
    scx.update(|_, cx| assert_eq!(guard::note("hello", cx), None));
    assert_eq!(guard::read(&root), Record::default());
}

/// A component that started and Pitwall quit cleanly before it had run
/// a while: crossed out by the quit, so the next launch starts it.
#[gpui::test]
fn a_clean_quit_crosses_out_what_was_starting(cx: &mut TestAppContext) {
    let t = TempDir::new("tiles-guard-quit");
    let root = setup(&t, cx);
    cx.update(|cx| guard::init_for_test(&root, &Record::default(), Duration::from_secs(3600), cx));
    let (tile, cx) = cx.add_window_view(|_, cx| TileView::new("hello", "main", cx));
    until(cx, "its first frame", |cx| tile.read_with(cx, |t, _| t.texts().iter().any(|s| s.contains("Clicked 0 times"))));
    assert!(guard::read(&root).starting.contains_key("hello"), "written down while it starts");
    cx.update(|_, cx| guard::quit(cx));
    assert_eq!(guard::read(&root), Record::default());
    // The next launch: nothing paused.
    cx.update(|_, cx| {
        guard::init(Some(&root), cx);
        assert!(!guard::paused("hello", cx));
    });
}
