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
        let join_at = edits.first().map(|(a, b, _)| self.clamp((*a).min(*b)));
        self.edit_inner(edits, kind, before, after, join_at)
    }

    /// Edits at several places at once (one per cursor, or one per line of
    /// a block): `edits` are ranges of the text as it is now, in order and
    /// apart. One undo step; a typing run goes on when the primary cursor's
    /// edit (`primary`) starts where the last step left it. Returns the
    /// splices and where every edit's text is in the text after.
    pub fn edit_many(&mut self, edits: Vec<(SrcPos, SrcPos, String)>, primary: usize, kind: Kind, before: SrcSel) -> (Vec<Splice>, Moved) {
        let edits: Vec<(SrcPos, SrcPos, String)> = edits.into_iter().map(|(a, b, t)| (self.clamp(a.min(b)), self.clamp(a.max(b)), t)).collect();
        let moved = Moved::new(&edits);
        let join_at = edits.get(primary).map(|e| e.0);
        let after = moved.spans.get(primary).map_or(before, |s| SrcSel::caret(s.3));
        // Last first: each edit's positions are still the text's as it is.
        let (splices, _) = self.edit_inner(edits.into_iter().rev().collect(), kind, before, Some(after), join_at);
        (splices, moved)
    }

    /// The selection the last undo step leaves (its redo puts it back).
    pub fn set_after(&mut self, after: SrcSel) {
        if let Some(g) = self.undo.last_mut() {
            g.after = after;
        }
    }

    fn edit_inner(&mut self, edits: Vec<(SrcPos, SrcPos, String)>, kind: Kind, before: SrcSel, after: Option<SrcSel>, join_at: Option<SrcPos>) -> (Vec<Splice>, SrcSel) {
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
                g.after.head == join_at.unwrap_or(after.head)
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

/// Where the texts of several edits went ([`Buffer::edit_many`]), and
/// where any other place of the text before them is now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Moved {
    /// Each edit: its range before, then its text's range after.
    spans: Vec<(SrcPos, SrcPos, SrcPos, SrcPos)>,
}

impl Moved {
    /// `edits` in order and apart, in the text before them.
    fn new(edits: &[(SrcPos, SrcPos, String)]) -> Moved {
        let mut spans: Vec<(SrcPos, SrcPos, SrcPos, SrcPos)> = Vec::with_capacity(edits.len());
        for (a, b, text) in edits {
            let na = match spans.last() {
                Some(&(_, ob, _, ne)) => shift(*a, ob, ne),
                None => *a,
            };
            spans.push((*a, *b, na, end_of(na, text)));
        }
        Moved { spans }
    }

    /// Where edit `i`'s text starts and ends now.
    pub fn range(&self, i: usize) -> Option<(SrcPos, SrcPos)> {
        self.spans.get(i).map(|s| (s.2, s.3))
    }

    /// Where `p` (a place before the edits) is now: after text inserted
    /// at it, at the end of text that replaced a range around it.
    pub fn map(&self, p: SrcPos) -> SrcPos {
        let i = self.spans.partition_point(|s| s.0 <= p);
        let Some(&(a, b, na, ne)) = i.checked_sub(1).and_then(|i| self.spans.get(i)) else { return p };
        if p == a && a != b {
            na
        } else if p < b {
            ne
        } else {
            shift(p, b, ne)
        }
    }
}

/// `p`, at or after `old` (an edit's end before it), once that end is at
/// `new`.
fn shift(p: SrcPos, old: SrcPos, new: SrcPos) -> SrcPos {
    if p.line == old.line {
        SrcPos::new(new.line, new.col + p.col.saturating_sub(old.col))
    } else {
        SrcPos::new((p.line as isize + new.line as isize - old.line as isize).max(0) as usize, p.col)
    }
}

/// How a language writes comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Comment {
    /// A line comment (`//`, `#`, `--`).
    pub line: Option<&'static str>,
    /// A block comment, used per line where there is no line comment.
    pub block: Option<(&'static str, &'static str)>,
}

/// The comments of `path`'s language (by its name), if it has any.
pub fn comment_syntax(path: &str) -> Option<Comment> {
    use super::highlight::Lang as L;
    let line = |t| Some(Comment { line: Some(t), block: None });
    let block = |a, b| Some(Comment { line: None, block: Some((a, b)) });
    match L::for_path(path) {
        Some(L::Rust | L::JavaScript | L::TypeScript | L::Tsx | L::Go | L::C | L::Cpp | L::Java | L::CSharp | L::Swift | L::Kotlin | L::Scala | L::Zig | L::Php | L::Scss) => line("//"),
        Some(L::Python | L::Bash | L::Ruby | L::Yaml | L::Elixir | L::R | L::PowerShell | L::Nix) => line("#"),
        Some(L::Lua | L::Haskell | L::Sql) => line("--"),
        Some(L::Css) => block("/*", "*/"),
        Some(L::Html | L::Xml | L::Markdown) => block("<!--", "-->"),
        Some(L::OCaml) => block("(*", "*)"),
        None => {
            let name = path.rsplit(['/', '\\']).next().unwrap_or(path).to_ascii_lowercase();
            let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
            match (name.as_str(), ext) {
                ("makefile" | "dockerfile" | "gnumakefile" | "justfile" | ".gitignore" | ".dockerignore" | ".env" | ".editorconfig" | "procfile", _) => line("#"),
                (_, "toml" | "ini" | "cfg" | "conf" | "properties" | "env" | "mk" | "cmake" | "tf" | "hcl" | "gitignore" | "dockerfile" | "fish" | "pl" | "pm" | "jl" | "nim" | "cr" | "graphql" | "gql") => line("#"),
                (_, "jsonc" | "json5" | "dart" | "proto" | "groovy" | "gradle" | "sol" | "v" | "glsl" | "wgsl" | "hlsl" | "less" | "svelte" | "vue") => line("//"),
                (_, "vim") => line("\""),
                (_, "lisp" | "el" | "clj" | "cljs" | "scm" | "rkt" | "asm" | "s" | "ini~") => line(";"),
                (_, "tex" | "sty" | "erl" | "hrl") => line("%"),
                (_, "elm" | "purs" | "hs" | "ada" | "adb") => line("--"),
                _ => None,
            }
        }
    }
}

/// Closing partners typing puts in.
pub fn closing(open: char) -> Option<char> {
    Some(match open {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '"' => '"',
        '\'' => '\'',
        '`' => '`',
        _ => return None,
    })
}

fn is_closer(c: char) -> bool {
    matches!(c, ')' | ']' | '}' | '"' | '\'' | '`')
}

/// One cursor's edit: `from..to` becomes `text`; the cursor then selects
/// bytes `anchor..head` of `text` (a caret at its end by default).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorEdit {
    pub from: SrcPos,
    pub to: SrcPos,
    pub text: String,
    pub anchor: usize,
    pub head: usize,
}

impl CursorEdit {
    pub fn new(from: SrcPos, to: SrcPos, text: String) -> CursorEdit {
        let n = text.len();
        CursorEdit { from, to, text, anchor: n, head: n }
    }

    /// The cursor at byte `at` of the text.
    pub fn caret_at(mut self, at: usize) -> CursorEdit {
        self.anchor = at;
        self.head = at;
        self
    }
}

impl Buffer {
    /// Enter at `p`: a line break with the line's indentation, one step
    /// more after an opening bracket (or a `:` ending a line of Python or
    /// YAML); between a bracket pair, the closing one goes on a line of its
    /// own below the cursor.
    pub fn smart_newline(&self, a: SrcPos, b: SrcPos, colon_opens: bool) -> CursorEdit {
        let line = self.line(a.line);
        let upto = &line[..a.col.min(line.len())];
        let lead: String = upto.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let before = upto.trim_end().chars().last();
        let after_line = self.line(b.line);
        let after = after_line[b.col.min(after_line.len())..].trim_start().chars().next();
        let opens = matches!(before, Some('{' | '[' | '(')) || (colon_opens && before == Some(':'));
        let nl = if self.crlf { "\r\n" } else { "\n" };
        if !opens {
            return CursorEdit::new(a, b, format!("{nl}{lead}"));
        }
        let inner = format!("{nl}{lead}{}", self.unit());
        let pair = matches!((before, after), (Some('{'), Some('}')) | (Some('['), Some(']')) | (Some('('), Some(')')));
        if pair {
            let at = inner.len();
            CursorEdit::new(a, b, format!("{inner}{nl}{lead}")).caret_at(at)
        } else {
            CursorEdit::new(a, b, inner)
        }
    }

    /// Typing `ch` over `a..b`: a bracket or quote gets its partner (around
    /// a selection: the selection is wrapped), a closing one typed before
    /// the same character steps over it. `None`: plain typing.
    pub fn typed_pair(&self, a: SrcPos, b: SrcPos, ch: char) -> Option<CursorEdit> {
        let line = self.line(a.line);
        let next = line[a.col.min(line.len())..].chars().next();
        let prev = line[..a.col.min(line.len())].chars().next_back();
        let quote = matches!(ch, '"' | '\'' | '`');
        if a == b && is_closer(ch) && next == Some(ch) {
            // Step over it.
            let end = SrcPos::new(a.line, a.col + ch.len_utf8());
            return Some(CursorEdit::new(a, end, ch.to_string()));
        }
        let close = closing(ch)?;
        if a != b {
            if a.line != b.line && quote {
                return None;
            }
            let inner = self.slice(a, b);
            let text = format!("{ch}{inner}{close}");
            let (x, y) = (ch.len_utf8(), ch.len_utf8() + inner.len());
            return Some(CursorEdit { from: a, to: b, text, anchor: x, head: y });
        }
        // Only where nothing follows that the pair would split.
        let free = next.is_none_or(|n| n.is_whitespace() || matches!(n, ')' | ']' | '}' | ',' | ';' | ':' | '.'));
        if !free {
            return None;
        }
        if quote && prev.is_some_and(|p| super::text::is_word_char(p) || p == ch) {
            return None;
        }
        Some(CursorEdit::new(a, b, format!("{ch}{close}")).caret_at(ch.len_utf8()))
    }

    /// Backspace at `p` inside an empty pair (`(|)`): both go.
    pub fn empty_pair_at(&self, p: SrcPos) -> bool {
        let line = self.line(p.line);
        let (Some(prev), Some(next)) = (line[..p.col.min(line.len())].chars().next_back(), line[p.col.min(line.len())..].chars().next()) else {
            return false;
        };
        closing(prev) == Some(next)
    }

    /// ⌘/ over lines `lines` (in order): commented out with the language's
    /// line comment at the block's indentation, or uncommented when every
    /// line with text is one already. Block-comment languages wrap each
    /// line. Blank lines are left alone.
    pub fn comment_edits(&self, lines: &[usize], c: Comment) -> Vec<(SrcPos, SrcPos, String)> {
        let texts: Vec<(usize, &str)> = lines.iter().map(|&l| (l, self.line(l))).filter(|(_, t)| !t.trim().is_empty()).collect();
        if texts.is_empty() {
            return Vec::new();
        }
        let indent = |t: &str| t.len() - t.trim_start_matches([' ', '\t']).len();
        if let Some(tok) = c.line {
            let all = texts.iter().all(|(_, t)| t.trim_start().starts_with(tok));
            if all {
                return texts
                    .iter()
                    .map(|&(l, t)| {
                        let at = indent(t);
                        let n = tok.len() + usize::from(t[at + tok.len()..].starts_with(' '));
                        (SrcPos::new(l, at), SrcPos::new(l, at + n), String::new())
                    })
                    .collect();
            }
            let min = texts.iter().map(|(_, t)| indent(t)).min().unwrap_or(0);
            return texts.iter().map(|&(l, _)| (SrcPos::new(l, min), SrcPos::new(l, min), format!("{tok} "))).collect();
        }
        let Some((open, close)) = c.block else { return Vec::new() };
        let wrapped = |t: &str| {
            let s = t.trim();
            s.starts_with(open) && s.ends_with(close) && s.len() >= open.len() + close.len()
        };
        let all = texts.iter().all(|(_, t)| wrapped(t));
        let mut out = Vec::new();
        for &(l, t) in &texts {
            let at = indent(t);
            let end = t.trim_end().len();
            if all {
                let body = &t[at + open.len()..end - close.len()];
                let lead = usize::from(body.starts_with(' '));
                let trail = usize::from(body.len() > lead && body.ends_with(' '));
                out.push((SrcPos::new(l, at), SrcPos::new(l, at + open.len() + lead), String::new()));
                out.push((SrcPos::new(l, end - close.len() - trail), SrcPos::new(l, end), String::new()));
            } else {
                out.push((SrcPos::new(l, at), SrcPos::new(l, at), format!("{open} ")));
                out.push((SrcPos::new(l, end), SrcPos::new(l, end), format!(" {close}")));
            }
        }
        out
    }
}

/// Where `text` ends when inserted at `at`.
pub(crate) fn end_of(at: SrcPos, text: &str) -> SrcPos {
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
    fn several_cursors_edit_as_one_step_and_land_after_their_text() {
        let mut b = Buffer::new("ab\ncd\nef\n");
        // A cursor after each letter `a`, `c`, `e`; two on one line too.
        let edits = vec![(p(0, 1), p(0, 1), "X".to_string()), (p(0, 2), p(0, 2), "Y".to_string()), (p(1, 1), p(1, 1), "X".to_string()), (p(2, 1), p(2, 1), "X\nZ".to_string())];
        let (_, moved) = b.edit_many(edits, 0, Kind::Typing, SrcSel::caret(p(0, 1)));
        assert_eq!(b.text(), "aXbY\ncXd\neX\nZf\n");
        assert_eq!(moved.range(0), Some((p(0, 1), p(0, 2))));
        assert_eq!(moved.range(1), Some((p(0, 3), p(0, 4))), "after the first one on its line");
        assert_eq!(moved.range(2), Some((p(1, 1), p(1, 2))));
        assert_eq!(moved.range(3), Some((p(2, 1), p(3, 1))));
        // Other places follow: the end of line 1, the `f` after the break.
        assert_eq!(moved.map(p(1, 2)), p(1, 3));
        assert_eq!(moved.map(p(2, 2)), p(3, 2));
        assert_eq!(moved.map(p(3, 0)), p(4, 0));
        // Typing goes on as one step at all three; one undo takes it all.
        let edits = vec![(p(0, 2), p(0, 2), "1".to_string()), (p(1, 2), p(1, 2), "1".to_string())];
        b.edit_many(edits, 0, Kind::Typing, SrcSel::caret(p(0, 2)));
        assert_eq!(b.text(), "aX1bY\ncX1d\neX\nZf\n");
        b.undo();
        assert_eq!(b.text(), "ab\ncd\nef\n");
        b.redo();
        assert_eq!(b.text(), "aX1bY\ncX1d\neX\nZf\n");
    }

    #[test]
    fn comments_toggle_by_language() {
        let rust = comment_syntax("src/main.rs").unwrap();
        let mut b = Buffer::new("fn a() {\n    let x = 1;\n\n        y();\n}\n");
        let e = b.comment_edits(&[1, 2, 3], rust);
        b.edit_many(e, 0, Kind::Other, SrcSel::default());
        assert_eq!(b.text(), "fn a() {\n    // let x = 1;\n\n    //     y();\n}\n", "at the block's indentation; blank lines alone");
        let e = b.comment_edits(&[1, 2, 3], rust);
        b.edit_many(e, 0, Kind::Other, SrcSel::default());
        assert_eq!(b.text(), "fn a() {\n    let x = 1;\n\n        y();\n}\n");
        // Mixed: everything is commented (once more where it was).
        let py = comment_syntax("x.py").unwrap();
        let mut b = Buffer::new("# a\nb\n");
        let e = b.comment_edits(&[0, 1], py);
        b.edit_many(e, 0, Kind::Other, SrcSel::default());
        assert_eq!(b.text(), "# # a\n# b\n");
        // Block comments wrap each line, and come off again.
        let css = comment_syntax("a.css").unwrap();
        let mut b = Buffer::new("  color: red;\n");
        let e = b.comment_edits(&[0], css);
        b.edit_many(e, 0, Kind::Other, SrcSel::default());
        assert_eq!(b.text(), "  /* color: red; */\n");
        let e = b.comment_edits(&[0], css);
        b.edit_many(e, 0, Kind::Other, SrcSel::default());
        assert_eq!(b.text(), "  color: red;\n");
        assert_eq!(comment_syntax("Cargo.toml").and_then(|c| c.line), Some("#"));
        assert_eq!(comment_syntax("notes.txt"), None);
    }

    #[test]
    fn enter_indents_after_an_opening_bracket() {
        let b = Buffer::new("fn a() {}\n    if x:\n");
        // Between `{` and `}`: the closing one on its own line.
        let e = b.smart_newline(p(0, 8), p(0, 8), false);
        assert_eq!(e.text, "\n    \n");
        assert_eq!(e.head, 5, "on the middle line");
        let e = b.smart_newline(p(1, 9), p(1, 9), true);
        assert_eq!(e.text, "\n        ");
        let e = b.smart_newline(p(1, 9), p(1, 9), false);
        assert_eq!(e.text, "\n    ", "a colon opens only where the language says");
        // CRLF files get CRLF.
        let b = Buffer::new("a {\r\n}\r\n");
        assert_eq!(b.smart_newline(p(0, 3), p(0, 3), false).text, "\r\n    ");
    }

    #[test]
    fn brackets_and_quotes_come_in_pairs() {
        let b = Buffer::new("x\nfoo(bar)\nit\n");
        let e = b.typed_pair(p(0, 1), p(0, 1), '(').unwrap();
        assert_eq!((e.text.as_str(), e.head), ("()", 1));
        // Before a word: no pair.
        assert!(b.typed_pair(p(1, 4), p(1, 4), '[').is_none());
        // Over the closing one: step over it.
        let e = b.typed_pair(p(1, 7), p(1, 7), ')').unwrap();
        assert_eq!((e.from, e.to, e.text.as_str()), (p(1, 7), p(1, 8), ")"));
        // Around a selection: wrapped, the inside selected.
        let e = b.typed_pair(p(1, 4), p(1, 7), '"').unwrap();
        assert_eq!((e.text.as_str(), e.anchor, e.head), ("\"bar\"", 1, 4));
        // An apostrophe after a letter is just one.
        assert!(b.typed_pair(p(2, 2), p(2, 2), '\'').is_none());
        let b = Buffer::new("()\n");
        assert!(b.empty_pair_at(p(0, 1)));
        assert!(!b.empty_pair_at(p(0, 2)));
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
