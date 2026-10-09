//! The rules screens' own pieces (`rules.css`), drawn with the kit:
//! `.rules-mini` buttons, the `.rules-list` box and its rows. The sized
//! select, `.check` rows and rich text are the kit's (re-exported here).

use crate::kit::Ellipsis as _;
use std::rc::Rc;

use gpui::{
    div, prelude::*, px, AnyElement, App, Div, ElementId, SharedString, Stateful, Window,
};

pub use crate::kit::{check_row, rich, select, Span};
use crate::kit::{self, BtnKind, OnClose, OnPick, MONO_FONT};
use crate::theme::{Theme, RADIUS};

/// `.ghost-btn.rules-mini`: 24 px, 12 px type.
pub fn mini(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    disabled: bool,
    t: &Theme,
) -> Stateful<Div> {
    kit::button(id, BtnKind::GhostSm, disabled, t)
        .h(px(24.))
        .px(px(8.))
        .text_size(px(12.))
        .child(text.into())
}

/// `.rules-list`: hairline box on surface-2, at most 220 px tall.
pub fn list(id: impl Into<ElementId>, rows: Vec<AnyElement>, t: &Theme) -> kit::ScrollArea {
    let n = rows.len();
    let line = t.line;
    let id: ElementId = id.into();
    let inner = id.clone();
    kit::scroll_area(id, t, move |h| {
        div()
            .id(inner)
            .track_scroll(h)
            .flex()
            .flex_col()
            .max_h(px(218.))
            .overflow_y_scroll()
            .children(rows.into_iter().enumerate().map(move |(i, r)| {
                div()
                    .flex_none()
                    .when(i + 1 < n, |d| d.border_b_1().border_color(line))
                    .child(r)
            }))
            .into_any_element()
    })
    .max_h(px(220.))
    .rounded(RADIUS)
    .border_1()
    .border_color(t.line)
    .bg(t.surface_2)
}

/// `.rules-list li`: 32 px tall, 12.5 px.
pub fn row(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .min_h(px(32.))
        .px(px(10.))
        .py(px(4.))
        .text_size(px(12.5))
}

/// `.rules-name`: 12 px text, in mono unless `ui`.
pub fn name(text: impl Into<SharedString>, mono: bool, t: &Theme) -> Div {
    div()
        .flex_none()
        .text_size(px(12.))
        .text_color(t.text)
        .whitespace_nowrap()
        .when(mono, |d| d.font_family(MONO_FONT))
        .child(text.into())
}

/// `.rules-desc`: the rest of the row, text-3, one line.
pub fn desc(text: impl Into<SharedString>, t: &Theme) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .ellipsis()
        .text_color(t.text_3)
        .child(crate::kit::one_line(text.into()))
}

/// Picks for a select from labels (`on` marks the current one).
pub fn items(labels: impl IntoIterator<Item = (String, bool)>) -> Vec<(SharedString, bool)> {
    labels.into_iter().map(|(l, on)| (l.into(), on)).collect()
}

/// `OnPick` from a closure.
pub fn on_pick(f: impl Fn(usize, &mut Window, &mut App) + 'static) -> OnPick {
    Rc::new(f)
}

/// `OnClose` from a closure.
pub fn on_close(f: impl Fn(&mut Window, &mut App) + 'static) -> OnClose {
    Rc::new(f)
}
