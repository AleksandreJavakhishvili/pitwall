//! The seam where a file's text is drawn: [`code_view`], Review's
//! read-only [`CodeView`] (`crate::code_view`): syntax highlighting, bracket
//! colours, indent guides, selection and copy, Find (⌘F), the target line
//! revealed with a search hit selected. One view per open file, so each tab
//! keeps its scroll position and selection.

use gpui::{div, prelude::*, AnyElement, App, AppContext, ElementId, Entity, SharedString, Window};

use crate::code_view::{CodeView, Side};

/// Where to bring the reader: a line (1-based) and, for a search hit, the
/// match's `char` columns on it (0-based, end exclusive). A new `nonce`
/// scrolls there again.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub line: u32,
    pub cols: Option<(usize, usize)>,
    pub nonce: u64,
}

/// What the viewer keeps for one file between frames.
struct ViewState {
    code: Entity<CodeView>,
    /// The text shown (by identity: the viewer shares one `SharedString`
    /// per read).
    text_id: (usize, usize),
    shown_nonce: Option<u64>,
}

/// The byte range of `char` columns `cols` on `line` (1-based) of `text`.
pub fn byte_cols(text: &str, line: u32, cols: (usize, usize)) -> Option<std::ops::Range<usize>> {
    let l = text.split('\n').nth(line.max(1) as usize - 1)?;
    let l = l.strip_suffix('\r').unwrap_or(l);
    let at = |c: usize| l.char_indices().nth(c).map_or(l.len(), |(i, _)| i);
    let (a, b) = (at(cols.0), at(cols.1));
    (a < b).then_some(a..b)
}

/// One file's text, read-only: `path` names it (and keys its view),
/// `text` is the whole file, `target` the line to show.
pub fn code_view(
    path: &str,
    text: &SharedString,
    target: Option<&Target>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let id = (text.as_ptr() as usize, text.len());
    let state = window.use_keyed_state(
        ElementId::Name(format!("ex-code:{path}").into()),
        cx,
        |_, cx| ViewState {
            code: cx.new(CodeView::new),
            text_id: (0, usize::MAX),
            shown_nonce: None,
        },
    );
    let (code, load, reveal) = state.update(cx, |s, _| {
        let load = s.text_id != id;
        s.text_id = id;
        let reveal = target.filter(|tg| s.shown_nonce != Some(tg.nonce)).cloned();
        if let Some(tg) = &reveal {
            s.shown_nonce = Some(tg.nonce);
        }
        (s.code.clone(), load, reveal)
    });
    if load || reveal.is_some() {
        let (path, text) = (path.to_string(), text.clone());
        code.update(cx, |c, cx| {
            if load {
                c.set_file(&path, text.to_string(), cx);
            }
            if let Some(tg) = reveal {
                let cols = tg.cols.and_then(|cols| byte_cols(&text, tg.line, cols));
                c.reveal(Side::New, tg.line.max(1) as usize - 1, cols, cx);
            }
        });
    }
    div()
        .id("ex-code")
        .debug_selector(|| "ex-code".into())
        .size_full()
        .min_h_0()
        .flex()
        .flex_col()
        .child(code)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_hits_become_byte_ranges() {
        let text = "a\r\n\tfoo(bar)\nlet სახელი = 1;\nx = ბოლო";
        assert_eq!(byte_cols(text, 2, (5, 8)), Some(5..8));
        // Georgian: 3 bytes a letter; columns are chars.
        let r = byte_cols(text, 3, (4, 10)).unwrap();
        assert_eq!(&"let სახელი = 1;"[r], "სახელი");
        // A match running to the end of the line.
        let r = byte_cols(text, 4, (4, 8)).unwrap();
        assert_eq!(&"x = ბოლო"[r], "ბოლო");
        assert_eq!(byte_cols(text, 9, (0, 1)), None);
        assert_eq!(byte_cols(text, 1, (3, 3)), None);
    }
}
