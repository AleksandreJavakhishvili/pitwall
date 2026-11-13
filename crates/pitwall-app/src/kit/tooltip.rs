//! A plain text tooltip (the web view's `title` bubbles, styled as the
//! app's own): raised surface, hairline, 11.5 px text-2.
//!
//! Under Liquid Glass it is a glass pop-up above the window
//! (`kit::passive`). GPUI's tooltips stay in charge of when and where:
//! what GPUI draws at the pointer after its delay is then only the
//! pop-up's anchor, and the pop-up is open while GPUI shows the tooltip.

use gpui::{div, prelude::*, px, AnyView, App, Div, IntoElement, Render, SharedString, Window};

use crate::theme::{self, RADIUS};

use super::fonts::UI_FONT;
use super::passive;

pub struct Tooltip(pub SharedString);

/// The bubble's text, without its surface.
fn bubble(text: SharedString, color: gpui::Hsla) -> Div {
    div()
        .font_family(UI_FONT)
        .max_w(px(360.))
        .px(px(8.))
        .py(px(5.))
        .text_size(px(11.5))
        .line_height(px(15.))
        .text_color(color)
        .child(text)
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        if super::pop::hosted(cx) {
            // Built from the text alone: its window reads nothing of this one.
            let text = self.0.clone();
            let build: passive::Build = std::rc::Rc::new(move |_, cx| {
                let t = passive::theme(cx);
                passive::panel(RADIUS.into(), bubble(text.clone(), t.text_2)).into_any_element()
            });
            let spec = passive::Passive { at: None, pin: (0., 0.), clicks: false, stamp: 0, build };
            return passive::anchor("tooltip", spec).into_any_element();
        }
        let t = theme::theme(cx).float();
        bubble(self.0.clone(), t.text_2)
            .rounded(RADIUS)
            .bg(t.raised)
            .border_1()
            .border_color(t.line_strong)
            .shadow_md()
            .into_any_element()
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, Modifiers, TestAppContext};

    struct Hovered;

    impl Render for Hovered {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            div().size_full().child(div().id("close").w(px(40.)).h(px(20.)).tooltip(tooltip("Close")))
        }
    }

    #[gpui::test]
    fn under_liquid_glass_a_tooltip_is_a_pop_up_while_gpui_shows_it(cx: &mut TestAppContext) {
        cx.update(passive::test_glass);
        let (_, cx) = cx.add_window_view(|_, _| Hovered);
        let open = |cx: &mut gpui::VisualTestContext| cx.update(|window, cx| passive::test_over(window, cx).len());
        cx.simulate_mouse_move(point(px(10.), px(10.)), None, Modifiers::none());
        cx.run_until_parked();
        assert_eq!(open(cx), 0, "not before GPUI's delay");
        cx.executor().advance_clock(std::time::Duration::from_millis(600));
        cx.run_until_parked();
        assert_eq!(open(cx), 1, "the tooltip, in a pop-up");
        // The pointer leaves what it is for.
        cx.simulate_mouse_move(point(px(200.), px(200.)), None, Modifiers::none());
        passive::test_draw(cx);
        assert_eq!(open(cx), 0);
    }
}
