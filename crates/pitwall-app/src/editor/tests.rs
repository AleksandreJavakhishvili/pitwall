//! The editor in a window: opening files by full path, typing, saving,
//! closing (asked first), new files and Save As, an agent's change while
//! editing (Keep mine / Take theirs / Show difference, a blocked save),
//! files reopened at the next start, go to line, git's marks.

use std::path::PathBuf;

use gpui::{Entity, TestAppContext, VisualTestContext};

use super::*;

/// A folder of its own under the system's temp folder.
fn folder(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pw-editor-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn setup(cx: &mut TestAppContext, restore: bool) -> (Entity<Editor>, &mut VisualTestContext) {
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::code_view::register(cx);
        cx.bind_keys(bindings());
    });
    let (editor, cx) = cx.add_window_view(|_, cx| Editor::new(restore, cx));
    (editor, cx)
}

fn text(e: &Entity<Editor>, cx: &mut VisualTestContext) -> String {
    e.read_with(cx, |e, cx| e.active_code().and_then(|c| c.read(cx).text()).unwrap_or_default())
}

fn dirty(e: &Entity<Editor>, cx: &mut VisualTestContext) -> bool {
    e.read_with(cx, |e, cx| e.tabs().iter().any(|t| t.is_dirty(cx)))
}

fn open(e: &Entity<Editor>, path: &std::path::Path, cx: &mut VisualTestContext) {
    let p = path.to_string_lossy().into_owned();
    e.update(cx, |e, cx| {
        e.open(&p, cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let f = e.read(cx).focus_handle(cx);
        window.focus(&f, cx);
    });
    cx.run_until_parked();
}

fn caret_at(e: &Entity<Editor>, line: usize, col: usize, cx: &mut VisualTestContext) {
    e.update(cx, |e, cx| {
        if let Some(c) = e.active_code() {
            c.update(cx, |c, cx| c.reveal(Side::New, line, Some(col..col), cx));
        }
    });
    cx.run_until_parked();
}

/// Typing edits the file (its CRLF kept); ⌘S writes it; the tab is clean
/// again and the file on disk is the text.
#[gpui::test]
fn opens_types_and_saves(cx: &mut TestAppContext) {
    let d = folder("save");
    let f = d.join("a.rs");
    std::fs::write(&f, "fn a() {\r\n}\r\n").unwrap();
    let (e, cx) = setup(cx, false);
    open(&e, &f, cx);
    assert_eq!(text(&e, cx), "fn a() {\r\n}\r\n");
    caret_at(&e, 0, 8, cx);
    cx.simulate_keystrokes("enter");
    cx.simulate_input("x();");
    assert_eq!(text(&e, cx), "fn a() {\r\n    x();\r\n}\r\n", "indented, CRLF kept");
    assert!(dirty(&e, cx));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-s" } else { "ctrl-s" });
    cx.run_until_parked();
    assert!(!dirty(&e, cx));
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "fn a() {\r\n    x();\r\n}\r\n");
    // Opening it again shows the same tab.
    open(&e, &f, cx);
    assert_eq!(e.read_with(cx, |e, _| e.tabs().len()), 1);
    let _ = std::fs::remove_dir_all(&d);
}

/// Closing a file with unsaved edits asks: Cancel keeps it, Save writes
/// and closes, Don't Save drops the edits.
#[gpui::test]
fn closing_asks_about_unsaved_edits(cx: &mut TestAppContext) {
    let d = folder("close");
    let (a, b) = (d.join("a.txt"), d.join("b.txt"));
    std::fs::write(&a, "one\n").unwrap();
    std::fs::write(&b, "two\n").unwrap();
    let (e, cx) = setup(cx, false);
    open(&e, &a, cx);
    caret_at(&e, 0, 3, cx);
    cx.simulate_input("!");
    let id = e.read_with(cx, |e, _| e.active_id().unwrap());
    cx.update(|window, cx| e.update(cx, |e, cx| e.request_close(id, window, cx)));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(e.read_with(cx, |e, _| e.tabs().len()), 1, "kept");
    cx.update(|window, cx| e.update(cx, |e, cx| e.request_close(id, window, cx)));
    cx.run_until_parked();
    cx.simulate_prompt_answer("Save");
    cx.run_until_parked();
    assert_eq!(e.read_with(cx, |e, _| e.tabs().len()), 0, "closed");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "one!\n", "saved first");

    open(&e, &b, cx);
    caret_at(&e, 0, 0, cx);
    cx.simulate_input("x");
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-w" } else { "ctrl-w" });
    cx.run_until_parked();
    cx.simulate_prompt_answer("Don't Save");
    cx.run_until_parked();
    assert_eq!(e.read_with(cx, |e, _| e.tabs().len()), 0);
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "two\n", "not written");
    // A clean file closes without asking.
    open(&e, &b, cx);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-w" } else { "ctrl-w" });
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(e.read_with(cx, |e, _| e.tabs().len()), 0);
    let _ = std::fs::remove_dir_all(&d);
}

/// An agent changes the file: shown at once while there's nothing
/// unsaved; with unsaved edits the bar asks, and Keep mine, Take theirs and
/// a blocked save each do what they say.
#[gpui::test]
fn an_agents_change_while_editing(cx: &mut TestAppContext) {
    let d = folder("agent");
    let f = d.join("a.py");
    std::fs::write(&f, "a = 1\n").unwrap();
    let (e, cx) = setup(cx, false);
    open(&e, &f, cx);
    // Nothing unsaved: the agent's text, quietly.
    std::fs::write(&f, "a = 2\n").unwrap();
    e.update(cx, |e, cx| e.recheck(cx));
    cx.run_until_parked();
    assert_eq!(text(&e, cx), "a = 2\n");
    assert!(!e.read_with(cx, |e, _| e.conflict()));

    // Unsaved edits: never replaced; the bar asks.
    caret_at(&e, 0, 5, cx);
    cx.simulate_input("0");
    std::fs::write(&f, "a = 3\n").unwrap();
    e.update(cx, |e, cx| e.recheck(cx));
    cx.run_until_parked();
    assert_eq!(text(&e, cx), "a = 20\n", "mine kept");
    assert!(e.read_with(cx, |e, _| e.conflict()));
    // Show difference: theirs against mine.
    e.update(cx, |e, cx| e.toggle_difference(cx));
    cx.run_until_parked();
    assert!(e.read_with(cx, |e, _| e.active_tab().unwrap().diff.is_some()));
    // Take theirs: the file on disk replaces the edits.
    e.update(cx, |e, cx| e.take_theirs(cx));
    cx.run_until_parked();
    assert_eq!(text(&e, cx), "a = 3\n");
    assert!(!dirty(&e, cx) && !e.read_with(cx, |e, _| e.conflict()));

    // Keep mine: a save writes over the agent's text.
    caret_at(&e, 0, 5, cx);
    cx.simulate_input("1");
    std::fs::write(&f, "a = 4\n").unwrap();
    e.update(cx, |e, cx| e.recheck(cx));
    cx.run_until_parked();
    assert!(e.read_with(cx, |e, _| e.conflict()));
    e.update(cx, |e, cx| e.keep_mine(cx));
    let id = e.read_with(cx, |e, _| e.active_id().unwrap());
    e.update(cx, |e, cx| e.save(id, cx)).detach();
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "a = 31\n");

    // A save over a change it hadn't seen is blocked; "Save mine" goes on.
    caret_at(&e, 0, 6, cx);
    cx.simulate_input("2");
    std::fs::write(&f, "a = 5\n").unwrap();
    e.update(cx, |e, cx| e.save(id, cx)).detach();
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "a = 5\n", "not written over");
    assert!(e.read_with(cx, |e, _| e.active_tab().unwrap().save_blocked));
    e.update(cx, |e, cx| e.keep_mine(cx));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "a = 312\n");
    assert!(!dirty(&e, cx));
    let _ = std::fs::remove_dir_all(&d);
}

/// A new file: named on its first save (the save panel), shown by its new
/// name after.
#[gpui::test]
fn a_new_file_is_named_when_saved(cx: &mut TestAppContext) {
    let d = folder("new");
    let (e, cx) = setup(cx, false);
    e.update(cx, |e, cx| {
        e.new_file(cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let f = e.read(cx).focus_handle(cx);
        window.focus(&f, cx);
    });
    cx.simulate_input("hello");
    assert!(dirty(&e, cx));
    assert_eq!(e.read_with(cx, |e, _| e.tabs()[0].name()), "Untitled-1");
    let id = e.read_with(cx, |e, _| e.active_id().unwrap());
    e.update(cx, |e, cx| e.save(id, cx)).detach();
    cx.run_until_parked();
    let to = d.join("hello.md");
    let to2 = to.clone();
    cx.simulate_new_path_selection(move |_| Some(to2));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&to).unwrap(), "hello");
    assert_eq!(e.read_with(cx, |e, _| e.tabs()[0].name()), "hello.md");
    assert!(!dirty(&e, cx));
    // Save As to an existing name replaces that file (the panel asked).
    let other = d.join("other.md");
    std::fs::write(&other, "old").unwrap();
    e.update(cx, |e, cx| e.save_to(id, &other, cx)).detach();
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "hello");
    assert_eq!(e.read_with(cx, |e, _| e.active_path().map(String::from)), Some(other.to_string_lossy().into_owned()));
    let _ = std::fs::remove_dir_all(&d);
}

/// The open files are kept in `ui.json` (not their unsaved text) and open
/// again at the next start, the same one shown.
#[gpui::test]
fn files_open_again_after_a_restart(cx: &mut TestAppContext) {
    let d = folder("restore");
    let (a, b) = (d.join("a.txt"), d.join("b.txt"));
    std::fs::write(&a, "a\n").unwrap();
    std::fs::write(&b, "b\n").unwrap();
    let (e, vcx) = setup(cx, false);
    open(&e, &a, vcx);
    open(&e, &b, vcx);
    caret_at(&e, 0, 0, vcx);
    vcx.simulate_input("unsaved ");
    e.update(vcx, |e, cx| {
        let first = e.tabs()[0].id;
        e.activate(first, cx);
    });
    let kept = vcx.update(|_, cx| crate::ui_state::get(cx).editor);
    assert_eq!(kept.tabs, vec![a.to_string_lossy().into_owned(), b.to_string_lossy().into_owned()]);
    assert_eq!(kept.active.as_deref(), Some(a.to_str().unwrap()));
    // "Restarted": a new editor from what was kept.
    let (e2, vcx) = setup(cx, true);
    vcx.run_until_parked();
    e2.read_with(vcx, |e, cx| {
        assert_eq!(e.tabs().len(), 2);
        assert_eq!(e.active_path(), Some(a.to_str().unwrap()));
        assert!(e.tabs().iter().all(|t| !t.is_dirty(cx)), "unsaved text isn't kept");
    });
    drop(e);
    let _ = std::fs::remove_dir_all(&d);
}

/// Go to line (and line:column); several cursors and comments in the
/// editor's file.
#[gpui::test]
fn go_to_line_and_editing_keys(cx: &mut TestAppContext) {
    let d = folder("goto");
    let f = d.join("a.toml");
    std::fs::write(&f, "a = 1\nb = 2\nc = 3\n").unwrap();
    let (e, cx) = setup(cx, false);
    open(&e, &f, cx);
    e.update(cx, |e, cx| e.go_to("2:3", cx));
    cx.run_until_parked();
    let caret = e.read_with(cx, |e, cx| e.active_code().unwrap().read(cx).caret());
    assert_eq!(caret, Some((1, 2)));
    // ⌃G opens the field; Enter goes there.
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "ctrl-g" } else { "ctrl-l" });
    cx.run_until_parked();
    assert!(e.read_with(cx, |e, _| e.goto.is_some()));
    cx.simulate_input("3");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(e.read_with(cx, |e, _| e.goto.is_none()));
    assert_eq!(e.read_with(cx, |e, cx| e.active_code().unwrap().read(cx).caret()), Some((2, 0)));
    // ⌘/ comments by the file's language (TOML: #).
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-/" } else { "ctrl-/" });
    assert_eq!(text(&e, cx), "a = 1\nb = 2\n# c = 3\n");
    let _ = std::fs::remove_dir_all(&d);
}

/// Files it doesn't open say why; large ones are read-only.
#[gpui::test]
fn what_it_does_not_edit(cx: &mut TestAppContext) {
    let d = folder("limits");
    let bin = d.join("a.bin");
    std::fs::write(&bin, [0u8, 1, 2, 0, 255]).unwrap();
    let big = d.join("big.txt");
    std::fs::write(&big, "x".repeat(TEXT_CAP as usize + 10)).unwrap();
    let (e, cx) = setup(cx, false);
    open(&e, &bin, cx);
    assert!(e.read_with(cx, |e, _| e.active_tab().unwrap().error.is_some()));
    open(&e, &big, cx);
    e.read_with(cx, |e, cx| {
        let t = e.active_tab().unwrap();
        assert_eq!(t.read_only, Some("too large to edit (over 2 MiB)"));
        assert!(!t.code.read(cx).is_editing());
    });
    open(&e, &d.join("missing.txt"), cx);
    assert!(e.read_with(cx, |e, _| e.active_tab().unwrap().error.is_some()));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn git_marks_lines_changed_since_head() {
    let git = GitFile { status: Some(FileStatus::M), head: None, in_repo: true };
    let m = line_marks(Some("a\nb\nc\n"), &git, "a\nB\nc\nd\n");
    assert_eq!(m, vec![(1..2, LineMark::Changed), (3..4, LineMark::Added)]);
    let m = line_marks(Some("a\nb\nc\n"), &git, "a\nc\n");
    assert_eq!(m, vec![(1..2, LineMark::Deleted)]);
    assert!(line_marks(Some("a\n"), &git, "a\n").is_empty());
    let new = GitFile { status: Some(FileStatus::U), head: None, in_repo: true };
    assert_eq!(line_marks(None, &new, "a\nb\n"), vec![(0..2, LineMark::Added)]);
    assert!(line_marks(None, &GitFile::default(), "a\n").is_empty());
}
