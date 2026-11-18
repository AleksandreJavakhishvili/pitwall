//! The seam where a file's text is drawn: [`FileCode`], Review's
//! [`CodeView`] (`crate::code_view`): syntax highlighting, bracket colours,
//! indent guides, selection and copy, Find (⌘F), the target line revealed
//! with a search hit selected, and editing when the viewer allows it. One
//! view per open file, kept by the viewer's document, so each tab keeps its
//! scroll position, selection and unsaved edits.

use std::cell::{Cell, RefCell};

use gpui::{div, prelude::*, AnyElement, App, AppContext, Entity, SharedString};

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

/// One open file's view, kept by the viewer for as long as the tab.
pub struct FileCode {
    pub code: Entity<CodeView>,
    /// The text shown (the same again keeps the view as it is).
    text: RefCell<Option<SharedString>>,
    shown_nonce: Cell<Option<u64>>,
}

/// The same text: the same string shared (cheap), else equal.
fn same(a: &SharedString, b: &SharedString) -> bool {
    a.len() == b.len() && (a.as_ptr() == b.as_ptr() || a == b)
}

#[cfg(test)]
thread_local! {
    /// Each drawn file's view (tests read its selection).
    static SHOWN: std::cell::RefCell<std::collections::HashMap<String, gpui::WeakEntity<CodeView>>> = Default::default();
}

/// The view drawn last for `path` (tests).
#[cfg(test)]
pub(crate) fn shown(path: &str) -> Option<Entity<CodeView>> {
    SHOWN.with(|m| m.borrow().get(path).and_then(|w| w.upgrade()))
}

/// The byte range of `char` columns `cols` on `line` (1-based) of `text`.
pub fn byte_cols(text: &str, line: u32, cols: (usize, usize)) -> Option<std::ops::Range<usize>> {
    let l = text.split('\n').nth(line.max(1) as usize - 1)?;
    let l = l.strip_suffix('\r').unwrap_or(l);
    let at = |c: usize| l.char_indices().nth(c).map_or(l.len(), |(i, _)| i);
    let (a, b) = (at(cols.0), at(cols.1));
    (a < b).then_some(a..b)
}

/// What to select on `line` (1-based): the columns given, else the whole
/// line (its text, without the line break), so a line without a column is
/// marked too. `None` for an empty line or one past the end.
pub fn mark(text: &str, line: u32, cols: Option<(usize, usize)>) -> Option<std::ops::Range<usize>> {
    match cols {
        Some(c) => byte_cols(text, line, c),
        None => {
            let l = text.split('\n').nth(line.max(1) as usize - 1)?;
            let l = l.strip_suffix('\r').unwrap_or(l);
            (!l.is_empty()).then_some(0..l.len())
        }
    }
}

impl FileCode {
    pub fn new(cx: &mut App) -> FileCode {
        FileCode { code: cx.new(CodeView::new), text: RefCell::new(None), shown_nonce: Cell::new(None) }
    }

    /// `text` is what the view holds already (just saved from it): no
    /// reload when it is drawn next.
    pub fn adopt(&self, text: &SharedString) {
        *self.text.borrow_mut() = Some(text.clone());
    }

    /// `path`'s `text` (the whole file), `target` the line to show. A new
    /// text is loaded; the same keeps the view as it is.
    pub fn element(&self, path: &str, text: &SharedString, target: Option<&Target>, cx: &mut App) -> AnyElement {
        #[cfg(test)]
        SHOWN.with(|m| m.borrow_mut().insert(path.to_string(), self.code.downgrade()));
        let load = !self.text.borrow().as_ref().is_some_and(|t| same(t, text));
        if load {
            *self.text.borrow_mut() = Some(text.clone());
        }
        let reveal = target.filter(|tg| self.shown_nonce.get() != Some(tg.nonce)).cloned();
        if let Some(tg) = &reveal {
            self.shown_nonce.set(Some(tg.nonce));
        }
        if load || reveal.is_some() {
            let (path, text) = (path.to_string(), text.clone());
            self.code.update(cx, |c, cx| {
                if load {
                    c.set_file(&path, text.to_string(), cx);
                }
                if let Some(tg) = reveal {
                    let cols = mark(&text, tg.line, tg.cols);
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
            .child(self.code.clone())
            .into_any_element()
    }
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

    #[test]
    fn a_line_without_a_column_is_marked_whole() {
        let text = "a\r\n\tfoo(bar)\n\nlast";
        assert_eq!(mark(text, 2, None), Some(0..9));
        assert_eq!(mark(text, 1, None), Some(0..1));
        assert_eq!(mark(text, 3, None), None);
        assert_eq!(mark(text, 9, None), None);
        assert_eq!(mark(text, 2, Some((1, 4))), Some(1..4));
    }
}
