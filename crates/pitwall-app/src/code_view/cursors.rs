//! More than one cursor (the editor, and editing in the viewer): ⌘-click
//! adds one (or takes one away), ⌘D selects the word at the cursor and
//! then adds its next match each time, ⌘⇧L puts one at every match, ⌥⌘↑ /
//! ⌥⌘↓ add one on the line above or below. Moving and selecting keys move
//! every cursor; cursors that meet become one; Esc (or a plain click)
//! leaves the primary one. Typing and the editing keys act at each
//! (`edit.rs`). Diffs and read-only files keep one cursor.

use gpui::{Context, Window};

use super::find::{self, Query};
use super::text::{self, Pos};
use super::{CodeView, Selection, Side};

impl CodeView {
    /// Run a move or a selection change (`f`, written for the primary
    /// cursor) for every cursor; the primary last, so the view follows it.
    pub(super) fn each(&mut self, window: &mut Window, cx: &mut Context<Self>, f: impl Fn(&mut Self, &mut Window, &mut Context<Self>)) {
        if self.extra.is_empty() {
            return f(self, window, cx);
        }
        let extras = std::mem::take(&mut self.extra);
        let (primary, goal) = (self.sel, self.goal_x);
        let mut moved = Vec::with_capacity(extras.len());
        for s in extras {
            self.sel = Some(s);
            self.goal_x = None;
            f(self, window, cx);
            moved.extend(self.sel);
        }
        self.sel = primary;
        self.goal_x = goal;
        f(self, window, cx);
        self.extra = moved;
        self.merge_cursors();
    }

    /// Cursors on the same place, or with overlapping selections, become
    /// one (the primary one stays).
    pub(super) fn merge_cursors(&mut self) {
        let Some(p) = self.sel else {
            self.extra.clear();
            return;
        };
        let mut kept: Vec<Selection> = Vec::with_capacity(self.extra.len());
        let overlaps = |a: &Selection, b: &Selection| {
            let (x, y) = (a.range(), b.range());
            x == y || (x.0 < y.1 && y.0 < x.1)
        };
        for s in std::mem::take(&mut self.extra) {
            if s.side != p.side || overlaps(&s, &p) || kept.iter().any(|k| overlaps(k, &s)) {
                continue;
            }
            kept.push(s);
        }
        self.extra = kept;
    }

    /// Only the primary cursor is left; whether there were others.
    pub(super) fn single_cursor(&mut self) -> bool {
        let had = !self.extra.is_empty();
        self.extra.clear();
        had
    }

    /// ⌘-click at `p`: a cursor there too, or (one there already) that
    /// one taken away.
    pub(super) fn toggle_cursor_at(&mut self, p: Pos) {
        let Some(primary) = self.sel else {
            self.sel = Some(Selection::caret(Side::New, p));
            return;
        };
        if let Some(i) = self.extra.iter().position(|s| s.is_empty() && s.head == p) {
            self.extra.remove(i);
            return;
        }
        if primary.is_empty() && primary.head == p {
            // The primary one goes: the last added takes its place.
            if let Some(next) = self.extra.pop() {
                self.sel = Some(next);
            }
            return;
        }
        self.extra.push(primary);
        self.sel = Some(Selection::caret(Side::New, p));
        self.merge_cursors();
    }

    /// The selected text's matches (case and all, as typed), in order.
    fn matches_of(&self, s: Selection) -> Vec<find::Match> {
        let Some(c) = self.content.as_ref() else { return Vec::new() };
        let (a, b) = s.range();
        let needle = c.doc(Side::New).text_between(a, b);
        if needle.is_empty() || needle.contains('\n') {
            return Vec::new();
        }
        let q = Query { text: needle, case: true, word: false, regex: false };
        find::find_all(c.doc(Side::New), &q, None).matches
    }

    /// The word at the primary caret, selected (⌘D's first press, and
    /// ⌘⇧L's with nothing selected). Whether there was one.
    fn select_word_at_caret(&mut self) -> bool {
        let Some(s) = self.sel.filter(|s| s.is_empty() && s.side == Side::New) else { return false };
        let Some(c) = self.content.as_ref() else { return false };
        let Some(line) = c.doc(Side::New).line(s.head.line) else { return false };
        let w = text::word_at(&line.text, s.head.col);
        if !line.text[w.clone()].chars().any(text::is_word_char) {
            return false;
        }
        self.sel = Some(Selection { side: Side::New, anchor: Pos::new(s.head.line, w.start), head: Pos::new(s.head.line, w.end) });
        true
    }

    /// ⌘D: with nothing selected, the word at the cursor; then a cursor
    /// at the selection's next match (after the last one added, wrapping),
    /// which the view shows.
    pub(super) fn add_next_match(&mut self, cx: &mut Context<Self>) {
        if self.content.as_ref().is_none_or(|c| c.is_diff) {
            return;
        }
        let Some(s) = self.sel else { return };
        if s.is_empty() {
            if self.select_word_at_caret() {
                cx.notify();
            }
            return;
        }
        let taken: Vec<(Pos, Pos)> = std::iter::once(s).chain(self.extra.iter().copied()).map(|x| x.range()).collect();
        let ms = self.matches_of(s);
        let after = s.range().1;
        let next = ms
            .iter()
            .filter(|m| m.start() >= after)
            .chain(ms.iter().filter(|m| m.start() < after))
            .find(|m| !taken.contains(&(m.start(), m.end())));
        let Some(m) = next.cloned() else { return };
        self.extra.push(s);
        self.sel = Some(Selection { side: Side::New, anchor: m.start(), head: m.end() });
        if let Some(row) = self.rows.row_of(Side::New, m.line) {
            self.scroll_to_row(row);
        }
        self.caret_x = true;
        cx.notify();
    }

    /// ⌘⇧L: a cursor at every match of the selection (the word at the
    /// cursor, with nothing selected).
    pub(super) fn select_all_matches(&mut self, cx: &mut Context<Self>) {
        if self.content.as_ref().is_none_or(|c| c.is_diff) {
            return;
        }
        let Some(s) = self.sel else { return };
        if s.is_empty() && !self.select_word_at_caret() {
            return;
        }
        let Some(s) = self.sel else { return };
        let ms = self.matches_of(s);
        if ms.is_empty() {
            return;
        }
        self.extra = ms
            .iter()
            .filter(|m| (m.start(), m.end()) != s.range())
            .map(|m| Selection { side: Side::New, anchor: m.start(), head: m.end() })
            .collect();
        cx.notify();
    }

    /// ⌥⌘↑ / ⌥⌘↓: a cursor on the line above (or below) the primary one,
    /// at the same place across; it becomes the primary one.
    pub(super) fn add_cursor_vertical(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.as_ref().is_none_or(|c| c.is_diff) {
            return;
        }
        let Some(s) = self.sel else { return };
        let extras = std::mem::take(&mut self.extra);
        self.vertical(if down { 1 } else { -1 }, false, window, cx);
        self.extra = extras;
        if self.sel.is_some_and(|n| n.head.line != s.head.line) {
            self.extra.push(Selection::caret(s.side, s.head));
            self.merge_cursors();
        }
        cx.notify();
    }

    /// Home: the line's start; in the editor, its text's start first (its
    /// indentation skipped), then the very start.
    pub(super) fn line_start(&mut self, extend: bool, cx: &mut Context<Self>) {
        if !self.editor {
            return self.horizontal(|_, _| 0, extend, cx);
        }
        self.horizontal(
            |t, c| {
                let first = t.len() - t.trim_start().len();
                if c == first {
                    0
                } else {
                    first
                }
            },
            extend,
            cx,
        )
    }

    /// The row showing position `p` of the file (a wrapped line's own).
    pub(super) fn row_of_pos(&self, p: Pos) -> Option<usize> {
        self.rows.row_of(Side::New, p.line)
    }

    /// The bench's typing: `text` at every cursor, as typed (a line break
    /// as Enter).
    pub fn debug_type(&mut self, text: &str, cx: &mut Context<Self>) {
        for (i, part) in text.split('\n').enumerate() {
            if i > 0 {
                self.newline(cx);
            }
            if !part.is_empty() {
                self.typed(part, cx);
            }
        }
    }

    /// The bench's keys: `undo`, `redo`, `comment`, `next-match`,
    /// `all-matches`, `cursor-above`, `cursor-below`, `escape`, `tab`,
    /// `backspace`, `select-all`.
    pub fn debug_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        match key {
            "undo" => self.undo(cx),
            "redo" => self.redo(cx),
            "comment" => self.toggle_comment(cx),
            "next-match" => self.add_next_match(cx),
            "all-matches" => self.select_all_matches(cx),
            "cursor-above" => self.add_cursor_vertical(false, window, cx),
            "cursor-below" => self.add_cursor_vertical(true, window, cx),
            "escape" => {
                self.single_cursor();
            }
            "tab" => self.indent(cx),
            "backspace" => self.backspace(cx),
            "select-all" => {
                if let Some(c) = self.content.as_ref() {
                    let end = c.doc(Side::New).end();
                    self.extra.clear();
                    self.sel = Some(Selection { side: Side::New, anchor: Pos::default(), head: end });
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Every cursor's selection (the primary one first), for drawing.
    pub(super) fn all_selections(&self) -> Vec<Selection> {
        self.sel.iter().chain(self.extra.iter()).copied().collect()
    }
}
