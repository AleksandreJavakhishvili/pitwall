//! The view as an entity in a window: loading, layouts, find, reveal,
//! keys and copying.

use gpui::{Entity, EntityInputHandler as _, TestAppContext, VisualTestContext};

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
    cx.update(|window, cx| window.focus(&view.focus_handle(cx), cx));
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
fn under_liquid_glass_the_read_only_hint_is_a_pop_up(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    cx.update(|_, cx| crate::kit::passive::test_glass(cx));
    let open = |cx: &mut VisualTestContext| cx.update(|window, cx| crate::kit::passive::test_over(window, cx).len());
    view.update(cx, |v, cx| v.set_diff("x.rs", OLD.into(), NEW.into(), cx));
    cx.run_until_parked();
    draw(cx);
    assert_eq!(open(cx), 0);
    view.update(cx, |v, _| v.sel = Some(Selection::caret(Side::New, Pos::new(0, 0))));
    cx.simulate_keystrokes("x");
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.readonly_at.is_some()));
    assert_eq!(open(cx), 1, "the hint, above the window");
    cx.simulate_keystrokes("escape");
    crate::kit::passive::test_draw(cx);
    assert_eq!(open(cx), 0);
}

#[gpui::test]
fn under_liquid_glass_the_context_menu_is_a_pop_up_with_the_keys(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    cx.update(|_, cx| crate::kit::passive::test_glass(cx));
    view.update(cx, |v, cx| {
        v.set_commentable(true, cx);
        v.set_diff("x.rs", OLD.into(), NEW.into(), cx);
    });
    cx.run_until_parked();
    draw(cx);
    let pops = |cx: &mut VisualTestContext| {
        let all = cx.windows();
        let open = crate::kit::pop::open_pops().into_iter();
        open.filter_map(|(id, _, _)| all.iter().find(|w| w.window_id() == id).copied()).collect::<Vec<_>>()
    };
    let open = |view: &Entity<CodeView>, cx: &mut VisualTestContext| {
        view.update(cx, |v, cx| {
            v.menu = Some(Menu { at: gpui::point(px(120.), px(60.)), line: Some(3), active: None });
            cx.notify();
        });
        draw(cx);
        let open = pops(cx);
        assert_eq!(open.len(), 1, "the menu's pop-up");
        open[0]
    };
    let key = |pop: gpui::AnyWindowHandle, key: &'static str, cx: &mut VisualTestContext| {
        let k = gpui::Keystroke::parse(key).unwrap();
        cx.update_window(pop, |_, window, cx| window.dispatch_keystroke(k, cx)).unwrap();
        cx.run_until_parked();
    };
    let said = std::rc::Rc::new(std::cell::Cell::new(None));
    let s = said.clone();
    let _sub = cx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &CodeViewEvent, _| {
            if let CodeViewEvent::Comment { line } = e {
                s.set(Some(*line));
            }
        })
    });
    // ↓ and Enter in the pop-up: the first item, on the menu's line.
    let pop = open(&view, cx);
    key(pop, "down", cx);
    assert_eq!(view.read_with(cx, |v, _| v.menu.as_ref().and_then(|m| m.active)), Some(0));
    key(pop, "enter", cx);
    draw(cx);
    assert_eq!(said.get(), Some(3), "Add review comment");
    assert!(view.read_with(cx, |v, _| v.menu.is_none()));
    assert!(pops(cx).is_empty());
    // Esc closes it.
    let pop = open(&view, cx);
    cx.update_window(pop, |_, window, cx| window.dispatch_action(Box::new(crate::menu::Dismiss), cx)).unwrap();
    cx.run_until_parked();
    assert!(view.read_with(cx, |v, _| v.menu.is_none()));
    assert!(pops(cx).is_empty());
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

fn editable<'a>(cx: &'a mut TestAppContext, path: &str, text: &str) -> (Entity<CodeView>, &'a mut VisualTestContext) {
    let (view, cx) = setup(cx);
    let (p, t) = (path.to_string(), text.to_string());
    view.update(cx, |v, cx| {
        v.set_editable(true, cx);
        v.set_file(&p, t, cx);
    });
    cx.run_until_parked();
    draw(cx);
    (view, cx)
}

fn text_of(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, _| v.text().unwrap())
}

fn caret(view: &Entity<CodeView>, line: usize, col: usize, cx: &mut VisualTestContext) {
    view.update(cx, |v, cx| {
        v.sel = Some(Selection::caret(Side::New, Pos::new(line, col)));
        cx.notify();
    });
}

/// Typing through the input handler (as the keyboard and input methods
/// do), Enter keeping the indent, Backspace by grapheme, undo as one step,
/// the file's CRLF line breaks and final newline kept.
#[gpui::test]
fn typing_edits_the_file_and_keeps_its_line_breaks(cx: &mut TestAppContext) {
    let (view, cx) = editable(cx, "src/a.rs", "fn a() {\r\n    x\r\n}\r\n");
    assert!(view.read_with(cx, |v, _| v.is_editing() && !v.is_dirty()));
    caret(&view, 1, 5, cx);
    cx.simulate_input("y");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("z");
    assert_eq!(text_of(&view, cx), "fn a() {\r\n    xy\r\n    z\r\n}\r\n");
    assert!(view.read_with(cx, |v, _| v.is_dirty()));
    cx.simulate_keystrokes("backspace");
    assert_eq!(text_of(&view, cx), "fn a() {\r\n    xy\r\n    \r\n}\r\n");
    // Undo: the backspace, then Enter + typing.
    let undo = if cfg!(target_os = "macos") { "cmd-z" } else { "ctrl-z" };
    cx.simulate_keystrokes(undo);
    assert_eq!(text_of(&view, cx), "fn a() {\r\n    xy\r\n    z\r\n}\r\n");
    cx.simulate_keystrokes(undo);
    cx.simulate_keystrokes(undo);
    cx.simulate_keystrokes(undo);
    assert_eq!(text_of(&view, cx), "fn a() {\r\n    x\r\n}\r\n");
    assert!(!view.read_with(cx, |v, _| v.is_dirty()), "back to the file as read");
    let redo = if cfg!(target_os = "macos") { "cmd-shift-z" } else { "ctrl-y" };
    cx.simulate_keystrokes(redo);
    assert_eq!(text_of(&view, cx), "fn a() {\r\n    xy\r\n}\r\n");
}

/// Georgian (three bytes a letter) and a combining mark: the caret moves
/// and deletes whole graphemes; an input method's composition is one step.
#[gpui::test]
fn georgian_combining_marks_and_composition(cx: &mut TestAppContext) {
    let (view, cx) = editable(cx, "notes.txt", "გამარჯობა\n");
    caret(&view, 0, "გამარჯობა".len(), cx);
    cx.simulate_input(" e\u{301}");
    assert_eq!(text_of(&view, cx), "გამარჯობა e\u{301}\n");
    cx.simulate_keystrokes("backspace");
    assert_eq!(text_of(&view, cx), "გამარჯობა \n", "e and its accent go together");
    caret(&view, 0, 3, cx);
    cx.simulate_keystrokes("shift-right");
    assert_eq!(view.read_with(cx, |v, _| v.selection().map(|s| (s.anchor.col, s.head.col))), Some((3, 6)));
    cx.simulate_input("ო");
    assert_eq!(text_of(&view, cx), "გომარჯობა \n");
    // Composition: marked text replaced as it changes, then committed.
    caret(&view, 0, 0, cx);
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.replace_and_mark_text_in_range(None, "k", None, window, cx);
            assert_eq!(v.marked_text_range(window, cx), Some(0..1));
            v.replace_and_mark_text_in_range(None, "ká", None, window, cx);
            v.replace_text_in_range(None, "ქ", window, cx);
            assert_eq!(v.marked_text_range(window, cx), None);
        })
    });
    assert_eq!(text_of(&view, cx), "ქგომარჯობა \n");
    let undo = if cfg!(target_os = "macos") { "cmd-z" } else { "ctrl-z" };
    cx.simulate_keystrokes(undo);
    assert_eq!(text_of(&view, cx), "გომარჯობა \n", "the whole composition undone at once");
}

/// Tab and Shift-Tab use the file's indentation (tabs here), over every
/// selected line; cut and paste go through the clipboard.
#[gpui::test]
fn indent_cut_and_paste(cx: &mut TestAppContext) {
    let (view, cx) = editable(cx, "x.go", "func a() {\n\tb()\n\tc()\n}\n");
    view.update(cx, |v, cx| {
        v.sel = Some(Selection { side: Side::New, anchor: Pos::new(1, 0), head: Pos::new(2, 5) });
        cx.notify();
    });
    cx.simulate_keystrokes("tab");
    assert_eq!(text_of(&view, cx), "func a() {\n\t\tb()\n\t\tc()\n}\n");
    cx.simulate_keystrokes("shift-tab shift-tab");
    assert_eq!(text_of(&view, cx), "func a() {\nb()\nc()\n}\n");
    caret(&view, 1, 0, cx);
    let (cut, paste) = if cfg!(target_os = "macos") { ("cmd-x", "cmd-v") } else { ("ctrl-x", "ctrl-v") };
    cx.simulate_keystrokes(cut);
    assert_eq!(text_of(&view, cx), "func a() {\nc()\n}\n", "nothing selected: the line");
    assert_eq!(cx.read_from_clipboard().and_then(|c| c.text()).as_deref(), Some("b()\n"));
    caret(&view, 2, 0, cx);
    cx.simulate_keystrokes(paste);
    assert_eq!(text_of(&view, cx), "func a() {\nc()\nb()\n}\n");
}

/// Find, then Replace one and Replace All (one undo step); a regular
/// expression's groups are expanded.
#[gpui::test]
fn replace_and_replace_all(cx: &mut TestAppContext) {
    let (view, cx) = editable(cx, "a.txt", "cat bat\ncat\nრა cat\n");
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.open_replace(false, window, cx);
            v.set_find_text("cat", window, cx);
        })
    });
    view.update(cx, |v, cx| v.replace_one("dog", cx));
    assert_eq!(text_of(&view, cx), "dog bat\ncat\nრა cat\n");
    let n = view.update(cx, |v, cx| v.replace_all("cow", cx));
    assert_eq!(n, 2);
    assert_eq!(text_of(&view, cx), "dog bat\ncow\nრა cow\n");
    let undo = if cfg!(target_os = "macos") { "cmd-z" } else { "ctrl-z" };
    cx.simulate_keystrokes(undo);
    assert_eq!(text_of(&view, cx), "dog bat\ncat\nრა cat\n", "Replace All is one step");
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.set_query(|q| q.regex = true, false, cx);
            v.set_find_text("(\\w)at", window, cx);
        })
    });
    let n = view.update(cx, |v, cx| v.replace_all("${1}ot", cx));
    assert_eq!(n, 3);
    assert_eq!(text_of(&view, cx), "dog bot\ncot\nრა cot\n");
}

/// ⌘S asks the owner; once saved the same text keeps the history, and a
/// different one (the file changed on disk) replaces the buffer.
#[gpui::test]
fn save_is_asked_and_a_reload_replaces_the_buffer(cx: &mut TestAppContext) {
    let (view, cx) = editable(cx, "a.txt", "one\n");
    let saves = std::rc::Rc::new(std::cell::Cell::new(0));
    let s = saves.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, e: &CodeViewEvent, _| {
            if *e == CodeViewEvent::Save {
                s.set(s.get() + 1)
            }
        })
        .detach()
    });
    caret(&view, 0, 3, cx);
    cx.simulate_input("!");
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-s" } else { "ctrl-s" });
    assert_eq!(saves.get(), 1);
    view.update(cx, |v, cx| v.set_file("a.txt", "one!\n".into(), cx));
    assert!(view.read_with(cx, |v, _| !v.is_dirty() && v.edit.as_ref().unwrap().buf.can_undo()));
    // The agent changed it: the new text, clean, caret kept in range.
    view.update(cx, |v, cx| v.set_file("a.txt", "two\nthree\n".into(), cx));
    cx.run_until_parked();
    assert_eq!(text_of(&view, cx), "two\nthree\n");
    assert!(view.read_with(cx, |v, _| !v.is_dirty() && !v.edit.as_ref().unwrap().buf.can_undo()));
    // Turned off: read-only again, typing says so.
    view.update(cx, |v, cx| v.set_editable(false, cx));
    cx.run_until_parked();
    draw(cx);
    cx.simulate_keystrokes("x");
    assert!(view.read_with(cx, |v, _| v.text().is_none() && v.readonly_at.is_some()));
}

/// Diffs stay read-only even with editing on.
#[gpui::test]
fn diffs_are_not_edited(cx: &mut TestAppContext) {
    let (view, cx) = setup(cx);
    view.update(cx, |v, cx| {
        v.set_editable(true, cx);
        v.set_diff("x.rs", OLD.into(), NEW.into(), cx);
    });
    cx.run_until_parked();
    draw(cx);
    assert!(!view.read_with(cx, |v, _| v.is_editing()));
}

/// A large file: each keystroke changes only its line (no re-highlight of
/// the whole file until typing pauses).
#[gpui::test]
fn typing_in_a_large_file_touches_only_its_line(cx: &mut TestAppContext) {
    let text: String = (0..20_000).map(|i| format!("let v{i} = {i};\n")).collect();
    let (view, cx) = editable(cx, "big.rs", &text);
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    let hl_before = view.read_with(cx, |v, _| v.content.as_ref().unwrap().new_hl.len());
    caret(&view, 10_000, 0, cx);
    cx.simulate_input("x");
    view.read_with(cx, |v, _| {
        let c = v.content.as_ref().unwrap();
        assert_eq!(c.new.lines[10_000].text.as_ref(), "xlet v10000 = 10000;");
        assert_eq!(c.new_hl.len(), hl_before, "spans kept, shifted on the line");
    });
    cx.executor().advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    assert!(text_of(&view, cx).contains("xlet v10000"));
}
