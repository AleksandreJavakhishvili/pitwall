//! `.chip` and its tones (`base.css`): label face, 10.5 px, upper case,
//! 0.08em, in a hairline box or a soft fill.

use gpui::{div, prelude::*, px, Div, Hsla, SharedString};

use crate::theme::Theme;

use super::text::label_t;

/// Chip tones (`.chip`, `.chip-subtle`, `.chip-ok`/`.chip-new`, `.chip-warn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Hairline box, text-3.
    Plain,
    /// Surface-3 fill.
    Subtle,
    /// Green on green-soft.
    Ok,
    /// Amber on amber-soft.
    Warn,
}

fn colors(tone: Tone, t: &Theme) -> (Hsla, Option<Hsla>, Hsla) {
    let clear = gpui::transparent_black();
    match tone {
        Tone::Plain => (t.text_3, None, t.line_strong),
        Tone::Subtle => (t.text_3, Some(t.surface_3), clear),
        Tone::Ok => (t.green, Some(t.green_soft), clear),
        Tone::Warn => (t.amber, Some(t.amber_soft), clear),
    }
}

/// A chip of `tone`.
pub fn chip_tone(text: impl Into<SharedString>, tone: Tone, t: &Theme) -> Div {
    let (fg, bg, border) = colors(tone, t);
    div()
        .flex_none()
        .flex()
        .items_center()
        .pt(px(3.))
        .pb(px(2.))
        .pl(px(5.))
        .pr(px(4.2))
        .rounded(px(3.))
        .border_1()
        .border_color(border)
        .when_some(bg, |d, bg| d.bg(bg))
        // `line-height: 1`: WebKit lays 10.5 px out as a 10 px line.
        .child(label_t(text, fg, 10.5, 0.08).line_height(px(10.)))
}

/// `.chip`.
pub fn chip(text: impl Into<SharedString>, t: &Theme) -> Div {
    chip_tone(text, Tone::Plain, t)
}

/// `.chip-subtle`.
pub fn chip_subtle(text: impl Into<SharedString>, t: &Theme) -> Div {
    chip_tone(text, Tone::Subtle, t)
}

/// The smaller chips of onboarding lists (`.onb-chips .chip`: 9.5 px).
pub fn chip_sm(text: impl Into<SharedString>, tone: Tone, t: &Theme) -> Div {
    let (fg, bg, border) = colors(tone, t);
    div()
        .flex_none()
        .flex()
        .items_center()
        .pt(px(2.))
        .pb(px(1.))
        .px(px(4.))
        .rounded(px(3.))
        .border_1()
        .border_color(border)
        .when_some(bg, |d, bg| d.bg(bg))
        .child(label_t(text, fg, 9.5, 0.08).line_height(px(9.)))
}
