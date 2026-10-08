//! Hover colours that keep the text's own style.
//!
//! gpui 0.2.2 merges a hover (or group-hover) style into the element's style
//! field by field, except the text style: a hover that sets `text_color`
//! replaces the element's whole text style, so its font family, size,
//! weight and line height fall back to the parent's while the pointer is
//! over it (labels lose Barlow Condensed and their tracking, buttons change
//! size). [`HoverText`] sets the colour on a copy of the element's own text
//! style instead. Call it after the element's text styles.

use gpui::{Hsla, InteractiveElement, SharedString, StyleRefinement, Styled, TextStyleRefinement};

fn keep(mut s: StyleRefinement, base: Option<TextStyleRefinement>, color: Hsla) -> StyleRefinement {
    let mut text = base.unwrap_or_default();
    text.color = Some(color);
    s.text = Some(text);
    s
}

pub trait HoverText: InteractiveElement + Styled + Sized {
    /// `.hover(|s| f(s).text_color(color))`, keeping the text style.
    fn hover_text(
        mut self,
        color: Hsla,
        f: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        let base = self.style().text.clone();
        self.hover(move |s| keep(f(s), base, color))
    }

    /// `.group_hover(group, |s| f(s).text_color(color))`, keeping the text
    /// style.
    fn group_hover_text(
        mut self,
        group: impl Into<SharedString>,
        color: Hsla,
        f: impl FnOnce(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        let base = self.style().text.clone();
        self.group_hover(group, move |s| keep(f(s), base, color))
    }
}

impl<E: InteractiveElement + Styled + Sized> HoverText for E {}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        canvas, div, point, prelude::*, px, Context, FontWeight, Pixels, Refineable, Render, Style,
        TestAppContext, Window,
    };
    use std::rc::Rc;

    #[test]
    fn a_hover_colour_keeps_font_size_and_weight() {
        let base = Some(TextStyleRefinement {
            font_family: Some("Barlow Condensed".into()),
            font_size: Some(px(12.).into()),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        });
        let normal = Style {
            text: base.clone().unwrap(),
            ..Default::default()
        };
        let mut hovered = normal.clone();
        hovered.refine(&keep(StyleRefinement::default(), base, gpui::white()));
        assert_eq!(hovered.text.font_family, normal.text.font_family);
        assert_eq!(hovered.text.font_size, normal.text.font_size);
        assert_eq!(hovered.text.font_weight, normal.text.font_weight);
        assert_eq!(hovered.text.color, Some(gpui::white()));
    }

    /// What the text inside a hovered element is painted with.
    #[derive(Default, Clone, PartialEq, Debug)]
    struct Seen {
        family: String,
        size: Option<Pixels>,
        weight: Option<FontWeight>,
        color: Option<gpui::Hsla>,
    }

    /// A project header: text that turns lighter on hover (`fixed` uses
    /// [`HoverText`], else a plain group hover).
    struct Head {
        seen: Rc<std::cell::RefCell<Seen>>,
        fixed: bool,
    }

    impl Render for Head {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let seen = self.seen.clone();
            let probe = canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let t = window.text_style();
                    *seen.borrow_mut() = Seen {
                        family: t.font_family.to_string(),
                        size: Some(t.font_size.to_pixels(px(16.))),
                        weight: Some(t.font_weight),
                        color: Some(t.color),
                    };
                },
            )
            .w(px(10.))
            .h(px(10.));
            let name = div()
                .font_family("Barlow Condensed")
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(gpui::black());
            let name = if self.fixed {
                name.group_hover_text("head", gpui::white(), |s| s)
            } else {
                name.group_hover("head", |s| s.text_color(gpui::white()))
            };
            div()
                .id("head")
                .group("head")
                .w(px(300.))
                .h(px(30.))
                .child(name.child(probe))
        }
    }

    fn paint_hovered(fixed: bool, cx: &mut TestAppContext) -> (Seen, Seen) {
        let seen = Rc::new(std::cell::RefCell::new(Seen::default()));
        let s = seen.clone();
        let (_, cx) = cx.add_window_view(move |_, _| Head {
            seen: s.clone(),
            fixed,
        });
        cx.simulate_mouse_move(point(px(400.), px(400.)), None, gpui::Modifiers::none());
        cx.run_until_parked();
        let before = seen.borrow().clone();
        cx.simulate_mouse_move(point(px(5.), px(5.)), None, gpui::Modifiers::none());
        cx.run_until_parked();
        let after = seen.borrow().clone();
        (before, after)
    }

    #[gpui::test]
    fn hovered_text_keeps_its_font_and_size(cx: &mut TestAppContext) {
        let (before, after) = paint_hovered(true, cx);
        assert_eq!(before.family, "Barlow Condensed");
        assert_ne!(after.color, before.color, "the hover colour applies");
        assert_eq!(
            (after.family, after.size, after.weight),
            (before.family, before.size, before.weight)
        );
    }

    #[gpui::test]
    fn a_plain_group_hover_loses_the_font(cx: &mut TestAppContext) {
        // The gpui 0.2.2 behaviour HoverText works around: if this starts
        // failing, gpui merges text styles now and HoverText can go.
        let (before, after) = paint_hovered(false, cx);
        assert_ne!(after.family, before.family);
    }
}
