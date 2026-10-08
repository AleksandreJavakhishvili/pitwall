//! The seam where a file's text is drawn: [`code_view`]. A minimal
//! read-only plain-text viewer (line numbers, the target line marked and
//! scrolled into view, the match on it highlighted, horizontal scroll). The
//! Review screen's viewer (`src/code_view/` on its branch: highlighting,
//! find, …) replaces the body of this one function at merge time; the
//! explorer calls nothing else.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    div, prelude::*, px, uniform_list, AnyElement, App, ElementId, HighlightStyle,
    ListHorizontalSizingBehavior, ScrollStrategy, SharedString, StyledText,
    UniformListScrollHandle, Window,
};

use super::widgets::mono;
use crate::theme;

/// Where to bring the reader: a line (1-based) and, for a search hit, the
/// match's `char` columns on it (0-based, end exclusive). A new `nonce`
/// scrolls there again.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub line: u32,
    pub cols: Option<(usize, usize)>,
    pub nonce: u64,
}

const ROW_H: f32 = 19.;
const TAB: &str = "    ";
/// Longest part of a line drawn (a minified file's one line can be MBs).
const MAX_COLS: usize = 4_000;

/// What the viewer keeps for one file between frames.
struct ViewState {
    text_id: (usize, usize),
    lines: Arc<Vec<Range<usize>>>,
    longest: usize,
    scroll: UniformListScrollHandle,
    shown_nonce: Option<u64>,
}

/// Line ranges (without `\n`/`\r\n`) and the longest line's index.
pub fn index_lines(text: &str) -> (Vec<Range<usize>>, usize) {
    let mut lines = Vec::new();
    let mut start = 0;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            let end = if i > start && text.as_bytes()[i - 1] == b'\r' {
                i - 1
            } else {
                i
            };
            lines.push(start..end);
            start = i + 1;
        }
    }
    lines.push(start..text.len());
    let longest = lines
        .iter()
        .enumerate()
        .max_by_key(|(_, r)| text[r.start..r.end].chars().count())
        .map(|(i, _)| i)
        .unwrap_or(0);
    (lines, longest)
}

/// A line as drawn (tabs as spaces, cut when very long) and the byte range
/// in it of `char` columns `cols`.
pub fn display_line(line: &str, cols: Option<(usize, usize)>) -> (String, Option<Range<usize>>) {
    let mut out = String::with_capacity(line.len());
    let (mut from, mut to) = (None, None);
    for (ci, c) in line.chars().enumerate() {
        if ci >= MAX_COLS {
            out.push('…');
            break;
        }
        if let Some((a, b)) = cols {
            if ci == a {
                from = Some(out.len());
            }
            if ci == b {
                to = Some(out.len());
            }
        }
        if c == '\t' {
            out.push_str(TAB);
        } else {
            out.push(c);
        }
    }
    let to = to.or(from.map(|_| out.len()));
    let hit = match (from, to) {
        (Some(f), Some(t)) if f < t => Some(f..t),
        _ => None,
    };
    (out, hit)
}

/// One file's text, read-only: `path` names it (and keys its scroll
/// position), `text` is the whole file, `target` the line to show.
pub fn code_view(
    path: &str,
    text: &SharedString,
    target: Option<&Target>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let t = theme::theme(cx).clone();
    let id = (text.as_ptr() as usize, text.len());
    let state = window.use_keyed_state(
        ElementId::Name(format!("ex-code:{path}").into()),
        cx,
        |_, _| ViewState {
            text_id: (0, usize::MAX),
            lines: Arc::default(),
            longest: 0,
            scroll: UniformListScrollHandle::new(),
            shown_nonce: None,
        },
    );
    let (lines, longest, scroll) = state.update(cx, |s, _| {
        if s.text_id != id {
            let (lines, longest) = index_lines(text);
            s.lines = Arc::new(lines);
            s.longest = longest;
            s.text_id = id;
        }
        if let Some(tg) = target {
            if s.shown_nonce != Some(tg.nonce) {
                s.shown_nonce = Some(tg.nonce);
                let ix = (tg.line.max(1) as usize - 1).min(s.lines.len().saturating_sub(1));
                s.scroll.scroll_to_item(ix, ScrollStrategy::Center);
            }
        }
        (s.lines.clone(), s.longest, s.scroll.clone())
    });
    let digits = lines.len().to_string().len().max(3);
    let gutter = px(digits as f32 * 7.4 + 22.);
    let text = text.clone();
    let target = target.cloned();
    let count = lines.len();
    let list = uniform_list("ex-code-lines", count, move |range, _, cx| {
        let t = theme::theme(cx);
        range
            .map(|ix| {
                let line_no = ix + 1;
                let is_target = target
                    .as_ref()
                    .is_some_and(|tg| tg.line as usize == line_no);
                let cols = target.as_ref().filter(|_| is_target).and_then(|tg| tg.cols);
                let (shown, hit) = display_line(&text[lines[ix].clone()], cols);
                let styled = StyledText::new(shown).with_highlights(hit.map(|h| {
                    (
                        h,
                        HighlightStyle {
                            background_color: Some(t.amber_soft),
                            color: Some(t.text),
                            ..Default::default()
                        },
                    )
                }));
                div()
                    .id(ix)
                    .w_full()
                    .h(px(ROW_H))
                    .flex()
                    .items_center()
                    .when(is_target, |d| d.bg(t.surface_3))
                    .child(
                        div()
                            .w(gutter)
                            .flex_none()
                            .pr(px(14.))
                            .flex()
                            .justify_end()
                            .text_color(if is_target { t.text_2 } else { t.text_4 })
                            .child(SharedString::from(line_no.to_string())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .pr(px(24.))
                            .whitespace_nowrap()
                            .text_color(t.text)
                            .child(styled),
                    )
            })
            .collect()
    })
    .track_scroll(scroll)
    .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
    .with_width_from_item(Some(longest))
    .size_full();
    div()
        .id("ex-code")
        .size_full()
        .min_h_0()
        .bg(t.term_bg)
        .font_family(mono())
        .text_size(px(12.5))
        .line_height(px(ROW_H))
        .pt(px(4.))
        .child(list)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_keep_crlf_out_and_find_the_longest() {
        let (lines, longest) = index_lines("a\r\nbbb\n\ncc");
        let text = "a\r\nbbb\n\ncc";
        let got: Vec<&str> = lines.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(got, ["a", "bbb", "", "cc"]);
        assert_eq!(longest, 1);
        assert_eq!(index_lines("").0.len(), 1);
        assert_eq!(index_lines("x\n").0.len(), 2);
    }

    #[test]
    fn display_lines_expand_tabs_and_mark_char_columns() {
        let (s, hit) = display_line("\tfoo(bar)", Some((5, 8)));
        assert_eq!(s, "    foo(bar)");
        assert_eq!(&s[hit.unwrap()], "bar");
        // Georgian: 3 bytes a letter; columns are chars.
        let (s, hit) = display_line("let სახელი = 1;", Some((4, 10)));
        assert_eq!(&s[hit.unwrap()], "სახელი");
        // A match running to the end of the line.
        let (s, hit) = display_line("x = ბოლო", Some((4, 8)));
        assert_eq!(&s[hit.unwrap()], "ბოლო");
        assert_eq!(display_line("abc", None).1, None);
    }

    #[test]
    fn very_long_lines_are_cut() {
        let long = "x".repeat(MAX_COLS + 50);
        let (s, _) = display_line(&long, None);
        assert_eq!(s.chars().count(), MAX_COLS + 1);
        assert!(s.ends_with('…'));
    }
}
