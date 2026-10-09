//! Agent status marks (`agents.css`): the status glyph, the status word and
//! the +/− counts. The dots and the blocked triangle are drawn, not typed:
//! the bundled fonts don't have those characters, and fallback fonts draw
//! them at other sizes on each OS.

use gpui::{
    canvas, div, point, prelude::*, px, Div, ElementId, FontWeight, Hsla, IntoElement,
    PathBuilder, SharedString,
};

use pitwall_proto::Status;

use crate::agents::{status_glyph as status_glyph_text, status_word};
use crate::theme::Theme;

use super::fonts::MONO_FONT;
use super::motion;
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

    /// The ink of `●` (idle), `■` (stopped) and `⚑`'s type size, as the web
    /// view draws them in Inter: their `0.8em`, `0.75em` and `1.15em` are of
    /// the row's type (13 px rows, 12 px chips and lists), not the glyph's.
    fn ink(self) -> (f32, f32, f32) {
        match self {
            GlyphSize::Sm => (8.0, 6.8, 13.8),
            GlyphSize::Md => (8.2, 6.94, 14.95),
            GlyphSize::Lg => (10.3, 8.7, 18.4),
        }
    }
}

/// How a status glyph moves.
#[derive(Debug, Clone, PartialEq)]
pub enum GlyphMotion {
    /// Nothing moves (the Wall).
    Still,
    /// The working dot pulses.
    Live,
    /// The working dot pulses, and "needs you" bounces and "done" pops when
    /// the status starts (keyed by this name: one per agent and place).
    Play(SharedString),
}

/// The blocked triangle, `w` px wide.
fn triangle(color: Hsla, w: f32) -> impl IntoElement {
    let h = w * 1.025;
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

/// The status flag (`StatusGlyph`). Working is the green `.pulse-dot`.
/// `play`: see [`GlyphMotion::Play`] (`None`: [`GlyphMotion::Live`]).
pub fn status_glyph_el(s: Status, t: &Theme, size: GlyphSize, play: Option<ElementId>) -> Div {
    let m = match play {
        Some(id) => GlyphMotion::Play(id.to_string().into()),
        None => GlyphMotion::Live,
    };
    status_glyph(s, t, size, m)
}

/// The status glyph with its motion (see [`GlyphMotion`]).
pub fn status_glyph(s: Status, t: &Theme, size: GlyphSize, m: GlyphMotion) -> Div {
    // `title` = the status word, as `StatusGlyph` sets it.
    let word = crate::agents::status_word(s);
    div().flex_none().child(
        glyph_body(s, t, size, m)
            .id(SharedString::from(format!("glyph-{word}")))
            .tooltip(super::tooltip(word)),
    )
}

fn glyph_body(s: Status, t: &Theme, size: GlyphSize, m: GlyphMotion) -> Div {
    let (side, base) = size.metrics();
    let (dot_ink, square_ink, flag_px) = size.ink();
    let color = t.status_color(s);
    let d = div()
        .size(px(side))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_color(color);
    let key = |what: &str| match &m {
        GlyphMotion::Play(k) => Some(ElementId::Name(format!("{k}-{what}").into())),
        _ => None,
    };
    match s {
        Status::Working => {
            let dot = if size == GlyphSize::Sm { 6. } else { 7. };
            d.child(motion::pulse_dot(dot, t.green, t.is_glass(), m == GlyphMotion::Still))
        }
        Status::Idle => d.child(div().size(px(dot_ink)).rounded_full().bg(color)),
        Status::Blocked => {
            let w = base * 0.611;
            // `text-shadow: 0 0 8px var(--amber-glow)` around the ▲.
            let glow = t.amber_glow;
            let draw = move |dy: f32, scale: f32| {
                let w = w * scale;
                div()
                    .relative()
                    .top(px(dy))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .absolute()
                            .size(px(w * 0.7))
                            .rounded_full()
                            .shadow(vec![gpui::BoxShadow {
                                color: glow,
                                offset: point(px(0.), px(0.)),
                                blur_radius: px(8.),
                                spread_radius: px(0.),
                            }]),
                    )
                    .child(triangle(color, w))
                    .into_any_element()
            };
            match key("blocked") {
                Some(id) => d.child(motion::keyframes(id, motion::BOUNCE, 2, move |t| {
                    let (dy, s) = motion::bounce(t);
                    draw(dy, s)
                })),
                None => d.child(draw(0., 1.)),
            }
        }
        // `■` at 0.75em.
        Status::Exited | Status::Stopped => {
            d.child(div().size(px(square_ink)).rounded(px(0.5)).bg(color))
        }
        Status::Done => {
            let draw = move |opacity: f32, scale: f32| {
                div()
                    .opacity(opacity)
                    .text_size(px(flag_px * scale))
                    .line_height(px(side))
                    .child(status_glyph_text(Status::Done))
                    .into_any_element()
            };
            match key("done") {
                Some(id) => d.child(motion::keyframes(id, motion::FLAG, 1, move |t| {
                    let (o, s) = motion::flag_pop(t);
                    draw(o, s)
                })),
                None => d.child(draw(1., 1.)),
            }
        }
        Status::Unknown => d
            .text_size(px(base))
            .line_height(px(side))
            .font_family(MONO_FONT)
            .font_weight(FontWeight::SEMIBOLD)
            .child("?"),
    }
}

/// The status glyph at `.glyph` (16 px) or `.glyph-sm` (12 px); the
/// working dot pulses.
pub fn glyph(s: Status, t: &Theme, small: bool) -> Div {
    status_glyph(s, t, glyph_size(small), GlyphMotion::Live)
}

/// [`glyph`] that also bounces ("needs you") or pops ("done") when the
/// status starts; `key` names it (one per agent and place).
pub fn glyph_play(key: impl Into<SharedString>, s: Status, t: &Theme, small: bool) -> Div {
    status_glyph(s, t, glyph_size(small), GlyphMotion::Play(key.into()))
}

/// [`glyph`] where nothing moves (the Wall).
pub fn glyph_still(s: Status, t: &Theme, small: bool) -> Div {
    status_glyph(s, t, glyph_size(small), GlyphMotion::Still)
}

fn glyph_size(small: bool) -> GlyphSize {
    if small {
        GlyphSize::Sm
    } else {
        GlyphSize::Md
    }
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

/// `text-shadow: 0 0 <blur> var(--amber-glow)` behind a small amber mark
/// (`▲ 2` in a project head, the tab's and the strip's ▲): GPUI has no text
/// shadow, so a blurred box sits behind the middle of the text.
pub fn text_glow(child: impl IntoElement, glow: Hsla, blur: f32) -> Div {
    div()
        .relative()
        .flex()
        .items_center()
        .child(
            div()
                .absolute()
                .top(gpui::relative(0.3))
                .bottom(gpui::relative(0.3))
                .left(gpui::relative(0.15))
                .right(gpui::relative(0.15))
                .shadow(vec![gpui::BoxShadow {
                    color: glow,
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(blur),
                    spread_radius: px(0.),
                }]),
        )
        .child(child)
}

