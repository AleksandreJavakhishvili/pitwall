//! Selecting the Preview's text: every text drawn (a heading, a paragraph,
//! a list item's text, a table cell, a code block) is a [`Piece`] in
//! drawing order, holding its `TextLayout` after the frame is laid out. A
//! place is a piece and a byte in it; the mouse finds one through the
//! layouts, and the selection is drawn as the text runs' background.

use std::ops::Range;

use gpui::{Hsla, Pixels, Point, SharedString, TextLayout, TextRun};

use crate::kit::edit::{clamp_to_char, word_at};

/// A place in the text: (piece, byte offset in it).
pub type Pos = (usize, usize);

/// A selectable text as drawn this frame.
pub struct Piece {
    pub text: SharedString,
    /// Copied before it when a selection runs on from the piece before
    /// (`"\n\n"` between blocks, `"\n"` between items, `"\t"` cells).
    pub sep: &'static str,
    /// Copied before it with a whole item: its list marker.
    pub prefix: SharedString,
    pub layout: TextLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Pos,
    /// Where the mouse is (shift-click extends from the anchor).
    pub head: Pos,
}

impl Selection {
    pub fn caret(p: Pos) -> Self {
        Selection { anchor: p, head: p }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Start and end in document order.
    pub fn range(&self) -> (Pos, Pos) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// The bytes selected in piece `k` of length `len`.
    pub fn in_piece(&self, k: usize, len: usize) -> Option<Range<usize>> {
        let (a, b) = self.range();
        if self.is_empty() || k < a.0 || k > b.0 {
            return None;
        }
        let s = if k == a.0 { a.1.min(len) } else { 0 };
        let e = if k == b.0 { b.1.min(len) } else { len };
        (s < e).then_some(s..e)
    }
}

/// `runs` with the bytes in `sel` on the selection colour.
pub fn highlight(runs: Vec<TextRun>, sel: Option<Range<usize>>, color: Hsla) -> Vec<TextRun> {
    let Some(sel) = sel else { return runs };
    let mut out = Vec::with_capacity(runs.len() + 2);
    let mut at = 0;
    for run in runs {
        let end = at + run.len;
        // Cut the run where the selection starts and ends.
        let mut cuts = vec![at];
        for c in [sel.start, sel.end] {
            if c > at && c < end {
                cuts.push(c);
            }
        }
        cuts.push(end);
        for w in cuts.windows(2) {
            let mut r = run.clone();
            r.len = w[1] - w[0];
            if w[0] >= sel.start && w[1] <= sel.end {
                r.background_color = Some(color);
            }
            if r.len > 0 {
                out.push(r);
            }
        }
        at = end;
    }
    out
}

/// The place under `p` (window coordinates), or the nearest one: on the
/// line at that height, else the start of the next text below.
pub fn hit(pieces: &[Piece], p: Point<Pixels>) -> Option<Pos> {
    let index = |k: usize, at: Point<Pixels>| {
        let piece = &pieces[k];
        let ix = match piece.layout.index_for_position(at) {
            Ok(i) | Err(i) => i,
        };
        (k, clamp_to_char(&piece.text, ix))
    };
    let mut beside: Option<(Pixels, usize)> = None;
    for (k, piece) in pieces.iter().enumerate() {
        let b = piece.layout.bounds();
        if p.y < b.top() || p.y >= b.bottom() {
            continue;
        }
        if p.x >= b.left() && p.x < b.right() {
            return Some(index(k, p));
        }
        let d = if p.x < b.left() { b.left() - p.x } else { p.x - b.right() };
        if beside.is_none_or(|(best, _)| d < best) {
            beside = Some((d, k));
        }
    }
    if let Some((_, k)) = beside {
        let b = pieces[k].layout.bounds();
        let x = p.x.clamp(b.left(), b.right());
        return Some(index(k, Point { x, y: p.y }));
    }
    // Between blocks: the start of the next text below, or the very end.
    match pieces.iter().position(|piece| piece.layout.bounds().top() > p.y) {
        Some(k) => Some((k, 0)),
        None => pieces.last().map(|piece| (pieces.len() - 1, piece.text.len())),
    }
}

/// The word at `p` (double click).
pub fn word(pieces: &[Piece], p: Pos) -> Selection {
    let r = word_at(&pieces[p.0].text, p.1);
    Selection {
        anchor: (p.0, r.start),
        head: (p.0, r.end),
    }
}

/// The whole piece at `p` (triple click).
pub fn block(pieces: &[Piece], p: Pos) -> Selection {
    Selection {
        anchor: (p.0, 0),
        head: (p.0, pieces[p.0].text.len()),
    }
}

/// Everything (⌘A).
pub fn all(pieces: &[Piece]) -> Option<Selection> {
    let last = pieces.last()?;
    Some(Selection {
        anchor: (0, 0),
        head: (pieces.len() - 1, last.text.len()),
    })
}

/// The selected text as plain text: blocks apart by a blank line, items
/// with their markers, table cells by tabs.
pub fn text(pieces: &[Piece], sel: &Selection) -> String {
    let (a, b) = sel.range();
    let mut out = String::new();
    for (k, piece) in pieces.iter().enumerate().take(b.0 + 1).skip(a.0) {
        let len = piece.text.len();
        let s = if k == a.0 { clamp_to_char(&piece.text, a.1) } else { 0 };
        let e = if k == b.0 { clamp_to_char(&piece.text, b.1) } else { len };
        if k > a.0 {
            out.push_str(piece.sep);
        }
        if s == 0 && b.0 > a.0 {
            out.push_str(&piece.prefix);
        }
        out.push_str(&piece.text[s..e.max(s)]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(len: usize) -> TextRun {
        TextRun {
            len,
            font: gpui::font("x"),
            color: Hsla::default(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }

    #[test]
    fn highlight_cuts_runs_at_the_selection() {
        let red = gpui::red();
        let out = highlight(vec![run(4), run(6)], Some(2..7), red);
        let lens: Vec<_> = out.iter().map(|r| (r.len, r.background_color.is_some())).collect();
        assert_eq!(lens, [(2, false), (2, true), (3, true), (3, false)]);
        assert_eq!(highlight(vec![run(3)], None, red).len(), 1);
    }

    #[test]
    fn a_selection_in_pieces() {
        let s = Selection {
            anchor: (3, 2),
            head: (1, 4),
        };
        assert_eq!(s.range(), ((1, 4), (3, 2)));
        assert_eq!(s.in_piece(0, 9), None);
        assert_eq!(s.in_piece(1, 9), Some(4..9));
        assert_eq!(s.in_piece(2, 9), Some(0..9));
        assert_eq!(s.in_piece(3, 9), Some(0..2));
        assert_eq!(Selection::caret((1, 1)).in_piece(1, 9), None);
    }
}
