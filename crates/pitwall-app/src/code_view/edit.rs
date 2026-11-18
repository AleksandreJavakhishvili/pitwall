//! Editing a single file in the view (the explorer's viewer: quick fixes
//! while reading, not an IDE). The [`Buffer`] holds the text; every change
//! splices only the lines it touched into the shown [`Document`] and the
//! highlighting (which is redone in the background a moment after typing
//! stops), so typing stays quick in large files. Text comes in through the
//! platform's input handler, as in [`crate::kit::input`]: typed keys, an
//! input method's composition (Georgian, Japanese, dead keys), dictation.
//! Diffs stay read-only.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    point, px, size, Bounds, ClipboardItem, Context, EntityInputHandler, Pixels, SharedString,
    Task, UTF16Selection, Window,
};

use super::buffer::{Buffer, Kind, Splice, SrcPos, SrcSel};
use super::highlight::{self, Lang};
use super::rows::{self, Rows};
use super::text::{self, Line, Pos};
use super::{CodeView, CodeViewEvent, Selection, Side};

/// Highlighting is redone this long after the last change.
const REHIGHLIGHT: Duration = Duration::from_millis(150);

pub(super) struct Editing {
    pub buf: Buffer,
    /// An input method's text being composed (source positions).
    pub marked: Option<(SrcPos, SrcPos)>,
    hl: Option<Task<()>>,
}

impl Editing {
    fn new(text: &str) -> Editing {
        Editing { buf: Buffer::new(text), marked: None, hl: None }
    }
}

fn utf16_to_byte(s: &str, u: usize) -> usize {
    let mut n = 0;
    for (i, ch) in s.char_indices() {
        if n >= u {
            return i;
        }
        n += ch.len_utf16();
    }
    s.len()
}

fn byte_to_utf16(s: &str, b: usize) -> usize {
    s[..text::clamp_to_char(s, b.min(s.len()))].chars().map(char::len_utf16).sum()
}

/// `spans` of a line after `removed` bytes at `col` became `inserted`.
fn shift_spans(spans: &mut highlight::LineSpans, col: usize, removed: usize, inserted: usize) {
    let (end_old, end_new) = (col + removed, col + inserted);
    spans.retain_mut(|(r, _)| {
        let start = if r.start <= col { r.start } else if r.start >= end_old { r.start - removed + inserted } else { end_new };
        let end = if r.end >= end_old { r.end - removed + inserted } else if r.end <= col { r.end } else { end_new };
        *r = start..end;
        start < end
    });
}

impl CodeView {
    // ── the owner's side ────────────────────────────────────────────────

    /// Let the user edit the file shown (not a diff). Off: read-only again
    /// (unsaved changes are dropped: the owner asks first).
    pub fn set_editable(&mut self, on: bool, cx: &mut Context<Self>) {
        self.editable = on;
        if !on {
            if self.edit.take().is_some() {
                self.restore_file_view(cx);
            }
        } else {
            self.start_editing();
        }
        cx.notify();
    }

    /// The text as read, shown again (edits dropped).
    fn restore_file_view(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.content.clone() {
            let (path, text) = (c.path.to_string(), c.new_text.to_string());
            self.content = None;
            self.load(&path, String::new(), text, false, cx);
        }
    }

    /// The buffer for the file shown, when editing is wanted and possible.
    pub(super) fn start_editing(&mut self) {
        if !self.editable || self.edit.is_some() {
            return;
        }
        if let Some(c) = self.content.as_ref().filter(|c| !c.is_diff) {
            self.edit = Some(Editing::new(&c.new_text));
        }
    }

    pub fn is_editing(&self) -> bool {
        self.edit.is_some()
    }

    /// The text differs from the file as read or last saved.
    pub fn is_dirty(&self) -> bool {
        self.edit.as_ref().is_some_and(|e| e.buf.dirty())
    }

    /// The text as it would be saved (`None`: not editing).
    pub fn text(&self) -> Option<String> {
        self.edit.as_ref().map(|e| e.buf.text())
    }

    /// Changes so far (the owner redraws a preview when it moves).
    pub fn version(&self) -> u64 {
        self.edit.as_ref().map_or(0, |e| e.buf.version)
    }

    /// `text` is on disk now (a save): the buffer is clean at it, and a
    /// later `set_file` with the same text keeps the undo history.
    pub fn mark_saved(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(e) = self.edit.as_mut() {
            e.buf.mark_saved();
        }
        if let Some(c) = self.content.as_mut() {
            c.new_text = text.into();
        }
        cx.emit(CodeViewEvent::Edited);
        cx.notify();
    }

    /// What typing says when the file can't be edited.
    pub fn set_readonly_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.readonly_text = text.into();
        cx.notify();
    }

    // ── positions ───────────────────────────────────────────────────────

    fn to_src(&self, p: Pos) -> SrcPos {
        let doc = self.doc(Side::New);
        let p = doc.clamp(p);
        SrcPos::new(p.line, doc.lines[p.line].to_source(p.col))
    }

    fn to_shown(&self, p: SrcPos) -> Pos {
        let doc = self.doc(Side::New);
        let line = p.line.min(doc.line_count() - 1);
        Pos::new(line, doc.lines[line].from_source(p.col))
    }

    fn src_sel(&self) -> SrcSel {
        let s = self.sel_or_start();
        SrcSel { anchor: self.to_src(s.anchor), head: self.to_src(s.head) }
    }

    /// The marked text on screen (for its underline).
    pub(super) fn marked_shown(&self) -> Option<(Pos, Pos)> {
        let (a, b) = self.edit.as_ref()?.marked?;
        Some((self.to_shown(a), self.to_shown(b)))
    }

    // ── changing the text ───────────────────────────────────────────────

    /// Apply `edits` as one undo step (`after`: the selection then, else a
    /// caret after the last edit).
    pub(super) fn apply_edits(&mut self, edits: Vec<(SrcPos, SrcPos, String)>, kind: Kind, after: Option<SrcSel>, cx: &mut Context<Self>) {
        if self.edit.is_none() || edits.is_empty() {
            return;
        }
        let before = self.src_sel();
        let ed = self.edit.as_mut().expect("editing");
        let (splices, sel) = ed.buf.edit(edits, kind, before, after);
        self.changed(splices, sel, cx);
    }

    /// Replace the selection with `text` (from outside: its line breaks
    /// become the file's).
    pub(super) fn insert(&mut self, text: &str, kind: Kind, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_ref() else { return };
        let text = ed.buf.normalize(text);
        let (a, b) = self.src_sel().range();
        self.apply_edits(vec![(a, b, text)], kind, None, cx);
    }

    /// The buffer changed: the shown lines, highlighting, rows, selection
    /// and Find follow.
    fn changed(&mut self, splices: Vec<Splice>, sel: SrcSel, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_ref() else { return };
        let Some(c) = self.content.as_mut() else { return };
        let doc = Arc::make_mut(&mut c.new);
        let tab = doc.tab_size;
        let hl = Arc::make_mut(&mut c.new_hl);
        for s in &splices {
            let fresh = (s.first..s.first + s.new).map(|i| Line::new(ed.buf.raw_line(i), tab));
            let end = (s.first + s.old).min(doc.lines.len());
            doc.lines.splice(s.first..end, fresh);
            if hl.is_empty() || s.first > hl.len() {
                continue;
            }
            match s.inline {
                Some((col, removed, inserted)) if s.first < hl.len() => shift_spans(&mut hl[s.first], col, removed, inserted),
                _ => {
                    let end = (s.first + s.old).min(hl.len());
                    hl.splice(s.first..end, std::iter::repeat_n(Vec::new(), s.new));
                }
            }
        }
        self.rows = Rows::file(doc.line_count());
        let (anchor, head) = (self.to_shown(sel.anchor), self.to_shown(sel.head));
        self.sel = Some(Selection { side: Side::New, anchor, head });
        self.goal_x = None;
        self.readonly_at = None;
        self.caret_x = true;
        if let Some(row) = self.rows.row_of(Side::New, head.line) {
            self.scroll_to_row(row);
        }
        self.refind();
        self.rehighlight(cx);
        cx.emit(CodeViewEvent::Edited);
        cx.notify();
    }

    /// Highlight the whole text again, in the background, once typing
    /// pauses.
    fn rehighlight(&mut self, cx: &mut Context<Self>) {
        let Some(lang) = self.content.as_ref().and_then(|c| Lang::for_path(&c.path)) else { return };
        let Some(ed) = self.edit.as_mut() else { return };
        let version = ed.buf.version;
        let bg = cx.background_executor().clone();
        ed.hl = Some(cx.spawn(async move |this, cx| {
            bg.timer(REHIGHLIGHT).await;
            let Ok(Some(text)) = this.update(cx, |v, _| v.edit.as_ref().map(|e| e.buf.text())) else { return };
            let spans = bg.spawn(async move { highlight::highlight(&text, Some(lang)) }).await;
            let _ = this.update(cx, |v, cx| {
                if v.edit.as_ref().is_some_and(|e| e.buf.version == version) {
                    if let Some(c) = v.content.as_mut() {
                        c.new_hl = Arc::new(spans);
                        cx.notify();
                    }
                }
            });
        }));
    }

    // ── what keys do ────────────────────────────────────────────────────

    /// Delete the selection, or from the caret to where `to` says (on the
    /// caret's source line; `None`: across the line break).
    fn delete_by(&mut self, to: impl Fn(&str, usize) -> Option<usize>, forward: bool, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_ref() else { return };
        let s = self.src_sel();
        let (a, b) = s.range();
        if a != b {
            return self.apply_edits(vec![(a, b, String::new())], Kind::Other, None, cx);
        }
        let line = ed.buf.line(a.line);
        let other = match to(line, a.col) {
            Some(col) => SrcPos::new(a.line, col),
            None if forward && a.line + 1 < ed.buf.line_count() => SrcPos::new(a.line + 1, 0),
            None if !forward && a.line > 0 => SrcPos::new(a.line - 1, ed.buf.line(a.line - 1).len()),
            None => return,
        };
        if other == a {
            return;
        }
        let (x, y) = if other < a { (other, a) } else { (a, other) };
        // A single character is typing (a run of them undoes together).
        let kind = if to(line, a.col).is_some() && line.get(x.col..y.col).is_some_and(|t| t.chars().count() == 1) { Kind::Typing } else { Kind::Other };
        self.apply_edits(vec![(x, y, String::new())], kind, Some(SrcSel::caret(x)), cx);
    }

    pub(super) fn backspace(&mut self, cx: &mut Context<Self>) {
        self.delete_by(|t, c| (c > 0).then(|| text::prev_grapheme(t, c)), false, cx);
    }

    pub(super) fn delete_forward(&mut self, cx: &mut Context<Self>) {
        self.delete_by(|t, c| (c < t.len()).then(|| text::next_grapheme(t, c)), true, cx);
    }

    pub(super) fn delete_word_left(&mut self, cx: &mut Context<Self>) {
        self.delete_by(|t, c| (c > 0).then(|| text::prev_word(t, c)), false, cx);
    }

    pub(super) fn delete_word_right(&mut self, cx: &mut Context<Self>) {
        self.delete_by(|t, c| (c < t.len()).then(|| text::next_word(t, c)), true, cx);
    }

    pub(super) fn delete_to_line_start(&mut self, cx: &mut Context<Self>) {
        self.delete_by(|_, c| (c > 0).then_some(0), false, cx);
    }

    /// Enter: a line break and the line's indentation.
    pub(super) fn newline(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_ref() else { return };
        let (a, b) = self.src_sel().range();
        let nl = ed.buf.newline_at(a);
        self.apply_edits(vec![(a, b, nl)], Kind::Other, None, cx);
    }

    /// The lines a selection covers (a selection ending at a line's start
    /// leaves that line out).
    fn sel_lines(&self) -> (usize, usize) {
        let (a, b) = self.src_sel().range();
        let last = if b.line > a.line && b.col == 0 { b.line - 1 } else { b.line };
        (a.line, last)
    }

    /// Tab: one indentation step in (a selection over lines: each of them).
    pub(super) fn indent(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_ref() else { return };
        let (a, b) = self.src_sel().range();
        if a.line == b.line {
            let unit = ed.buf.unit();
            return self.apply_edits(vec![(a, b, unit)], Kind::Other, None, cx);
        }
        let (first, last) = self.sel_lines();
        let edits = ed.buf.indent_edits(first, last);
        let unit = ed.buf.unit().len();
        let end = SrcPos::new(last, ed.buf.line(last).len() + unit);
        self.apply_edits(edits, Kind::Other, Some(SrcSel { anchor: SrcPos::new(first, 0), head: end }), cx);
    }

    /// Shift-Tab: one indentation step out on each line covered.
    pub(super) fn outdent(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_ref() else { return };
        let s = self.src_sel();
        let (first, last) = self.sel_lines();
        let edits = ed.buf.outdent_edits(first, last);
        let removed_on = |line: usize| edits.iter().find(|(p, _, _)| p.line == line).map_or(0, |(_, q, _)| q.col);
        let after = if first == last {
            let n = removed_on(first);
            SrcSel { anchor: SrcPos::new(s.anchor.line, s.anchor.col.saturating_sub(n)), head: SrcPos::new(s.head.line, s.head.col.saturating_sub(n)) }
        } else {
            let end_len = ed.buf.line(last).len() - removed_on(last);
            SrcSel { anchor: SrcPos::new(first, 0), head: SrcPos::new(last, end_len) }
        };
        self.apply_edits(edits, Kind::Other, Some(after), cx);
    }

    pub(super) fn cut(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_ref() else { return };
        let (a, b) = self.src_sel().range();
        let (from, to) = if a == b {
            // Nothing selected: the whole line, as the editor's own cut.
            if a.line + 1 < ed.buf.line_count() {
                (SrcPos::new(a.line, 0), SrcPos::new(a.line + 1, 0))
            } else {
                (SrcPos::new(a.line, 0), SrcPos::new(a.line, ed.buf.line(a.line).len()))
            }
        } else {
            (a, b)
        };
        let mut copied = ed.buf.slice(from, to).replace("\r\n", "\n");
        if a == b && !copied.ends_with('\n') {
            copied.push('\n');
        }
        cx.write_to_clipboard(ClipboardItem::new_string(copied));
        self.apply_edits(vec![(from, to, String::new())], Kind::Other, Some(SrcSel::caret(from)), cx);
    }

    pub(super) fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return };
        self.insert(&text, Kind::Other, cx);
    }

    pub(super) fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_mut() else { return };
        ed.marked = None;
        if let Some((splices, sel)) = ed.buf.undo() {
            self.changed(splices, sel, cx);
        }
    }

    pub(super) fn redo(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_mut() else { return };
        ed.marked = None;
        if let Some((splices, sel)) = ed.buf.redo() {
            self.changed(splices, sel, cx);
        }
    }

    // ── replace ─────────────────────────────────────────────────────────

    /// Replace the selected match with `with` and go to the next one.
    pub fn replace_one(&mut self, with: &str, cx: &mut Context<Self>) {
        let Some(f) = self.find.as_ref() else { return };
        if self.edit.is_none() {
            return;
        }
        let q = f.query.clone();
        let current = f.current.and_then(|i| f.found.matches.get(i).cloned()).filter(|m| {
            self.sel.is_some_and(|s| s.range() == (m.start(), m.end()))
        });
        let Some(m) = current else {
            // Nothing selected yet: select the first match from here.
            let from = self.sel.map_or(Pos::default(), |s| s.range().0);
            if let Some(i) = super::find::next_from(&f.found.matches, from) {
                self.select_match(i);
                cx.notify();
            }
            return;
        };
        let (a, b) = (self.to_src(m.start()), self.to_src(m.end()));
        let matched = self.edit.as_ref().expect("editing").buf.slice(a, b);
        let text = q.expand(&matched, with);
        let text = self.edit.as_ref().expect("editing").buf.normalize(&text);
        self.apply_edits(vec![(a, b, text)], Kind::Other, None, cx);
        let at = self.sel.map_or(Pos::default(), |s| s.head);
        if let Some(f) = self.find.as_ref() {
            if let Some(i) = super::find::next_from(&f.found.matches, at) {
                self.select_match(i);
            }
        }
        cx.notify();
    }

    /// Replace every match (one undo step); how many.
    pub fn replace_all(&mut self, with: &str, cx: &mut Context<Self>) -> usize {
        let Some(f) = self.find.as_ref() else { return 0 };
        let Some(ed) = self.edit.as_ref() else { return 0 };
        let q = f.query.clone();
        let matches = f.found.matches.clone();
        if matches.is_empty() {
            return 0;
        }
        // Last first: earlier positions stay where they were.
        let edits: Vec<(SrcPos, SrcPos, String)> = matches
            .iter()
            .rev()
            .map(|m| {
                let (a, b) = (self.to_src(m.start()), self.to_src(m.end()));
                let text = ed.buf.normalize(&q.expand(&ed.buf.slice(a, b), with));
                (a, b, text)
            })
            .collect();
        let n = edits.len();
        let first = edits.last().map(|e| e.0).unwrap_or_default();
        self.apply_edits(edits, Kind::Other, Some(SrcSel::caret(first)), cx);
        n
    }

    // ── input method ────────────────────────────────────────────────────

    /// The line the input method sees (the caret's), as source text.
    fn ime_line(&self) -> Option<(usize, String)> {
        let ed = self.edit.as_ref()?;
        let head = self.src_sel().head;
        Some((head.line, ed.buf.line(head.line).to_string()))
    }

    /// A UTF-16 range on the caret's line, as source positions.
    fn ime_range(&self, r: &Range<usize>) -> Option<(SrcPos, SrcPos)> {
        let (line, t) = self.ime_line()?;
        Some((SrcPos::new(line, utf16_to_byte(&t, r.start)), SrcPos::new(line, utf16_to_byte(&t, r.end))))
    }

    /// Where the replaced text goes: the range given, else what is being
    /// composed, else the selection.
    fn ime_target(&self, range: Option<&Range<usize>>) -> Option<(SrcPos, SrcPos)> {
        let ed = self.edit.as_ref()?;
        range.and_then(|r| self.ime_range(r)).or(ed.marked).or_else(|| Some(self.src_sel().range()))
    }

    /// Where position `p` of the shown text is on screen.
    pub(super) fn screen_point(&self, p: Pos, window: &mut Window) -> Option<Bounds<Pixels>> {
        let g = self.geom.as_ref()?;
        let pane = g.panes.first()?;
        let row = self.rows.row_of(Side::New, p.line)?;
        let y = g.bounds.top() + self.top_pad() + px(self.rows.top(row)) - self.scroll_y;
        let line = &self.doc(Side::New).lines[p.line].text;
        let x = pane.text_left() - self.scroll_x + super::paint::x_for(window, line, p.col);
        Some(Bounds::new(point(x, y), size(px(1.), px(rows::LINE_H))))
    }
}

impl EntityInputHandler for CodeView {
    fn text_for_range(&mut self, range: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let (_, t) = self.ime_line()?;
        let (a, b) = (utf16_to_byte(&t, range.start), utf16_to_byte(&t, range.end));
        actual.replace(byte_to_utf16(&t, a)..byte_to_utf16(&t, b));
        Some(t[a..b.max(a)].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let (line, t) = self.ime_line()?;
        let s = self.src_sel();
        let anchor = if s.anchor.line == line { s.anchor.col } else { s.head.col };
        let (a, b) = (anchor.min(s.head.col), anchor.max(s.head.col));
        Some(UTF16Selection { range: byte_to_utf16(&t, a)..byte_to_utf16(&t, b), reversed: s.head.col < anchor })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let (a, b) = self.edit.as_ref()?.marked?;
        let (line, t) = self.ime_line()?;
        (a.line == line && b.line == line).then(|| byte_to_utf16(&t, a.col)..byte_to_utf16(&t, b.col))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ed) = self.edit.as_mut() {
            ed.marked = None;
            ed.buf.end_compose();
            cx.notify();
        }
    }

    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let Some((a, b)) = self.ime_target(range.as_ref()) else { return };
        let Some(ed) = self.edit.as_mut() else { return };
        let composing = ed.marked.take().is_some();
        let text = ed.buf.normalize(text);
        let one_char = text.chars().count() == 1 && !text.contains(['\n', '\r']);
        let kind = if composing { Kind::Compose } else if one_char && a == b { Kind::Typing } else { Kind::Other };
        self.apply_edits(vec![(a, b, text)], kind, None, cx);
        if let Some(ed) = self.edit.as_mut() {
            if composing {
                ed.buf.end_compose();
            }
        }
    }

    fn replace_and_mark_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, new_selected: Option<Range<usize>>, _: &mut Window, cx: &mut Context<Self>) {
        let Some((a, b)) = self.ime_target(range.as_ref()) else { return };
        let Some(ed) = self.edit.as_ref() else { return };
        let text = ed.buf.normalize(text);
        let end = SrcPos::new(a.line, a.col + text.len());
        let sel = new_selected
            .map(|r| {
                let (x, y) = (utf16_to_byte(&text, r.start), utf16_to_byte(&text, r.end));
                SrcSel { anchor: SrcPos::new(a.line, a.col + x), head: SrcPos::new(a.line, a.col + y) }
            })
            .unwrap_or(SrcSel::caret(end));
        let marked = (!text.is_empty() && !text.contains('\n')).then_some((a, end));
        self.apply_edits(vec![(a, b, text)], Kind::Compose, Some(sel), cx);
        if let Some(ed) = self.edit.as_mut() {
            ed.marked = marked;
        }
        cx.notify();
    }

    fn bounds_for_range(&mut self, range: Range<usize>, _: Bounds<Pixels>, window: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let (a, b) = self.ime_range(&range)?;
        let (pa, pb) = (self.to_shown(a), self.to_shown(b));
        let start = self.screen_point(pa, window)?;
        let end = self.screen_point(pb, window)?;
        Some(Bounds::from_corners(start.origin, point(end.origin.x.max(start.origin.x + px(1.)), end.bottom())))
    }

    fn character_index_for_point(&mut self, _: gpui::Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offsets_on_georgian_and_emoji() {
        let s = "aბ👍c";
        assert_eq!(byte_to_utf16(s, 1), 1);
        assert_eq!(byte_to_utf16(s, 4), 2, "ბ is one UTF-16 unit, three bytes");
        assert_eq!(byte_to_utf16(s, 8), 4, "👍 is two units, four bytes");
        assert_eq!(utf16_to_byte(s, 4), 8);
        assert_eq!(utf16_to_byte(s, 99), s.len());
    }

    #[test]
    fn spans_follow_an_edit_on_their_line() {
        let st = highlight::Style::default();
        let mut spans = vec![(0..3, st), (4..9, st), (10..12, st)];
        // "abc defgh ij": "X" typed inside the second span (at 6).
        shift_spans(&mut spans, 6, 0, 1);
        assert_eq!(spans.iter().map(|s| s.0.clone()).collect::<Vec<_>>(), [0..3, 4..10, 11..13]);
        // The first span deleted whole.
        shift_spans(&mut spans, 0, 3, 0);
        assert_eq!(spans.iter().map(|s| s.0.clone()).collect::<Vec<_>>(), [1..7, 8..10]);
    }
}
