//! A plain text tooltip (the web view's `title` bubbles, styled as the
//! app's own): raised surface, hairline, 11.5 px text-2.

use gpui::{div, prelude::*, px, AnyView, App, IntoElement, Render, SharedString, Window};

use crate::theme::{self, RADIUS};

use super::fonts::UI_FONT;

pub struct Tooltip(pub SharedString);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx).float();
        div()
            .font_family(UI_FONT)
            .max_w(px(360.))
            .px(px(8.))
            .py(px(5.))
            .rounded(RADIUS)
            .bg(t.raised)
            .border_1()
            .border_color(t.line_strong)
            .shadow_md()
            .text_size(px(11.5))
            .line_height(px(15.))
            .text_color(t.text_2)
            .child(self.0.clone())
    }
}

/// A tooltip builder for `.tooltip(…)`.
pub fn tooltip(
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text: SharedString = text.into();
    move |_, cx| cx.new(|_| Tooltip(text.clone())).into()
}

/// The tooltip view itself (for builders that take `&mut App` only).
pub fn tooltip_view(text: impl Into<SharedString>, cx: &mut App) -> AnyView {
    let text: SharedString = text.into();
    cx.new(|_| Tooltip(text)).into()
}
