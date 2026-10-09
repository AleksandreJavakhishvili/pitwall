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
    // What the hover itself adds to the text (an underline) goes on top.
    if let Some(extra) = s.text.take() {
        gpui::Refineable::refine(&mut text, &extra);
    }
    text.color = Some(color);
    s.text = Some(text);
    s
}

/// Tests: every hoverable element gets a debug selector (`hover-<n>` in
/// render order), so the hover pass can compare the layout before and
/// while it is hovered. Nothing in other builds.
#[cfg(test)]
pub mod probe {
    use std::cell::Cell;

    thread_local! {
        static NEXT: Cell<usize> = const { Cell::new(0) };
        static GEN: Cell<usize> = const { Cell::new(0) };
    }

    /// Before a frame: names restart at 0, in a new generation (gpui keeps
    /// the selectors of earlier frames).
    pub fn reset() {
        NEXT.with(|n| n.set(0));
        GEN.with(|g| g.set(g.get() + 1));
    }

    fn name(i: usize) -> String {
        format!("hover-{}-{i}", GEN.with(|g| g.get()))
    }

    pub fn next() -> String {
        NEXT.with(|n| {
            let i = n.get();
            n.set(i + 1);
            name(i)
        })
    }

    use gpui::{point, px, Bounds, Modifiers, Pixels, StyleRefinement, VisualTestContext};

    /// A hover style may change how an element is painted, never its box or
    /// its text: gpui replaces the whole text style with a hover's (the font
    /// falls back to the parent's), so text changes must keep it.
    pub fn paint_only(r: &StyleRefinement) {
        let d = StyleRefinement::default();
        assert_eq!(
            format!("{:?}", r.text),
            format!("{:?}", d.text),
            "a hover style changes the text style (use hover_text): {r:?}"
        );
        box_only(r);
    }

    /// The box stays: no padding, margin, size, border width or gap.
    pub fn box_only(r: &StyleRefinement) {
        let d = StyleRefinement::default();
        let same = |a: String, b: String, what: &str| {
            assert_eq!(a, b, "a hover style changes {what}: {r:?}");
        };
        same(format!("{:?}", r.padding), format!("{:?}", d.padding), "padding");
        same(format!("{:?}", r.margin), format!("{:?}", d.margin), "margin");
        same(format!("{:?}", r.size), format!("{:?}", d.size), "size");
        same(format!("{:?}", r.border_widths), format!("{:?}", d.border_widths), "border widths");
        same(format!("{:?}", r.gap), format!("{:?}", d.gap), "gap");
    }
    use std::collections::BTreeMap;

    /// Draw a fresh frame and read every probed element's bounds.
    pub fn layout(cx: &mut VisualTestContext) -> BTreeMap<String, Bounds<Pixels>> {
        // Some widgets fit themselves to last frame's measure: let it settle.
        for _ in 0..3 {
            reset();
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
        }
        // Only this frame's names.
        read(cx, NEXT.with(|n| n.get()))
    }

    fn read(cx: &mut VisualTestContext, count: usize) -> BTreeMap<String, Bounds<Pixels>> {
        let mut out = BTreeMap::new();
        for i in 0..count {
            let name = name(i);
            if let Some(b) = cx.debug_bounds(Box::leak(name.clone().into_boxed_str())) {
                out.insert(name, b);
            }
        }
        out
    }

    /// The hover pass: the pointer goes over every probed element that
    /// has a size and, while it is there, nothing moves or resizes (no
    /// hover style touches layout or fonts; gpui's text-style reset would
    /// change sizes). Returns how many elements were hovered.
    pub fn assert_hover_keeps_layout(cx: &mut VisualTestContext, what: &str) -> usize {
        // No looping animation: one frame per draw, so the names line up.
        crate::kit::motion::set_on(false);
        let away = point(px(1.), px(1.));
        cx.simulate_mouse_move(away, None, Modifiers::none());
        // Opening animations (real time) finish first.
        std::thread::sleep(std::time::Duration::from_millis(450));
        let base = layout(cx);
        assert!(!base.is_empty(), "{what}: nothing hoverable was drawn");
        let mut hovered = 0;
        for (name, b) in &base {
            if b.size.width <= px(1.) || b.size.height <= px(1.) {
                continue;
            }
            reset();
            cx.simulate_mouse_move(b.center(), None, Modifiers::none());
            // Names follow render order, which a hover-only element can
            // shift: compare the drawn boxes themselves (each one still
            // there, at the same place and size).
            let after = layout(cx);
            let now: Vec<Bounds<Pixels>> = after.values().copied().collect();
            let moved: Vec<(&String, &Bounds<Pixels>)> =
                base.iter().filter(|(_, nb)| !now.contains(nb)).collect();
            // A list that scrolls the hovered row into view (the palette,
            // as `scrollIntoView` does) shifts its rows together: allowed.
            let shift = moved.first().and_then(|(_, f)| {
                now.iter()
                    .find(|x| x.size == f.size && x.origin.x == f.origin.x)
                    .map(|x| x.origin.y - f.origin.y)
            });
            let scrolled = shift.is_some_and(|dy| {
                moved.iter().all(|(_, m)| {
                    now.iter().any(|x| {
                        x.size == m.size && x.origin.x == m.origin.x && x.origin.y - m.origin.y == dy
                    })
                })
            });
            if let (Some((n, nb)), false) = (moved.first(), scrolled) {
                let near: Vec<_> = after
                    .iter()
                    .filter(|(_, x)| (x.origin.y - nb.origin.y).abs() < px(12.))
                    .collect();
                panic!(
                    "{what}: hovering {name} {b:?} moved or resized {n} {nb:?}; \
                     now on that line: {near:?}"
                );
            }
            hovered += 1;
            reset();
            cx.simulate_mouse_move(away, None, Modifiers::none());
        }
        hovered
    }
}

/// An element that reacts to the pointer some other way (a row the
/// pointer selects): the hover pass checks it too.
pub fn probed<E: InteractiveElement>(el: E) -> E {
    #[cfg(test)]
    let el = el.debug_selector(probe::next);
    el
}

pub trait HoverText: InteractiveElement + Styled + Sized {
    /// `.hover(|s| f(s).text_color(color))`, keeping the text style.
    fn hover_text(
        mut self,
        color: Hsla,
        f: impl Fn(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        #[cfg(test)]
        probe::box_only(&f(StyleRefinement::default()));
        let base = self.style().text.clone();
        probed(self).hover(move |s| keep(f(s), base, color))
    }

    /// An underline on hover (a link), keeping the text style.
    fn hover_underline(mut self) -> Self {
        let base = self.style().text.clone();
        probed(self).hover(move |mut s| {
            let mut text = base.clone().unwrap_or_default();
            text.underline = Some(gpui::UnderlineStyle {
                thickness: gpui::px(1.),
                ..Default::default()
            });
            s.text = Some(text);
            s
        })
    }

    /// `.hover(f)` for paint-only styles (background, border colour,
    /// shadow, opacity); text colours go through [`HoverText::hover_text`].
    /// Tests check both, and the hover pass sees the element.
    fn hover_probed(self, f: impl Fn(StyleRefinement) -> StyleRefinement) -> Self {
        #[cfg(test)]
        probe::paint_only(&f(StyleRefinement::default()));
        probed(self).hover(f)
    }

    /// `.group_hover(group, f)` for paint-only styles.
    fn group_hover_probed(
        self,
        group: impl Into<SharedString>,
        f: impl Fn(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        #[cfg(test)]
        probe::paint_only(&f(StyleRefinement::default()));
        probed(self).group_hover(group, f)
    }

    /// `.group_hover(group, |s| f(s).text_color(color))`, keeping the text
    /// style.
    fn group_hover_text(
        mut self,
        group: impl Into<SharedString>,
        color: Hsla,
        f: impl Fn(StyleRefinement) -> StyleRefinement,
    ) -> Self {
        #[cfg(test)]
        probe::box_only(&f(StyleRefinement::default()));
        let base = self.style().text.clone();
        probed(self).group_hover(group, move |s| keep(f(s), base, color))
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
