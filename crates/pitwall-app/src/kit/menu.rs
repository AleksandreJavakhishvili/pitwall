//! The popup menu's look (`terminals.css` `.menu`, `.menu-item`): a raised
//! panel with the app's shadow and rows with a 16 px icon column. The
//! owner keeps the open state and keyboard cursor (see
//! `main_screen::menu`); these draw it.

use crate::kit::HoverText as _;
use gpui::{div, prelude::*, px, Div, ElementId, SharedString, Stateful};

use crate::theme::{Theme, RADIUS, RADIUS_SM};

use super::fonts::UI_FONT;
use super::icons::icon;

/// `.menu`: the panel (draw with [`Theme::float`]); open it with
/// [`super::motion::Fx::MENU`].
pub fn menu_panel(t: &Theme) -> Div {
    div()
        .min_w(px(190.))
        .p(px(4.))
        .flex()
        .flex_col()
        .font_family(UI_FONT)
        .rounded(RADIUS)
        .bg(t.raised)
        .shadow(t.float_shadow())
}

/// `.menu-item`: icon (or an empty 16 px column), label, optional hint.
pub fn menu_item(
    id: impl Into<ElementId>,
    icon_name: &str,
    label: impl Into<SharedString>,
    hint: Option<SharedString>,
    selected: bool,
    t: &Theme,
) -> Stateful<Div> {
    let hover = t.surface_3;
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .py(px(6.))
        .rounded(RADIUS_SM)
        .text_size(px(13.))
        .line_height(px(18.))
        .text_color(t.text)
        .when(selected, |d| d.bg(hover))
        .hover_probed(move |s| s.bg(hover))
        .cursor_pointer()
        .child(div().w(px(16.)).flex().when(!icon_name.is_empty(), |d| {
            d.child(icon(icon_name, 14., t.text_3))
        }))
        .child(div().flex_1().child(label.into()))
        .when_some(hint, |d, h| {
            d.child(div().text_size(px(11.5)).text_color(t.text_3).child(h))
        })
}
