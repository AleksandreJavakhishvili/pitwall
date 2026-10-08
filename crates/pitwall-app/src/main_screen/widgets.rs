//! Small elements the main screen draws everywhere, with the React app's
//! exact metrics (`src/styles/base.css`, `agents.css`): status glyphs and
//! words, +/− counts, chips, key labels, icon / small / primary / ghost /
//! danger buttons, checkboxes, tooltips. Candidates for the shared kit.

use gpui::{
    div, prelude::*, px, AnyView, App, Div, FontWeight, Hsla, IntoElement, SharedString, Stateful,
    Window,
};

use pitwall_proto::Status;

use crate::agents::{status_glyph, status_word};
use crate::theme::{self, Theme, RADIUS, RADIUS_SM};

use super::assets::{icon, LABEL_FONT, MONO_FONT};
use super::model::keys;

/// `.label`: Barlow Condensed 600, upper case, 0.12em letter spacing.
pub fn label(text: impl Into<SharedString>, color: Hsla, size: f32) -> Div {
    label_t(text, color, size, 0.12)
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

/// A label with its own letter spacing (`em`). GPUI 0.2.2 text runs have no
/// tracking, so each character is its own box with the spacing after it.
pub fn label_t(text: impl Into<SharedString>, color: Hsla, size: f32, tracking: f32) -> Div {
    let text: SharedString = text.into();
    tracked(&text.to_uppercase(), size, tracking)
        .font_family(LABEL_FONT)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
}

/// `text` with `tracking` em between characters (CSS `letter-spacing`).
pub fn tracked(text: &str, size: f32, tracking: f32) -> Div {
    let gap = size * tracking;
    div()
        .flex()
        .flex_none()
        .items_baseline()
        .text_size(px(size))
        .line_height(px((size * 1.25).round()))
        .whitespace_nowrap()
        .children(text.chars().map(move |c| {
            let s = if c == ' ' {
                "\u{a0}".to_string()
            } else {
                c.to_string()
            };
            div().flex_none().mr(px(gap)).child(s)
        }))
}

/// The status flag (`.glyph`, `.glyph-sm`): working is a green dot with a
/// soft ring, the others their glyph.
pub fn glyph(s: Status, t: &Theme, small: bool) -> Div {
    let box_size = if small { 12. } else { 16. };
    let base = if small { 9. } else { 11. };
    let font = match s {
        Status::Done => base * 1.15,
        Status::Idle => base * 0.8,
        Status::Exited | Status::Stopped => base * 0.75,
        _ => base,
    };
    let d = div()
        .size(px(box_size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(font))
        .line_height(px(box_size))
        .text_color(t.status_color(s));
    match s {
        Status::Working => {
            let dot = if small { 6. } else { 7. };
            d.child(
                div()
                    .size(px(dot + 4.))
                    .rounded_full()
                    .bg(t.green.opacity(0.18))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(px(dot)).rounded_full().bg(t.green)),
            )
        }
        Status::Unknown => d
            .font_family(MONO_FONT)
            .font_weight(FontWeight::SEMIBOLD)
            .child("?"),
        _ => d.child(status_glyph(s)),
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

/// `.chip`: label face 10.5 px in a hairline box.
pub fn chip(text: impl Into<SharedString>, t: &Theme) -> Div {
    let text: SharedString = text.into();
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
        .border_color(t.line_strong)
        .child(label_t(text, t.text_3, 10.5, 0.08).line_height(px(10.5)))
}

/// `.chip-subtle`.
pub fn chip_subtle(text: impl Into<SharedString>, t: &Theme) -> Div {
    chip(text, t)
        .border_color(gpui::transparent_black())
        .bg(t.surface_3)
}

/// `.kbd`: a key label ("⌘K"; "Ctrl+Shift+K" on other desktops).
pub fn kbd(label: &str, t: &Theme) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .px(px(5.))
        .py(px(3.))
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(t.line_strong)
        .bg(t.surface_2)
        .font_family(MONO_FONT)
        .text_size(px(10.5))
        .line_height(px(10.5))
        .text_color(t.text_3)
        .whitespace_nowrap()
        .child(keys(label, false))
}

/// `.icon-btn` (28 px, 22 px small): muted icon, surface-3 on hover.
pub fn icon_btn(
    id: impl Into<SharedString>,
    icon_name: &str,
    tip: impl Into<SharedString>,
    sm: bool,
    t: &Theme,
) -> Stateful<Div> {
    icon_btn_state(id, icon_name, tip, sm, if sm { 14. } else { 16. }, None, t)
}

/// An icon button with a pressed state (`aria-pressed` / `data-on`): on is
/// text colour on surface-3 (`.wall-btn[data-on]`), off is text-4.
#[allow(clippy::too_many_arguments)]
pub fn icon_btn_state(
    id: impl Into<SharedString>,
    icon_name: &str,
    tip: impl Into<SharedString>,
    sm: bool,
    size: f32,
    pressed: Option<bool>,
    t: &Theme,
) -> Stateful<Div> {
    let side = if sm { 22. } else { 28. };
    let tip: SharedString = tip.into();
    let (color, bg) = match pressed {
        Some(true) => (t.text, Some(t.surface_3)),
        Some(false) => (t.text_4, None),
        None => (t.text_3, None),
    };
    div()
        .id(gpui::ElementId::Name(id.into()))
        .group("ibtn")
        .size(px(side))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS)
        .when_some(bg, |d, bg| d.bg(bg))
        .cursor_pointer()
        .hover(|s| s.bg(t.surface_3))
        .child(icon(icon_name, size, color).group_hover("ibtn", |s| s.text_color(t.text)))
        .tooltip(tooltip(tip))
}

/// `.small-btn`: 26 px, surface-3, hairline border.
pub fn small_btn(
    id: impl Into<SharedString>,
    text: impl Into<SharedString>,
    t: &Theme,
) -> Stateful<Div> {
    div()
        .id(gpui::ElementId::Name(id.into()))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(7.))
        .h(px(26.))
        .px(px(9.))
        .rounded(RADIUS)
        .bg(t.surface_3)
        .border_1()
        .border_color(t.line_strong)
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.text)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.border_color(t.text_4))
        .child(text.into())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BtnKind {
    Primary,
    Ghost,
    Danger,
}

/// `.primary-btn`, `.ghost-btn`, `.danger-btn`: 30 px, 12.5 px medium.
pub fn button(
    id: impl Into<SharedString>,
    kind: BtnKind,
    disabled: bool,
    t: &Theme,
) -> Stateful<Div> {
    let (bg, fg, border) = match kind {
        BtnKind::Primary => (Some(t.text), t.bg, None),
        BtnKind::Ghost => (None, t.text_2, Some(t.line_strong)),
        BtnKind::Danger => (Some(t.red), gpui::white(), None),
    };
    let hover_bg = match kind {
        BtnKind::Primary => Some(t.text.blend(t.bg.opacity(0.12))),
        BtnKind::Ghost => Some(t.surface_3),
        BtnKind::Danger => None,
    };
    let d = div()
        .id(gpui::ElementId::Name(id.into()))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(7.))
        .h(px(30.))
        .px(px(12.))
        .rounded(RADIUS)
        .when_some(bg, |d, bg| d.bg(bg))
        .when_some(border, |d, b| d.border_1().border_color(b))
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(fg)
        .whitespace_nowrap();
    if disabled {
        d.opacity(0.45)
    } else {
        d.cursor_pointer().hover(move |s| {
            let s = match hover_bg {
                Some(b) => s.bg(b),
                None => s,
            };
            if kind == BtnKind::Ghost {
                s.text_color(t.text)
            } else {
                s
            }
        })
    }
}

/// A key label inside a primary button (`.primary-btn .kbd`).
pub fn kbd_on_primary(label: &str, t: &Theme) -> Div {
    kbd(label, t)
        .bg(gpui::transparent_black())
        .border_color(t.bg.opacity(0.35))
        .text_color(t.bg.opacity(0.7))
}

/// The box of a checkbox (`label.check input`).
pub fn check(checked: bool, t: &Theme) -> Div {
    div()
        .size(px(14.))
        .flex_none()
        .mt(px(2.))
        .rounded(px(3.))
        .border_1()
        .border_color(if checked { t.text } else { t.line_strong })
        .bg(if checked { t.text } else { t.bg })
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(10.))
        .font_weight(FontWeight::BOLD)
        .text_color(t.bg)
        .when(checked, |d| d.child("✓"))
}

/// A tooltip builder for `.tooltip()`.
pub fn tooltip(text: SharedString) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |_, cx| cx.new(|_| Tooltip(text.clone())).into()
}

pub struct Tooltip(pub SharedString);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let t = theme::theme(cx);
        div()
            .font_family(super::assets::UI_FONT)
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

/// `.chev`: the chevron icon, turned down when open.
pub fn chevron(open: bool, size: f32, color: Hsla) -> gpui::Svg {
    let i = icon("chevron", size, color);
    if open {
        i.with_transformation(gpui::Transformation::rotate(gpui::radians(
            std::f32::consts::FRAC_PI_2,
        )))
    } else {
        i
    }
}

/// `--backdrop`.
pub fn backdrop(t: &Theme) -> Hsla {
    match t.mode {
        theme::Mode::Dark => gpui::rgba(0x0405069e).into(),
        theme::Mode::Light => gpui::rgba(0x1e222847).into(),
    }
}

/// `--diff-add-bg` / `--diff-del-bg` (and the line-number tints).
pub fn diff_bg(t: &Theme, add: bool, number: bool) -> Hsla {
    let a = match (t.mode, number) {
        (theme::Mode::Dark, false) => 0.1,
        (theme::Mode::Dark, true) => 0.16,
        (theme::Mode::Light, false) => 0.09,
        (theme::Mode::Light, true) => 0.15,
    };
    if add {
        t.green.opacity(a)
    } else {
        t.red.opacity(a)
    }
}

/// `--amber-glow` as a soft outer glow (blocked rows, panes, the loud count).
pub fn amber_glow(t: &Theme, blur: f32, spread: f32) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color: t.amber.opacity(if t.mode == theme::Mode::Dark {
            0.38
        } else {
            0.35
        }),
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(blur),
        spread_radius: px(spread),
    }]
}
