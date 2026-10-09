//! TerminalView in GPUI's test context: keystrokes and text go through the
//! real dispatch path (bindings → key listeners → input handler) and come
//! out of the terminal's stream as bytes; frames reshape only changed rows.

use std::sync::{Arc, Mutex};

use gpui::{ClipboardItem, Entity, Focusable, TestAppContext, VisualTestContext};
use pitwall_term_view::{
    default_key_bindings, Feed, TermSize, TermStream, Terminal, TerminalConfig, TerminalView, ViewMode, ViewSettings,
};

/// Records what the view sends to the program.
#[derive(Clone, Default)]
struct Recorder {
    input: Arc<Mutex<Vec<u8>>>,
    sizes: Arc<Mutex<Vec<TermSize>>>,
}

impl Recorder {
    fn take(&self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut *self.input.lock().unwrap())).into_owned()
    }
}

impl TermStream for Recorder {
    fn attach(&self, _feed: Feed) {}
    fn write(&self, bytes: &[u8]) {
        self.input.lock().unwrap().extend_from_slice(bytes);
    }
    fn resize(&self, size: TermSize) {
        self.sizes.lock().unwrap().push(size);
    }
}

fn open(
    cx: &mut TestAppContext,
    settings: ViewSettings,
) -> (Entity<TerminalView>, Terminal, Recorder, &mut VisualTestContext) {
    cx.update(|cx| cx.bind_keys(default_key_bindings()));
    let rec = Recorder::default();
    let terminal = Terminal::new(rec.clone(), TermSize::new(80, 24), TerminalConfig::default());
    let t = terminal.clone();
    let (view, cx) = cx.add_window_view(|window, cx| TerminalView::new(t, ViewMode::Interactive, settings, window, cx));
    cx.update(|window, cx| window.focus(&view.focus_handle(cx)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    rec.take(); // focus report etc.
    (view, terminal, rec, cx)
}

#[gpui::test]
fn typing_and_keys_reach_the_program(cx: &mut TestAppContext) {
    let (_view, _t, rec, cx) = open(cx, ViewSettings::default());
    cx.simulate_input("ls");
    assert_eq!(rec.take(), "ls");
    cx.simulate_keystrokes("enter");
    assert_eq!(rec.take(), "\r");
    cx.simulate_keystrokes("ctrl-c up tab shift-tab backspace escape");
    assert_eq!(rec.take(), "\x03\x1b[A\t\x1b[Z\x7f\x1b");
    // Option as Meta (Pitwall's default).
    cx.simulate_keystrokes("alt-b");
    assert_eq!(rec.take(), "\x1bb");
    // ⌘ keys are shortcuts, not input.
    cx.simulate_keystrokes("cmd-shift-x");
    assert_eq!(rec.take(), "");
}

#[gpui::test]
fn application_cursor_mode_changes_arrows(cx: &mut TestAppContext) {
    let (_view, t, rec, cx) = open(cx, ViewSettings::default());
    t.feed().push(b"\x1b[?1h");
    cx.run_until_parked();
    cx.simulate_keystrokes("up left");
    assert_eq!(rec.take(), "\x1bOA\x1bOD");
}

#[gpui::test]
fn paste_is_bracketed_when_asked(cx: &mut TestAppContext) {
    let (_view, t, rec, cx) = open(cx, ViewSettings::default());
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("a\nb".into())));
    cx.simulate_keystrokes("cmd-v");
    assert_eq!(rec.take(), "a\rb");
    t.feed().push(b"\x1b[?2004h");
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-v");
    assert_eq!(rec.take(), "\x1b[200~a\rb\x1b[201~");
}

#[gpui::test]
fn copy_takes_the_selection(cx: &mut TestAppContext) {
    let (_view, t, _rec, cx) = open(cx, ViewSettings::default());
    t.feed().push(b"hello world");
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-a cmd-c");
    let text = cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text())).unwrap();
    assert!(text.starts_with("hello world"), "{text:?}");
}

#[gpui::test]
fn search_bar_takes_typing(cx: &mut TestAppContext) {
    let (_view, t, rec, cx) = open(cx, ViewSettings::default());
    t.feed().push(b"alpha\r\nbeta\r\n");
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-f");
    cx.simulate_input("beta");
    cx.simulate_keystrokes("enter escape");
    // Nothing typed into the find bar reaches the program.
    assert_eq!(rec.take(), "");
    cx.simulate_input("x");
    assert_eq!(rec.take(), "x");
}

#[gpui::test]
fn the_view_sizes_the_terminal(cx: &mut TestAppContext) {
    let (_view, t, rec, cx) = open(cx, ViewSettings::default());
    cx.run_until_parked();
    let size = t.size();
    // Whatever the test window's size, the terminal follows the view, and the
    // stream heard about it.
    assert!(size.cols > 1 && size.rows > 1, "{size:?}");
    assert_eq!(rec.sizes.lock().unwrap().last().map(|s| (s.cols, s.rows)), Some((size.cols, size.rows)));
}

#[gpui::test]
fn only_changed_rows_are_reshaped(cx: &mut TestAppContext) {
    let (view, t, _rec, cx) = open(cx, ViewSettings { cursor_blink: false, ..Default::default() });
    for i in 0..10 {
        t.feed().push(format!("line {i}\r\n").as_bytes());
    }
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let before = view.read_with(cx, |v, _| v.render_stats());
    assert!(before.frames > 0);
    // One more character on the cursor's line.
    t.feed().push(b"x");
    cx.run_until_parked();
    let after = view.read_with(cx, |v, _| v.render_stats());
    assert!(after.frames > before.frames);
    assert_eq!(after.rows_shaped - before.rows_shaped, 1, "{before:?} → {after:?}");
}

fn center(
    view: &Entity<TerminalView>,
    cx: &mut VisualTestContext,
    col: usize,
    row: usize,
) -> gpui::Point<gpui::Pixels> {
    view.read_with(cx, |v, _| v.cell_bounds(col, row).center())
}

#[gpui::test]
fn mouse_reports_in_sgr_mode(cx: &mut TestAppContext) {
    let (view, t, rec, cx) = open(cx, ViewSettings::default());
    t.feed().push(b"\x1b[?1000h\x1b[?1006h");
    cx.run_until_parked();
    let at = center(&view, cx, 2, 1);
    cx.simulate_click(at, gpui::Modifiers::none());
    assert_eq!(rec.take(), "\x1b[<0;3;2M\x1b[<0;3;2m");
    // Shift bypasses reporting (selection instead).
    cx.simulate_click(at, gpui::Modifiers::shift());
    assert_eq!(rec.take(), "");
}

#[gpui::test]
fn dragging_selects_and_copy_takes_it(cx: &mut TestAppContext) {
    let (view, t, _rec, cx) = open(cx, ViewSettings::default());
    t.feed().push(b"hello world");
    cx.run_until_parked();
    // From the left half of the first cell to the middle of the fifth.
    let first = view.read_with(cx, |v, _| v.cell_bounds(0, 0));
    let a = gpui::point(first.left() + gpui::px(1.0), first.center().y);
    let b = center(&view, cx, 4, 0);
    cx.simulate_mouse_down(a, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_move(b, Some(gpui::MouseButton::Left), gpui::Modifiers::none());
    cx.simulate_mouse_up(b, gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_keystrokes("cmd-c");
    let text = cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text())).unwrap();
    assert_eq!(text, "hello");
    // Hovering (which shows the scrollbar) must not resize the terminal.
    let cols = t.size().cols;
    let elsewhere = center(&view, cx, 50, 3);
    cx.simulate_mouse_move(elsewhere, None, gpui::Modifiers::none());
    assert_eq!(t.size().cols, cols);
}

#[gpui::test]
fn wheel_scrolls_back_and_typing_returns(cx: &mut TestAppContext) {
    let (view, t, rec, cx) = open(cx, ViewSettings::default());
    for i in 0..200 {
        t.feed().push(format!("{i}\r\n").as_bytes());
    }
    cx.run_until_parked();
    let at = center(&view, cx, 1, 1);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta: gpui::ScrollDelta::Lines(gpui::point(0.0, 3.0)),
        ..Default::default()
    });
    assert_eq!(t.lock().term().grid().display_offset(), 3);
    cx.simulate_input("q");
    assert_eq!(rec.take(), "q");
    assert_eq!(t.lock().term().grid().display_offset(), 0);
    // On the alternate screen the wheel sends arrows (pagers, editors).
    t.feed().push(b"\x1b[?1049h");
    cx.run_until_parked();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta: gpui::ScrollDelta::Lines(gpui::point(0.0, -2.0)),
        ..Default::default()
    });
    assert_eq!(rec.take(), "\x1b[B\x1b[B");
}

#[gpui::test]
fn focus_reports_when_asked(cx: &mut TestAppContext) {
    let (view, t, rec, cx) = open(cx, ViewSettings::default());
    t.feed().push(b"\x1b[?1004h");
    cx.run_until_parked();
    cx.update(|window, _| window.blur());
    cx.run_until_parked();
    assert_eq!(rec.take(), "\x1b[O");
    cx.update(|window, cx| window.focus(&view.focus_handle(cx)));
    cx.run_until_parked();
    assert_eq!(rec.take(), "\x1b[I");
}

#[gpui::test]
fn a_changed_run_is_the_only_one_shaped_again(cx: &mut TestAppContext) {
    let (view, t, _rec, cx) = open(cx, ViewSettings { cursor_blink: false, ..Default::default() });
    // A spinner line: the spinner in its own colour, then the same text.
    let frame = |spin: char| format!("\x1b[2K\r\x1b[33m{spin}\x1b[0m Thinking about the plan (esc to interrupt)");
    t.feed().push(frame('·').as_bytes());
    cx.run_until_parked();
    t.feed().push(frame('✢').as_bytes());
    cx.run_until_parked();
    let before = view.read_with(cx, |v, _| v.render_stats());
    t.feed().push(frame('✳').as_bytes());
    cx.run_until_parked();
    let after = view.read_with(cx, |v, _| v.render_stats());
    assert_eq!(after.rows_shaped - before.rows_shaped, 1, "{before:?} → {after:?}");
    assert_eq!(after.runs_shaped - before.runs_shaped, 1, "only the spinner: {before:?} → {after:?}");
}
