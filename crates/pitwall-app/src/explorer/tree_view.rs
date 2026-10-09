//! An agent's folder as VS Code's Explorer shows it (a port of
//! `ExplorerTree.tsx`): lazy folders, file icons, change letters and a dot
//! on folders with changes, ignored entries dimmed. ↑/↓ move, → / ← open
//! and close folders (or go to the parent), ↵ opens a file. One view per
//! place it is shown; the [`TreeModel`] behind it is shared.

use crate::kit::HoverText as _;
use crate::kit::Ellipsis as _;
use std::rc::Rc;

use gpui::{
    actions, div, prelude::*, px, uniform_list, App, Context, Entity, EventEmitter, FocusHandle,
    Focusable, FontWeight, KeyBinding, MouseButton, ScrollStrategy, SharedString, Subscription,
    UniformListScrollHandle, Window,
};

use pitwall_core::explorer::EntryKind;

use crate::kit::file_icons::{file_icon, folder_icon};
use super::logic::{split_path, TreeRow};
use super::tree::TreeModel;
use super::widgets::status_mark;
use crate::kit::{tooltip_view, MONO_FONT};
use crate::theme;

actions!(explorer_tree, [Up, Down, Collapse, Expand, Confirm]);

pub const CONTEXT: &str = "ExplorerTree";
pub const ROW_H: f32 = 22.;

pub fn bindings() -> Vec<KeyBinding> {
    let c = Some(CONTEXT);
    vec![
        KeyBinding::new("up", Up, c),
        KeyBinding::new("down", Down, c),
        KeyBinding::new("left", Collapse, c),
        KeyBinding::new("right", Expand, c),
        KeyBinding::new("enter", Confirm, c),
        KeyBinding::new("space", Confirm, c),
    ]
}

/// A file was picked (click or ↵).
pub struct OpenFile(pub String);

pub struct TreeView {
    tree: Entity<TreeModel>,
    rows: Rc<Vec<TreeRow>>,
    focus: FocusHandle,
    /// The row the keyboard is on (a path).
    cursor: Option<String>,
    /// The file shown in the viewer (marked).
    selected: Option<String>,
    scroll: UniformListScrollHandle,
    _observe: Subscription,
}

impl EventEmitter<OpenFile> for TreeView {}

impl Focusable for TreeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TreeView {
    pub fn new(tree: Entity<TreeModel>, cx: &mut Context<Self>) -> TreeView {
        let observe = cx.observe(&tree, |this, tree, cx| {
            this.rows = Rc::new(tree.read(cx).state().rows());
            cx.notify();
        });
        TreeView {
            rows: Rc::new(tree.read(cx).state().rows()),
            tree,
            focus: cx.focus_handle(),
            cursor: None,
            selected: None,
            scroll: UniformListScrollHandle::new(),
            _observe: observe,
        }
    }

    pub fn tree(&self) -> &Entity<TreeModel> {
        &self.tree
    }

    /// Mark the file shown in the viewer, and scroll it into view.
    pub fn set_selected(&mut self, path: Option<String>, cx: &mut Context<Self>) {
        if self.selected != path {
            self.selected = path.clone();
            if let Some(p) = path {
                self.cursor = Some(p.clone());
                if let Some(ix) = self.index_of(&p) {
                    self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
                }
            }
            cx.notify();
        }
    }

    fn index_of(&self, path: &str) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| r.entry().is_some_and(|e| e.path == path))
    }

    fn cursor_index(&self) -> Option<usize> {
        self.cursor.as_deref().and_then(|p| self.index_of(p))
    }

    fn go(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(e) = self.rows.get(ix).and_then(TreeRow::entry) {
            self.cursor = Some(e.path.clone());
            self.scroll.scroll_to_item(ix, ScrollStrategy::Top);
            cx.notify();
        }
    }

    /// The next entry row from `from` in direction `step` (skipping notes).
    fn next_entry(&self, from: Option<usize>, down: bool) -> Option<usize> {
        let n = self.rows.len();
        let mut ix = match (from, down) {
            (None, _) => return (0..n).find(|&i| self.rows[i].entry().is_some()),
            (Some(i), true) => i + 1,
            (Some(0), false) => return None,
            (Some(i), false) => i - 1,
        };
        loop {
            if ix >= n {
                return None;
            }
            if self.rows[ix].entry().is_some() {
                return Some(ix);
            }
            if !down {
                ix = ix.checked_sub(1)?;
            } else {
                ix += 1;
            }
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.next_entry(self.cursor_index(), false) {
            self.go(ix, cx);
        }
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.next_entry(self.cursor_index(), true) {
            self.go(ix, cx);
        }
    }

    fn expand(&mut self, _: &Expand, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.cursor_index() else {
            return;
        };
        let Some(e) = self.rows[ix].entry().cloned() else {
            return;
        };
        if e.kind != EntryKind::Dir {
            return;
        }
        if !self.tree.read(cx).is_open(&e.path) {
            self.tree.update(cx, |t, cx| t.expand(&e.path, cx));
        } else if let Some(n) = self.next_entry(Some(ix), true) {
            self.go(n, cx);
        }
    }

    fn collapse(&mut self, _: &Collapse, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.cursor_index() else {
            return;
        };
        let Some(e) = self.rows[ix].entry().cloned() else {
            return;
        };
        if e.kind == EntryKind::Dir && self.tree.read(cx).is_open(&e.path) {
            self.tree.update(cx, |t, cx| t.collapse(&e.path, cx));
            return;
        }
        let (parent, _) = split_path(&e.path);
        if !parent.is_empty() {
            if let Some(p) = self.index_of(parent) {
                self.go(p, cx);
            }
        }
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.cursor_index() else {
            return;
        };
        if let Some(e) = self.rows[ix].entry().cloned() {
            self.activate(&e.path, e.kind, cx);
        }
    }

    fn activate(&mut self, path: &str, kind: EntryKind, cx: &mut Context<Self>) {
        self.cursor = Some(path.to_string());
        if kind == EntryKind::Dir {
            self.tree.update(cx, |t, cx| t.toggle(path, cx));
        } else {
            cx.emit(OpenFile(path.to_string()));
        }
        cx.notify();
    }

    fn render_row(&self, ix: usize, focused: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = theme::theme(cx).clone();
        let row = &self.rows[ix];
        let indent = px(6. + row.depth() as f32 * 12.);
        let note = |text: SharedString, color| {
            div()
                .id(ix)
                .w_full()
                .h(px(ROW_H))
                .pl(indent + px(22.))
                .pr(px(8.))
                .flex()
                .items_center()
                .text_size(px(11.5))
                .text_color(color)
                .overflow_hidden()
                .whitespace_nowrap()
                .child(text)
                .into_any_element()
        };
        let e = match row {
            TreeRow::Loading { .. } => return note("Reading…".into(), t.text_4),
            TreeRow::Truncated { .. } => {
                return note("Only the first 5 000 entries are shown".into(), t.text_4)
            }
            TreeRow::Error { error, .. } => {
                let first = error.lines().next().unwrap_or_default().to_string();
                return note(first.into(), t.red);
            }
            TreeRow::Entry { entry, open, .. } => (entry.clone(), *open),
        };
        let (e, open) = e;
        let is_dir = e.kind == EntryKind::Dir;
        let current = self.selected.as_deref() == Some(e.path.as_str());
        let at_cursor = focused && self.cursor.as_deref() == Some(e.path.as_str());
        let dim = if e.ignored { 0.5 } else { 1.0 };
        let path = e.path.clone();
        let kind = e.kind;
        let hover = t.surface_2;
        let title: SharedString = if kind == EntryKind::Symlink {
            format!("{} (symbolic link)", e.path).into()
        } else {
            e.path.clone().into()
        };
        let mark = match (e.status, is_dir && e.changes > 0) {
            (Some(s), _) => Some(status_mark(&t, ("mark", ix), s)),
            (None, true) => {
                let n: SharedString = if e.changes == 1 {
                    "1 change inside".into()
                } else {
                    format!("{} changes inside", e.changes).into()
                };
                Some(
                    div()
                        .id(("dot", ix))
                        .size(px(6.))
                        .mx(px(2.))
                        .flex_none()
                        .rounded_full()
                        .bg(t.text_3)
                        .tooltip(move |_, cx| tooltip_view(n.clone(), cx))
                        .into_any_element(),
                )
            }
            _ => None,
        };
        div()
            .id(ix)
            .w_full()
            .h(px(ROW_H))
            .pl(indent)
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(theme::RADIUS_SM)
            .cursor_pointer()
            .hover_probed(move |s| s.bg(hover))
            .when(current, |d| d.bg(t.surface_3))
            .when(at_cursor && !current, |d| {
                d.border_1().border_color(t.focus)
            })
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_click(cx.listener(move |this, _, window, cx| {
                window.focus(&this.focus);
                this.activate(&path, kind, cx);
            }))
            .tooltip(move |_, cx| tooltip_view(title.clone(), cx))
            .child(div().w(px(10.)).flex_none().when(is_dir, |d| {
                d.child(crate::kit::chevron("chev", open, 10., t.text_4))
            }))
            .child(
                if is_dir {
                    folder_icon(&e.path, open, 16.)
                } else {
                    file_icon(&e.path, 16.)
                }
                .opacity(dim),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .ellipsis()
                    .opacity(dim)
                    .when(is_dir, |d| d.text_color(t.text_2))
                    .when(!is_dir, |d| {
                        d.text_color(t.text).font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(e.status == Some(pitwall_proto::FileStatus::D), |d| {
                        d.line_through()
                    })
                    .flex()
                    .child(crate::kit::one_line(e.name.clone()))
                    .when(kind == EntryKind::Symlink, |d| {
                        d.child(div().flex_none().text_color(t.text_4).child(" ↗"))
                    }),
            )
            .children(mark)
            .when(current, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(2.))
                        .bg(t.text_2),
                )
            })
            .relative()
            .into_any_element()
    }
}

impl Render for TreeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.contains_focused(window, cx);
        let count = self.rows.len();
        let list = uniform_list(
            "ex-tree",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|ix| this.render_row(ix, focused, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .size_full()
        .px(px(4.))
        .pb(px(12.));
        let t = crate::theme::theme(cx).chrome();
        div()
            .id("ex-tree-box")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::expand))
            .on_action(cx.listener(Self::collapse))
            .on_action(cx.listener(Self::confirm))
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .font_family(MONO_FONT)
            .text_size(px(12.))
            .child(crate::kit::vscroll_list_fill("ex-tree-bar", &self.scroll, &t, list).flex_1())
    }
}
