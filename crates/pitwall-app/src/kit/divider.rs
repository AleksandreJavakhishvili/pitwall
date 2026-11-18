//! The grab bar between two things the user sizes by dragging (the pane
//! splits, the sidebar's edge): a few points across, the resize cursor,
//! and a short bar in its middle that lights while the pointer is on it or
//! it is being dragged. What it sizes listens for the press on it and
//! follows the mouse itself (the screen's mouse move and up), so a drag
//! keeps going when the pointer runs ahead of the bar.

use gpui::{div, prelude::*, px, relative, Div, ElementId, Stateful};

use super::HoverText as _;
use crate::theme::Theme;

/// How wide the grab area is, across the divider.
pub const DIVIDER: f32 = 6.;

/// A divider between things side by side (`row`: a vertical bar, dragged
/// left and right) or stacked. `active`: it is being dragged (lit
/// without the pointer on it).
pub fn divider(id: impl Into<ElementId>, row: bool, active: bool, t: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .when(row, |d| d.w(px(DIVIDER)).h_full().cursor_col_resize())
        .when(!row, |d| d.h(px(DIVIDER)).w_full().cursor_row_resize())
        .flex()
        .items_center()
        .justify_center()
        .group("divider")
        .child(
            div()
                .rounded(px(2.))
                .when(row, |d| d.w(px(2.)).h(relative(0.4)))
                .when(!row, |d| d.h(px(2.)).w(relative(0.4)))
                .when(active, |d| d.bg(t.text_4))
                .group_hover_probed("divider", |s| s.bg(t.text_4)),
        )
}
