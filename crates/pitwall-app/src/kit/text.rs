//! Text with CSS `letter-spacing` and the label faces (`.label`,
//! `.status-word`, chips). GPUI 0.2.2 text runs have no tracking, so
//! [`Tracked`] places each character at its measured advance plus the gap,
//! as browsers do: the box is exactly as wide as the browser's. Font, size
//! and colour come from the inherited text style, so the usual `Div`
//! styling (`text_color`, `font_family`, group hover) applies to it.

use gpui::{
    div, point, prelude::*, px, App, Bounds, Div, Element, ElementId, FontWeight, GlobalElementId,
    Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, SharedString, Style, TextRun, Window,
};

use super::fonts::LABEL_FONT;

/// One line of text with `em` letter spacing (see the module docs).
pub struct Tracked {
    text: SharedString,
    em: f32,
}

impl Tracked {
    pub fn new(text: impl Into<SharedString>, em: f32) -> Tracked {
        Tracked {
            text: text.into(),
            em,
        }
    }
}

/// The measured characters (advance of each, in px) and the gap.
pub struct TrackedLayout {
    chars: Vec<(char, f32)>,
    gap: f32,
}

impl IntoElement for Tracked {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Tracked {
    type RequestLayoutState = TrackedLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, TrackedLayout) {
        let style = window.text_style();
        let rem = window.rem_size();
        let size = style.font_size.to_pixels(rem);
        let gap = f32::from(size) * self.em;
        let ts = window.text_system();
        let id = ts.resolve_font(&style.font());
        let chars: Vec<(char, f32)> = self
            .text
            .chars()
            .map(|c| {
                let w = ts
                    .advance(id, size, c)
                    .map(|s| f32::from(s.width))
                    .unwrap_or(f32::from(size) * 0.5);
                (c, w)
            })
            .collect();
        let width: f32 = chars.iter().map(|(_, w)| w + gap).sum();
        let mut layout = Style::default();
        layout.size.width = px(width).into();
        layout.size.height = style.line_height_in_pixels(rem).into();
        layout.flex_shrink = 0.;
        (
            window.request_layout(layout, [], cx),
            TrackedLayout { chars, gap },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut TrackedLayout,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut TrackedLayout,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let style = window.text_style();
        let size = style.font_size.to_pixels(window.rem_size());
        let font = style.font();
        let mut x = bounds.left();
        for (c, w) in &layout.chars {
            if !c.is_whitespace() {
                let s: SharedString = c.to_string().into();
                let run = TextRun {
                    len: s.len(),
                    font: font.clone(),
                    color: style.color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(s, size, &[run], None);
                let _ = line.paint(point(x, bounds.top()), bounds.size.height, window, cx);
            }
            x += px(w + layout.gap);
        }
    }
}

/// `text` with `tracking` em between characters (CSS `letter-spacing`),
/// at `size` px; line height 1.25.
pub fn tracked(text: &str, size: f32, tracking: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .text_size(px(size))
        .line_height(px((size * 1.25).round()))
        .whitespace_nowrap()
        .child(Tracked::new(text.to_string(), tracking))
}

/// `.label`: Barlow Condensed 600, upper case, 0.12em letter spacing.
pub fn label(text: impl Into<SharedString>, color: Hsla, size: f32) -> Div {
    label_t(text, color, size, 0.12)
}

/// A label with its own letter spacing (`em`).
pub fn label_t(text: impl Into<SharedString>, color: Hsla, size: f32, tracking: f32) -> Div {
    let text: SharedString = text.into();
    tracked(&text.to_uppercase(), size, tracking)
        .font_family(LABEL_FONT)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
}

/// A label that may not fit (project names, paths): tracked when short,
/// else one text run that ends with an ellipsis.
pub fn label_fit(text: impl Into<SharedString>, color: Hsla, size: f32, max_chars: usize) -> Div {
    let text: SharedString = text.into();
    if text.chars().count() <= max_chars {
        return label(text, color, size);
    }
    div()
        .min_w_0()
        .truncate()
        .font_family(LABEL_FONT)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(size))
        .text_color(color)
        .child(SharedString::from(text.to_uppercase()))
}

/// `.muted`: body text in text-2.
pub fn muted(text: impl Into<SharedString>, t: &crate::theme::Theme) -> Div {
    div().text_color(t.text_2).child(text.into())
}

/// `.hint` / `.muted-sm`: 12 px text-3.
pub fn hint(text: impl Into<SharedString>, t: &crate::theme::Theme) -> Div {
    div()
        .text_size(px(12.))
        .text_color(t.text_3)
        .child(text.into())
}

/// `.hint-error` / a field's error: 12 px red.
pub fn error_text(text: impl Into<SharedString>, t: &crate::theme::Theme) -> Div {
    div()
        .text_size(px(12.))
        .text_color(t.red)
        .child(text.into())
}

/// `.mono` at 12 px text-3 (paths, commands).
pub fn mono_text(text: impl Into<SharedString>, t: &crate::theme::Theme) -> Div {
    div()
        .font_family(super::fonts::MONO_FONT)
        .text_size(px(12.))
        .text_color(t.text_3)
        .child(text.into())
}

/// The project's name for headings: the last part of its folder (the
/// engine gives `~/code/checkout-web`; the heading says `checkout-web`).
pub fn project_name(display: &str) -> &str {
    let t = display.trim_end_matches(['/', '\\']);
    match t.rfind(['/', '\\']) {
        Some(i) if i + 1 < t.len() => &t[i + 1..],
        _ if t.is_empty() => display,
        _ => t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_use_the_folder_name() {
        assert_eq!(project_name("~/code/checkout-web"), "checkout-web");
        assert_eq!(project_name("/srv/work/api/"), "api");
        assert_eq!(project_name(r"C:\work\beta"), "beta");
        assert_eq!(project_name("handbook"), "handbook");
        assert_eq!(project_name("~"), "~");
        assert_eq!(project_name("/"), "/");
    }

    #[gpui::test]
    fn tracked_text_is_as_wide_as_its_advances_plus_the_gaps(cx: &mut gpui::TestAppContext) {
        use gpui::{Render, VisualTestContext};
        struct V;
        impl Render for V {
            fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
                div().child(label("ab", gpui::black(), 10.))
            }
        }
        cx.update(super::super::fonts::register);
        let (_, vcx) = cx.add_window_view(|_, _| V);
        let vcx: &mut VisualTestContext = vcx;
        vcx.run_until_parked();
    }
}
