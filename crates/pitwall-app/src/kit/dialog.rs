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

/// `.modal-foot`: buttons on the right over a hairline.
pub fn foot(t: &Theme) -> Div {
    div()
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
