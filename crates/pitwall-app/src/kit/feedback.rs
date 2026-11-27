//! Feedback and small display pieces in the kit's look: a progress bar,
//! a slider's track, a loading placeholder, a badge, an avatar and an
//! inline banner. Pitwall's own screens and extensions' tiles draw them
//! the same way (docs/spec/tile-sdk/ui.md, "kit parity").

use gpui::{div, prelude::*, px, relative, Animation, AnimationExt as _, AnyElement, Div, ElementId, Hsla, SharedString};

use super::icon;
use crate::theme::{Theme, RADIUS, RADIUS_SM};

/// The colours of a tone: the strong one (text, marks) and its soft fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Info,
    Good,
    Warn,
    Bad,
}

/// Tone `tone`'s (strong, soft) colours.
pub fn tone_colors(tone: Tone, t: &Theme) -> (Hsla, Hsla) {
    match tone {
        Tone::Plain => (t.text_2, t.surface_3),
        Tone::Info => (t.focus, t.focus.opacity(0.14)),
        Tone::Good => (t.green, t.green_soft),
        Tone::Warn => (t.amber, t.amber_soft),
        Tone::Bad => (t.red, t.red_soft),
    }
}

/// A tone's icon.
pub fn tone_icon(tone: Tone) -> &'static str {
    match tone {
        Tone::Good => "check",
        Tone::Warn | Tone::Bad => "alert",
        _ => "eye",
    }
}

/// A progress bar 4 px tall filling its width: `share` done (0–1), or
/// still going with no share known (`None`: a segment sweeping across,
/// still when `motion` is off).
pub fn progress(id: impl Into<ElementId>, share: Option<f32>, color: Hsla, motion: bool, t: &Theme) -> AnyElement {
    let track = div().relative().w_full().h(px(4.)).rounded(px(2.)).overflow_hidden().bg(t.surface_3);
    match share {
        Some(s) => track.child(div().absolute().top_0().left_0().h_full().rounded(px(2.)).bg(color).w(relative(s.clamp(0., 1.)))).into_any_element(),
        None => {
            let seg = move |p: f32| div().absolute().top_0().h_full().rounded(px(2.)).bg(color).w(relative(0.3)).left(relative(-0.3 + 1.3 * p));
            if motion {
                track.child(seg(0.).with_animation(id, Animation::new(std::time::Duration::from_millis(1400)).repeat(), move |_, p| seg(p))).into_any_element()
            } else {
                track.child(seg(0.35)).into_any_element()
            }
        }
    }
}

/// A slider's track and knob at `share` (0–1) of the way.
pub fn slider_track(share: f32, focused: bool, disabled: bool, t: &Theme) -> Div {
    let s = share.clamp(0., 1.);
    let knob = div()
        .absolute()
        .top(px(-5.))
        .left(relative(s))
        .ml(px(-7.))
        .size(px(14.))
        .rounded_full()
        .bg(t.raised)
        .border_1()
        .border_color(t.line_strong)
        .shadow_sm()
        .when(focused, |d| d.border_color(t.focus));
    div()
        .relative()
        .w_full()
        .h(px(4.))
        .rounded(px(2.))
        .bg(t.surface_3)
        .child(div().absolute().top_0().left_0().h_full().rounded(px(2.)).bg(t.text).w(relative(s)))
        .child(knob)
        .when(disabled, |d| d.opacity(0.45))
}

/// A grey placeholder bar `height` tall where content is loading.
pub fn skeleton_bar(height: f32, t: &Theme) -> Div {
    div().h(px(height)).rounded(RADIUS_SM).bg(t.text.opacity(0.08))
}

/// A badge: a small count or status label in a pill.
pub fn badge(text: impl Into<SharedString>, tone: Tone, t: &Theme) -> Div {
    let (fg, bg) = tone_colors(tone, t);
    let fg = if tone == Tone::Plain { t.text_2 } else { fg };
    div()
        .flex_none()
        .flex()
        .items_center()
        .h(px(18.))
        .px(px(6.))
        .rounded_full()
        .bg(bg)
        .text_size(px(11.))
        .font_weight(super::fonts::WEIGHT_550)
        .text_color(fg)
        .whitespace_nowrap()
        .child(text.into())
}

/// An avatar: `initials` in a circle `size` across.
pub fn avatar(initials: impl Into<SharedString>, size: f32, bg: Hsla, fg: Hsla) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .bg(bg)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px((size * 0.4).round()))
        .font_weight(super::fonts::WEIGHT_550)
        .text_color(fg)
        .child(initials.into())
}

/// An inline banner's box in `tone`: its icon, then what is put in it.
pub fn banner(tone: Tone, t: &Theme) -> Div {
    let (fg, bg) = tone_colors(tone, t);
    div()
        .flex()
        .items_start()
        .gap(px(8.))
        .px(px(10.))
        .py(px(8.))
        .rounded(RADIUS)
        .bg(bg)
        .border_1()
        .border_color(fg.opacity(0.35))
        .text_size(px(12.5))
        .text_color(t.text)
        .child(div().pt(px(1.)).child(icon(tone_icon(tone), 14., fg)))
}
