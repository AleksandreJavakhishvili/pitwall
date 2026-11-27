//! Native building blocks: Pitwall's own code view and diff view, drawn
//! by Pitwall with the text the tile gives (docs/spec/tile-sdk/
//! native-blocks.md §16.2).
//!
//! ```ignore
//! v_flex().size_full()
//!     .child(code_view(self.source.clone()).language("rust").flex_1()
//!         .mark(3..5, Mark::Added)
//!         .on_line_click(cx.listener(|this, l: &LineClick, cx| this.pick(l.line, cx))))
//!     .child(diff_view(self.before.clone(), self.after.clone()).language("main.rs").inline().flex_1())
//! ```
//!
//! Both are read-only: highlighting by language, line numbers, selection
//! and copy, Find (⌘F); a diff adds line and character changes and folds
//! unchanged runs, as Review does. The text goes to Pitwall in each frame
//! (at most 1 MiB a text); the same text again keeps the user's scroll
//! position and selection.

use crate::bindings::pitwall::tile::types as wit;
use crate::element::{builder, AnyElement, Handler, Interactive, IntoElement, Kind, Listen};
use crate::events::{CodeSelection, LineClick};
use crate::style::{StyleSet, Styled};

/// What a margin mark says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Added lines (a green bar).
    Added,
    /// Changed lines (a blue bar).
    Changed,
    /// Lines were deleted before the range's start (a red mark between
    /// lines).
    Deleted,
}

impl Mark {
    fn wit(self) -> wit::LineMarkKind {
        match self {
            Mark::Added => wit::LineMarkKind::Added,
            Mark::Changed => wit::LineMarkKind::Changed,
            Mark::Deleted => wit::LineMarkKind::Deleted,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CodeSpec(pub wit::CodeView);

#[derive(Debug, Clone)]
pub(crate) struct DiffSpec(pub wit::DiffView);

/// Pitwall's read-only code view. Made by [`code_view`].
#[derive(Debug, Clone)]
pub struct CodeView(AnyElement);
builder!(CodeView);

impl CodeView {
    fn spec(&mut self) -> &mut wit::CodeView {
        match &mut self.0.kind {
            Kind::Code(s) => &mut s.0,
            _ => unreachable!("a code view"),
        }
    }

    /// Its language, for highlighting: a name (`rust`, `typescript`,
    /// `python`, `go`, `markdown`, …) or a file name (`main.rs`). Unknown:
    /// plain text.
    pub fn language(mut self, language: impl Into<String>) -> CodeView {
        self.spec().language = Some(language.into());
        self
    }

    /// Mark `lines` (0-based, end exclusive) in the margin.
    pub fn mark(mut self, lines: std::ops::Range<usize>, mark: Mark) -> CodeView {
        self.spec().marks.push(wit::LineMark { lines: wit::LineRange { start: lines.start as u32, end: lines.end as u32 }, kind: mark.wit() });
        self
    }

    /// Scroll `line` (0-based) into the middle and put the cursor on it,
    /// when it differs from the last render's.
    pub fn reveal(mut self, line: usize) -> CodeView {
        self.spec().reveal = Some(line as u32);
        self
    }

    /// Call `handler` when a line number is clicked.
    pub fn on_line_click(mut self, handler: Handler<LineClick>) -> CodeView {
        self.0.listen(Listen::LineClick, handler);
        self
    }

    /// Call `handler` when the selection changes.
    pub fn on_select(mut self, handler: Handler<CodeSelection>) -> CodeView {
        self.0.listen(Listen::Select, handler);
        self
    }
}

/// Pitwall's read-only code view showing `text`. Give it a size
/// (`.flex_1()`, `.size_full()` or a height): it scrolls inside.
pub fn code_view(text: impl Into<String>) -> CodeView {
    let spec = wit::CodeView { text: text.into(), language: None, marks: Vec::new(), reveal: None };
    CodeView(AnyElement::new_kind(Kind::Code(Box::new(CodeSpec(spec)))))
}

/// Pitwall's diff view. Made by [`diff_view`] and [`diff_view_unified`].
#[derive(Debug, Clone)]
pub struct DiffView(AnyElement);
builder!(DiffView);

impl DiffView {
    fn spec(&mut self) -> &mut wit::DiffView {
        match &mut self.0.kind {
            Kind::Diff(s) => &mut s.0,
            _ => unreachable!("a diff view"),
        }
    }

    /// The language, as [`CodeView::language`] takes it.
    pub fn language(mut self, language: impl Into<String>) -> DiffView {
        self.spec().language = Some(language.into());
        self
    }

    /// One column, removed lines above added ones (side by side is the
    /// default).
    pub fn inline(mut self) -> DiffView {
        self.spec().layout = wit::DiffLayout::Inline;
        self
    }

    /// Old on the left, new on the right (the default).
    pub fn side_by_side(mut self) -> DiffView {
        self.spec().layout = wit::DiffLayout::SideBySide;
        self
    }

    /// Scroll `line` of the new version (0-based) into the middle, when
    /// it differs from the last render's.
    pub fn reveal(mut self, line: usize) -> DiffView {
        self.spec().reveal = Some(line as u32);
        self
    }

    /// Call `handler` when a line number is clicked (either side).
    pub fn on_line_click(mut self, handler: Handler<LineClick>) -> DiffView {
        self.0.listen(Listen::LineClick, handler);
        self
    }

    /// Call `handler` when the selection changes.
    pub fn on_select(mut self, handler: Handler<CodeSelection>) -> DiffView {
        self.0.listen(Listen::Select, handler);
        self
    }
}

fn diff(source: wit::DiffSource) -> DiffView {
    let spec = wit::DiffView { source, language: None, layout: wit::DiffLayout::SideBySide, reveal: None };
    DiffView(AnyElement::new_kind(Kind::Diff(Box::new(DiffSpec(spec)))))
}

/// Pitwall's diff view of `old` → `new` (each at most 1 MiB): line and
/// character changes, unchanged runs folded. Give it a size.
pub fn diff_view(old: impl Into<String>, new: impl Into<String>) -> DiffView {
    diff(wit::DiffSource::Texts(wit::DiffTexts { old: old.into(), new: new.into() }))
}

/// Pitwall's diff view of a unified diff of one file (`git diff` output:
/// `@@` hunks with `+`, `-` and context lines; headers are skipped). Lines
/// keep their numbers from the hunk headers; what the diff doesn't show is
/// folded.
pub fn diff_view_unified(unified: impl Into<String>) -> DiffView {
    diff(wit::DiffSource::Unified(unified.into()))
}
