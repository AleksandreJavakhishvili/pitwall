//! The editable text of one file (the explorer's viewer, when editing is
//! on): the file as it was read, split at `\n` only, so every line keeps
//! its own `\r` and the last line says whether there is a final newline;
//! nothing changes in a line no edit touched. Positions are source byte
//! columns on a line's text without its `\r`. Inserted line breaks take the
//! file's own (`\r\n` when most of its lines have one).
//!
//! Undo and redo work on groups of edits (a run of typing, one paste, a
//! whole Replace All or block indent), each kept as what was removed and
//! inserted, never as copies of the file. Pure: tests run without a window.

use std::time::{Duration, Instant};

use super::text::{clamp_to_char, Indent};

/// A place in the source: a line and a byte column on its text (without
/// its `\r`), on a char boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct SrcPos {
    pub line: usize,
    pub col: usize,
}

impl SrcPos {
    pub fn new(line: usize, col: usize) -> SrcPos {
        SrcPos { line, col }
    }
}

/// The selection, in source positions (kept with undo steps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SrcSel {
    pub anchor: SrcPos,
    pub head: SrcPos,
}

impl SrcSel {
    pub fn caret(p: SrcPos) -> SrcSel {
        SrcSel { anchor: p, head: p }
    }

    pub fn range(&self) -> (SrcPos, SrcPos) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }
}

/// Lines `first .. first + old` were replaced by `new` lines (applied in
/// order: each one's line numbers are the text's after the ones before).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Splice {
    pub first: usize,
    pub old: usize,
    pub new: usize,
    /// A change within one line: at `col`, `removed` bytes became
    /// `inserted` (for keeping its highlighting while it is redone).
    pub inline: Option<(usize, usize, usize)>,
}

/// How an edit joins the undo history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One typed character: a run of them is one step.
    Typing,
    /// An input method's text while composing (one step until committed).
    Compose,
    /// Anything else: a step of its own.
    Other,
}

#[derive(Debug, Clone)]
struct Edit {
    at: SrcPos,
    removed: String,
    inserted: String,
}

#[derive(Debug, Clone)]
struct Group {
    id: u64,
    kind: Kind,
    edits: Vec<Edit>,
    before: SrcSel,
    after: SrcSel,
    at: Instant,
}

/// A typing run joins within this long.
const RUN: Duration = Duration::from_secs(1);
/// Undo steps kept.
const MAX_UNDO: usize = 1000;

#[derive(Debug, Clone)]
pub struct Buffer {
    /// The file split at `\n`; a line may end in `\r`.
    lines: Vec<String>,
    /// New line breaks are `\r\n`.
    crlf: bool,
    pub indent: Indent,
    undo: Vec<Group>,
    redo: Vec<Group>,
    next_id: u64,
    /// The undo step the file on disk matches (0: as read).
    saved: Option<u64>,
    /// No typing run continues past a save or a selection change.
    run_open: bool,
    /// Changes since it was made (for the highlighter and the Preview).
    pub version: u64,
}

impl Buffer {
    pub fn new(text: &str) -> Buffer {
        let lines: Vec<String> = text.split('\n').map(String::from).collect();
        let breaks = lines.len() - 1;
        let cr = lines[..breaks].iter().filter(|l| l.ends_with('\r')).count();
        Buffer {
            crlf: breaks > 0 && cr * 2 > breaks,
            indent: super::text::detect_indent(text),
            lines,
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 1,
            saved: Some(0),
            run_open: false,
            version: 0,
        }
    }

    /// The whole file, as it would be written.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Line `i` as written (with its `\r`, if any).
    pub fn raw_line(&self, i: usize) -> &str {
        self.lines.get(i).map_or("", String::as_str)
    }

    /// Line `i`'s text, without its `\r`.
    pub fn line(&self, i: usize) -> &str {
        let l = self.raw_line(i);
        l.strip_suffix('\r').unwrap_or(l)
    }

    pub fn crlf(&self) -> bool {
        self.crlf
    }

    /// The text differs from the file as read or last saved.
    pub fn dirty(&self) -> bool {
        self.saved != Some(self.current())
    }

    /// The file on disk now holds this text.
    pub fn mark_saved(&mut self) {
        self.saved = Some(self.current());
        self.run_open = false;
    }

    /// Typing after a click or a move starts a new undo step.
    pub fn break_run(&mut self) {
        self.run_open = false;
    }

    fn current(&self) -> u64 {
        self.undo.last().map_or(0, |g| g.id)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// `p` moved onto the text: an existing line, a char boundary.
    pub fn clamp(&self, p: SrcPos) -> SrcPos {
        let line = p.line.min(self.lines.len() - 1);
        let t = self.line(line);
        SrcPos::new(line, clamp_to_char(t, p.col.min(t.len())))
    }

    pub fn end(&self) -> SrcPos {
        let last = self.lines.len() - 1;
        SrcPos::new(last, self.line(last).len())
    }

    /// The text between two positions (line breaks as written).
    pub fn slice(&self, a: SrcPos, b: SrcPos) -> String {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        if a.line == b.line {
            return self.line(a.line)[a.col..b.col].to_string();
        }
        let mut out = String::new();
        out.push_str(&self.raw_line(a.line)[a.col..]);
        for l in a.line + 1..b.line {
            out.push('\n');
            out.push_str(self.raw_line(l));
        }
        out.push('\n');
        out.push_str(&self.line(b.line)[..b.col]);
        out
    }

    /// Text from outside (typing, a paste): its line breaks become the
    /// file's own.
    pub fn normalize(&self, text: &str) -> String {
        let lf = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.crlf {
            lf.replace('\n', "\r\n")
        } else {
            lf
        }
    }

    /// Replace `a..b` with `text` (taken as it is: see [`normalize`]).
    /// Returns the splice and where the inserted text ends.
    fn replace_raw(&mut self, a: SrcPos, b: SrcPos, text: &str) -> (Splice, SrcPos, String) {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        let removed = self.slice(a, b);
        // What follows `b` on its line, with that line's own `\r`.
        let tail = self.raw_line(b.line)[b.col..].to_string();
        let head = &self.lines[a.line][..a.col];
        let joined = format!("{head}{text}{tail}");
        let new_lines: Vec<String> = joined.split('\n').map(String::from).collect();
        let n = new_lines.len();
        let old = b.line - a.line + 1;
        self.lines.splice(a.line..=b.line, new_lines);
        let end = match text.rfind('\n') {
            Some(i) => SrcPos::new(a.line + n - 1, text.len() - i - 1),
            None => SrcPos::new(a.line, a.col + text.len()),
        };
        // The end column on a line that now ends in `\r` + text: as bytes.
        let end = SrcPos::new(end.line, end.col.min(self.line(end.line).len()));
        let inline = (old == 1 && n == 1).then_some((a.col, removed.len(), text.len()));
        self.version += 1;
        (Splice { first: a.line, old, new: n, inline }, end, removed)
    }

    /// Apply `edits` (each `(from, to, text)`, positions in the text as it
    /// is when that edit runs) as one undo step. `before` and the returned
    /// selection are kept with it; the selection after is the end of the
    /// last edit unless `after` says otherwise.
    pub fn edit(&mut self, edits: Vec<(SrcPos, SrcPos, String)>, kind: Kind, before: SrcSel, after: Option<SrcSel>) -> (Vec<Splice>, SrcSel) {
        let mut splices = Vec::new();
        let mut done = Vec::new();
        let mut end = before.head;
        for (a, b, text) in edits {
            let at = self.clamp(a.min(b));
            let (s, e, removed) = self.replace_raw(a, b, &text);
            splices.push(s);
            end = e;
            done.push(Edit { at, removed, inserted: text });
        }
        let after = after.unwrap_or(SrcSel::caret(end));
        self.redo.clear();
        let now = Instant::now();
        let joins = match (self.undo.last(), kind) {
            (Some(g), Kind::Typing) if self.run_open && g.kind == Kind::Typing && now.duration_since(g.at) < RUN => {
                g.after.head == done.first().map_or(after.head, |e| e.at)
            }
            (Some(g), Kind::Compose) => g.kind == Kind::Compose && self.run_open,
            _ => false,
        };
        // Never join the step the file on disk matches.
        let joins = joins && self.saved != Some(self.current());
        if joins {
            let g = self.undo.last_mut().expect("a step");
            g.edits.extend(done);
            g.after = after;
            g.at = now;
        } else {
            let id = self.next_id;
            self.next_id += 1;
            self.undo.push(Group { id, kind, edits: done, before, after, at: now });
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.run_open = matches!(kind, Kind::Typing | Kind::Compose);
        (splices, after)
    }

    /// Composing is over: the next edit starts a new step.
    pub fn end_compose(&mut self) {
        if let Some(g) = self.undo.last_mut() {
            if g.kind == Kind::Compose {
                g.kind = Kind::Other;
            }
        }
        self.run_open = false;
    }

    /// Undo the last step: the splices and the selection it had before.
    pub fn undo(&mut self) -> Option<(Vec<Splice>, SrcSel)> {
        let g = self.undo.pop()?;
        let mut splices = Vec::new();
        for e in g.edits.iter().rev() {
            let to = end_of(e.at, &e.inserted);
            let (s, _, _) = self.replace_raw(e.at, to, &e.removed);
            splices.push(s);
        }
        let sel = g.before;
        self.redo.push(g);
        self.run_open = false;
        Some((splices, sel))
    }

    pub fn redo(&mut self) -> Option<(Vec<Splice>, SrcSel)> {
        let g = self.redo.pop()?;
        let mut splices = Vec::new();
        for e in &g.edits {
            let to = end_of(e.at, &e.removed);
            let (s, _, _) = self.replace_raw(e.at, to, &e.inserted);
            splices.push(s);
        }
        let sel = g.after;
        self.undo.push(g);
        self.run_open = false;
        Some((splices, sel))
    }

    /// Replace the whole text with `text` (the file on disk, taken): no
    /// undo history, clean.
    pub fn reset(&mut self, text: &str) {
        *self = Buffer { version: self.version + 1, ..Buffer::new(text) };
    }

    // ── what keys do ────────────────────────────────────────────────────

    /// One indentation step of this file.
    pub fn unit(&self) -> String {
        match self.indent {
            Indent::Tabs => "\t".into(),
            Indent::Spaces(n) => " ".repeat(n),
        }
    }

    /// Enter: a line break, then the line's indentation (up to the caret).
    pub fn newline_at(&self, p: SrcPos) -> String {
        let line = self.line(p.line);
        let upto = &line[..p.col.min(line.len())];
        let lead: String = upto.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        self.normalize(&format!("\n{lead}"))
    }

    /// Tab over lines `first..=last`: one step more at the start of each
    /// line that has text.
    pub fn indent_edits(&self, first: usize, last: usize) -> Vec<(SrcPos, SrcPos, String)> {
        let unit = self.unit();
        (first..=last)
            .rev()
            .filter(|&l| !self.line(l).trim().is_empty())
            .map(|l| (SrcPos::new(l, 0), SrcPos::new(l, 0), unit.clone()))
            .collect()
    }

    /// Shift-Tab over lines `first..=last`: up to one step less (a tab, or
    /// the file's number of spaces).
    pub fn outdent_edits(&self, first: usize, last: usize) -> Vec<(SrcPos, SrcPos, String)> {
        let width = self.indent.width();
        (first..=last)
            .rev()
            .filter_map(|l| {
                let t = self.line(l);
                let n = if t.starts_with('\t') {
                    1
                } else {
                    t.bytes().take(width).take_while(|b| *b == b' ').count()
                };
                (n > 0).then(|| (SrcPos::new(l, 0), SrcPos::new(l, n), String::new()))
            })
            .collect()
    }
}

/// Where `text` ends when inserted at `at`.
fn end_of(at: SrcPos, text: &str) -> SrcPos {
    match text.rfind('\n') {
        Some(i) => SrcPos::new(at.line + text.matches('\n').count(), text.len() - i - 1),
        None => SrcPos::new(at.line, at.col + text.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(l: usize, c: usize) -> SrcPos {
        SrcPos::new(l, c)
    }

    fn put(b: &mut Buffer, a: SrcPos, z: SrcPos, t: &str, kind: Kind) -> SrcSel {
        let t = b.normalize(t);
        b.edit(vec![(a, z, t)], kind, SrcSel::caret(a), None).1
    }

    #[test]
    fn untouched_lines_keep_their_endings_and_the_final_newline() {
        let src = "one\r\ntwo\nthree\r\n";
        let mut b = Buffer::new(src);
        assert_eq!(b.text(), src);
        put(&mut b, p(1, 3), p(1, 3), "!", Kind::Other);
        assert_eq!(b.text(), "one\r\ntwo!\nthree\r\n");
        // No final newline stays without one.
        let mut b = Buffer::new("a\nb");
        put(&mut b, p(1, 1), p(1, 1), "c", Kind::Other);
        assert_eq!(b.text(), "a\nbc");
    }

    #[test]
    fn new_line_breaks_are_the_files_own() {
        let mut b = Buffer::new("a\r\nb\r\nc\r\n");
        assert!(b.crlf());
        put(&mut b, p(0, 1), p(0, 1), "x\ny", Kind::Other);
        assert_eq!(b.text(), "ax\r\ny\r\nb\r\nc\r\n");
        // A pasted CRLF in an LF file: LF.
        let mut b = Buffer::new("a\nb\n");
        put(&mut b, p(1, 0), p(1, 0), "1\r\n2\r\n", Kind::Other);
        assert_eq!(b.text(), "a\n1\n2\nb\n");
        // Deleting across a CRLF break and undoing brings it back.
        let mut b = Buffer::new("ab\r\ncd\r\n");
        put(&mut b, p(0, 1), p(1, 1), "", Kind::Other);
        assert_eq!(b.text(), "ad\r\n");
        b.undo();
        assert_eq!(b.text(), "ab\r\ncd\r\n");
    }

    #[test]
    fn typing_is_one_undo_step_and_saving_marks_it_clean() {
        let mut b = Buffer::new("x\n");
        for (i, ch) in "hello".chars().enumerate() {
            put(&mut b, p(0, i), p(0, i), &ch.to_string(), Kind::Typing);
        }
        assert_eq!(b.text(), "hellox\n");
        assert!(b.dirty());
        let (_, sel) = b.undo().unwrap();
        assert_eq!(b.text(), "x\n");
        assert_eq!(sel, SrcSel::caret(p(0, 0)));
        assert!(!b.dirty(), "back to the file as read");
        b.redo();
        assert_eq!(b.text(), "hellox\n");
        b.mark_saved();
        assert!(!b.dirty());
        // Typing after a save is a new step: undo goes back to the saved text.
        put(&mut b, p(0, 5), p(0, 5), "!", Kind::Typing);
        assert!(b.dirty());
        b.undo();
        assert!(!b.dirty());
        assert_eq!(b.text(), "hellox\n");
        // A break (a click) ends a run.
        put(&mut b, p(0, 0), p(0, 0), "a", Kind::Typing);
        b.break_run();
        put(&mut b, p(0, 1), p(0, 1), "b", Kind::Typing);
        b.undo();
        assert_eq!(b.text(), "ahellox\n");
    }

    #[test]
    fn several_edits_are_one_step() {
        let mut b = Buffer::new("a\nb\nc\n");
        let edits = b.indent_edits(0, 2);
        b.edit(edits, Kind::Other, SrcSel::default(), None);
        assert_eq!(b.text(), "    a\n    b\n    c\n");
        b.undo();
        assert_eq!(b.text(), "a\nb\nc\n");
    }

    #[test]
    fn indentation_uses_the_files_own() {
        let mut b = Buffer::new("fn x() {\n\tlet a = 1;\n\tif a {\n\t\tb();\n\t}\n}\n");
        assert_eq!(b.unit(), "\t");
        assert_eq!(b.newline_at(p(3, 4)), "\n\t\t");
        let e = b.indent_edits(0, 1);
        b.edit(e, Kind::Other, SrcSel::default(), None);
        assert!(b.text().starts_with("\tfn x() {\n\t\tlet a"));
        let e = b.outdent_edits(0, 1);
        b.edit(e, Kind::Other, SrcSel::default(), None);
        assert!(b.text().starts_with("fn x() {\n\tlet a"));
        // Two-space files.
        let b = Buffer::new("a:\n  b:\n    c\n  d\n");
        assert_eq!(b.unit(), "  ");
        assert_eq!(b.outdent_edits(2, 2), vec![(p(2, 0), p(2, 2), String::new())]);
        // A blank line isn't indented.
        assert!(b.indent_edits(0, 0).len() == 1 && Buffer::new("\n").indent_edits(0, 0).is_empty());
    }

    #[test]
    fn georgian_and_combining_text() {
        let mut b = Buffer::new("გამარჯობა\n");
        // After the third letter (3 bytes each).
        put(&mut b, p(0, 9), p(0, 9), "ე\u{301}", Kind::Other);
        assert_eq!(b.line(0), "გამე\u{301}არჯობა");
        // A column inside a letter is clamped to its start.
        assert_eq!(b.clamp(p(0, 4)), p(0, 3));
        let s = b.slice(p(0, 3), p(0, 9));
        assert_eq!(s, "ამ");
        put(&mut b, p(0, 9), p(0, 14), "", Kind::Other);
        assert_eq!(b.line(0), "გამარჯობა");
    }

    #[test]
    fn splices_say_which_lines_changed() {
        let mut b = Buffer::new("a\nb\nc");
        let (s, sel) = b.edit(vec![(p(0, 1), p(2, 0), "X\nY\nZ\nW".into())], Kind::Other, SrcSel::default(), None);
        assert_eq!(s, vec![Splice { first: 0, old: 3, new: 4, inline: None }]);
        assert_eq!(b.text(), "aX\nY\nZ\nWc");
        assert_eq!(sel.head, p(3, 1));
        let (s, _) = b.edit(vec![(p(1, 0), p(1, 1), "yy".into())], Kind::Other, SrcSel::default(), None);
        assert_eq!(s[0].inline, Some((0, 1, 2)));
    }
}
