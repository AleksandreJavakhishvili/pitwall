//! Moving and selecting in text by what a reader sees: grapheme
//! clusters (a Georgian letter with its mark is one step) and Unicode
//! words. Shared by the text input and the code view.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// The largest char boundary at or before `i`.
pub fn clamp_to_char(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The previous grapheme boundary before `col` (0 at the start).
pub fn prev_grapheme(s: &str, col: usize) -> usize {
    s.grapheme_indices(true)
        .rev()
        .find_map(|(i, _)| (i < col).then_some(i))
        .unwrap_or(0)
}

/// The next grapheme boundary after `col` (the end at the end).
pub fn next_grapheme(s: &str, col: usize) -> usize {
    s.grapheme_indices(true)
        .find_map(|(i, _)| (i > col).then_some(i))
        .unwrap_or(s.len())
}

/// The word around `col` (Unicode word boundaries: Georgian words are
/// words), or the single non-word character there.
pub fn word_at(s: &str, col: usize) -> Range<usize> {
    let col = clamp_to_char(s, col);
    for (i, w) in s.split_word_bound_indices() {
        let end = i + w.len();
        if col >= i && col < end || (col == end && end == s.len() && col > i) {
            return i..end;
        }
    }
    col..col
}

/// Word characters, for "Match Whole Word" and word motion.
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The start of the previous word before `col` (Alt/Ctrl+Left).
pub fn prev_word(s: &str, col: usize) -> usize {
    let mut last = 0;
    for (i, w) in s.split_word_bound_indices() {
        if i >= col {
            break;
        }
        if w.chars().next().is_some_and(is_word_char) {
            last = i;
        }
    }
    last
}

/// The end of the next word after `col` (Alt/Ctrl+Right).
pub fn next_word(s: &str, col: usize) -> usize {
    for (i, w) in s.split_word_bound_indices() {
        let end = i + w.len();
        if end > col && w.chars().next().is_some_and(is_word_char) {
            return end;
        }
    }
    s.len()
}
