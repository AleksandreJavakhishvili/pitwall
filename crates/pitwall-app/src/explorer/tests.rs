//! The explorer end to end over a made-up folder: shortcuts refused with a
//! reason, the viewer opening files (Georgian names, binary, large), quick
//! open's recent list, refresh when the agent's changes move.

use std::collections::HashMap;
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

/// `file:line` (an agent showing it, a ⌘-clicked path): the line is
/// scrolled to and marked, the whole line when no column is given.
#[gpui::test]
async fn a_line_given_is_scrolled_to_and_marked(cx: &mut TestAppContext) {
    let text: String = (1..=400).map(|i| format!("line number {i}\n")).collect();
    let src = FakeSource::new(&[("src/long.rs", text.as_str())]);
    cx.update(|cx| cx.set_global(crate::theme::Theme::dark()));
    let store = cx.new(|_| AgentStore::new(vec![readable("a1")]));
    let source: Arc<dyn Source> = src;
    let (viewer, cx) = cx.add_window_view(move |window, cx| {
        let tree = cx.new(|_| super::tree::TreeModel::new(source.clone(), "a1".into(), false));
        viewer::Viewer::new(source, store, "a1".into(), tree, window, cx)
    });
    let at = |line: u32, column: Option<u32>, cx: &mut VisualTestContext| {
        viewer.update(cx, |v, cx| v.open("src/long.rs".into(), super::file_link::target(Some(line), column), cx));
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let code = super::code_view::shown("src/long.rs").expect("the file is drawn");
        code.read_with(cx, |c, _| c.selection().map(|s| (s.anchor.line, s.anchor.col, s.head.col)))
    };
    // No column: line 300 (0-based 299) marked from its start to its end.
    assert_eq!(at(300, None, cx), Some((299, 0, "line number 300".len())));
    // A column: from there to the end of the word.
    let (line, from, _) = at(12, Some(6), cx).unwrap();
    assert_eq!((line, from), (11, 5));
}

#[gpui::test]
async fn under_liquid_glass_a_notice_is_a_pop_up(cx: &mut TestAppContext) {
    let (_, explorer, cx) = setup(cx, vec![], FakeSource::new(&[]));
    cx.update(|_, cx| crate::kit::passive::test_glass(cx));
    let open = |cx: &mut VisualTestContext| cx.update(|window, cx| crate::kit::passive::test_over(window, cx).len());
    cx.run_until_parked();
    assert_eq!(open(cx), 0);
    cx.update(|window, cx| explorer.update(cx, |e, cx| e.go_to_file(window, cx)));
    cx.run_until_parked();
    assert_eq!(explorer.read_with(cx, |e, _| e.notice().map(str::to_string)).as_deref(), Some("Focus an agent to browse its files"));
    assert_eq!(open(cx), 1, "the notice, above the window");
    // It goes after its time, and its pop-up with it.
    cx.executor().advance_clock(std::time::Duration::from_secs(3));
    crate::kit::passive::test_draw(cx);
    assert_eq!(open(cx), 0);
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
        ("bin/tool", "\u{0}ELF"),
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

    // An image: its bytes read for drawing it.
    open("img/logo.png", cx);
    viewer.read_with(cx, |v, _| {
        assert_eq!(v.doc_kind("img/logo.png"), Some(ContentKind::Image));
        assert!(v.image_loaded("img/logo.png"));
    });
    open("bin/tool", cx);
    viewer.read_with(cx, |v, _| {
        assert_eq!(v.doc_kind("bin/tool"), Some(ContentKind::Binary));
        assert!(!v.image_loaded("bin/tool"));
    });
    viewer.update(cx, |v, cx| v.close_tab("bin/tool", cx));

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
            ["dump.json".to_string(), "bin/tool".to_string()]
        );
    });

    // Changed on disk with nothing unsaved: read again quietly.
    src.files
        .lock()
        .unwrap()
        .insert("docs/ანგარიში.md".into(), "ახალი\n".into());
    viewer.update(cx, |v, cx| v.close_tab("dump.json", cx));
    viewer.update(cx, |v, cx| v.close_tab("img/logo.png", cx));
    viewer.read_with(cx, |v, _| assert_eq!(v.active(), Some("docs/ანგარიში.md")));
    viewer.update(cx, |v, cx| v.recheck(cx));
    cx.run_until_parked();
    viewer.read_with(cx, |v, _| assert!(!v.is_stale() && v.shows_text("docs/ანგარიში.md", "ახალი\n")));

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

/// A Markdown file: Code / Preview in the header; Preview parses the file
/// and the choice holds for the next Markdown file, not for code.
#[gpui::test]
async fn markdown_files_preview_in_the_viewer(cx: &mut TestAppContext) {
    let src = FakeSource::new(&[
        ("README.md", "# Hello\n\n- [x] done\n\n| a | b |\n|---|--:|\n| 1 | 2 |\n"),
        ("src/main.rs", "fn main() {}\n"),
    ]);
    let (_, explorer, cx) = setup(cx, vec![readable("a1")], src);
    explorer.update(cx, |e, cx| e.set_agent(Some("a1".into()), cx));
    cx.run_until_parked();
    let open = |path: &str, cx: &mut VisualTestContext| {
        let path = path.to_string();
        cx.update(|window, cx| {
            explorer.update(cx, |e, cx| e.open_viewer("a1", Some(path), None, None, window, cx))
        });
        cx.run_until_parked();
    };
    open("README.md", cx);
    let viewer = explorer.read_with(cx, |e, _| e.viewers["a1"].clone());
    // (The explorer alone doesn't draw the viewer: its state says what is.)
    assert!(viewer.read_with(cx, |v, _| v.md_text().is_some()), "Code / Preview");
    assert!(!viewer.read_with(cx, |v, _| v.showing_preview()), "Code by default");
    viewer.update(cx, |v, cx| v.set_preview(true, cx));
    cx.run_until_parked();
    viewer.read_with(cx, |v, cx| {
        assert!(v.showing_preview());
        let md = v.md.read(cx);
        let d = md.doc().expect("parsed");
        assert_eq!(d.blocks.len(), 3);
        assert!(d.blocks.iter().all(|b| b.change.is_none()), "no diff here");
    });

    // Drawn on its own, the Preview selects and copies as Review's does.
    struct Host(Entity<Viewer>);
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().flex().child(self.0.clone())
        }
    }
    cx.update(|_, cx| crate::markdown_view::register(cx));
    let v2 = viewer.clone();
    let (_, vcx) = cx.add_window_view(move |_, _| Host(v2));
    vcx.simulate_resize(gpui::size(gpui::px(1100.), gpui::px(700.)));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("f12");
    let body = vcx.debug_bounds("ex-md-body").expect("the Preview is drawn");
    let md = viewer.read_with(vcx, |v, _| v.md.clone());
    let first = md.read_with(vcx, |m, _| m.text_position(0, 1));
    assert!(body.contains(&first), "{first:?} in {body:?}");
    vcx.simulate_click(first, gpui::Modifiers::none());
    vcx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-a cmd-c" } else { "ctrl-a ctrl-c" });
    assert_eq!(
        vcx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("Hello\n\n- [x] done\n\na\tb\n1\t2")
    );

    open("src/main.rs", cx);
    viewer.read_with(cx, |v, _| {
        assert!(v.md_text().is_none() && !v.showing_preview(), "code stays code")
    });
}

// ── editing in the viewer ───────────────────────────────────────────────

const SAVE: &str = if cfg!(target_os = "macos") { "cmd-s" } else { "ctrl-s" };

/// A viewer over `files` of agent `agent`, in a window of its own.
fn edit_setup<'a>(
    cx: &'a mut TestAppContext,
    agent: pitwall_proto::AgentView,
    files: &[(&str, &str)],
) -> (Entity<viewer::Viewer>, Arc<FakeSource>, &'a mut VisualTestContext) {
    let src = FakeSource::new(files);
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::code_view::register(cx);
        cx.bind_keys(viewer::bindings());
    });
    let store = cx.new(|_| AgentStore::new(vec![agent.clone()]));
    let source: Arc<dyn Source> = src.clone();
    let id = agent.id.clone();
    let (viewer, cx) = cx.add_window_view(move |window, cx| {
        let tree = cx.new(|_| super::tree::TreeModel::new(source.clone(), id.clone(), false));
        viewer::Viewer::new(source, store, id, tree, window, cx)
    });
    (viewer, src, cx)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    // (A key nothing binds makes the test window draw.)
    cx.simulate_keystrokes("f12");
    cx.run_until_parked();
}

/// Open `path` and put the keyboard in its text, the caret at its end of
/// line `line`.
fn open_for_typing(viewer: &Entity<viewer::Viewer>, path: &str, line: usize, cx: &mut VisualTestContext) -> Entity<crate::code_view::CodeView> {
    viewer.update(cx, |v, cx| v.open(path.into(), Some((line as u32 + 1, None)), cx));
    redraw(cx);
    let code = super::code_view::shown(path).expect("the file is drawn");
    cx.update(|window, cx| window.focus(&gpui::Focusable::focus_handle(code.read(cx), cx), cx));
    redraw(cx);
    code.update(cx, |c, cx| {
        let s = c.selection().unwrap();
        c.reveal(crate::code_view::Side::New, line, Some(s.head.col..s.head.col.max(s.anchor.col)), cx);
    });
    cx.simulate_keystrokes("end");
    code
}

fn on_disk(src: &FakeSource, path: &str) -> String {
    src.files.lock().unwrap()[path].clone()
}

/// Typing marks the tab, ⌘S writes the file (its CRLF kept) through the
/// source and clears the mark; nothing goes anywhere else.
#[gpui::test]
async fn edits_are_saved_with_the_save_key(cx: &mut TestAppContext) {
    let (viewer, src, cx) = edit_setup(cx, readable("a1"), &[("src/ფაილი.rs", "fn a() {\r\n    1\r\n}\r\n")]);
    let code = open_for_typing(&viewer, "src/ფაილი.rs", 1, cx);
    assert!(code.read_with(cx, |c, _| c.is_editing()));
    cx.simulate_input("2");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("ბ");
    redraw(cx);
    viewer.read_with(cx, |v, cx| assert_eq!(v.unsaved(cx), ["src/ფაილი.rs"]));
    assert!(cx.debug_bounds("ex-tab-dirty-0").is_some(), "the tab shows the dot");
    cx.simulate_keystrokes(SAVE);
    redraw(cx);
    assert_eq!(on_disk(&src, "src/ფაილი.rs"), "fn a() {\r\n    12\r\n    ბ\r\n}\r\n");
    assert_eq!(src.writes.load(Ordering::SeqCst), 1);
    viewer.read_with(cx, |v, cx| assert!(v.unsaved(cx).is_empty()));
    assert!(cx.debug_bounds("ex-tab-dirty-0").is_none());
    // Saved: the undo history stays (the view wasn't read again).
    assert!(code.read_with(cx, |c, _| c.is_editing() && c.text().unwrap().contains('ბ')));
    let undo = if cfg!(target_os = "macos") { "cmd-z" } else { "ctrl-z" };
    cx.simulate_keystrokes(undo);
    redraw(cx);
    viewer.read_with(cx, |v, cx| assert_eq!(v.unsaved(cx).len(), 1, "undone past the save: unsaved again"));
}

/// Another machine's agent, a file outside the folder, a file that isn't
/// UTF-8, an image, and the setting off: read-only, the header says why.
#[gpui::test]
async fn files_that_cannot_be_edited_say_why(cx: &mut TestAppContext) {
    let mut far = agent("vm", "/srv/x", "idle", 0, false);
    far.caps = AgentCaps { explorer: true, review: true, ..AgentCaps::default() };
    let (viewer, src, cx) = edit_setup(cx, far, &[("a.txt", "x\n")]);
    let code = open_for_typing(&viewer, "a.txt", 0, cx);
    assert!(!code.read_with(cx, |c, _| c.is_editing()));
    let why = cx.debug_bounds("ex-read-only");
    assert!(why.is_some(), "the header says it is read-only");
    cx.simulate_keystrokes("x");
    assert_eq!(on_disk(&src, "a.txt"), "x\n");
    assert!(code.read_with(cx, |c, _| c.text().is_none()));
}

#[gpui::test]
async fn the_setting_turns_editing_off(cx: &mut TestAppContext) {
    let (viewer, _src, cx) = edit_setup(cx, readable("a1"), &[("a.txt", "x\n")]);
    let code = open_for_typing(&viewer, "a.txt", 0, cx);
    assert!(code.read_with(cx, |c, _| c.is_editing()));
    assert!(cx.debug_bounds("ex-read-only").is_none());
    let def = pitwall_proto::settings::find(super::show::EDIT_SETTING).unwrap();
    cx.update(|_, cx| crate::ui_state::update(cx, |s| *s = crate::settings::data::with(s, def, &serde_json::Value::Bool(false))));
    redraw(cx);
    assert!(!cx.update(|_, cx| super::show::edit_enabled(cx)));
    assert!(!code.read_with(cx, |c, _| c.is_editing()));
    assert!(cx.debug_bounds("ex-read-only").is_some());
    cx.update(|_, cx| crate::ui_state::update(cx, |s| *s = crate::settings::data::with(s, def, &serde_json::Value::Bool(true))));
    redraw(cx);
    assert!(code.read_with(cx, |c, _| c.is_editing()));
}

/// Closing a tab with unsaved changes asks: Cancel keeps it, Don't Save
/// drops the edits, Save writes them first.
#[gpui::test]
async fn closing_a_tab_asks_about_unsaved_changes(cx: &mut TestAppContext) {
    let (viewer, src, cx) = edit_setup(cx, readable("a1"), &[("a.txt", "a\n"), ("b.txt", "b\n")]);
    open_for_typing(&viewer, "a.txt", 0, cx);
    cx.simulate_input("1");
    let close = |cx: &mut VisualTestContext, path: &str| {
        let p = path.to_string();
        cx.update(|window, cx| viewer.update(cx, |v, cx| v.request_close_tab(&p, window, cx)));
        cx.run_until_parked();
    };
    close(cx, "a.txt");
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    viewer.read_with(cx, |v, _| assert!(v.tabs().contains(&"a.txt".to_string())));

    close(cx, "a.txt");
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    assert_eq!(on_disk(&src, "a.txt"), "a1\n");
    viewer.read_with(cx, |v, _| assert!(!v.tabs().contains(&"a.txt".to_string())));

    open_for_typing(&viewer, "b.txt", 0, cx);
    cx.simulate_input("2");
    close(cx, "b.txt");
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert_eq!(on_disk(&src, "b.txt"), "b\n");
    viewer.read_with(cx, |v, _| assert!(v.tabs().is_empty()));
    // A clean tab closes without asking.
    open_for_typing(&viewer, "b.txt", 0, cx);
    close(cx, "b.txt");
    assert!(!cx.has_pending_prompt());
    viewer.read_with(cx, |v, _| assert!(v.tabs().is_empty()));
}

/// A files tile's Close only goes once unsaved changes are answered for;
/// so does closing the window. Moving the files ("Open in window", the
/// places menu, "Dock right of the terminal") asks nothing: the same viewer
/// moves, its edits with it.
#[gpui::test]
async fn closing_asks_but_moving_keeps_the_edits(cx: &mut TestAppContext) {
    let (viewer, src, cx) = edit_setup(cx, readable("a1"), &[("a.txt", "a\n")]);
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&viewer, move |_, e: &viewer::ViewerEvent, _| {
            let name = match e {
                viewer::ViewerEvent::Close => "close".to_string(),
                viewer::ViewerEvent::Undock => "undock".to_string(),
                viewer::ViewerEvent::Place(p) => format!("{p:?}"),
                _ => return,
            };
            seen.lock().unwrap().push(name);
        })
        .detach()
    });
    viewer.update(cx, |v, cx| v.set_host(viewer::Host::Tile, Some("a1".into()), cx));
    open_for_typing(&viewer, "a.txt", 0, cx);
    cx.simulate_input("!");
    redraw(cx);
    // Close, cancelled: nothing happens.
    let at = cx.debug_bounds("ex-close-tile").expect("close").center();
    cx.simulate_click(at, gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(events.lock().unwrap().is_empty(), "cancelled: nothing happens");
    // "Open in window": at once, unsaved still.
    let at = cx.debug_bounds("ex-undock").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(*events.lock().unwrap(), ["undock"]);
    assert!(viewer.read_with(cx, |v, cx| v.unsaved(cx).len()) == 1);
    // "Move…": the places, one picked.
    let at = cx.debug_bounds("ex-move").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    redraw(cx);
    assert!(viewer.read_with(cx, |v, _| v.place_menu_open()));
    let at = cx.debug_bounds("files-place-2").expect("above").center();
    cx.simulate_click(at, gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(!viewer.read_with(cx, |v, _| v.place_menu_open()));
    assert_eq!(events.lock().unwrap().last().map(String::as_str), Some("Next(Top)"));

    // In a window of its own: "Dock right of the terminal", at once.
    viewer.update(cx, |v, cx| {
        v.set_host(viewer::Host::Window, Some("a1".into()), cx);
        v.set_can_dock(true, cx);
    });
    redraw(cx);
    let at = cx.debug_bounds("ex-dock").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(events.lock().unwrap().last().map(String::as_str), Some("Next(Right)"));
    assert_eq!(on_disk(&src, "a.txt"), "a\n", "not saved: still unsaved");

    let v = viewer.downgrade();
    cx.update(|window, cx| viewer::guard_window_close(window, cx, move |_| v.upgrade()));
    // Closing the window asks.
    assert!(!cx.simulate_close(), "kept open while it asks");
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(on_disk(&src, "a.txt"), "a\n");
}

/// A window that draws a files tile's viewer (a Pitwall window's layout,
/// for these tests).
struct Holder(Option<gpui::AnyView>);

impl gpui::Render for Holder {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        use gpui::{div, ParentElement, Styled};
        div().size_full().flex().children(self.0.clone())
    }
}

/// One viewer per agent moves between a files tile and windows of its
/// own, as the same viewer: tabs and unsaved edits go with it, nothing
/// asks, nothing is written. What the tile has open is kept in `ui.json`
/// and opened again when the tile is drawn after a restart.
#[gpui::test]
async fn files_move_between_a_tile_and_windows_with_their_edits(cx: &mut TestAppContext) {
    let src = FakeSource::new(&[("a.txt", "a\n"), ("b.txt", "b\n")]);
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::code_view::register(cx);
        cx.bind_keys(viewer::bindings());
    });
    let store = cx.new(|_| AgentStore::new(vec![readable("a1")]));
    let source: Arc<dyn Source> = src.clone();
    cx.update(|cx| super::dock::remember_env(Some(source.clone()), &store, cx));

    // A tile in one Pitwall window.
    let (holder, tile_cx) = cx.add_window_view(|_, _| Holder(None));
    let viewer = tile_cx.update(|window, cx| super::dock::make("a1", "a1", window, cx)).expect("readable");
    tile_cx.update(|_, cx| super::dock::adopt("a1", "a1", viewer.clone(), cx));
    holder.update(tile_cx, |h, cx| {
        h.0 = Some(viewer.clone().into());
        cx.notify();
    });
    viewer.read_with(tile_cx, |v, _| assert_eq!(v.host(), viewer::Host::Tile));
    open_for_typing(&viewer, "a.txt", 0, tile_cx);
    tile_cx.simulate_input("!");
    redraw(tile_cx);
    assert_eq!(viewer.read_with(tile_cx, |v, cx| v.unsaved(cx).len()), 1);
    // What it has open is kept.
    let kept = tile_cx.update(|_, cx| crate::ui_state::get(cx).docked_files.get("a1").cloned()).expect("kept");
    assert_eq!((kept.key.as_str(), kept.tabs.clone(), kept.active.as_deref()), ("a1", vec!["a.txt".to_string()], Some("a.txt")));

    // Out to a window of its own: the same viewer, still unsaved.
    let (key, moved) = tile_cx.update(|_, cx| super::dock::release("a1", cx)).expect("a tile");
    assert_eq!((key.as_str(), &moved), ("a1", &viewer));
    holder.update(tile_cx, |h, cx| {
        h.0 = None;
        cx.notify();
    });
    let (s2, st2) = (source.clone(), store.clone());
    let v2 = viewer.clone();
    let win = tile_cx.update(|_, cx| {
        cx.open_window(gpui::WindowOptions::default(), move |window, cx| {
            cx.new(|cx| super::window::ViewerWindow::new("a1".into(), "a1".into(), Some(v2), s2, st2, window, cx))
        })
        .unwrap()
    });
    let win_cx = &mut VisualTestContext::from_window(*win, tile_cx);
    redraw(win_cx);
    viewer.read_with(win_cx, |v, cx| {
        assert_eq!(v.host(), viewer::Host::Window);
        assert_eq!(v.tabs(), ["a.txt"]);
        assert_eq!(v.unsaved(cx).len(), 1);
    });
    assert!(!win_cx.has_pending_prompt(), "a move asks nothing");
    // The edit goes on in its new window.
    open_for_typing(&viewer, "a.txt", 0, win_cx);
    win_cx.simulate_input("?");
    redraw(win_cx);

    // Back into a tile (a drop, the menu): the window's viewer moves.
    let back = win.update(win_cx, |w, _, cx| w.take_viewer(cx)).unwrap();
    assert_eq!(back, viewer);
    win_cx.update(|_, cx| super::dock::adopt("a1", "a1", back.clone(), cx));
    // The window closes without asking: its viewer went.
    assert!(win_cx.simulate_close(), "closed at once");
    win_cx.run_until_parked();
    assert!(!win_cx.has_pending_prompt());
    holder.update(tile_cx, |h, cx| {
        h.0 = Some(viewer.clone().into());
        cx.notify();
    });
    redraw(tile_cx);
    viewer.read_with(tile_cx, |v, cx| {
        assert_eq!(v.host(), viewer::Host::Tile);
        assert_eq!(v.unsaved(cx).len(), 1);
        let state = v.debug_state(cx);
        assert!(state.contains(r#"text=Some("a!?\n")"#), "{state}");
    });
    assert_eq!(on_disk(&src, "a.txt"), "a\n", "nothing written");

    // After a restart: the tile is drawn again from what was kept.
    viewer.update(tile_cx, |v, cx| v.open("b.txt".into(), None, cx));
    tile_cx.run_until_parked();
    tile_cx.update(|_, cx| {
        super::dock::release("a1", cx);
    });
    let again = tile_cx.update(|window, cx| super::dock::view_for("a1", window, cx)).expect("made again");
    let again = again.downcast::<viewer::Viewer>().unwrap();
    assert_ne!(again, viewer);
    again.read_with(tile_cx, |v, _| {
        assert_eq!(v.tabs(), ["a.txt", "b.txt"]);
        assert_eq!(v.active(), Some("b.txt"));
    });
}

/// An agent removed from Pitwall: its files close (the tile leaves the
/// layout, `dockedFiles` forgets it), unless they hold unsaved edits; those
/// stay, reading its folder on this computer, until saved or discarded.
/// Another machine's agent leaves nothing to save here: they close. A
/// restart (the same id) changes nothing.
#[gpui::test]
async fn a_removed_agents_files_close_or_stay_for_unsaved_edits(cx: &mut TestAppContext) {
    use crate::main_screen::workspace::FilesSpot;
    let src = FakeSource::new(&[("a.txt", "a\n")]);
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::code_view::register(cx);
        cx.bind_keys(viewer::bindings());
    });
    let mut far = agent("far", "/far/away", "idle", 0, false);
    far.caps = readable("far").caps;
    let ids = ["clean", "edited", "far", "saved"];
    let mut agents: Vec<_> = ids[..2].iter().map(|id| readable(id)).collect();
    agents.push(far);
    agents.push(readable("saved"));
    let store = cx.new(|_| AgentStore::new(agents));
    let source: Arc<dyn Source> = src.clone();
    cx.update(|cx| super::dock::remember_env(Some(source.clone()), &store, cx));
    let (holder, tcx) = cx.add_window_view(|_, _| Holder(None));
    let mut viewers = HashMap::new();
    for id in ids {
        let v = tcx.update(|window, cx| super::dock::make(id, id, window, cx)).expect("readable");
        tcx.update(|_, cx| {
            super::dock::adopt(id, id, v.clone(), cx);
            crate::ui_state::update(cx, |ui| {
                let space = ui.spaces[0].id.clone();
                *ui = ui.clone().dock_files(&space, id, &FilesSpot::Auto, 0.45, None).0;
            });
        });
        viewers.insert(id, v);
    }
    // Unsaved edits in all but "clean" (each drawn in turn to type in);
    // another machine's files are read-only here: none in "far".
    for id in ["edited", "far", "saved"] {
        let v = viewers[id].clone();
        holder.update(tcx, |h, cx| {
            h.0 = Some(v.clone().into());
            cx.notify();
        });
        open_for_typing(&v, "a.txt", 0, tcx);
        tcx.simulate_input("!");
        redraw(tcx);
        assert_eq!(v.read_with(tcx, |v, cx| v.unsaved(cx).len()), usize::from(id != "far"), "{id}");
    }
    let tile = |id: &str, cx: &mut VisualTestContext| cx.update(|_, cx| crate::ui_state::get(cx).files_tile(id).is_some());
    let docked = |id: &str, cx: &mut VisualTestContext| cx.update(|_, cx| crate::ui_state::get(cx).docked_files.contains_key(id));
    tcx.run_until_parked();

    // A restart or resume: the same ids, nothing happens.
    let all: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
    let kept = tcx.update(|_, cx| super::dock::agents_gone(&all, cx));
    assert!(kept.is_empty());
    for id in ids {
        assert!(tcx.update(|_, cx| super::dock::viewer(id, cx).is_some()) && tile(id, tcx) && docked(id, tcx), "{id}");
    }

    // All removed.
    let kept = tcx.update(|_, cx| super::dock::agents_gone(&[], cx));
    tcx.update(|_, cx| crate::ui_state::update(cx, |ui| *ui = ui.clone().prune_agents(&kept)));
    tcx.run_until_parked();
    let mut sorted = kept.clone();
    sorted.sort();
    assert_eq!(sorted, ["edited", "saved"], "kept for their edits; far's can't be saved here");
    for id in ["clean", "far"] {
        assert!(tcx.update(|_, cx| super::dock::viewer(id, cx).is_none()), "{id}");
        assert!(!tile(id, tcx) && !docked(id, tcx), "{id}: the tile and what it had open, gone");
    }
    for id in ["edited", "saved"] {
        let key = super::source::path_key("/work/alpha");
        assert!(tile(id, tcx), "{id} stays in the layout");
        assert_eq!(tcx.update(|_, cx| super::dock::key(id, cx)), Some(key.clone()), "read from its folder");
        assert_eq!(viewers[id].read_with(tcx, |v, _| v.agent_id().to_string()), key);
        assert_eq!(tcx.update(|_, cx| crate::ui_state::get(cx).docked_files[id].key.clone()), key);
        assert!(tcx.update(|_, cx| super::dock::orphaned(id, cx)));
        assert_eq!(viewers[id].read_with(tcx, |v, cx| v.unsaved(cx).len()), 1, "the edits are still there");
    }
    // Their note, with Save and Discard.
    let v = viewers["edited"].clone();
    let bar = tcx.update(|_, cx| super::dock::orphan_bar("edited", &v, crate::theme::theme(cx), cx).is_some());
    assert!(bar);
    assert!(tcx.update(|_, cx| super::dock::orphan_bar("clean", &v, crate::theme::theme(cx), cx).is_none()));

    // Discard: closed, nothing written.
    tcx.update(|_, cx| super::dock::discard_orphan("edited", cx));
    tcx.run_until_parked();
    assert!(!tile("edited", tcx) && !docked("edited", tcx));
    assert!(tcx.update(|_, cx| super::dock::viewer("edited", cx).is_none() && !super::dock::orphaned("edited", cx)));
    assert_eq!(on_disk(&src, "a.txt"), "a\n");
    // Save: written into its folder, then closed.
    tcx.update(|_, cx| super::dock::save_orphan("saved", cx));
    tcx.run_until_parked();
    assert_eq!(on_disk(&src, "a.txt"), "a!\n");
    assert!(!tile("saved", tcx) && !docked("saved", tcx));
    assert!(tcx.update(|_, cx| super::dock::viewer("saved", cx).is_none() && !super::dock::orphaned("saved", cx)));
}

/// The same for a window of its own: clean, it closes; with unsaved edits
/// it stays, reading the agent's folder, until they are discarded.
#[gpui::test]
async fn a_removed_agents_window_closes_or_stays_for_unsaved_edits(cx: &mut TestAppContext) {
    let src = FakeSource::new(&[("a.txt", "a\n")]);
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::code_view::register(cx);
        cx.bind_keys(viewer::bindings());
    });
    let store = cx.new(|_| AgentStore::new(vec![readable("w1"), readable("w2")]));
    let source: Arc<dyn Source> = src.clone();
    cx.update(|cx| super::dock::remember_env(Some(source.clone()), &store, cx));
    let mut wins = HashMap::new();
    for id in ["w1", "w2"] {
        let (s2, st2) = (source.clone(), store.clone());
        let win = cx.update(|cx| {
            let h = cx
                .open_window(gpui::WindowOptions::default(), move |window, cx| {
                    cx.new(|cx| super::window::ViewerWindow::new(id.into(), id.into(), None, s2, st2, window, cx))
                })
                .unwrap();
            crate::windows::add_viewer_window(id, h, cx);
            h
        });
        wins.insert(id, win);
    }
    let w2 = wins["w2"];
    let wcx = &mut VisualTestContext::from_window(*w2, cx);
    redraw(wcx);
    let v2 = w2.update(wcx, |w, _, _| w.viewer().clone()).unwrap();
    open_for_typing(&v2, "a.txt", 0, wcx);
    wcx.simulate_input("!");
    redraw(wcx);
    assert_eq!(v2.read_with(wcx, |v, cx| v.unsaved(cx).len()), 1);
    assert_eq!(wcx.update(|_, cx| crate::windows::viewer_window_owners(cx)).len(), 2);
    // (As a Pitwall window's screen does: not from inside the viewer's window.)
    let _ = wcx.cx.update(|cx| super::dock::agents_gone(&["w1".into(), "w2".into()], cx));
    let kept = wcx.cx.update(|cx| super::dock::agents_gone(&[], cx));
    wcx.run_until_parked();
    assert_eq!(kept, ["w2"]);
    assert!(wcx.update(|_, cx| crate::windows::viewer_window("w1", cx).is_none()), "the clean one closed");
    assert!(wcx.update(|_, cx| crate::windows::viewer_window("w2", cx).is_some()));
    let key = super::source::path_key("/work/alpha");
    assert_eq!(w2.update(wcx, |w, _, _| w.key().to_string()).unwrap(), key);
    redraw(wcx);
    assert!(wcx.debug_bounds("files-orphan").is_some(), "the note shows in the window");
    wcx.cx.update(|cx| super::dock::discard_orphan("w2", cx));
    wcx.cx.run_until_parked();
    assert!(wcx.cx.update(|cx| crate::windows::viewer_window("w2", cx).is_none()), "discarded: closed");
    assert_eq!(on_disk(&src, "a.txt"), "a\n");
}

/// A files window has Go to file (its empty state, ⌘P) and Search in
/// files (⌘⇧F) on the folder it shows, as a Pitwall window does.
#[gpui::test]
async fn a_files_window_goes_to_a_file_and_searches(cx: &mut TestAppContext) {
    let src = FakeSource::new(&[("a.txt", "a\n"), ("sub/b.txt", "b\n")]);
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::code_view::register(cx);
        cx.bind_keys(viewer::bindings());
        cx.bind_keys(super::bindings());
        cx.bind_keys(super::quick_open::bindings());
    });
    let store = cx.new(|_| AgentStore::new(vec![readable("w1")]));
    let source: Arc<dyn Source> = src.clone();
    let win = cx.update(|cx| {
        cx.open_window(gpui::WindowOptions::default(), move |window, cx| {
            cx.new(|cx| super::window::ViewerWindow::new("w1".into(), "w1".into(), None, source, store, window, cx))
        })
        .unwrap()
    });
    let wcx = &mut VisualTestContext::from_window(*win, cx);
    redraw(wcx);
    let v = win.update(wcx, |w, _, _| w.viewer().clone()).unwrap();
    // No file open: Go to file is offered, and opens the picker.
    let at = wcx.debug_bounds("ex-empty-goto").expect("offered in a window too").center();
    wcx.simulate_click(at, gpui::Modifiers::none());
    wcx.run_until_parked();
    assert!(win.update(wcx, |w, _, _| w.quick_open_shown()).unwrap());
    wcx.simulate_input("b.txt");
    wcx.run_until_parked();
    wcx.simulate_keystrokes("enter");
    wcx.run_until_parked();
    assert!(!win.update(wcx, |w, _, _| w.quick_open_shown()).unwrap(), "closed once picked");
    assert_eq!(v.read_with(wcx, |v, _| v.opened().active), Some("sub/b.txt".into()), "opened in its viewer");
    // ⌘P again, then Escape: closed, nothing else opened.
    redraw(wcx);
    wcx.dispatch_action(super::GoToFile);
    wcx.run_until_parked();
    assert!(win.update(wcx, |w, _, _| w.quick_open_shown()).unwrap());
    wcx.simulate_keystrokes("escape");
    wcx.run_until_parked();
    assert!(!win.update(wcx, |w, _, _| w.quick_open_shown()).unwrap());
    assert_eq!(v.read_with(wcx, |v, _| v.opened().tabs.len()), 1);
    // ⌘⇧F: the viewer's Search.
    wcx.dispatch_action(super::SearchInFiles);
    wcx.run_until_parked();
    assert!(!v.read_with(wcx, |v, _| v.opened().pane_files), "on Search");
}

/// Quitting with unsaved changes anywhere asks first; Save writes them
/// and then quits, Cancel doesn't quit.
#[gpui::test]
async fn quitting_asks_about_unsaved_changes(cx: &mut TestAppContext) {
    let (viewer, src, cx) = edit_setup(cx, readable("a1"), &[("a.txt", "a\n")]);
    open_for_typing(&viewer, "a.txt", 0, cx);
    cx.simulate_input("q");
    let quit = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let try_quit = |cx: &mut VisualTestContext| {
        let q = quit.clone();
        cx.update(|_, cx| super::unsaved::quit_guarded(cx, move |_| {
            q.fetch_add(1, Ordering::SeqCst);
        }));
        cx.run_until_parked();
    };
    try_quit(cx);
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(quit.load(Ordering::SeqCst), 0);
    try_quit(cx);
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    assert_eq!(quit.load(Ordering::SeqCst), 1);
    assert_eq!(on_disk(&src, "a.txt"), "aq\n");
    // Nothing unsaved: quits at once.
    try_quit(cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(quit.load(Ordering::SeqCst), 2);
}

/// The agent changes the file: with nothing unsaved it is read again
/// quietly (caret kept); with unsaved changes a bar offers Keep mine, Take
/// theirs and Show difference, and a save over it asks first.
#[gpui::test]
async fn changes_on_disk_never_overwrite_either_side(cx: &mut TestAppContext) {
    let (viewer, src, cx) = edit_setup(cx, readable("a1"), &[("a.txt", "one\ntwo\nthree\n")]);
    let code = open_for_typing(&viewer, "a.txt", 2, cx);
    let agent_writes = |text: &str| {
        src.files.lock().unwrap().insert("a.txt".into(), text.into());
    };
    // Nothing unsaved: quietly, caret where it was.
    agent_writes("one\ntwo\nthree!\n");
    viewer.update(cx, |v, cx| v.recheck(cx));
    redraw(cx);
    assert!(viewer.read_with(cx, |v, _| !v.is_stale()));
    assert_eq!(code.read_with(cx, |c, _| c.text()).as_deref(), Some("one\ntwo\nthree!\n"));
    assert_eq!(code.read_with(cx, |c, _| c.selection().map(|s| s.head.line)), Some(2));

    // Unsaved changes: the bar; nothing replaced.
    cx.simulate_input("X");
    agent_writes("one\nTWO\nthree!\n");
    viewer.update(cx, |v, cx| v.recheck(cx));
    redraw(cx);
    assert!(viewer.read_with(cx, |v, _| v.is_stale()));
    assert!(cx.debug_bounds("ex-changed-on-disk").is_some());
    assert_eq!(code.read_with(cx, |c, _| c.text()).as_deref(), Some("one\ntwo\nthreeX!\n"));
    // Show difference: disk against mine.
    let at = cx.debug_bounds("ex-show-difference").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    redraw(cx);
    assert!(cx.debug_bounds("ex-difference").is_some());
    let at = cx.debug_bounds("ex-show-difference").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    redraw(cx);
    assert!(cx.debug_bounds("ex-difference").is_none());
    // Keep mine: the bar goes; the next save writes over what was seen.
    let at = cx.debug_bounds("ex-keep-mine").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    redraw(cx);
    assert!(!viewer.read_with(cx, |v, _| v.is_stale()));
    assert_eq!(on_disk(&src, "a.txt"), "one\nTWO\nthree!\n", "keeping isn't saving");
    cx.update(|window, cx| window.focus(&gpui::Focusable::focus_handle(code.read(cx), cx), cx));
    cx.simulate_keystrokes(SAVE);
    redraw(cx);
    assert_eq!(on_disk(&src, "a.txt"), "one\ntwo\nthreeX!\n");

    // A save over a file changed since: nothing written, the bar asks;
    // Take theirs replaces the edits.
    cx.simulate_input("Y");
    agent_writes("theirs\n");
    cx.simulate_keystrokes(SAVE);
    redraw(cx);
    assert_eq!(on_disk(&src, "a.txt"), "theirs\n");
    assert!(viewer.read_with(cx, |v, _| v.is_stale()));
    assert!(cx.debug_bounds("ex-changed-on-disk").is_some());
    let at = cx.debug_bounds("ex-reload").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    redraw(cx);
    assert_eq!(code.read_with(cx, |c, _| c.text()).as_deref(), Some("theirs\n"));
    viewer.read_with(cx, |v, cx| assert!(v.unsaved(cx).is_empty()));
    assert!(cx.debug_bounds("ex-changed-on-disk").is_none());

    // Again, and "Save mine" this time: written over theirs, as asked.
    cx.simulate_input("Z");
    agent_writes("theirs 2\n");
    cx.simulate_keystrokes(SAVE);
    redraw(cx);
    let at = cx.debug_bounds("ex-keep-mine").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    redraw(cx);
    assert_eq!(on_disk(&src, "a.txt"), code.read_with(cx, |c, _| c.text()).unwrap());
    viewer.read_with(cx, |v, cx| assert!(v.unsaved(cx).is_empty()));
}

/// A Markdown file's Preview follows the edits.
#[gpui::test]
async fn the_markdown_preview_follows_the_edits(cx: &mut TestAppContext) {
    let (viewer, _src, cx) = edit_setup(cx, readable("a1"), &[("README.md", "# Title\n")]);
    open_for_typing(&viewer, "README.md", 0, cx);
    cx.simulate_input(" two");
    redraw(cx);
    let md = viewer.read_with(cx, |v, _| v.md_text().map(|(_, t)| t.to_string()));
    assert_eq!(md.as_deref(), Some("# Title two\n"));
}

const TOGGLE_TREE: &str = if cfg!(target_os = "macos") { "cmd-shift-e" } else { "ctrl-shift-alt-e" };

fn kept_tree(cx: &mut VisualTestContext) -> super::column::FilesTree {
    cx.update(|_, cx| crate::ui_state::get(cx).files_tree)
}

fn tree_shown(viewer: &Entity<viewer::Viewer>, cx: &mut VisualTestContext) -> bool {
    viewer.read_with(cx, |v, cx| v.tree_shown(cx))
}

/// The tree column hides and shows with its button and ⌘⇧E, and each
/// kind of place keeps its own choice.
#[gpui::test]
async fn the_file_tree_hides_and_each_place_keeps_its_choice(cx: &mut TestAppContext) {
    let (viewer, _src, cx) = edit_setup(cx, readable("a1"), &[("a.txt", "a\n")]);
    cx.simulate_resize(gpui::size(gpui::px(1200.), gpui::px(800.)));
    viewer.update(cx, |v, cx| {
        v.set_host(viewer::Host::Window, Some("a1".into()), cx);
        v.open("a.txt".into(), None, cx);
    });
    redraw(cx);
    assert!(tree_shown(&viewer, cx), "shown at first");
    assert!(cx.debug_bounds("ex-side").is_some());

    // The bar's button.
    let at = cx.debug_bounds("ex-tree-toggle").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    redraw(cx);
    assert!(!tree_shown(&viewer, cx));
    assert_eq!(kept_tree(cx).window.open, Some(false));
    assert_eq!(kept_tree(cx).tile.open, None, "only for windows");

    // The shortcut, with the keyboard in the viewer.
    cx.update(|window, cx| window.focus(&gpui::Focusable::focus_handle(viewer.read(cx), cx), cx));
    cx.simulate_keystrokes(TOGGLE_TREE);
    redraw(cx);
    assert!(tree_shown(&viewer, cx));
    assert_eq!(kept_tree(cx).window.open, Some(true));
    cx.simulate_keystrokes(TOGGLE_TREE);
    redraw(cx);
    assert!(!tree_shown(&viewer, cx));

    // In a files tile it was never hidden: shown there (a wide tile).
    viewer.update(cx, |v, cx| v.set_host(viewer::Host::Tile, Some("a1".into()), cx));
    redraw(cx);
    assert!(tree_shown(&viewer, cx));
    // In a window again: hidden, as kept.
    viewer.update(cx, |v, cx| v.set_host(viewer::Host::Window, Some("a1".into()), cx));
    redraw(cx);
    assert!(!tree_shown(&viewer, cx));
}

/// Hidden, the file has the viewer's whole width, and Go to file (⌘P) and
/// Search in files still work.
#[gpui::test]
async fn with_the_tree_hidden_the_file_has_the_width(cx: &mut TestAppContext) {
    let (viewer, _src, cx) = edit_setup(cx, readable("a1"), &[("a.txt", "a\n")]);
    cx.simulate_resize(gpui::size(gpui::px(1200.), gpui::px(800.)));
    let quick = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = quick.clone();
    cx.update(|_, cx| {
        cx.subscribe(&viewer, move |_, e: &viewer::ViewerEvent, _| {
            if matches!(e, viewer::ViewerEvent::QuickOpen) {
                seen.store(true, Ordering::SeqCst);
            }
        })
        .detach()
    });
    redraw(cx);
    let at = cx.debug_bounds("ex-tree-toggle").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    // (The column slides out, in real time.)
    for _ in 0..4 {
        std::thread::sleep(std::time::Duration::from_millis(60));
        redraw(cx);
    }
    assert!(cx.debug_bounds("ex-side").is_none(), "gone once it has slid out");
    // No file open: the empty view offers the tree back, and Go to file.
    assert!(cx.debug_bounds("ex-empty-tree").is_some());
    let at = cx.debug_bounds("ex-viewer-goto").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(quick.load(Ordering::SeqCst), "Go to file still asks for quick open");
    // Search in files brings the column back, on Search.
    let at = cx.debug_bounds("ex-empty-search").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    redraw(cx);
    assert!(tree_shown(&viewer, cx));
    assert_eq!(kept_tree(cx).main.open, Some(false), "for now: what is kept doesn't change");
}

/// A pane too narrow for the tree and a file starts with the tree hidden;
/// the user can still show it, and it stays.
#[gpui::test]
async fn a_narrow_pane_starts_with_the_tree_hidden(cx: &mut TestAppContext) {
    let (viewer, _src, cx) = edit_setup(cx, readable("a1"), &[("a.txt", "a\n")]);
    viewer.update(cx, |v, cx| v.set_host(viewer::Host::Tile, Some("a1".into()), cx));
    cx.simulate_resize(gpui::size(gpui::px(480.), gpui::px(700.)));
    redraw(cx);
    assert!(!tree_shown(&viewer, cx), "narrow: hidden");
    let at = cx.debug_bounds("ex-tree-toggle").unwrap().center();
    cx.simulate_click(at, gpui::Modifiers::none());
    redraw(cx);
    assert!(tree_shown(&viewer, cx), "shown when asked");
    cx.simulate_resize(gpui::size(gpui::px(460.), gpui::px(700.)));
    redraw(cx);
    assert!(tree_shown(&viewer, cx), "and it stays");
    // Wider: the kept choice (shown).
    cx.simulate_resize(gpui::size(gpui::px(1200.), gpui::px(700.)));
    redraw(cx);
    assert!(tree_shown(&viewer, cx));
}

/// The column's edge drags its width, kept for the kind of place, within
/// limits that leave the file its room; a double click resets it.
#[gpui::test]
async fn the_tree_column_width_is_dragged_and_kept(cx: &mut TestAppContext) {
    let (viewer, _src, cx) = edit_setup(cx, readable("a1"), &[("a.txt", "a\n")]);
    cx.simulate_resize(gpui::size(gpui::px(1000.), gpui::px(700.)));
    viewer.update(cx, |v, cx| v.set_host(viewer::Host::Window, Some("a1".into()), cx));
    redraw(cx);
    let w = |viewer: &Entity<viewer::Viewer>, cx: &mut VisualTestContext| viewer.read_with(cx, |v, cx| v.tree_width(cx));
    assert_eq!(w(&viewer, cx), super::column::default_width(viewer::Host::Window));
    // Dragged with the mouse from its edge.
    let edge = cx.debug_bounds("ex-side-edge").unwrap().center();
    cx.simulate_mouse_down(edge, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(gpui::point(edge.x + gpui::px(20.), edge.y), Some(gpui::MouseButton::Left), gpui::Modifiers::none());
    cx.simulate_mouse_move(gpui::point(edge.x + gpui::px(60.), edge.y), Some(gpui::MouseButton::Left), gpui::Modifiers::none());
    cx.simulate_mouse_up(gpui::point(edge.x + gpui::px(60.), edge.y), gpui::MouseButton::Left, gpui::Modifiers::none());
    redraw(cx);
    let dragged = w(&viewer, cx);
    assert!((dragged - 292.).abs() < 4., "{dragged}");
    assert_eq!(kept_tree(cx).window.width.map(|w| w.round()), Some(dragged.round()));
    // Too wide: no wider than the limit…
    viewer.update(cx, |v, cx| v.debug_tree_width(900., cx));
    redraw(cx);
    assert_eq!(w(&viewer, cx), super::column::MAX_W);
    // …and in a smaller window, the file keeps 380.
    cx.simulate_resize(gpui::size(gpui::px(700.), gpui::px(700.)));
    redraw(cx);
    assert_eq!(w(&viewer, cx), 700. - super::column::MIN_FILE_W);
    viewer.update(cx, |v, cx| v.debug_tree_width(20., cx));
    assert_eq!(w(&viewer, cx), super::column::MIN_W);
    // A double click on the edge: the default again.
    viewer.update(cx, |v, cx| v.reset_tree_width(cx));
    redraw(cx);
    assert_eq!(w(&viewer, cx), super::column::default_width(viewer::Host::Window));
    assert_eq!(kept_tree(cx).window.width, None);
}

/// The bar names the folder shortened from the middle, with the whole
/// path on hover.
#[gpui::test]
async fn the_bar_names_the_folder_briefly(cx: &mut TestAppContext) {
    let mut a = readable("a1");
    a.cwd = "/Users/someone/Desktop/Projects/pitwall/.claude/worktrees/agent-a0829081/target/pw-demo".into();
    a.cwd_display = "~/Desktop/Projects/pitwall/.claude/worktrees/agent-a0829081/target/pw-demo".into();
    a.branch = Some("feat/viewer".into());
    let (viewer, _src, cx) = edit_setup(cx, a, &[("a.txt", "a\n")]);
    cx.simulate_resize(gpui::size(gpui::px(1200.), gpui::px(700.)));
    viewer.update(cx, |v, cx| v.set_host(viewer::Host::Window, Some("a1".into()), cx));
    redraw(cx);
    assert!(cx.debug_bounds("ex-viewer-hint").is_some());
    let shown = super::logic::short_path("~/Desktop/Projects/pitwall/.claude/worktrees/agent-a0829081/target/pw-demo", 48);
    assert_eq!(shown, "~/…/worktrees/agent-a0829081/target/pw-demo");
    assert!(cx.debug_bounds("ex-viewer-branch").is_some(), "the branch of an agent's folder");
}
