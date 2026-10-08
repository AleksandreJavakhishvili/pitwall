//! The explorer end to end over a made-up folder: shortcuts refused with a
//! reason, the viewer opening files (Georgian names, binary, large), quick
//! open's recent list, refresh when the agent's changes move.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};

use pitwall_core::explorer::ContentKind;
use pitwall_proto::AgentCaps;

use super::source::fake::FakeSource;
use super::source::TEXT_CAP;
use super::*;
use crate::agents::tests::agent;

fn readable(id: &str) -> pitwall_proto::AgentView {
    let mut a = agent(id, "/work/alpha", "idle", 0, true);
    a.caps = AgentCaps {
        explorer: true,
        review: true,
        ..AgentCaps::default()
    };
    a
}

fn setup(
    cx: &mut TestAppContext,
    agents: Vec<pitwall_proto::AgentView>,
    src: Arc<FakeSource>,
) -> (Entity<AgentStore>, Entity<Explorer>, &mut VisualTestContext) {
    cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
    let store = cx.new(|_| AgentStore::new(agents));
    let s = store.clone();
    let source: Arc<dyn Source> = src;
    let (explorer, cx) =
        cx.add_window_view(move |_, cx| Explorer::with_source(Some(source), s, cx));
    (store, explorer, cx)
}

#[gpui::test]
async fn shortcuts_say_why_they_cannot_run(cx: &mut TestAppContext) {
    let mut plain = agent("vm", "/srv/x", "idle", 0, false);
    plain.name = "builder".into();
    let (_, explorer, cx) = setup(cx, vec![plain], FakeSource::new(&[]));
    cx.update(|window, cx| explorer.update(cx, |e, cx| e.go_to_file(window, cx)));
    explorer.read_with(cx, |e, _| {
        assert_eq!(e.notice(), Some("Focus an agent to browse its files"))
    });
    explorer.update(cx, |e, cx| e.set_agent(Some("vm".into()), cx));
    cx.update(|window, cx| explorer.update(cx, |e, cx| e.search_in_files(window, cx)));
    explorer.read_with(cx, |e, _| {
        assert_eq!(e.notice(), Some("builder's files can't be read from here"));
        assert!(!e.viewer_open());
    });
}

#[gpui::test]
async fn the_viewer_opens_text_binary_and_large_files(cx: &mut TestAppContext) {
    let big = "x".repeat(TEXT_CAP as usize + 10);
    let src = FakeSource::new(&[
        ("docs/ანგარიში.md", "სათაური\nline two\n"),
        ("img/logo.png", "\u{0}PNG"),
        ("dump.json", big.as_str()),
    ]);
    let (_, explorer, cx) = setup(cx, vec![readable("a1")], src.clone());
    explorer.update(cx, |e, cx| e.set_agent(Some("a1".into()), cx));
    cx.run_until_parked();

    let open = |path: &str, cx: &mut VisualTestContext| {
        let path = path.to_string();
        cx.update(|window, cx| {
            explorer.update(cx, |e, cx| {
                e.open_viewer("a1", Some(path), Some((2, None)), None, window, cx)
            })
        });
        cx.run_until_parked();
    };
    open("docs/ანგარიში.md", cx);
    let viewer = explorer.read_with(cx, |e, _| {
        assert!(e.viewer_open());
        e.viewers["a1"].clone()
    });
    viewer.read_with(cx, |v, _| {
        assert_eq!(v.active(), Some("docs/ანგარიში.md"));
        assert_eq!(v.doc_kind("docs/ანგარიში.md"), Some(ContentKind::Text));
    });
    // The tree opened the file's folder to show it.
    explorer.read_with(cx, |e, cx| {
        assert!(e.trees["a1"].read(cx).is_open("docs"));
    });

    open("img/logo.png", cx);
    viewer.read_with(cx, |v, _| {
        assert_eq!(v.doc_kind("img/logo.png"), Some(ContentKind::Binary))
    });

    open("dump.json", cx);
    viewer.read_with(cx, |v, _| {
        assert_eq!(v.doc_kind("dump.json"), Some(ContentKind::TooLarge))
    });
    viewer.update(cx, |v, cx| v.load_anyway("dump.json", cx));
    cx.run_until_parked();
    viewer.read_with(cx, |v, _| {
        assert_eq!(v.doc_kind("dump.json"), Some(ContentKind::Text));
        assert_eq!(v.tabs().len(), 3);
    });

    // Recent files come first in quick open, newest first.
    explorer.read_with(cx, |e, _| {
        assert_eq!(
            e.recents["a1"][..2],
            ["dump.json".to_string(), "img/logo.png".to_string()]
        );
    });

    // Changed on disk: a quiet re-read offers the newer text.
    src.files
        .lock()
        .unwrap()
        .insert("docs/ანგარიში.md".into(), "ახალი\n".into());
    viewer.update(cx, |v, cx| v.close_tab("dump.json", cx));
    viewer.update(cx, |v, cx| v.close_tab("img/logo.png", cx));
    viewer.read_with(cx, |v, _| assert_eq!(v.active(), Some("docs/ანგარიში.md")));
    viewer.update(cx, |v, cx| v.recheck(cx));
    cx.run_until_parked();
    viewer.read_with(cx, |v, _| assert!(v.is_stale()));

    explorer.update(cx, |e, cx| e.close_viewer(cx));
    explorer.read_with(cx, |e, _| assert!(!e.viewer_open()));
}

#[gpui::test]
async fn moving_change_totals_refresh_the_tree(cx: &mut TestAppContext) {
    let src = FakeSource::new(&[("a.rs", "a")]);
    let (store, explorer, cx) = setup(cx, vec![readable("a1")], src.clone());
    explorer.update(cx, |e, cx| e.set_agent(Some("a1".into()), cx));
    cx.run_until_parked();
    // The first sighting of the totals is remembered.
    store.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    let before = src.lists.load(Ordering::SeqCst);
    store.update(cx, |s, cx| {
        s.agents[0].files_changed = 3;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(src.lists.load(Ordering::SeqCst) > before);
}
