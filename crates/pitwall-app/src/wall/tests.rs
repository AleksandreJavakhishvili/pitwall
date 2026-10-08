//! The Wall in a test window, with made-up agents and a fake screen source.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};

use pitwall_core::term::FrameSink;
use pitwall_proto::{ScreenFrame, ScreenLine, ScreenRun};

use super::source::{ScreenSource, Screens};
use super::{WallEvent, WallView};
use crate::agents::tests::agent;
use crate::agents::AgentStore;
use crate::theme::Theme;

/// Records watches and keeps their sinks so the test can send frames.
#[derive(Default)]
struct Fake {
    sinks: Mutex<Vec<(String, u64, FrameSink)>>,
    unwatched: Mutex<Vec<String>>,
    next: Mutex<u64>,
}

impl ScreenSource for Fake {
    fn watch(&self, agent_id: &str, sink: FrameSink) -> Result<u64, String> {
        if agent_id.starts_with("broken") {
            return Err("not running".into());
        }
        let mut n = self.next.lock().unwrap();
        *n += 1;
        self.sinks.lock().unwrap().push((agent_id.into(), *n, sink));
        Ok(*n)
    }
    fn unwatch(&self, agent_id: &str, watch_id: u64) {
        self.sinks
            .lock()
            .unwrap()
            .retain(|(_, id, _)| *id != watch_id);
        self.unwatched.lock().unwrap().push(agent_id.into());
    }
}

/// Let the Wall's frame batching run.
fn settle(vcx: &mut VisualTestContext) {
    vcx.executor().advance_clock(super::FRAME_BATCH);
    vcx.run_until_parked();
}

impl Fake {
    fn send(&self, agent_id: &str, frame: &ScreenFrame) {
        for (a, _, sink) in self.sinks.lock().unwrap().iter_mut() {
            if a == agent_id {
                sink(frame);
            }
        }
    }
}

fn frame(text: &str) -> ScreenFrame {
    ScreenFrame {
        cols: 40,
        rows: 4,
        cursor: Some((0, 3)),
        full: true,
        lines: vec![ScreenLine(3, vec![ScreenRun(text.into(), 3, 0, 0)])],
    }
}

fn open(
    cx: &mut TestAppContext,
    agents: Vec<pitwall_proto::AgentView>,
) -> (
    Arc<Fake>,
    Entity<AgentStore>,
    Entity<WallView>,
    VisualTestContext,
) {
    let fake = Arc::new(Fake::default());
    let f = fake.clone();
    cx.update(|cx| {
        cx.set_global(Theme::dark());
        cx.set_global(Screens(f));
        cx.bind_keys(super::bindings());
    });
    let store = cx.new(|_| AgentStore::new(agents));
    let s = store.clone();
    let window = cx.add_window(move |window, cx| WallView::new(s, window, cx));
    let wall = window.root(cx).unwrap();
    let vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    (fake, store, wall, vcx)
}

#[gpui::test]
fn visible_running_tiles_are_watched_and_drawn_from_frames(cx: &mut TestAppContext) {
    let mut stopped = agent("c-stopped", "/work/alpha", "stopped", 2, true);
    stopped.running = false;
    let agents = vec![
        agent("a-work", "/work/alpha", "working", 0, true),
        agent("b-idle", "/work/alpha", "idle", 1, true),
        stopped,
        agent("broken", "/work/beta", "idle", 0, true),
    ];
    let (fake, _store, wall, mut vcx) = open(cx, agents);
    wall.read_with(&vcx, |w, _| {
        assert_eq!(
            w.watching(),
            BTreeSet::from(["a-work".to_string(), "b-idle".to_string()]),
            "running agents only; a failed watch is not counted"
        );
    });
    wall.read_with(&vcx, |w, cx| {
        assert!(w.tile("broken").unwrap().read(cx).failed);
        assert_eq!(
            w.tile("broken").unwrap().read(cx).off_label(),
            Some("NOT RUNNING")
        );
        assert_eq!(
            w.tile("c-stopped").unwrap().read(cx).off_label(),
            Some("STOPPED")
        );
    });

    // A watch that sends no first frame (a terminal being replaced) and a
    // failed one are tried again.
    vcx.executor().advance_clock(super::RETRY);
    vcx.run_until_parked();
    assert!(fake
        .unwatched
        .lock()
        .unwrap()
        .contains(&"a-work".to_string()));
    wall.read_with(&vcx, |w, _| {
        assert_eq!(w.watching().len(), 2, "watched again")
    });

    fake.send("a-work", &frame("$ cargo test"));
    vcx.run_until_parked();
    wall.read_with(&vcx, |w, cx| {
        let t = w.tile("a-work").unwrap().read(cx);
        assert!(t.screen.has_frame());
        assert_eq!(t.screen.lines[3].text(), "$ cargo test");
        assert!(!w.tile("b-idle").unwrap().read(cx).screen.has_frame());
    });
    // Partial frames update only their rows.
    fake.send(
        "a-work",
        &ScreenFrame {
            cols: 40,
            rows: 4,
            cursor: None,
            full: false,
            lines: vec![ScreenLine(0, vec![ScreenRun("ok".into(), 0, 0, 0)])],
        },
    );
    settle(&mut vcx);
    wall.read_with(&vcx, |w, cx| {
        let t = w.tile("a-work").unwrap().read(cx);
        assert_eq!(t.screen.lines[0].text(), "ok");
        assert_eq!(t.screen.lines[3].text(), "$ cargo test");
    });

    // Leaving the Wall stops every watch.
    drop(wall);
    vcx.update(|window, _| window.remove_window());
    vcx.run_until_parked();
    assert!(fake.sinks.lock().unwrap().is_empty());
}

#[gpui::test]
fn stopping_an_agent_ends_its_watch_and_keeps_its_screen(cx: &mut TestAppContext) {
    let (fake, store, wall, mut vcx) =
        open(cx, vec![agent("a", "/work/alpha", "working", 0, true)]);
    fake.send("a", &frame("last words"));
    vcx.run_until_parked();
    let mut a = agent("a", "/work/alpha", "stopped", 0, true);
    a.running = false;
    store.update(&mut vcx, |s, cx| {
        s.agents = vec![a];
        cx.notify();
    });
    vcx.run_until_parked();
    assert_eq!(fake.unwatched.lock().unwrap().as_slice(), ["a"]);
    wall.read_with(&vcx, |w, cx| {
        assert!(w.watching().is_empty());
        let t = w.tile("a").unwrap().read(cx);
        assert_eq!(t.off_label(), None, "the last screen stays");
    });
    // Started again: watched again.
    store.update(&mut vcx, |s, cx| {
        s.agents = vec![agent("a", "/work/alpha", "working", 0, true)];
        cx.notify();
    });
    vcx.run_until_parked();
    wall.read_with(&vcx, |w, _| assert_eq!(w.watching().len(), 1));
    // Gone from the engine: its tile goes too.
    store.update(&mut vcx, |s, cx| {
        s.agents.clear();
        cx.notify();
    });
    vcx.run_until_parked();
    wall.read_with(&vcx, |w, _| {
        assert!(w.tile("a").is_none());
        assert!(w.watching().is_empty());
    });
}

#[gpui::test]
fn keys_select_tiles_and_open_them(cx: &mut TestAppContext) {
    let agents = vec![
        agent("a0", "/work/alpha", "idle", 0, true),
        agent("a1", "/work/alpha", "idle", 1, true),
        agent("b0", "/work/beta", "idle", 0, true),
    ];
    let (_fake, _store, wall, mut vcx) = open(cx, agents);
    let events = Arc::new(Mutex::new(Vec::new()));
    let e = events.clone();
    vcx.update(|_, cx| {
        cx.subscribe(&wall, move |_, ev: &WallEvent, _| {
            e.lock().unwrap().push(ev.clone())
        })
        .detach();
    });
    vcx.simulate_keystrokes("right");
    wall.read_with(&vcx, |w, _| assert_eq!(w.selected(), Some("a0")));
    vcx.simulate_keystrokes("right right");
    wall.read_with(&vcx, |w, cx| {
        assert_eq!(w.selected(), Some("b0"));
        assert!(w.tile("b0").unwrap().read(cx).selected);
        assert!(!w.tile("a0").unwrap().read(cx).selected);
    });
    vcx.simulate_keystrokes("shift-tab enter");
    vcx.run_until_parked();
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [WallEvent::Open("a1".into())]
    );

    // Folding a section hides its tiles and reports it.
    let key = wall.read_with(&vcx, |w, cx| w.store.read(cx).groups()[0].key.clone());
    wall.update(&mut vcx, |w, cx| w.toggle_section(&key, cx));
    vcx.run_until_parked();
    wall.read_with(&vcx, |w, _| {
        assert_eq!(w.selected(), None, "the selection was folded away");
        assert_eq!(w.watching(), BTreeSet::from(["b0".to_string()]));
    });
    assert!(
        matches!(events.lock().unwrap().last(), Some(WallEvent::Collapsed(k)) if k.contains(&key))
    );
}
