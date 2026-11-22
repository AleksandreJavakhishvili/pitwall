//! A files tile in a test window: a press anywhere in it makes it the
//! space's focused tile and gives its viewer the keys, ⌘⏎ maximises and
//! restores it with what it has open, and what is typed in its editor
//! stays there (⌘⏎ there maximises, adding nothing).

use std::sync::Arc;

use gpui::{Focusable, Modifiers, TestAppContext, VisualTestContext};
use pitwall_proto::AgentCaps;

use super::workspace::{FilesSpot, Space};
use super::*;
use crate::agents::tests::agent;
use crate::explorer::source::fake::FakeSource;
use crate::explorer::source::Source;
use crate::explorer::viewer::Viewer;

/// The platform's Maximise key, as `main_screen::bindings` has it (⌘⏎ on
/// macOS, Ctrl+Shift+Enter elsewhere: "cmd" is Super off macOS).
const MAXIMISE: &str = if cfg!(target_os = "macos") { "cmd-enter" } else { "ctrl-shift-enter" };

fn readable(id: &str) -> AgentView {
    let mut a = agent(id, "/work/alpha", "idle", 1, true);
    a.caps = AgentCaps { explorer: true, review: true, ..AgentCaps::default() };
    a
}

/// Two terminals (web focused), web's files in a tile beside them, a.txt
/// open in it.
fn setup(cx: &mut TestAppContext) -> (Entity<MainScreen>, Entity<Viewer>, String, &mut VisualTestContext) {
    let src = FakeSource::new(&[("a.txt", "alpha\nbeta\n"), ("b.txt", "b\n")]);
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        // The fields' own keys first, as at start.
        crate::kit::init(cx);
        init(cx);
        crate::code_view::register(cx);
        cx.bind_keys(crate::explorer::viewer::bindings());
    });
    let store = cx.new(|_| AgentStore::new(vec![readable("web"), readable("api")]));
    let source: Arc<dyn Source> = src;
    cx.update(|cx| crate::explorer::dock::remember_env(Some(source), &store, cx));
    let s = store.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| MainScreen::new(s.clone(), window, cx));
    cx.simulate_resize(gpui::size(gpui::px(1800.), gpui::px(900.)));
    view.update_in(cx, |s, window, cx| s.show_agent("web", window, cx));
    frame(cx);
    view.update(cx, |s, cx| {
        let space = s.active_space().unwrap().id.clone();
        s.update_ui(|u| u.dock_files(&space, "web", &FilesSpot::Auto, 0.45, None).0, cx);
    });
    frame(cx);
    let viewer = cx.update(|_, cx| crate::explorer::dock::viewer("web", cx)).expect("the tile's viewer");
    viewer.update(cx, |v, cx| v.open("a.txt".into(), None, cx));
    frame(cx);
    let pane = space(&view, cx).layout.leaves().iter().find(|p| p.files.as_deref() == Some("web")).expect("a files tile").id.clone();
    view.update_in(cx, |s, window, cx| s.show_agent("web", window, cx));
    frame(cx);
    assert_ne!(space(&view, cx).focused_pane_id.as_deref(), Some(pane.as_str()), "web's terminal is focused");
    (view, viewer, pane, cx)
}

fn frame(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn space(view: &Entity<MainScreen>, cx: &mut VisualTestContext) -> Space {
    view.read_with(cx, |s, _| s.active_space().cloned().unwrap())
}

fn viewer_has_keys(viewer: &Entity<Viewer>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| viewer.read(cx).focus_handle(cx).contains_focused(window, cx))
}

/// The bug: a press in a files tile left the focused tile (and ⌘⏎) where
/// it was, a terminal. Now a press on its bar's empty part makes it the
/// focused tile and gives its viewer the keys; ⌘⏎ maximises it, keeping
/// the viewer and what it has open, and restores it.
#[gpui::test]
fn the_files_tile_takes_part_in_the_tile_keys(cx: &mut TestAppContext) {
    let (view, viewer, pane, cx) = setup(cx);
    assert!(!viewer_has_keys(&viewer, cx));
    let tile = cx.debug_bounds("files-tile-web").expect("the files tile");
    let bar = cx.debug_bounds("ex-viewer-bar").expect("its bar");
    let toggle = cx.debug_bounds("ex-tree-toggle").expect("its tree toggle");
    cx.simulate_click(gpui::point(toggle.right() + gpui::px(3.), bar.center().y), Modifiers::none());
    frame(cx);
    assert_eq!(space(&view, cx).focused_pane_id.as_deref(), Some(pane.as_str()), "a press focuses its tile");
    assert!(viewer_has_keys(&viewer, cx), "its viewer has the keys");
    // Still its agent's: the panel stays, nothing moves.
    assert_eq!(view.read_with(cx, |s, _| s.focused_agent_id()).as_deref(), Some("web"));
    assert_eq!(cx.debug_bounds("files-tile-web").map(|b| b.size), Some(tile.size), "nothing moved");

    cx.simulate_keystrokes(MAXIMISE);
    frame(cx);
    assert_eq!(space(&view, cx).maximized_pane_id.as_deref(), Some(pane.as_str()), "maximised");
    let big = cx.debug_bounds("files-tile-web").expect("drawn maximised");
    assert!(big.size.width > tile.size.width, "{big:?} over {tile:?}");
    let same = cx.update(|_, cx| crate::explorer::dock::viewer("web", cx)).is_some_and(|v| v == viewer);
    assert!(same, "the same viewer");
    viewer.read_with(cx, |v, _| assert_eq!((v.tabs(), v.active()), (&["a.txt".to_string()][..], Some("a.txt"))));
    assert!(viewer_has_keys(&viewer, cx), "the keys stay in it");

    cx.simulate_keystrokes(MAXIMISE);
    frame(cx);
    assert_eq!(space(&view, cx).maximized_pane_id, None, "restored");
    assert_eq!(cx.debug_bounds("files-tile-web").map(|b| b.size), Some(tile.size));
    viewer.read_with(cx, |v, _| assert_eq!(v.active(), Some("a.txt")));

    // Back on the terminal, ⌘⏎ is its again.
    view.update_in(cx, |s, window, cx| s.show_agent("web", window, cx));
    frame(cx);
    let term = space(&view, cx).focused_pane_id.clone();
    assert_ne!(term.as_deref(), Some(pane.as_str()));
    cx.simulate_keystrokes(MAXIMISE);
    frame(cx);
    assert_eq!(space(&view, cx).maximized_pane_id, term);
}

/// A press in the file's text: the editor keeps the keys (the tile's
/// press doesn't take them back), what is typed goes in the file, and
/// ⌘⏎ maximises the tile without adding a line; typing goes on after.
#[gpui::test]
fn what_is_typed_in_a_files_tile_goes_in_the_file(cx: &mut TestAppContext) {
    let (view, viewer, pane, cx) = setup(cx);
    let code = crate::explorer::code_view::shown("a.txt").expect("a.txt is drawn");
    let tile = cx.debug_bounds("files-tile-web").expect("the files tile");
    // In the file's text, right of the tree.
    cx.simulate_click(gpui::point(tile.right() - gpui::px(60.), tile.bottom() - gpui::px(60.)), Modifiers::none());
    frame(cx);
    assert_eq!(space(&view, cx).focused_pane_id.as_deref(), Some(pane.as_str()));
    assert!(cx.update(|window, cx| code.read(cx).focus_handle(cx).is_focused(window)), "the text has the keys");
    assert!(code.read_with(cx, |c, _| c.is_editing()));
    cx.simulate_input("gamma");
    frame(cx);
    let text = |cx: &mut VisualTestContext| code.read_with(cx, |c, _| c.text().unwrap().to_string());
    let typed = text(cx);
    assert_eq!(typed.replacen("gamma", "", 1), "alpha\nbeta\n", "typed where the caret is: {typed:?}");
    viewer.read_with(cx, |v, cx| assert_eq!(v.unsaved(cx), ["a.txt"]));

    cx.simulate_keystrokes(MAXIMISE);
    frame(cx);
    assert_eq!(space(&view, cx).maximized_pane_id.as_deref(), Some(pane.as_str()));
    assert_eq!(text(cx), typed, "no line added");
    cx.simulate_input("!");
    frame(cx);
    assert_eq!(text(cx), typed.replacen("gamma", "gamma!", 1), "the keys stayed in the text");
    viewer.read_with(cx, |v, cx| assert_eq!(v.unsaved(cx), ["a.txt"]));
}
