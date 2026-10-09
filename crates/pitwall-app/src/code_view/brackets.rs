//! Bracket matching and the active indent guide, as the React app's Review
//! editor shows them (CodeMirror `bracketMatching()`, the indentation
//! markers' active scope).
//!
//! - With an empty selection, the bracket just before the caret (else just
//!   after it) and its partner get the `--rv-bm-*` box; a bracket without
//!   a partner gets it alone (`cm-nonmatchingBracket` looks the same).
//!   Brackets inside strings and comments don't count: in a highlighted
//!   file only the highlighter's bracket tokens are brackets; a plain file
//!   counts every bracket character.
//! - The guide of the innermost indented block holding the caret line is
//!   drawn in `--rv-guide-active`.

use std::ops::Range;

use super::highlight::{Style, Token};
use super::text::{Document, Pos};

const OPEN: &[u8] = b"([{";
const CLOSE: &[u8] = b")]}";
/// CodeMirror's `maxScanDistance`.
const MAX_SCAN: usize = 10_000;

/// The brackets to box for a caret at `head` (shown columns): none, one
/// (no partner) or two.
pub fn matching<'a>(
    doc: &Document,
    spans: impl Fn(usize) -> &'a [(Range<usize>, Style)],
    plain: bool,
    head: Pos,
) -> Vec<Pos> {
    let Some(line) = doc.line(head.line) else {
        return Vec::new();
    };
    let src = line.source().as_bytes();
    let at = line.to_source(head.col);
    let is_bracket = |l: usize, b: usize| -> bool {
        let Some(s) = doc.line(l).map(|x| x.source().as_bytes()) else {
            return false;
        };
        if !s.get(b).is_some_and(|c| OPEN.contains(c) || CLOSE.contains(c)) {
            return false;
        }
        plain
            || spans(l).iter().any(|(r, st)| {
                r.contains(&b) && matches!(st.tok, Some(Token::Bracket(_) | Token::StrayBracket))
            })
    };
    // Before the caret first, then after it.
    let start = [at.checked_sub(1), Some(at)]
        .into_iter()
        .flatten()
        .find(|&b| b < src.len() && is_bracket(head.line, b));
    let Some(b) = start else { return Vec::new() };
    let mut out = vec![Pos::new(head.line, line.from_source(b))];
    let ch = src[b];
    let forward = OPEN.contains(&ch);
    let kind = OPEN.iter().chain(CLOSE).position(|c| *c == ch).unwrap_or(0) % 3;
    let mut depth = 0usize;
    let mut scanned = 0usize;
    let mut l = head.line;
    let mut first = true;
    loop {
        let s = doc.line(l).map(|x| x.source().as_bytes()).unwrap_or(&[]);
        let range: Vec<usize> = match (forward, first) {
            (true, true) => (b..s.len()).collect(),
            (true, false) => (0..s.len()).collect(),
            (false, true) => (0..=b).rev().collect(),
            (false, false) => (0..s.len()).rev().collect(),
        };
        first = false;
        for i in range {
            scanned += 1;
            if scanned > MAX_SCAN {
                return out;
            }
            if !is_bracket(l, i) {
                continue;
            }
            let c = s[i];
            if OPEN.contains(&c) == forward {
                depth += 1;
            } else {
                depth -= 1;
                if depth == 0 {
                    let k = OPEN.iter().chain(CLOSE).position(|x| *x == c).unwrap_or(0) % 3;
                    if k == kind {
                        let ln = doc.line(l).expect("scanned line");
                        out.push(Pos::new(l, ln.from_source(i)));
                    }
                    return out;
                }
            }
        }
        if forward {
            l += 1;
            if l >= doc.line_count() {
                return out;
            }
        } else {
            if l == 0 {
                return out;
            }
            l -= 1;
        }
    }
}

/// The indent levels of a shown line (its leading spaces over the tab
/// size; blank lines take the next non-blank line's).
pub fn levels(doc: &Document, line: usize) -> usize {
    let tab = doc.tab_size.max(1);
    let lead = |t: &str| t.len() - t.trim_start_matches(' ').len();
    let Some(l) = doc.lines.get(line) else { return 0 };
    let n = if l.text.trim().is_empty() {
        doc.lines[line + 1..]
            .iter()
            .take(100)
            .find(|x| !x.text.trim().is_empty())
            .map_or(0, |x| lead(&x.text))
    } else {
        lead(&l.text)
    };
    n / tab
}

/// The active guide for a caret on `line`: (guide index, lines it spans),
/// the innermost block the line sits in.
pub fn active_guide(doc: &Document, line: usize) -> Option<(usize, Range<usize>)> {
    let lv = levels(doc, line);
    if lv == 0 {
        return None;
    }
    let mut a = line;
    while a > 0 && levels(doc, a - 1) >= lv {
        a -= 1;
    }
    let mut b = line + 1;
    while b < doc.line_count() && levels(doc, b) >= lv {
        b += 1;
    }
    Some((lv - 1, a..b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str, line: usize, col: usize) -> Vec<Pos> {
        let doc = Document::new(text);
        matching(&doc, |_| &[], true, Pos::new(line, col))
    }

    #[test]
    fn the_bracket_before_the_caret_and_its_partner() {
        // Caret after "(": boxes it and its ")".
        assert_eq!(plain("f(a, (b))", 2, 0).len(), 0, "no such line");
        assert_eq!(
            plain("f(a, (b))", 0, 2),
            vec![Pos::new(0, 1), Pos::new(0, 8)]
        );
        // Caret before ")" (nothing before is a bracket): after it.
        assert_eq!(plain("x)", 0, 1), vec![Pos::new(0, 1)], "no partner: alone");
    }

    #[test]
    fn matching_runs_across_lines_and_backwards() {
        let text = "fn a() {\n    b[0];\n}";
        assert_eq!(plain(text, 2, 1), vec![Pos::new(2, 0), Pos::new(0, 7)]);
        assert_eq!(plain(text, 1, 7), vec![Pos::new(1, 7), Pos::new(1, 5)]);
    }

    #[test]
    fn a_wrong_closer_leaves_the_opener_alone() {
        assert_eq!(plain("(]", 0, 1), vec![Pos::new(0, 0)]);
    }

    #[test]
    fn highlighted_files_skip_brackets_in_strings() {
        let text = "fn f() { g(\")\") }";
        let doc = Document::new(text);
        let hl = super::super::highlight::highlight(text, Some(super::super::highlight::Lang::Rust));
        let m = matching(&doc, |l| hl[l].as_slice(), false, Pos::new(0, 11));
        assert_eq!(m, vec![Pos::new(0, 10), Pos::new(0, 14)]);
    }

    #[test]
    fn the_active_guide_is_the_innermost_block() {
        let doc = Document::new("a {\n  b {\n    c\n    d\n  }\n}\n");
        assert_eq!(active_guide(&doc, 2), Some((1, 2..4)));
        assert_eq!(active_guide(&doc, 1), Some((0, 1..5)));
        assert_eq!(active_guide(&doc, 0), None);
    }

    #[test]
    fn tabs_count_as_tab_stops() {
        let doc = Document::new("a\n\tb\n\t\tc\n");
        assert_eq!(levels(&doc, 1), 1);
        assert_eq!(levels(&doc, 2), 2);
    }
}
