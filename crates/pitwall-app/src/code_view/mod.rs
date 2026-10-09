//! A read-only code and diff viewer, built in-house
//! (docs/spec/gpui/in-house.md "Code / diff viewer"): Review's diff editor
//! and the code explorer's file viewer (Tauri: CodeMirror 6 +
//! @codemirror/merge in `src/components/review/diffView.ts`,
//! `editorSetup.ts`, `findPanel.ts`).
//!
//! [`CodeView`] is one entity showing either a file
//! ([`CodeView::set_file`]) or two versions of one
//! ([`CodeView::set_diff`]), inline or side by side ([`Layout`]):
//! virtualized rows with line numbers, syntax highlighting (tree-sitter),
//! bracket pair colours, indent guides, selection and copy, find (⌘F) with
//! case / whole word / regex / in selection, collapsed unchanged regions,
//! next/previous change, an overview ruler and overlay scrollbars, an
//! optional comment gutter (it emits [`CodeViewEvent::Comment`]) and the
//! "Read-only" hint when typing. All text is UTF-8 and measured by the
//! shaper, so any script (Georgian included) lines up.
//!
//! Lines in this API are 0-based; what the user sees is 1-based.

pub mod diff;
pub mod find;
mod brackets;
pub mod highlight;
mod markdown;
/// The text field moved to the kit ([`crate::kit::input`]); these names
/// stay for code written against Review's own.
pub mod input {
    pub use crate::kit::input::{InputEvent, TextFieldEvent, TextInput as TextField};
}
mod paint;
pub mod rows;
pub mod style;
pub mod text;

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use gpui::{
    actions, div, prelude::*, px, App, ClipboardItem, Context, Entity, EventEmitter, FocusHandle,
    Focusable, KeyBinding, KeyDownEvent, Pixels, Point, SharedString, Subscription, Task, Window,
};

pub use rows::{Layout, Side};

use diff::FileDiff;
use find::{Found, Query};
use highlight::{Lang, LineSpans};
use crate::kit::{InputEvent, TextInput};
use rows::Rows;
use text::{Document, Pos};

actions!(
    code_view,
    [
        MoveUp,
        MoveDown,
        MoveLeft,
        MoveRight,
        SelectUp,
        SelectDown,
        SelectLeft,
        SelectRight,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        LineStart,
        LineEnd,
        SelectLineStart,
        SelectLineEnd,
        PageUp,
        PageDown,
        SelectPageUp,
        SelectPageDown,
        DocStart,
        DocEnd,
        SelectDocStart,
        SelectDocEnd,
        SelectAll,
        Copy,
        /// ⌘F: Find, seeded with the selection or the word at the cursor.
        Find,
        FindNext,
        FindPrevious,
        /// The next change of a diff (F7, Alt+F5).
        NextChange,
        /// The previous change of a diff (Shift+F7, Shift+Alt+F5).
        PreviousChange,
        /// Esc: close the menu, the read-only hint or Find.
        Cancel,
    ]
);

const CONTEXT: &str = "CodeView";

/// Key bindings of the view and its text fields (call once at start).
pub fn register(cx: &mut App) {
    cx.bind_keys(bindings());
}

pub fn bindings() -> Vec<KeyBinding> {
    let c = Some(CONTEXT);
    let mac = cfg!(target_os = "macos");
    let m = |k: &str| {
        if mac {
            format!("cmd-{k}")
        } else {
            format!("ctrl-{k}")
        }
    };
    let w = |k: &str| {
        if mac {
            format!("alt-{k}")
        } else {
            format!("ctrl-{k}")
        }
    };
    let mut b = vec![
        KeyBinding::new("up", MoveUp, c),
        KeyBinding::new("down", MoveDown, c),
        KeyBinding::new("left", MoveLeft, c),
        KeyBinding::new("right", MoveRight, c),
        KeyBinding::new("shift-up", SelectUp, c),
        KeyBinding::new("shift-down", SelectDown, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new(&w("left"), WordLeft, c),
        KeyBinding::new(&w("right"), WordRight, c),
        KeyBinding::new(&w("shift-left"), SelectWordLeft, c),
        KeyBinding::new(&w("shift-right"), SelectWordRight, c),
        KeyBinding::new("home", LineStart, c),
        KeyBinding::new("end", LineEnd, c),
        KeyBinding::new("shift-home", SelectLineStart, c),
        KeyBinding::new("shift-end", SelectLineEnd, c),
        KeyBinding::new("pageup", PageUp, c),
        KeyBinding::new("pagedown", PageDown, c),
        KeyBinding::new("shift-pageup", SelectPageUp, c),
        KeyBinding::new("shift-pagedown", SelectPageDown, c),
        KeyBinding::new(&m("a"), SelectAll, c),
        KeyBinding::new(&m("c"), Copy, c),
        KeyBinding::new(&m("f"), Find, c),
        KeyBinding::new("f3", FindNext, c),
        KeyBinding::new("shift-f3", FindPrevious, c),
        KeyBinding::new(&m("g"), FindNext, c),
        KeyBinding::new(&m("shift-g"), FindPrevious, c),
        KeyBinding::new("f7", NextChange, c),
        KeyBinding::new("shift-f7", PreviousChange, c),
        KeyBinding::new("alt-f5", NextChange, c),
        KeyBinding::new("shift-alt-f5", PreviousChange, c),
        KeyBinding::new("escape", Cancel, c),
    ];
    if mac {
        b.extend([
            KeyBinding::new("cmd-up", DocStart, c),
            KeyBinding::new("cmd-down", DocEnd, c),
            KeyBinding::new("cmd-shift-up", SelectDocStart, c),
            KeyBinding::new("cmd-shift-down", SelectDocEnd, c),
            KeyBinding::new("cmd-left", LineStart, c),
            KeyBinding::new("cmd-right", LineEnd, c),
            KeyBinding::new("cmd-shift-left", SelectLineStart, c),
            KeyBinding::new("cmd-shift-right", SelectLineEnd, c),
        ]);
    } else {
        b.extend([
            KeyBinding::new("ctrl-home", DocStart, c),
            KeyBinding::new("ctrl-end", DocEnd, c),
            KeyBinding::new("ctrl-shift-home", SelectDocStart, c),
            KeyBinding::new("ctrl-shift-end", SelectDocEnd, c),
        ]);
    }
    b
}

/// What the view tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeViewEvent {
    /// The user asked to comment on `line` (0-based) of the modified file:
    /// a click in the margin or "Add review comment".
    Comment { line: usize },
    /// New content is laid out (after `set_file` / `set_diff`).
    Loaded,
}

impl EventEmitter<CodeViewEvent> for CodeView {}

/// The selection: in one version, from `anchor` to `head` (the cursor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub side: Side,
    pub anchor: Pos,
    pub head: Pos,
}

impl Selection {
    pub fn caret(side: Side, at: Pos) -> Selection {
        Selection {
            side,
            anchor: at,
            head: at,
        }
    }

    pub fn range(&self) -> (Pos, Pos) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// What the view shows.
#[derive(Clone)]
struct Content {
    path: SharedString,
    old_text: Arc<str>,
    new_text: Arc<str>,
    /// A diff (else a single file: `new_*`).
    is_diff: bool,
    old: Arc<Document>,
    new: Arc<Document>,
    diff: Arc<FileDiff>,
    old_hl: Arc<Vec<LineSpans>>,
    new_hl: Arc<Vec<LineSpans>>,
}

impl Content {
    fn doc(&self, side: Side) -> &Document {
        match side {
            Side::Old => &self.old,
            Side::New => &self.new,
        }
    }

    fn spans(&self, side: Side, line: usize) -> &[(Range<usize>, highlight::Style)] {
        let hl = match side {
            Side::Old => &self.old_hl,
            Side::New => &self.new_hl,
        };
        hl.get(line).map(Vec::as_slice).unwrap_or(&[])
    }
}

struct FindState {
    field: Entity<TextInput>,
    query: Query,
    found: Found,
    /// Index of the match that is selected.
    current: Option<usize>,
    /// Search only here (Find in Selection).
    within: Option<(Pos, Pos)>,
    side: Side,
    /// Where typing starts looking (the selection when Find opened).
    origin: Pos,
    _sub: Subscription,
}

struct Menu {
    at: Point<Pixels>,
    /// The modified file's line it was opened on (`None` on the original).
    line: Option<usize>,
    active: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    Select { side: Side },
    VBar { grab: Pixels },
    HBar,
    Ruler { grab: Pixels },
}

pub struct CodeView {
    focus: FocusHandle,
    content: Option<Content>,
    layout: Layout,
    /// Collapse unchanged regions in diffs.
    fold: bool,
    expanded: HashSet<usize>,
    rows: Rows,
    scroll_y: Pixels,
    scroll_x: Pixels,
    /// Widest line seen (for horizontal scrolling).
    max_width: Pixels,
    sel: Option<Selection>,
    goal_x: Option<Pixels>,
    hover_line: Option<usize>,
    hover_comment: Option<(usize, Point<Pixels>)>,
    comments: BTreeMap<usize, String>,
    commentable: bool,
    readonly_text: SharedString,
    readonly_at: Option<Point<Pixels>>,
    find: Option<FindState>,
    menu: Option<Menu>,
    drag: Option<Drag>,
    hovered: bool,
    geom: Option<paint::Geom>,
    pending_reveal: Option<(Side, usize, Option<Range<usize>>)>,
    loading: bool,
    generation: u64,
    /// Esc with nothing to close propagates ([`CodeView::set_escape_passes`]).
    escape_passes: bool,
    _load: Option<Task<()>>,
}

impl Focusable for CodeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl CodeView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        CodeView {
            focus: cx.focus_handle(),
            content: None,
            layout: Layout::Split,
            fold: true,
            expanded: HashSet::new(),
            rows: Rows::default(),
            scroll_y: px(0.),
            scroll_x: px(0.),
            max_width: px(0.),
            sel: None,
            goal_x: None,
            hover_line: None,
            hover_comment: None,
            comments: BTreeMap::new(),
            commentable: false,
            readonly_text: "Read-only".into(),
            readonly_at: None,
            find: None,
            menu: None,
            drag: None,
            hovered: false,
            geom: None,
            pending_reveal: None,
            loading: false,
            generation: 0,
            escape_passes: false,
            _load: None,
        }
    }

    // ── content ─────────────────────────────────────────────────────────

    /// Show one file, read-only.
    pub fn set_file(&mut self, path: &str, text: String, cx: &mut Context<Self>) {
        self.load(path, String::new(), text, false, cx);
    }

    /// Show `old` → `new` of `path` as a diff. The same texts again are
    /// ignored (a refresh keeps the scroll position and selection).
    pub fn set_diff(&mut self, path: &str, old: String, new: String, cx: &mut Context<Self>) {
        self.load(path, old, new, true, cx);
    }

    /// Show nothing.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.content = None;
        self.rows = Rows::default();
        self.sel = None;
        self.find = None;
        self.menu = None;
        self.loading = false;
        cx.notify();
    }

    fn load(
        &mut self,
        path: &str,
        old: String,
        new: String,
        is_diff: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(c) = &self.content {
            if c.path.as_ref() == path
                && c.is_diff == is_diff
                && *c.old_text == *old
                && *c.new_text == *new
            {
                return;
            }
        }
        let same_path = self
            .content
            .as_ref()
            .is_some_and(|c| c.path.as_ref() == path);
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        let path: SharedString = path.to_string().into();
        let lang = Lang::for_path(&path);
        let bg = cx.background_executor().clone();
        self._load = Some(cx.spawn(async move |this, cx| {
            let (old_text, new_text): (Arc<str>, Arc<str>) = (old.into(), new.into());
            let (o, n) = (old_text.clone(), new_text.clone());
            // First the lines and the diff (fast), then highlighting.
            let (old_doc, new_doc, d) = bg
                .spawn(async move {
                    let d = if is_diff {
                        diff::diff(&o, &n)
                    } else {
                        FileDiff::default()
                    };
                    (Document::new(&o), Document::new(&n), d)
                })
                .await;
            let content = Content {
                path: path.clone(),
                old_text: old_text.clone(),
                new_text: new_text.clone(),
                is_diff,
                old: Arc::new(old_doc),
                new: Arc::new(new_doc),
                diff: Arc::new(d),
                old_hl: Arc::default(),
                new_hl: Arc::default(),
            };
            let ok = this.update(cx, |v, cx| {
                if v.generation != generation {
                    return false;
                }
                v.apply(content, same_path, cx);
                true
            });
            if !matches!(ok, Ok(true)) || lang.is_none() {
                return;
            }
            let (o, n) = (old_text.clone(), new_text.clone());
            let (ohl, nhl) = bg
                .spawn(async move {
                    let ohl = if is_diff {
                        highlight::highlight(&o, lang)
                    } else {
                        Vec::new()
                    };
                    (ohl, highlight::highlight(&n, lang))
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                if v.generation != generation {
                    return;
                }
                if let Some(c) = v.content.as_mut() {
                    c.old_hl = Arc::new(ohl);
                    c.new_hl = Arc::new(nhl);
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn apply(&mut self, content: Content, same_path: bool, cx: &mut Context<Self>) {
        let diff_changed = self
            .content
            .as_ref()
            .is_none_or(|c| c.diff.hunks != content.diff.hunks);
        if !same_path || diff_changed {
            self.expanded.clear();
        }
        if !same_path {
            self.scroll_y = px(0.);
            self.scroll_x = px(0.);
            self.max_width = px(0.);
            self.sel = None;
            self.find = None;
            self.menu = None;
        }
        self.content = Some(content);
        self.loading = false;
        self.rebuild_rows();
        if let Some(s) = self.sel {
            // Keep the selection inside the new text.
            let doc = self.doc(s.side);
            self.sel = Some(Selection {
                side: s.side,
                anchor: doc.clamp(s.anchor),
                head: doc.clamp(s.head),
            });
        }
        self.refind();
        cx.emit(CodeViewEvent::Loaded);
        cx.notify();
    }

    fn rebuild_rows(&mut self) {
        let Some(c) = &self.content else {
            self.rows = Rows::default();
            return;
        };
        self.rows = if c.is_diff {
            Rows::diff(
                &c.diff,
                c.old.line_count(),
                c.new.line_count(),
                self.layout,
                self.fold,
                &self.expanded,
            )
        } else {
            Rows::file(c.new.line_count())
        };
    }

    /// Inline or side by side (diffs only).
    pub fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        if self.layout == layout {
            return;
        }
        // Keep the line at the top in view.
        let top = self.top_line();
        self.layout = layout;
        if layout == Layout::Unified {
            if let Some(s) = self.sel.filter(|s| s.side == Side::Old) {
                let _ = s;
                self.sel = None;
            }
            if let Some(f) = self.find.as_mut() {
                f.side = Side::New;
            }
        }
        self.rebuild_rows();
        self.refind();
        if let Some((side, line)) = top {
            if let Some(r) = self.rows.row_of(side, line) {
                self.scroll_y = px(self.rows.top(r));
            }
        }
        cx.notify();
    }

    /// Collapse unchanged regions (on by default).
    pub fn set_fold(&mut self, fold: bool, cx: &mut Context<Self>) {
        self.fold = fold;
        self.rebuild_rows();
        cx.notify();
    }

    /// Comments on the modified file's lines (0-based), shown in the margin.
    /// Several on one line are joined.
    pub fn set_comments(
        &mut self,
        comments: impl IntoIterator<Item = (usize, String)>,
        cx: &mut Context<Self>,
    ) {
        let mut map: BTreeMap<usize, String> = BTreeMap::new();
        for (line, text) in comments {
            map.entry(line)
                .and_modify(|t| {
                    t.push_str("\n\n");
                    t.push_str(&text);
                })
                .or_insert(text);
        }
        if map != self.comments {
            self.comments = map;
            cx.notify();
        }
    }

    /// Whether the margin starts comments, and what typing says.
    pub fn set_commentable(&mut self, on: bool, cx: &mut Context<Self>) {
        self.commentable = on;
        self.readonly_text = if on {
            "Read-only: click a line number to comment".into()
        } else {
            "Read-only".into()
        };
        cx.notify();
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn has_content(&self) -> bool {
        self.content.is_some()
    }

    pub fn path(&self) -> Option<&str> {
        self.content.as_ref().map(|c| c.path.as_ref())
    }

    /// Number of changes (hunks) in the diff.
    pub fn change_count(&self) -> usize {
        self.content.as_ref().map_or(0, |c| c.diff.hunks.len())
    }

    pub fn selection(&self) -> Option<Selection> {
        self.sel
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// Scroll `line` (0-based) of `side` to the middle and put the cursor
    /// there, selecting `cols` of it when given. Folded lines are unfolded.
    /// Before the content is laid out, it happens once it is.
    pub fn reveal(
        &mut self,
        side: Side,
        line: usize,
        cols: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        self.pending_reveal = Some((side, line, cols));
        self.apply_pending_reveal();
        cx.notify();
    }

    fn apply_pending_reveal(&mut self) {
        let Some((side, line, cols)) = self.pending_reveal.clone() else {
            return;
        };
        let Some(c) = &self.content else { return };
        if self.loading || self.geom.is_none() {
            return;
        }
        let side = if !c.is_diff || self.layout == Layout::Unified {
            Side::New
        } else {
            side
        };
        if line >= c.doc(side).line_count() {
            self.pending_reveal = None;
            return;
        }
        if let Some(region) = self.rows.region_hiding(side, line) {
            self.expanded.insert(region);
            self.rebuild_rows();
        }
        let doc = self.doc(side);
        let (a, b) = match cols {
            Some(r) => (
                doc.clamp(Pos::new(line, r.start)),
                doc.clamp(Pos::new(line, r.end)),
            ),
            None => (Pos::new(line, 0), Pos::new(line, 0)),
        };
        self.sel = Some(Selection {
            side,
            anchor: a,
            head: b,
        });
        self.pending_reveal = None;
        if let Some(row) = self.rows.row_of(side, line) {
            self.center_row(row);
        }
    }

    /// Open Find (as ⌘F does).
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_action(&Find, window, cx);
    }

    /// Go to the next change; whether there is one.
    pub fn next_change(&mut self, cx: &mut Context<Self>) -> bool {
        self.go_change(true, cx)
    }

    pub fn previous_change(&mut self, cx: &mut Context<Self>) -> bool {
        self.go_change(false, cx)
    }

    // ── helpers ─────────────────────────────────────────────────────────

    fn doc(&self, side: Side) -> &Document {
        self.content.as_ref().map(|c| c.doc(side)).expect("content")
    }

    fn viewport_h(&self) -> Pixels {
        self.geom.as_ref().map_or(px(400.), |g| g.text_h())
    }

    fn top_pad(&self) -> Pixels {
        if self.find.is_some() {
            px(paint::FIND_ZONE)
        } else {
            px(0.)
        }
    }

    fn max_scroll_y(&self) -> Pixels {
        (px(self.rows.height()) + self.top_pad() - self.viewport_h()).max(px(0.))
    }

    fn clamp_scroll(&mut self) {
        self.scroll_y = self.scroll_y.clamp(px(0.), self.max_scroll_y());
        let max_x = self.geom.as_ref().map_or(px(0.), |g| {
            (self.max_width - g.text_w() + px(paint::CHAR_SLACK)).max(px(0.))
        });
        self.scroll_x = self.scroll_x.clamp(px(0.), max_x);
    }

    /// The first fully shown line (for keeping it in view across layouts).
    fn top_line(&self) -> Option<(Side, usize)> {
        let r = self.rows.row_at(f32::from(self.scroll_y));
        let row = self.rows.rows.get(r)?;
        row.line(Side::New)
            .map(|l| (Side::New, l))
            .or_else(|| row.line(Side::Old).map(|l| (Side::Old, l)))
    }

    fn center_row(&mut self, row: usize) {
        let h = self.viewport_h();
        let y = px(self.rows.top(row)) + self.top_pad();
        self.scroll_y = y - h / 2. + px(rows::LINE_H / 2.);
        self.clamp_scroll();
    }

    /// Scroll the least needed to show `row`.
    fn scroll_to_row(&mut self, row: usize) {
        let h = self.viewport_h();
        let y = px(self.rows.top(row)) + self.top_pad();
        let rh = px(self.rows.rows.get(row).map_or(rows::LINE_H, |r| r.height()));
        if y < self.scroll_y + self.top_pad() {
            self.scroll_y = y - self.top_pad();
        } else if y + rh > self.scroll_y + h {
            self.scroll_y = y + rh - h;
        }
        self.clamp_scroll();
    }

    fn go_change(&mut self, forward: bool, cx: &mut Context<Self>) -> bool {
        let from = match self.sel {
            Some(s) => self
                .rows
                .row_of(s.side, s.head.line)
                .unwrap_or_else(|| self.rows.row_at(f32::from(self.scroll_y))),
            None if forward => self.rows.row_at(f32::from(self.scroll_y)).saturating_sub(1),
            None => self.rows.row_at(f32::from(self.scroll_y)),
        };
        let target = if forward {
            self.rows.next_hunk(from)
        } else {
            self.rows.prev_hunk(from)
        };
        let Some(row) = target else { return false };
        if let Some(r) = self.rows.rows.get(row) {
            let at = r
                .line(Side::New)
                .map(|l| (Side::New, l))
                .or_else(|| r.line(Side::Old).map(|l| (Side::Old, l)));
            if let Some((side, line)) = at {
                let side = if self.layout == Layout::Unified {
                    Side::New
                } else {
                    side
                };
                let line = if side == Side::New {
                    r.line(Side::New).unwrap_or(line)
                } else {
                    line
                };
                self.sel = Some(Selection::caret(side, Pos::new(line, 0)));
            }
        }
        self.center_row(row);
        cx.notify();
        true
    }

    /// The side a selection may live on when clicking `pane_side`.
    fn selectable_side(&self, side: Side) -> Side {
        match self.content.as_ref() {
            Some(c) if c.is_diff && self.layout == Layout::Split => side,
            _ => Side::New,
        }
    }

    // ── find ────────────────────────────────────────────────────────────

    fn refind(&mut self) {
        let Some(f) = self.find.as_mut() else { return };
        let Some(c) = self.content.as_ref() else {
            return;
        };
        f.found = find::find_all(c.doc(f.side), &f.query, f.within);
        f.current = self.sel.filter(|s| s.side == f.side).and_then(|s| {
            let (a, b) = s.range();
            f.found
                .matches
                .iter()
                .position(|m| m.start() == a && m.end() == b)
        });
    }

    fn find_action(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.is_none() {
            return;
        }
        let side = self.sel.map_or(Side::New, |s| s.side);
        // Seed: the selection (one line), or the word at the cursor.
        let mut seed = None;
        if let Some(s) = self.sel {
            let doc = self.doc(s.side);
            let (a, b) = s.range();
            if s.is_empty() {
                if let Some(line) = doc.line(a.line) {
                    let w = text::word_at(&line.text, a.col);
                    let word = line.text[w.clone()].to_string();
                    if word.chars().any(text::is_word_char) {
                        seed = Some(word);
                        self.sel = Some(Selection {
                            side: s.side,
                            anchor: Pos::new(a.line, w.start),
                            head: Pos::new(a.line, w.end),
                        });
                    }
                }
            } else if a.line == b.line {
                seed = Some(doc.text_between(a, b));
            }
        }
        let origin = self.sel.map_or(Pos::default(), |s| s.range().0);
        if let Some(f) = self.find.as_mut() {
            f.side = side;
            f.origin = origin;
            let field = f.field.clone();
            field.update(cx, |t, cx| {
                if let Some(s) = seed {
                    t.replace_text(&s, cx);
                }
                t.select_all_text(cx);
                t.focus(window);
            });
            return;
        }
        let field = cx.new(|cx| TextInput::new(cx, "", "Find").plain());
        let sub = cx.subscribe_in(
            &field,
            window,
            |this, field, e: &InputEvent, window, cx| match e {
                InputEvent::Changed => {
                    let text = field.read(cx).text().to_string();
                    this.set_query(|q| q.text = text, true, cx);
                }
                InputEvent::Submit => this.find_next(&FindNext, window, cx),
                InputEvent::SubmitShift => this.find_previous(&FindPrevious, window, cx),
                InputEvent::Cancel => this.close_find(window, cx),
                InputEvent::Blur => {}
            },
        );
        let seed_text = seed.clone().unwrap_or_default();
        self.find = Some(FindState {
            field: field.clone(),
            query: Query {
                text: seed_text.clone(),
                ..Query::default()
            },
            found: Found::default(),
            current: None,
            within: None,
            side,
            origin,
            _sub: sub,
        });
        // The zone above the first line: keep what is shown in place.
        if self.scroll_y > px(0.) {
            self.scroll_y += px(paint::FIND_ZONE);
        }
        field.update(cx, |t, cx| {
            t.replace_text(&seed_text, cx);
            t.select_all_text(cx);
            t.focus(window);
        });
        self.refind();
        cx.notify();
    }

    /// Change the query; `jump`: select the first match from where Find
    /// opened (as typing does).
    fn set_query(&mut self, change: impl FnOnce(&mut Query), jump: bool, cx: &mut Context<Self>) {
        let Some(f) = self.find.as_mut() else { return };
        change(&mut f.query);
        let origin = f.origin;
        self.refind();
        if jump {
            let f = self.find.as_ref().expect("find");
            if let Some(i) = find::next_from(&f.found.matches, origin) {
                self.select_match(i);
            }
        }
        cx.notify();
    }

    fn select_match(&mut self, i: usize) {
        let Some(f) = self.find.as_mut() else { return };
        let Some(m) = f.found.matches.get(i).cloned() else {
            return;
        };
        f.current = Some(i);
        let side = f.side;
        if let Some(region) = self.rows.region_hiding(side, m.line) {
            self.expanded.insert(region);
            self.rebuild_rows();
        }
        self.sel = Some(Selection {
            side,
            anchor: m.start(),
            head: m.end(),
        });
        if let Some(row) = self.rows.row_of(side, m.line) {
            let before = self.scroll_y;
            self.scroll_to_row(row);
            if self.scroll_y != before {
                self.center_row(row);
            }
        }
    }

    fn find_next(&mut self, _: &FindNext, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_none() {
            return self.find_action(&Find, window, cx);
        }
        let f = self.find.as_ref().expect("find");
        let from = self
            .sel
            .filter(|s| s.side == f.side)
            .map_or(f.origin, |s| s.range().1);
        if let Some(i) = find::next_after(&f.found.matches, from) {
            self.select_match(i);
        }
        cx.notify();
    }

    fn find_previous(&mut self, _: &FindPrevious, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_none() {
            return self.find_action(&Find, window, cx);
        }
        let f = self.find.as_ref().expect("find");
        let from = self
            .sel
            .filter(|s| s.side == f.side)
            .map_or(f.origin, |s| s.range().0);
        if let Some(i) = find::prev_before(&f.found.matches, from) {
            self.select_match(i);
        }
        cx.notify();
    }

    fn toggle_in_selection(&mut self, cx: &mut Context<Self>) {
        let Some(f) = self.find.as_mut() else { return };
        f.within = match (f.within, self.sel) {
            (Some(_), _) => None,
            (None, Some(s)) if !s.is_empty() && s.side == f.side => Some(s.range()),
            _ => None,
        };
        self.refind();
        cx.notify();
    }

    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.take().is_some() {
            if self.scroll_y > px(0.) {
                self.scroll_y = (self.scroll_y - px(paint::FIND_ZONE)).max(px(0.));
            }
            window.focus(&self.focus);
            cx.notify();
        }
    }

    // ── keyboard ────────────────────────────────────────────────────────

    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.take().is_some() || self.readonly_at.take().is_some() {
            cx.notify();
            return;
        }
        if self.find.is_some() {
            self.close_find(window, cx);
            return;
        }
        // Esc stays in the editor (Review: the editor has it), as in the
        // React app; a view in a dialog lets it close the dialog.
        if self.escape_passes {
            cx.propagate();
        }
    }

    /// Esc with nothing to close goes on to the view's owner (a dialog).
    pub fn set_escape_passes(&mut self, on: bool) {
        self.escape_passes = on;
    }

    fn sel_or_start(&self) -> Selection {
        self.sel.unwrap_or_else(|| {
            let side = self
                .top_line()
                .map_or(Side::New, |(s, _)| self.selectable_side(s));
            let line = self.top_line().map_or(0, |(_, l)| l);
            Selection::caret(side, Pos::new(line, 0))
        })
    }

    fn set_head(&mut self, head: Pos, extend: bool, keep_goal: bool, cx: &mut Context<Self>) {
        let s = self.sel_or_start();
        let head = self.doc(s.side).clamp(head);
        self.sel = Some(Selection {
            side: s.side,
            anchor: if extend { s.anchor } else { head },
            head,
        });
        if !keep_goal {
            self.goal_x = None;
        }
        self.readonly_at = None;
        if let Some(row) = self.rows.row_of(s.side, head.line) {
            self.scroll_to_row(row);
        }
        if let Some(f) = self.find.as_mut() {
            f.current = None;
        }
        cx.notify();
    }

    /// The line `delta` visible lines away on the selection's side.
    fn line_by(&self, side: Side, line: usize, delta: isize) -> usize {
        let Some(mut r) = self.rows.row_of(side, line) else {
            return line;
        };
        let mut left = delta.unsigned_abs();
        let mut last = line;
        while left > 0 {
            let next = if delta > 0 { r + 1 } else { r.wrapping_sub(1) };
            let Some(row) = self.rows.rows.get(next) else {
                break;
            };
            r = next;
            if let Some(l) = row.line(side) {
                last = l;
                left -= 1;
            }
        }
        last
    }

    fn vertical(
        &mut self,
        delta: isize,
        extend: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.content.is_none() {
            return;
        }
        let s = self.sel_or_start();
        let doc = self.doc(s.side);
        let line = self.line_by(s.side, s.head.line, delta);
        let x = self
            .goal_x
            .unwrap_or_else(|| paint::x_for(window, &doc.lines[s.head.line].text, s.head.col));
        let target = &doc.lines[line].text;
        let col = if line == s.head.line && delta != 0 {
            if delta < 0 {
                0
            } else {
                target.len()
            }
        } else {
            paint::col_for(window, target, x)
        };
        self.set_head(Pos::new(line, col), extend, true, cx);
        self.goal_x = Some(x);
    }

    fn horizontal(
        &mut self,
        f: impl Fn(&str, usize) -> usize,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        if self.content.is_none() {
            return;
        }
        let s = self.sel_or_start();
        if !extend && !s.is_empty() {
            // Collapse to the side of travel is done by callers via `f`.
        }
        let doc = self.doc(s.side);
        let text = &doc.lines[s.head.line].text;
        let col = f(text, s.head.col);
        let head = if col == usize::MAX {
            // Past the end: next line start.
            if s.head.line + 1 < doc.line_count() {
                Pos::new(self.line_by(s.side, s.head.line, 1), 0)
            } else {
                s.head
            }
        } else if col == usize::MAX - 1 {
            // Before the start: previous line end.
            if s.head.line > 0 {
                let l = self.line_by(s.side, s.head.line, -1);
                Pos::new(l, doc.lines[l].len())
            } else {
                s.head
            }
        } else {
            Pos::new(s.head.line, col)
        };
        self.set_head(head, extend, false, cx);
    }

    fn page(&mut self, down: bool, extend: bool, window: &mut Window, cx: &mut Context<Self>) {
        let n = (f32::from(self.viewport_h()) / rows::LINE_H).max(1.0) as isize - 1;
        let d = if down { n } else { -n };
        self.scroll_y += px(rows::LINE_H * d as f32);
        self.clamp_scroll();
        self.vertical(d, extend, window, cx);
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.sel else { return };
        if self.content.is_none() {
            return;
        }
        let doc = self.doc(s.side);
        let text = if s.is_empty() {
            // Nothing selected: the whole line, as the editor's own copy.
            let mut l = doc.lines[s.head.line].source().to_string();
            l.push('\n');
            l
        } else {
            let (a, b) = s.range();
            doc.text_between(a, b)
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        if self.content.is_none() {
            return;
        }
        let side = self.sel.map_or(Side::New, |s| s.side);
        let end = self.doc(side).end();
        self.sel = Some(Selection {
            side,
            anchor: Pos::default(),
            head: end,
        });
        cx.notify();
    }

    /// Move the open context menu's highlight by `delta` (wrapping); false
    /// when no menu is open.
    fn menu_step(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let commentable = self.commentable;
        let Some(menu) = self.menu.as_mut() else {
            return false;
        };
        let n = paint::menu_items(menu.line.is_some() && commentable).len();
        menu.active = Some(step_wrap(menu.active, delta, n));
        cx.notify();
        true
    }

    /// Typing into a read-only view says so (as Monaco did).
    fn on_key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if let Some(menu) = self.menu.as_mut() {
            let n = paint::menu_items(menu.line.is_some() && self.commentable).len();
            match k.key.as_str() {
                "down" => menu.active = Some(step_wrap(menu.active, 1, n)),
                "up" => menu.active = Some(step_wrap(menu.active, -1, n)),
                "enter" | "space" => {
                    if let Some(a) = menu.active {
                        self.run_menu(a, cx);
                    }
                }
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let m = &k.modifiers;
        let cut_or_paste = (m.platform || m.control) && (k.key == "x" || k.key == "v");
        if (m.platform || m.control || m.alt || m.function) && !cut_or_paste {
            return;
        }
        let typing = k.key_char.as_ref().is_some_and(|c| !c.is_empty())
            || matches!(k.key.as_str(), "backspace" | "delete" | "enter" | "tab");
        if typing || cut_or_paste {
            if let (Some(g), Some(s)) = (self.geom.as_ref(), self.sel) {
                self.readonly_at = g.caret_point(self, s);
            } else {
                self.readonly_at = self
                    .geom
                    .as_ref()
                    .map(|g| g.bounds.origin + Point::new(px(80.), px(24.)));
            }
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn run_menu(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.take() else { return };
        let items = paint::menu_items(menu.line.is_some() && self.commentable);
        match items.get(ix).copied() {
            Some(paint::MenuItem::Comment) => {
                if let Some(line) = menu.line {
                    cx.emit(CodeViewEvent::Comment { line });
                }
            }
            Some(paint::MenuItem::Copy) => {
                let Some(s) = self.sel else { return };
                let doc = self.doc(s.side);
                let text = if s.is_empty() {
                    format!("{}\n", doc.lines[s.head.line].source())
                } else {
                    let (a, b) = s.range();
                    doc.text_between(a, b)
                };
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            None => {}
        }
        cx.notify();
    }
}

impl Render for CodeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.apply_pending_reveal();
        let t = crate::theme::theme(cx).clone();
        let colors = style::Colors::for_theme(&t);
        let this = cx.entity();
        let mut root = div()
            .id("code-view")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(colors.bg)
            .font_family(crate::kit::MONO_FONT)
            .text_size(style::FONT_SIZE)
            .line_height(style::LINE_HEIGHT)
            .on_hover(cx.listener(|this, h: &bool, _, cx| {
                this.hovered = *h;
                if !*h {
                    this.hover_line = None;
                    this.hover_comment = None;
                }
                cx.notify();
            }))
            .on_key_down(cx.listener(Self::on_key_down))
            // With the context menu open, ↑/↓ move its highlight (the
            // bindings run before key-down listeners).
            .on_action(cx.listener(|v, _: &MoveUp, w, cx| {
                if !v.menu_step(-1, cx) {
                    v.vertical(-1, false, w, cx)
                }
            }))
            .on_action(cx.listener(|v, _: &MoveDown, w, cx| {
                if !v.menu_step(1, cx) {
                    v.vertical(1, false, w, cx)
                }
            }))
            .on_action(cx.listener(|v, _: &SelectUp, w, cx| v.vertical(-1, true, w, cx)))
            .on_action(cx.listener(|v, _: &SelectDown, w, cx| v.vertical(1, true, w, cx)))
            .on_action(cx.listener(|v, _: &PageUp, w, cx| v.page(false, false, w, cx)))
            .on_action(cx.listener(|v, _: &PageDown, w, cx| v.page(true, false, w, cx)))
            .on_action(cx.listener(|v, _: &SelectPageUp, w, cx| v.page(false, true, w, cx)))
            .on_action(cx.listener(|v, _: &SelectPageDown, w, cx| v.page(true, true, w, cx)))
            .on_action(cx.listener(|v, _: &MoveLeft, _, cx| {
                if let Some(s) = v.sel.filter(|s| !s.is_empty()) {
                    let a = s.range().0;
                    return v.set_head(a, false, false, cx);
                }
                v.horizontal(
                    |t, c| {
                        if c == 0 {
                            usize::MAX - 1
                        } else {
                            text::prev_grapheme(t, c)
                        }
                    },
                    false,
                    cx,
                )
            }))
            .on_action(cx.listener(|v, _: &MoveRight, _, cx| {
                if let Some(s) = v.sel.filter(|s| !s.is_empty()) {
                    let b = s.range().1;
                    return v.set_head(b, false, false, cx);
                }
                v.horizontal(
                    |t, c| {
                        if c >= t.len() {
                            usize::MAX
                        } else {
                            text::next_grapheme(t, c)
                        }
                    },
                    false,
                    cx,
                )
            }))
            .on_action(cx.listener(|v, _: &SelectLeft, _, cx| {
                v.horizontal(
                    |t, c| {
                        if c == 0 {
                            usize::MAX - 1
                        } else {
                            text::prev_grapheme(t, c)
                        }
                    },
                    true,
                    cx,
                )
            }))
            .on_action(cx.listener(|v, _: &SelectRight, _, cx| {
                v.horizontal(
                    |t, c| {
                        if c >= t.len() {
                            usize::MAX
                        } else {
                            text::next_grapheme(t, c)
                        }
                    },
                    true,
                    cx,
                )
            }))
            .on_action(
                cx.listener(|v, _: &WordLeft, _, cx| v.horizontal(text::prev_word, false, cx)),
            )
            .on_action(
                cx.listener(|v, _: &WordRight, _, cx| v.horizontal(text::next_word, false, cx)),
            )
            .on_action(
                cx.listener(|v, _: &SelectWordLeft, _, cx| v.horizontal(text::prev_word, true, cx)),
            )
            .on_action(
                cx.listener(|v, _: &SelectWordRight, _, cx| {
                    v.horizontal(text::next_word, true, cx)
                }),
            )
            .on_action(cx.listener(|v, _: &LineStart, _, cx| v.horizontal(|_, _| 0, false, cx)))
            .on_action(cx.listener(|v, _: &LineEnd, _, cx| v.horizontal(|t, _| t.len(), false, cx)))
            .on_action(
                cx.listener(|v, _: &SelectLineStart, _, cx| v.horizontal(|_, _| 0, true, cx)),
            )
            .on_action(
                cx.listener(|v, _: &SelectLineEnd, _, cx| v.horizontal(|t, _| t.len(), true, cx)),
            )
            .on_action(cx.listener(|v, _: &DocStart, _, cx| {
                if v.content.is_some() {
                    v.set_head(Pos::default(), false, false, cx)
                }
            }))
            .on_action(cx.listener(|v, _: &DocEnd, _, cx| {
                if v.content.is_some() {
                    let s = v.sel_or_start();
                    let end = v.doc(s.side).end();
                    v.set_head(end, false, false, cx)
                }
            }))
            .on_action(cx.listener(|v, _: &SelectDocStart, _, cx| {
                if v.content.is_some() {
                    v.set_head(Pos::default(), true, false, cx)
                }
            }))
            .on_action(cx.listener(|v, _: &SelectDocEnd, _, cx| {
                if v.content.is_some() {
                    let s = v.sel_or_start();
                    let end = v.doc(s.side).end();
                    v.set_head(end, true, false, cx)
                }
            }))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::find_action))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(|v, _: &NextChange, _, cx| {
                v.go_change(true, cx);
            }))
            .on_action(cx.listener(|v, _: &PreviousChange, _, cx| {
                v.go_change(false, cx);
            }))
            .on_action(cx.listener(Self::cancel))
            .child(paint::canvas_for(this, colors.clone()));
        if self.content.is_none() && self.loading {
            root = root.child(
                div()
                    .absolute()
                    .top(px(8.))
                    .left(px(12.))
                    .text_color(t.text_3)
                    .font_family(crate::kit::UI_FONT)
                    .text_size(px(12.))
                    .child("Loading…"),
            );
        }
        root = root.children(paint::overlays(self, &colors, &t, window, cx));
        root
    }
}

/// A menu highlight moved by `delta` over `n` items, wrapping; from none,
/// ↓ takes the first and ↑ the last.
fn step_wrap(active: Option<usize>, delta: isize, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    match active {
        None if delta >= 0 => 0,
        None => n - 1,
        Some(a) => (a as isize + delta).rem_euclid(n as isize) as usize,
    }
}

#[cfg(test)]
mod tests;
