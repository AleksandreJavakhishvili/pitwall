//! The view as an entity in a window: loading, layouts, find, reveal,
//! keys and copying.

use gpui::{Entity, TestAppContext, VisualTestContext};

use super::*;
use crate::theme::Theme;

const OLD: &str = "fn a() {}\n\nfn b() {\n    1\n}\n";
const NEW: &str = "fn a() {}\n\nfn b() {\n    2\n}\n// გამარჯობა\n";

struct Host(Entity<CodeView>);

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.0.clone())
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<CodeView>, &mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(Theme::dark());
        register(cx);
    });
    let (host, cx) = cx.add_window_view(|_, cx| Host(cx.new(CodeView::new)));
    let view = host.read_with(cx, |h, _| h.0.clone());
    cx.update(|window, cx| window.focus(&view.focus_handle(cx)));
    (view, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    // A keystroke nothing binds makes the window draw.
    cx.simulate_keystrokes("f12");
}

#[gpui::test]
fn loads_a_diff_and_switches_layouts(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update(cx, |v, cx| {
        v.set_diff("src/x.rs", OLD.into(), NEW.into(), cx)
    });
    assert!(view.read_with(cx, |v, _| v.is_loading()));
    cx.run_until_parked();
    draw(cx);
    view.read_with(cx, |v, _| {
        assert!(!v.is_loading());
        assert_eq!(v.change_count(), 2);
        assert_eq!(v.path(), Some("src/x.rs"));
        assert_eq!(v.layout(), Layout::Split);
    });
    let split_rows = view.read_with(cx, |v, _| v.rows.len());
    view.update(cx, |v, cx| v.set_layout(Layout::Unified, cx));
    let inline_rows = view.read_with(cx, |v, _| v.rows.len());
    assert!(
        inline_rows > split_rows,
        "deleted and inserted rows stack inline"
    );
    // The same text again keeps the view as it is.
    view.update(cx, |v, cx| {
        v.sel = Some(Selection::caret(Side::New, Pos::new(3, 2)));
        v.set_diff("src/x.rs", OLD.into(), NEW.into(), cx);
        assert!(!v.is_loading());
        assert!(v.sel.is_some());
    });
}

#[gpui::test]
fn next_change_and_reveal(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update(cx, |v, cx| v.set_diff("x.rs", OLD.into(), NEW.into(), cx));
    cx.run_until_parked();
    draw(cx);
    cx.simulate_keystrokes("f7");
    let first = view.read_with(cx, |v, _| v.selection().unwrap().head.line);
    assert_eq!(first, 3, "the changed line");
    cx.simulate_keystrokes("f7");
    let second = view.read_with(cx, |v, _| v.selection().unwrap().head.line);
    assert_eq!(second, 5, "the added comment line");
    view.update(cx, |v, cx| v.reveal(Side::New, 0, Some(0..2), cx));
    let s = view.read_with(cx, |v, _| v.selection().unwrap());
    assert_eq!((s.anchor, s.head), (Pos::new(0, 0), Pos::new(0, 2)));
}

#[gpui::test]
fn find_selects_matches_and_wraps(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    let text = "alpha beta\nbeta gamma\nBETA\n".to_string();
    view.update(cx, |v, cx| v.set_file("notes.txt", text, cx));
    cx.run_until_parked();
    draw(cx);
    view.update_in(cx, |v, window, cx| v.open_find(window, cx));
    view.update(cx, |v, cx| {
        v.set_query(|q| q.text = "beta".into(), true, cx)
    });
    view.read_with(cx, |v, _| {
        let f = v.find.as_ref().unwrap();
        assert_eq!(f.found.matches.len(), 3, "case-insensitive by default");
        assert_eq!(f.current, Some(0));
    });
    cx.simulate_keystrokes("f3 f3");
    assert_eq!(
        view.read_with(cx, |v, _| v.find.as_ref().unwrap().current),
        Some(2)
    );
    cx.simulate_keystrokes("f3");
    assert_eq!(
        view.read_with(cx, |v, _| v.find.as_ref().unwrap().current),
        Some(0),
        "wraps"
    );
    view.update(cx, |v, cx| v.set_query(|q| q.case = true, true, cx));
    assert_eq!(
        view.read_with(cx, |v, _| v.find.as_ref().unwrap().found.matches.len()),
        2
    );
    view.update_in(cx, |v, window, cx| v.close_find(window, cx));
    assert!(view.read_with(cx, |v, _| v.find.is_none()));
}

#[gpui::test]
fn copies_the_selection_or_the_line(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update(cx, |v, cx| v.set_file("a.txt", "one\tx\nორი\n".into(), cx));
    cx.run_until_parked();
    draw(cx);
    view.update(cx, |v, _| {
        v.sel = Some(Selection::caret(Side::New, Pos::new(1, 3)))
    });
    let copy = if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-c"
    };
    cx.simulate_keystrokes(copy);
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("ორი\n")
    );
    let all = if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    };
    cx.simulate_keystrokes(all);
    cx.simulate_keystrokes(copy);
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("one\tx\nორი\n")
    );
    // Moving right over Georgian steps a whole letter (3 bytes).
    view.update(cx, |v, _| {
        v.sel = Some(Selection::caret(Side::New, Pos::new(1, 0)))
    });
    cx.simulate_keystrokes("right");
    assert_eq!(
        view.read_with(cx, |v, _| v.selection().unwrap().head),
        Pos::new(1, 3)
    );
    cx.simulate_keystrokes("shift-end");
    let s = view.read_with(cx, |v, _| v.selection().unwrap());
    assert_eq!(
        (s.anchor, s.head),
        (Pos::new(1, 3), Pos::new(1, "ორი".len()))
    );
}

#[gpui::test]
fn typing_says_read_only_and_comments_join(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update(cx, |v, cx| {
        v.set_commentable(true, cx);
        v.set_diff("x.rs", OLD.into(), NEW.into(), cx);
        v.set_comments([(3, "first".to_string()), (3, "second".to_string())], cx);
    });
    cx.run_until_parked();
    draw(cx);
    view.update(cx, |v, _| {
        v.sel = Some(Selection::caret(Side::New, Pos::new(0, 0)))
    });
    cx.simulate_keystrokes("x");
    assert!(view.read_with(cx, |v, _| v.readonly_at.is_some()));
    assert_eq!(
        view.read_with(cx, |v, _| v.readonly_text.clone()).as_ref(),
        "Read-only: click a line number to comment"
    );
    cx.simulate_keystrokes("escape");
    assert!(view.read_with(cx, |v, _| v.readonly_at.is_none()));
    assert_eq!(
        view.read_with(cx, |v, _| v.comments[&3].clone()),
        "first\n\nsecond"
    );
}

#[gpui::test]
fn folded_regions_open_for_reveal(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    let old: String = (0..60).map(|i| format!("line {i}\n")).collect();
    let new = old.replace("line 50\n", "line fifty\n");
    view.update(cx, |v, cx| v.set_diff("long.txt", old, new, cx));
    cx.run_until_parked();
    draw(cx);
    assert!(
        view.read_with(cx, |v, _| v.rows.row_of(Side::New, 5).is_none()),
        "folded"
    );
    view.update(cx, |v, cx| v.reveal(Side::New, 5, None, cx));
    assert!(
        view.read_with(cx, |v, _| v.rows.row_of(Side::New, 5).is_some()),
        "unfolded to show it"
    );
}
