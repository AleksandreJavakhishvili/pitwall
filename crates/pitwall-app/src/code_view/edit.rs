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

use super::buffer::{Buffer, CursorEdit, Kind, Splice, SrcPos, SrcSel};
use super::highlight::{self, Lang};
use super::rows;
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
        let text = text.into();
        if self.readonly_text != text {
            self.readonly_text = text;
            cx.notify();
        }
    }

    /// Editing is wanted ([`CodeView::set_editable`]).
    pub fn wants_editing(&self) -> bool {
        self.editable
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
    /// caret after the last edit). Other cursors go.
    pub(super) fn apply_edits(&mut self, edits: Vec<(SrcPos, SrcPos, String)>, kind: Kind, after: Option<SrcSel>, cx: &mut Context<Self>) {
        if self.edit.is_none() || edits.is_empty() {
            return;
        }
        let before = self.src_sel();
        let ed = self.edit.as_mut().expect("editing");
        let (splices, sel) = ed.buf.edit(edits, kind, before, after);
        self.changed(splices, vec![sel], 0, cx);
    }

    /// Every cursor in order, as source selections (overlapping ones
    /// merged), and which of them is the primary one.
    pub(super) fn cursors(&self) -> (Vec<SrcSel>, usize) {
        let primary = self.src_sel();
        if self.extra.is_empty() {
            return (vec![primary], 0);
        }
        let mut all: Vec<(SrcSel, bool)> = self.extra.iter().map(|s| (SrcSel { anchor: self.to_src(s.anchor), head: self.to_src(s.head) }, false)).collect();
        all.push((primary, true));
        all.sort_by_key(|(s, _)| s.range());
        let mut out: Vec<(SrcSel, bool)> = Vec::with_capacity(all.len());
        for (s, p) in all {
            match out.last_mut() {
                Some((last, lp)) if s.range().0 < last.range().1 || s.range() == last.range() => {
                    // Overlapping: one selection over both.
                    let (a, b) = (last.range().0, last.range().1.max(s.range().1));
                    *last = if last.anchor <= last.head { SrcSel { anchor: a, head: b } } else { SrcSel { anchor: b, head: a } };
                    *lp |= p;
                }
                _ => out.push((s, p)),
            }
        }
        let primary = out.iter().position(|(_, p)| *p).unwrap_or(0);
        (out.into_iter().map(|(s, _)| s).collect(), primary)
    }

    /// One edit at every cursor (`f` gives each cursor's, or none: that
    /// cursor stays where it is), as one undo step.
    pub(super) fn edit_each(&mut self, kind: Kind, f: impl Fn(&Buffer, SrcSel) -> Option<CursorEdit>, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_ref() else { return };
        let (sels, primary) = self.cursors();
        let mut edits: Vec<(usize, CursorEdit)> = sels.iter().enumerate().filter_map(|(i, s)| f(&ed.buf, *s).map(|e| (i, e))).collect();
        edits.sort_by_key(|(_, e)| (e.from.min(e.to), e.from.max(e.to)));
        // Two cursors can't change the same text (deleting toward each
        // other): the first one does.
        let mut kept: Vec<(usize, CursorEdit)> = Vec::with_capacity(edits.len());
        for (i, e) in edits {
            let a = e.from.min(e.to);
            if kept.last().is_some_and(|(_, k)| a < k.from.max(k.to)) {
                continue;
            }
            kept.push((i, e));
        }
        if kept.is_empty() {
            return;
        }
        let primary_edit = kept.iter().position(|(i, _)| *i == primary).unwrap_or(0);
        let raw: Vec<(SrcPos, SrcPos, String)> = kept.iter().map(|(_, e)| (e.from.min(e.to), e.from.max(e.to), e.text.clone())).collect();
        let ed = self.edit.as_mut().expect("editing");
        let (splices, moved) = ed.buf.edit_many(raw, primary_edit, kind, sels[primary]);
        let out: Vec<SrcSel> = sels
            .iter()
            .enumerate()
            .map(|(ci, s)| match kept.iter().position(|(i, _)| *i == ci) {
                Some(k) => {
                    let start = moved.range(k).map_or(s.head, |r| r.0);
                    let e = &kept[k].1;
                    let at = |off: usize| super::buffer::end_of(start, &e.text[..text::clamp_to_char(&e.text, off.min(e.text.len()))]);
                    SrcSel { anchor: at(e.anchor), head: at(e.head) }
                }
                None => SrcSel { anchor: moved.map(s.anchor), head: moved.map(s.head) },
            })
            .collect();
        ed.buf.set_after(out[primary]);
        self.changed(splices, out, primary, cx);
    }

    /// Edits over the lines every cursor covers (`f` gets them in order),
    /// as one undo step; the cursors keep their places in the text (a
    /// line's start stays its start).
    fn edit_lines(&mut self, f: impl Fn(&Buffer, &[usize]) -> Vec<(SrcPos, SrcPos, String)>, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_ref() else { return };
        let (sels, primary) = self.cursors();
        let mut lines: Vec<usize> = sels
            .iter()
            .flat_map(|s| {
                let (a, b) = s.range();
                let last = if b.line > a.line && b.col == 0 { b.line - 1 } else { b.line };
                a.line..=last
            })
            .collect();
        lines.sort_unstable();
        lines.dedup();
        let mut edits = f(&ed.buf, &lines);
        if edits.is_empty() {
            return;
        }
        edits.sort_by_key(|(a, b, _)| (*a, *b));
        let ed = self.edit.as_mut().expect("editing");
        let (splices, moved) = ed.buf.edit_many(edits, 0, Kind::Other, sels[primary]);
        let keep = |p: SrcPos| {
            let m = moved.map(p);
            if p.col == 0 {
                SrcPos::new(m.line, 0)
            } else {
                m
            }
        };
        let out: Vec<SrcSel> = sels.iter().map(|s| SrcSel { anchor: keep(s.anchor), head: keep(s.head) }).collect();
        ed.buf.set_after(out[primary]);
        self.changed(splices, out, primary, cx);
    }

    /// Replace the selection (each cursor's) with `text` (from outside:
    /// its line breaks become the file's).
    pub(super) fn insert(&mut self, text: &str, kind: Kind, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_ref() else { return };
        let text = ed.buf.normalize(text);
        self.edit_each(
            kind,
            |_, s| {
                let (a, b) = s.range();
                Some(CursorEdit::new(a, b, text.clone()))
            },
            cx,
        );
    }

    /// Text typed (not composed) at every cursor: in the editor, a bracket
    /// or a quote comes with its partner.
    pub(super) fn typed(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_ref() else { return };
        let text = ed.buf.normalize(text);
        let mut chars = text.chars();
        let one = match (chars.next(), chars.next()) {
            (Some(c), None) if c != '\n' && c != '\r' => Some(c),
            _ => None,
        };
        let pairs = self.editor && one.is_some() && !self.no_quote_pair(one);
        // A single character is typing (a run, the selection it replaced
        // with it, undoes together).
        let kind = if one.is_some() { Kind::Typing } else { Kind::Other };
        self.edit_each(
            kind,
            |buf, s| {
                let (a, b) = s.range();
                let paired = one.filter(|_| pairs).and_then(|c| buf.typed_pair(a, b, c));
                Some(paired.unwrap_or_else(|| CursorEdit::new(a, b, text.clone())))
            },
            cx,
        );
    }

    /// An apostrophe isn't paired where it is mostly one (Rust's lifetimes,
    /// prose).
    fn no_quote_pair(&self, c: Option<char>) -> bool {
        if c != Some('\'') {
            return false;
        }
        let path = self.content.as_ref().map(|c| c.path.to_string()).unwrap_or_default();
        matches!(Lang::for_path(&path), Some(Lang::Rust | Lang::Markdown | Lang::OCaml) | None)
    }

    /// The buffer changed: the shown lines, highlighting, rows, the
    /// cursors and Find follow. `sels` are every cursor's, `primary` the
    /// one the view follows.
    fn changed(&mut self, splices: Vec<Splice>, sels: Vec<SrcSel>, primary: usize, cx: &mut Context<Self>) {
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
        self.rebuild_rows();
        let shown: Vec<Selection> = sels.iter().map(|s| Selection { side: Side::New, anchor: self.to_shown(s.anchor), head: self.to_shown(s.head) }).collect();
        let head = shown.get(primary).map_or(Pos::default(), |s| s.head);
        self.sel = shown.get(primary).copied();
        self.extra = shown.iter().enumerate().filter(|(i, _)| *i != primary).map(|(_, s)| *s).collect();
        self.goal_x = None;
        self.readonly_at = None;
        self.caret_x = true;
        if let Some(row) = self.row_of_pos(head) {
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

    /// Delete each selection, or from each caret to where `to` says (on
    /// the caret's source line; `None`: across the line break).
    fn delete_by(&mut self, to: impl Fn(&str, usize) -> Option<usize>, forward: bool, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_ref() else { return };
        let (sels, primary) = self.cursors();
        // A single character from the primary caret is typing (a run of
        // them undoes together).
        let p = sels[primary].range();
        let line = ed.buf.line(p.0.line);
        let one = p.0 == p.1
            && to(line, p.0.col).is_some_and(|c| {
                let (x, y) = (c.min(p.0.col), c.max(p.0.col));
                line.get(x..y).is_some_and(|t| t.chars().count() == 1)
            });
        let pairs = self.editor && !forward;
        self.edit_each(
            if one { Kind::Typing } else { Kind::Other },
            |buf, s| {
                let (a, b) = s.range();
                if a != b {
                    return Some(CursorEdit::new(a, b, String::new()));
                }
                let line = buf.line(a.line);
                let other = match to(line, a.col) {
                    Some(col) => SrcPos::new(a.line, col),
                    None if forward && a.line + 1 < buf.line_count() => SrcPos::new(a.line + 1, 0),
                    None if !forward && a.line > 0 => SrcPos::new(a.line - 1, buf.line(a.line - 1).len()),
                    None => return None,
                };
                if other == a {
                    return None;
                }
                let (x, mut y) = if other < a { (other, a) } else { (a, other) };
                // Backspace inside an empty pair: both go.
                if pairs && y == a && x.line == a.line && buf.empty_pair_at(a) && line.get(x.col..a.col).is_some_and(|t| t.chars().count() == 1) {
                    let next = line[a.col..].chars().next().map_or(0, char::len_utf8);
                    y = SrcPos::new(a.line, a.col + next);
                }
                Some(CursorEdit::new(x, y, String::new()))
            },
            cx,
        );
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

    /// Enter: a line break and the line's indentation at each cursor (in
    /// the editor, one step more after an opening bracket).
    pub(super) fn newline(&mut self, cx: &mut Context<Self>) {
        let smart = self.editor;
        let colon = matches!(self.content.as_ref().and_then(|c| Lang::for_path(&c.path)), Some(Lang::Python | Lang::Yaml | Lang::Nix));
        self.edit_each(
            Kind::Other,
            |buf, s| {
                let (a, b) = s.range();
                Some(if smart { buf.smart_newline(a, b, colon) } else { CursorEdit::new(a, b, buf.newline_at(a)) })
            },
            cx,
        );
    }

    /// Tab: one indentation step in at each cursor; over selected lines,
    /// each of them.
    pub(super) fn indent(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_ref() else { return };
        let multiline = self.cursors().0.iter().any(|s| s.anchor.line != s.head.line);
        if !multiline {
            let unit = ed.buf.unit();
            return self.edit_each(
                Kind::Other,
                |_, s| {
                    let (a, b) = s.range();
                    Some(CursorEdit::new(a, b, unit.clone()))
                },
                cx,
            );
        }
        self.edit_lines(|buf, lines| lines.iter().flat_map(|&l| buf.indent_edits(l, l)).collect(), cx);
    }

    /// Shift-Tab: one indentation step out on each line covered.
    pub(super) fn outdent(&mut self, cx: &mut Context<Self>) {
        self.edit_lines(|buf, lines| lines.iter().flat_map(|&l| buf.outdent_edits(l, l)).collect(), cx);
    }

    /// ⌘/: the lines of every cursor commented out, or back in (by the
    /// file's language; nothing for a language without comments).
    pub fn toggle_comment(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.content.as_ref().and_then(|c| super::buffer::comment_syntax(&c.path)) else { return };
        self.edit_lines(|buf, lines| buf.comment_edits(lines, c), cx);
    }

    /// What each cursor copies or cuts: its selection, or (nothing
    /// selected) its whole line once; in order (`true`: a whole line).
    fn cut_ranges(&self) -> Vec<(SrcPos, SrcPos, bool)> {
        let Some(ed) = self.edit.as_ref() else { return Vec::new() };
        let mut out: Vec<(SrcPos, SrcPos, bool)> = Vec::new();
        for s in self.cursors().0 {
            let (a, b) = s.range();
            if a != b {
                out.push((a, b, false));
                continue;
            }
            if out.last().is_some_and(|l| l.2 && l.0.line == a.line) {
                continue;
            }
            if a.line + 1 < ed.buf.line_count() {
                out.push((SrcPos::new(a.line, 0), SrcPos::new(a.line + 1, 0), true));
            } else {
                out.push((SrcPos::new(a.line, 0), SrcPos::new(a.line, ed.buf.line(a.line).len()), true));
            }
        }
        out
    }

    /// The text ⌘C copies with the cursors there are (editing only).
    pub(super) fn copied_text(&self) -> Option<String> {
        let ed = self.edit.as_ref()?;
        let parts: Vec<String> = self
            .cut_ranges()
            .iter()
            .map(|&(a, b, whole)| {
                let mut t = ed.buf.slice(a, b).replace("\r\n", "\n");
                if whole && !t.ends_with('\n') {
                    t.push('\n');
                }
                t
            })
            .collect();
        if parts.len() == 1 {
            return parts.into_iter().next();
        }
        Some(parts.iter().map(|p| p.strip_suffix('\n').unwrap_or(p)).collect::<Vec<_>>().join("\n"))
    }

    pub(super) fn cut(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.copied_text() else { return };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let ranges = self.cut_ranges();
        let edits: Vec<(SrcPos, SrcPos, String)> = ranges.iter().map(|&(a, b, _)| (a, b, String::new())).collect();
        if edits.len() == 1 && self.extra.is_empty() {
            let from = edits[0].0;
            return self.apply_edits(edits, Kind::Other, Some(SrcSel::caret(from)), cx);
        }
        let Some(ed) = self.edit.as_mut() else { return };
        let before = SrcSel::caret(edits[0].0);
        let (splices, moved) = ed.buf.edit_many(edits, 0, Kind::Other, before);
        let out: Vec<SrcSel> = (0..ranges.len()).filter_map(|i| moved.range(i)).map(|r| SrcSel::caret(r.0)).collect();
        self.changed(splices, out, 0, cx);
    }

    /// ⌘V: the clipboard at each cursor; as many lines as cursors: a line
    /// each.
    pub(super) fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return };
        let n = self.cursors().0.len();
        let lines: Vec<&str> = text.strip_suffix('\n').unwrap_or(&text).split('\n').collect();
        if n > 1 && lines.len() == n {
            let Some(ed) = self.edit.as_ref() else { return };
            let parts: Vec<String> = lines.iter().map(|l| ed.buf.normalize(l.strip_suffix('\r').unwrap_or(l))).collect();
            let next = std::cell::Cell::new(0usize);
            return self.edit_each(
                Kind::Other,
                |_, s| {
                    let i = next.get();
                    next.set(i + 1);
                    let (a, b) = s.range();
                    Some(CursorEdit::new(a, b, parts.get(i).cloned().unwrap_or_default()))
                },
                cx,
            );
        }
        self.insert(&text, Kind::Other, cx);
    }

    pub(super) fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_mut() else { return };
        ed.marked = None;
        if let Some((splices, sel)) = ed.buf.undo() {
            self.changed(splices, vec![sel], 0, cx);
        }
    }

    pub(super) fn redo(&mut self, cx: &mut Context<Self>) {
        let Some(ed) = self.edit.as_mut() else { return };
        ed.marked = None;
        if let Some((splices, sel)) = ed.buf.redo() {
            self.changed(splices, vec![sel], 0, cx);
        }
    }

    // ── the editor's way ────────────────────────────────────────────────

    /// The editor's way of editing on or off ([`CodeView::is_editor`]).
    pub fn set_editor(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.editor != on {
            self.editor = on;
            cx.notify();
        }
    }

    /// The editor's way: the cursor's line marked, brackets and quotes in
    /// pairs, Enter indenting after an opening bracket.
    pub fn is_editor(&self) -> bool {
        self.editor
    }

    /// The margin's marks of lines changed since git's HEAD.
    pub fn set_line_marks(&mut self, marks: Vec<(Range<usize>, super::LineMark)>, cx: &mut Context<Self>) {
        if self.line_marks != marks {
            self.line_marks = marks;
            cx.notify();
        }
    }

    pub fn line_marks(&self) -> &[(Range<usize>, super::LineMark)] {
        &self.line_marks
    }

    /// What error and warning marks say, drawn after their first line's
    /// text: (line, 0-based; the mark; its message).
    pub fn set_line_notes(&mut self, notes: Vec<(usize, super::LineMark, SharedString)>, cx: &mut Context<Self>) {
        if self.line_notes != notes {
            self.line_notes = notes;
            cx.notify();
        }
    }

    pub fn line_notes(&self) -> &[(usize, super::LineMark, SharedString)] {
        &self.line_notes
    }

    /// How many cursors there are.
    pub fn cursor_count(&self) -> usize {
        usize::from(self.sel.is_some()) + self.extra.len()
    }

    /// The primary cursor: its 0-based line and its character column
    /// (`None`: no file, or no cursor yet).
    pub fn caret(&self) -> Option<(usize, usize)> {
        let s = self.sel?;
        let line = self.content.as_ref()?.new.line(s.head.line)?;
        let src = line.source();
        let col = line.to_source(s.head.col).min(src.len());
        Some((s.head.line, src[..text::clamp_to_char(src, col)].chars().count()))
    }

    /// The file's indentation, whether it breaks lines with CRLF, and its
    /// line count (the editor's status line).
    pub fn file_facts(&self) -> Option<(super::text::Indent, bool, usize)> {
        let ed = self.edit.as_ref()?;
        Some((ed.buf.indent, ed.buf.crlf(), ed.buf.line_count()))
    }

    /// Every cursor's head, in order: a line and a source byte column
    /// (tests).
    pub fn cursor_heads(&self) -> Vec<(usize, usize)> {
        if self.content.is_none() || self.sel.is_none() {
            return Vec::new();
        }
        self.cursors().0.iter().map(|s| (s.head.line, s.head.col)).collect()
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
        if !composing && range.is_none() {
            // Typed: at every cursor.
            return self.typed(text, cx);
        }
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
        // Composing happens at the primary cursor only.
        self.extra.clear();
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
