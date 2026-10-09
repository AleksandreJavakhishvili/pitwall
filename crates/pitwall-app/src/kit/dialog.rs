//! The parts inside a [`Modal`](super::Modal) (`overlays.css`): the body
//! column, the footer row of buttons and the form's error box.

use gpui::{div, prelude::*, px, Div, SharedString};

use crate::theme::{Theme, RADIUS};

/// `.modal-body`: a column, 14 px apart.
pub fn body() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .px(px(18.))
        .pt(px(8.))
        .pb(px(16.))
}

/// `.modal-foot`: buttons on the right over a hairline. Its bottom corners
/// follow the modal's (gpui clips to rectangles only).
pub fn foot(t: &Theme) -> Div {
    div()
        .rounded_b(px(12.))
        .flex()
        .justify_end()
        .gap(px(8.))
        .px(px(18.))
        .py(px(12.))
        .border_t_1()
        .border_color(t.line)
        .bg(t.surface)
}

/// `.form-error`: red on red-soft.
pub fn form_error(text: impl Into<SharedString>, t: &Theme) -> Div {
    div()
        .px(px(10.))
        .py(px(8.))
        .rounded(RADIUS)
        .bg(t.red_soft)
        .text_size(px(12.))
        .text_color(t.red)
        .child(text.into())
}

/// A `.modal-body` that scrolls when the window is too short for it
/// (`overflow-y: auto`), with the app's scrollbar.
pub fn scrolling(id: impl Into<gpui::ElementId>, body: Div, t: &Theme) -> super::ScrollArea {
    let id: gpui::ElementId = id.into();
    let inner = id.clone();
    super::scroll_area(id, t, move |h| {
        body.id(inner)
            .track_scroll(h)
            .min_h_0()
            .overflow_y_scroll()
            .into_any_element()
    })
    .min_h_0()
    .flex_shrink()
}
