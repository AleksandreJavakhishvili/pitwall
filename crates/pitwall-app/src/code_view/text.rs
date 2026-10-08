//! The read-only text model: a file split into lines, each kept as it is
//! shown (tabs expanded to tab stops, very long lines cut) with a map back
//! to the source bytes for copying. Columns are UTF-8 byte offsets that
//! always sit on character boundaries, so Georgian, emoji and combining
//! marks never split.

use std::ops::Range;

use gpui::SharedString;

/// Lines longer than this (bytes) are cut when shown, as Monaco's
/// `stopRenderingLineAfter` does; copying still copies the whole line.
pub const MAX_SHOWN_LINE: usize = 10_000;

/// One line as shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// What is drawn: tabs expanded, no line break, maybe cut.
    pub text: SharedString,
    /// The source line, when it differs from `text` (tabs, cut).
    src: Option<String>,
    /// Shown byte -> source byte at each expanded tab (`(shown, src)` pairs,
    /// sorted), for lines with tabs.
    tabs: Vec<(usize, usize)>,
    /// The line was cut at [`MAX_SHOWN_LINE`].
    pub cut: bool,
}

impl Line {
    fn new(src: &str, tab_size: usize) -> Line {
        let src = src.strip_suffix('\r').unwrap_or(src);
        let mut text = String::with_capacity(src.len());
        let mut tabs = Vec::new();
        let mut col = 0usize; // in characters, for tab stops
        let mut cut = false;
        for (i, ch) in src.char_indices() {
            if text.len() >= MAX_SHOWN_LINE {
                cut = true;
                break;
            }
            if ch == '\t' {
                let n = tab_size - (col % tab_size);
                tabs.push((text.len(), i));
                text.extend(std::iter::repeat_n(' ', n));
                col += n;
                tabs.push((text.len(), i + 1));
            } else if ch.is_control() {
                // Control characters draw as the replacement glyph, one
                // character for one (same byte length is not needed: the
                // tab map handles any width).
                tabs.push((text.len(), i));
                text.push('\u{fffd}');
                col += 1;
                tabs.push((text.len(), i + ch.len_utf8()));
            } else {
                text.push(ch);
                col += 1;
            }
        }
        let same = !cut && tabs.is_empty();
        Line {
            text: text.into(),
            src: (!same).then(|| src.to_string()),
            tabs,
            cut,
        }
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The source text of the line (what copying copies).
    pub fn source(&self) -> &str {
        self.src.as_deref().unwrap_or(&self.text)
    }

    /// A shown column -> the source byte offset.
    pub fn to_source(&self, col: usize) -> usize {
        if self.tabs.is_empty() {
            return if self.cut && col >= self.text.len() {
                self.source().len()
            } else {
                col
            };
        }
        // The last mapping point at or before `col`, then plain bytes on.
        match self.tabs.iter().rposition(|(shown, _)| *shown <= col) {
            None => col,
            Some(i) => {
                let (shown, src) = self.tabs[i];
                // Inside an expanded tab: its start.
                if i % 2 == 0 && col > shown {
                    return src;
                }
                let out = src + (col - shown);
                if self.cut && col >= self.text.len() {
                    self.source().len()
                } else {
                    out.min(self.source().len())
                }
            }
        }
    }

    /// A source byte offset -> the shown column (clamped to what is shown).
    pub fn from_source(&self, src_col: usize) -> usize {
        if self.tabs.is_empty() {
            return clamp_to_char(&self.text, src_col.min(self.text.len()));
        }
        match self.tabs.iter().rposition(|(_, s)| *s <= src_col) {
            None => clamp_to_char(&self.text, src_col.min(self.text.len())),
            Some(i) => {
                let (shown, s) = self.tabs[i];
                clamp_to_char(&self.text, (shown + (src_col - s)).min(self.text.len()))
            }
        }
    }

    /// The source text of shown columns `range`.
    pub fn source_slice(&self, range: Range<usize>) -> &str {
        let src = self.source();
        let a = self.to_source(range.start).min(src.len());
        let b = self.to_source(range.end).min(src.len()).max(a);
        &src[clamp_to_char(src, a)..clamp_to_char(src, b)]
    }
}

/// A whole file, as lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub lines: Vec<Line>,
    /// Columns per tab (detected from the file's indentation, as Monaco does).
    pub tab_size: usize,
}

impl Document {
    pub fn new(text: &str) -> Document {
        let tab_size = detect_indent(text).width();
        Document {
            lines: text.split('\n').map(|l| Line::new(l, tab_size)).collect(),
            tab_size,
        }
    }

    /// An empty document (one empty line, as an editor shows it).
    pub fn empty() -> Document {
        Document::new("")
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line(&self, ix: usize) -> Option<&Line> {
        self.lines.get(ix)
    }

    /// The text between two positions, in source form, lines joined by `\n`.
    pub fn text_between(&self, a: Pos, b: Pos) -> String {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let (a, b) = (self.clamp(a), self.clamp(b));
        let mut out = String::new();
        for ix in a.line..=b.line {
            let line = &self.lines[ix];
            if ix > a.line {
                out.push('\n');
            }
            let from = if ix == a.line { a.col } else { 0 };
            if ix == b.line {
                out.push_str(line.source_slice(from..b.col.max(from)));
            } else {
                let src = line.source();
                out.push_str(&src[clamp_to_char(src, line.to_source(from))..]);
            }
        }
        out
    }

    /// A position clamped into the document, on a character boundary.
    pub fn clamp(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines.len().saturating_sub(1));
        let text = &self.lines[line].text;
        Pos {
            line,
            col: clamp_to_char(text, p.col.min(text.len())),
        }
    }

    pub fn end(&self) -> Pos {
        let line = self.lines.len().saturating_sub(1);
        Pos {
            line,
            col: self.lines[line].len(),
        }
    }
}

/// A place in a document: line index and shown byte column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub fn new(line: usize, col: usize) -> Pos {
        Pos { line, col }
    }
}

pub use crate::kit::edit::{
    clamp_to_char, is_word_char, next_grapheme, next_word, prev_grapheme, prev_word, word_at,
};

/// How a file is indented (Monaco's detection: tabs if most indented lines
/// use them, otherwise the most common step of 2–8 spaces).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indent {
    Tabs,
    Spaces(usize),
}

impl Indent {
    /// Columns per tab stop.
    pub fn width(self) -> usize {
        match self {
            Indent::Tabs => 4,
            Indent::Spaces(n) => n,
        }
    }
}

pub fn detect_indent(text: &str) -> Indent {
    let (mut tabs, mut spaces, mut prev) = (0, 0, 0usize);
    let mut deltas = [0usize; 9];
    for line in text.split('\n').take(5000) {
        if line.trim().is_empty() {
            continue;
        }
        let lead: &str = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
        if lead.starts_with('\t') {
            tabs += 1;
        } else if !lead.is_empty() {
            spaces += 1;
        }
        let n = lead.len();
        let d = n.abs_diff(prev);
        if (2..=8).contains(&d) && !lead.contains('\t') {
            deltas[d] += 1;
        }
        prev = n;
    }
    if tabs > spaces {
        return Indent::Tabs;
    }
    let (mut best, mut count) = (4, 0);
    for (d, &c) in deltas.iter().enumerate().skip(2) {
        if c > count {
            (best, count) = (d, c);
        }
    }
    Indent::Spaces(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lines_and_keeps_the_last_empty_one() {
        let d = Document::new("a\r\nb\n");
        let texts: Vec<_> = d.lines.iter().map(|l| l.text.to_string()).collect();
        assert_eq!(texts, ["a", "b", ""]);
        assert_eq!(d.lines[0].source(), "a", "CR is not shown");
    }

    #[test]
    fn tabs_expand_to_tab_stops_and_map_back() {
        let d = Document::new("\tx\n  y\n    z\n");
        // Two-space steps are not the majority here; a tab is one line of three.
        let l = &d.lines[0];
        assert_eq!(l.text.as_ref(), format!("{}x", " ".repeat(d.tab_size)));
        assert_eq!(l.source(), "\tx");
        assert_eq!(l.to_source(0), 0);
        assert_eq!(l.to_source(1), 0, "inside the tab: its start");
        assert_eq!(l.to_source(d.tab_size), 1);
        assert_eq!(l.from_source(1), d.tab_size);
        assert_eq!(l.source_slice(0..d.tab_size + 1), "\tx");
    }

    #[test]
    fn georgian_stays_whole() {
        // "გამარჯობა მსოფლიო" — three bytes per letter.
        let d = Document::new("გამარჯობა მსოფლიო");
        let s = &d.lines[0].text;
        assert_eq!(clamp_to_char(s, 1), 0);
        assert_eq!(clamp_to_char(s, 4), 3);
        assert_eq!(next_grapheme(s, 0), 3);
        assert_eq!(prev_grapheme(s, 6), 3);
        let w = word_at(s, 4);
        assert_eq!(&s[w], "გამარჯობა");
        assert_eq!(next_word(s, 0), "გამარჯობა".len());
        assert_eq!(prev_word(s, s.len()), "გამარჯობა ".len());
        let p = d.clamp(Pos::new(5, 1000));
        assert_eq!(p, Pos::new(0, s.len()));
    }

    #[test]
    fn combining_marks_and_emoji_are_one_grapheme() {
        let s = "e\u{301}x👍🏽y";
        assert_eq!(next_grapheme(s, 0), 3, "e + combining acute");
        let thumbs = next_grapheme(s, 4);
        assert_eq!(&s[4..thumbs], "👍🏽");
    }

    #[test]
    fn text_between_copies_source_text() {
        let d = Document::new("ab\tc\nსამი\nend");
        let t = d.text_between(Pos::new(0, 1), Pos::new(2, 1));
        assert_eq!(t, "b\tc\nსამი\ne");
        let one = d.text_between(Pos::new(1, 3), Pos::new(1, 9));
        assert_eq!(one, "ამ");
    }

    #[test]
    fn very_long_lines_are_cut_for_display_only() {
        let long = "x".repeat(MAX_SHOWN_LINE + 50);
        let d = Document::new(&long);
        let l = &d.lines[0];
        assert!(l.cut);
        assert_eq!(l.len(), MAX_SHOWN_LINE);
        assert_eq!(l.source().len(), MAX_SHOWN_LINE + 50);
        assert_eq!(
            d.text_between(Pos::new(0, 0), d.end()).len(),
            MAX_SHOWN_LINE + 50
        );
    }

    #[test]
    fn indentation_is_detected() {
        assert_eq!(detect_indent("a\n\tb\n\tc\n"), Indent::Tabs);
        assert_eq!(detect_indent("a\n  b\n    c\n  d\n"), Indent::Spaces(2));
        assert_eq!(detect_indent("plain\n"), Indent::Spaces(4));
    }
}
