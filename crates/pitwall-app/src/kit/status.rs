//! Agent status marks (`agents.css`): the status glyph, the status word and
//! the +/− counts. The dots and the blocked triangle are drawn, not typed:
//! the bundled fonts don't have those characters, and fallback fonts draw
//! them at other sizes on each OS.

use gpui::{
    canvas, div, point, prelude::*, px, Animation, AnimationExt, Div, ElementId, FontWeight, Hsla,
    IntoElement, PathBuilder,
};

use pitwall_proto::Status;

use crate::agents::{status_glyph, status_word};
use crate::theme::Theme;

use super::fonts::MONO_FONT;
use super::text::label_t;

/// `.glyph` sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphSize {
    /// `.glyph-sm`: 12 px box, 9 px type.
    Sm,
    /// `.glyph`: 16 px box, 11 px type.
    Md,
    /// `.glyph-lg`: 20 px box, 14 px type.
    Lg,
}

impl GlyphSize {
    fn metrics(self) -> (f32, f32) {
        match self {
            GlyphSize::Sm => (12., 9.),
            GlyphSize::Md => (16., 11.),
            GlyphSize::Lg => (20., 14.),
        }
    }
}

/// The blocked triangle, `w` px wide.
fn triangle(color: Hsla, w: f32) -> impl IntoElement {
    let h = w * 0.93;
    canvas(
        |_, _, _| {},
        move |b, _, window, _| {
            let (l, t) = (b.left(), b.top());
            let mut p = PathBuilder::fill();
            p.move_to(point(l + px(w / 2.), t));
            p.line_to(point(l + px(w), t + px(h)));
            p.line_to(point(l, t + px(h)));
            p.close();
            if let Ok(path) = p.build() {
                window.paint_path(path, color);
            }
        },
    )
    .w(px(w))
    .h(px(h))
    .flex_none()
}

/// The status flag (`StatusGlyph`). Working is a green dot; with `pulse`
/// (an id, when motion is on) its ring pulses as `.pulse-dot` does.
pub fn status_glyph_el(s: Status, t: &Theme, size: GlyphSize, pulse: Option<ElementId>) -> Div {
    let (side, base) = size.metrics();
    let color = t.status_color(s);
    let d = div()
        .size(px(side))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_color(color);
    match s {
        Status::Working => {
            let dot = if size == GlyphSize::Sm { 6. } else { 7. };
            let green = t.green;
            let ring = div()
                .absolute()
                .size(px(dot))
                .rounded_full()
                .bg(green.opacity(0.55));
            let ring = match pulse {
                Some(id) => ring
                    .with_animation(
                        id,
                        Animation::new(std::time::Duration::from_millis(1800))
                            .repeat()
                            .with_easing(gpui::ease_out_quint()),
                        move |el, p| {
                            let s = dot * (1. + 1.6 * p);
                            let off = (dot - s) / 2.;
                            el.size(px(s))
                                .left(px(off))
                                .top(px(off))
                                .bg(green.opacity(0.55 * (1. - p)))
                        },
                    )
                    .into_any_element(),
                None => ring.bg(green.opacity(0.18)).into_any_element(),
            };
            d.child(
                div()
                    .relative()
                    .size(px(dot))
                    .child(ring)
                    .child(div().absolute().size(px(dot)).rounded_full().bg(green)),
            )
        }
        // `●` at 0.8em.
        Status::Idle => {
            let dot = (base * 0.8 * 0.72).round();
            d.child(div().size(px(dot)).rounded_full().bg(color))
        }
        Status::Blocked => d.child(triangle(color, (base * 0.64).round())),
        // `■` at 0.75em.
        Status::Exited | Status::Stopped => {
            let side = (base * 0.75 * 0.62).round();
            d.child(div().size(px(side)).rounded(px(1.)).bg(color))
        }
        _ => {
            let font = match s {
                Status::Done => base * 1.15,
                Status::Exited | Status::Stopped => base * 0.75,
                _ => base,
            };
            let d = d.text_size(px(font)).line_height(px(side));
            if s == Status::Unknown {
                d.font_family(MONO_FONT)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("?")
            } else {
                d.child(status_glyph(s))
            }
        }
    }
}

/// The status glyph at `.glyph` (16 px) or `.glyph-sm` (12 px), still.
pub fn glyph(s: Status, t: &Theme, small: bool) -> Div {
    status_glyph_el(
        s,
        t,
        if small { GlyphSize::Sm } else { GlyphSize::Md },
        None,
    )
}

/// `.status-word[data-status]`.
pub fn word_color(s: Status, t: &Theme) -> Hsla {
    match s {
        Status::Working => t.green,
        Status::Blocked => t.amber,
        Status::Done => t.finish,
        Status::Idle => t.text_3,
        Status::Unknown | Status::Exited | Status::Stopped => t.text_4,
    }
}

/// `.status-word`: label face, 11 px.
pub fn status_label(s: Status, t: &Theme) -> Div {
    label_t(status_word(s), word_color(s, t), 11., 0.09)
}

/// `.diffstat`: "+12 −3" in mono 11 px, or a dim "±0".
pub fn diffstat(added: u32, removed: u32, t: &Theme) -> Div {
    let d = div()
        .flex()
        .gap(px(6.))
        .flex_none()
        .text_size(px(11.))
        .font_family(MONO_FONT);
    if added == 0 && removed == 0 {
        return d.text_color(t.text_4).child("±0");
    }
    d.child(div().text_color(t.green).child(format!("+{added}")))
        .child(div().text_color(t.red).child(format!("−{removed}")))
}
