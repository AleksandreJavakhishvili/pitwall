//! Buttons and key labels (`base.css`; pills under Glass, `glass.css`):
//! `.primary-btn` (and `.primary-lg`), `.ghost-btn`, `.danger-btn`,
//! `.small-btn`, `.icon-btn` (and `-sm`), a text link, `.kbd`. The theme
//! passed in decides Flat or Glass: draw chrome with [`Theme::chrome`] and
//! dialogs with [`Theme::float`].

use crate::kit::HoverText;
use gpui::{div, prelude::*, px, Div, ElementId, FontWeight, Hsla, Pixels, SharedString, Stateful};

use crate::theme::{Theme, RADIUS, RADIUS_SM};

use super::fonts::MONO_FONT;
use super::icons::icon;
use super::tooltip::tooltip;

/// A shortcut as this desktop writes it: unchanged on macOS; elsewhere ⌘
/// becomes Ctrl+Shift (and ⌘⇧ Ctrl+Shift+Alt), or "⌃⇧" in the compact form
/// rows use (`keys()` in `src/lib/host.ts`).
pub fn keys(label: &str, compact: bool) -> String {
    keys_for(label, compact, cfg!(target_os = "macos"))
}

pub fn keys_for(label: &str, compact: bool, mac: bool) -> String {
    if mac {
        return label.to_string();
    }
    let (shifted, plain) = if compact {
        ("⌃⇧⌥", "⌃⇧")
    } else {
        ("Ctrl+Shift+Alt+", "Ctrl+Shift+")
    };
    label.replace("⌘⇧", shifted).replace('⌘', plain)
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

/// A key label inside a primary button (`.primary-btn .kbd`).
pub fn kbd_on_primary(label: &str, t: &Theme) -> Div {
    let on = on_primary(t);
    kbd(label, t)
        .bg(gpui::transparent_black())
        .border_color(on.opacity(0.35))
        .text_color(on.opacity(0.7))
}

/// The text colour on a primary button.
pub fn on_primary(t: &Theme) -> gpui::Hsla {
    if t.is_glass() {
        t.g.on_primary
    } else {
        t.bg
    }
}

/// The CSS button kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BtnKind {
    /// `.primary-btn`: 30 px, ink on the text colour.
    Primary,
    /// `.primary-btn.primary-lg`: 38 px, 14 px type.
    PrimaryLg,
    /// `.ghost-btn`: 30 px, hairline.
    Ghost,
    /// `.ghost-btn` at 26 px (setting rows, viewer bars).
    GhostSm,
    /// `.danger-btn`.
    Danger,
    /// `.small-btn`: 26 px on surface-3.
    Small,
    /// An underlined text button (`.retry-btn`, footer links).
    Link,
}

/// Rounded corners of a button: pills under Glass.
pub fn btn_radius(t: &Theme) -> Pixels {
    if t.is_glass() {
        px(999.)
    } else {
        RADIUS
    }
}

/// A button of `kind`; add its label and icon as children, then `.on_click`.
/// `disabled` dims it (0.45) and drops the hover.
pub fn button(id: impl Into<ElementId>, kind: BtnKind, disabled: bool, t: &Theme) -> Stateful<Div> {
    let radius = btn_radius(t);
    let base = div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(7.))
        .whitespace_nowrap()
        .font_weight(FontWeight::MEDIUM)
        .when(disabled, |d| d.opacity(0.45))
        .when(!disabled, |d| d.cursor_pointer());
    let (text, bg3, text4) = (t.text, t.surface_3, t.text_4);
    match kind {
        BtnKind::Primary | BtnKind::PrimaryLg => {
            let lg = kind == BtnKind::PrimaryLg;
            let hover = if t.is_glass() {
                t.text.blend(t.g.on_primary.opacity(0.10))
            } else {
                t.text.blend(t.bg.opacity(0.12))
            };
            base.h(px(if lg { 38. } else { 30. }))
                .px(px(if lg { 16. } else { 12. }))
                .text_size(px(if lg { 14. } else { 12.5 }))
                .rounded(radius)
                .bg(t.text)
                .text_color(on_primary(t))
                .when(t.is_glass(), |d| {
                    d.shadow(vec![gpui::BoxShadow {
                        color: gpui::black().opacity(0.5),
                        offset: gpui::point(px(0.), px(6.)),
                        blur_radius: px(18.),
                        spread_radius: px(-8.),
                    }])
                })
                .when(!disabled, |d| d.hover(move |s| s.bg(hover)))
        }
        BtnKind::Ghost | BtnKind::GhostSm => base
            .h(px(if kind == BtnKind::Ghost { 30. } else { 26. }))
            .px(px(if kind == BtnKind::Ghost { 12. } else { 10. }))
            .text_size(px(12.5))
            .rounded(radius)
            .border_1()
            .border_color(t.line_strong)
            .text_color(t.text_2)
            .when(!disabled, |d| d.hover_text(text, move |s| s.bg(bg3))),
        BtnKind::Danger => base
            .h(px(30.))
            .px(px(12.))
            .text_size(px(12.5))
            .rounded(radius)
            .bg(t.red)
            .text_color(gpui::white()),
        BtnKind::Small => base
            .h(px(26.))
            .px(px(9.))
            .text_size(px(12.))
            .rounded(radius)
            .bg(t.surface_3)
            .border_1()
            .border_color(t.line_strong)
            .text_color(t.text)
            .when(!disabled, |d| d.hover(move |s| s.border_color(text4))),
        BtnKind::Link => base
            .font_weight(FontWeight::NORMAL)
            .text_size(px(12.))
            .text_color(t.text_2)
            .underline()
            .when(!disabled, |d| d.hover_text(text, move |s| s)),
    }
}

/// A primary-shaped button in its own colours (the amber Allow of the
/// approval dialog): `bg` lightened by 15 % on hover.
pub fn accent_button(id: impl Into<ElementId>, bg: Hsla, fg: Hsla, t: &Theme) -> Stateful<Div> {
    let hover = bg.blend(gpui::white().opacity(0.15));
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(7.))
        .h(px(30.))
        .px(px(12.))
        .rounded(btn_radius(t))
        .whitespace_nowrap()
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(12.5))
        .bg(bg)
        .text_color(fg)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
}

/// A button with a text label (`button(…).child(text)`).
pub fn text_button(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    kind: BtnKind,
    disabled: bool,
    t: &Theme,
) -> Stateful<Div> {
    button(id, kind, disabled, t).child(text.into())
}

/// `.small-btn` with its label (26 px, surface-3, hairline border).
pub fn small_btn(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    t: &Theme,
) -> Stateful<Div> {
    text_button(id, text, BtnKind::Small, false, t)
}

/// `.icon-btn` (28 px, 22 px small): muted icon, surface-3 on hover.
pub fn icon_btn(
    id: impl Into<SharedString>,
    icon_name: &str,
    tip: impl Into<SharedString>,
    sm: bool,
    t: &Theme,
) -> Stateful<Div> {
    icon_btn_state(id, icon_name, tip, sm, if sm { 13. } else { 16. }, None, t)
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
    let id: SharedString = id.into();
    let group = SharedString::from(format!("ibtn-{id}"));
    let (hover_bg, hover_fg) = (t.surface_3, t.text);
    div()
        .id(ElementId::Name(id))
        .group(group.clone())
        .size(px(side))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS)
        .when_some(bg, |d, bg| d.bg(bg))
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .child(icon(icon_name, size, color).group_hover_text(group, hover_fg, move |s| s))
        .when(!tip.is_empty(), |d| d.tooltip(tooltip(tip)))
}

/// An icon button showing a text glyph instead of an icon (rare).
pub fn glyph_btn(id: impl Into<ElementId>, glyph: &'static str, t: &Theme) -> Stateful<Div> {
    let (fg, bg) = (t.text, t.surface_3);
    div()
        .id(id)
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS)
        .text_size(px(13.))
        .text_color(t.text_3)
        .cursor_pointer()
        .hover_text(fg, move |s| s.bg(bg))
        .child(glyph)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_labels_follow_the_desktop() {
        assert_eq!(keys_for("⌘K", false, true), "⌘K");
        assert_eq!(keys_for("⌘K", false, false), "Ctrl+Shift+K");
        assert_eq!(keys_for("⌘⇧N", false, false), "Ctrl+Shift+Alt+N");
        assert_eq!(keys_for("⌘⇧N", true, false), "⌃⇧⌥N");
    }

    #[test]
    fn glass_makes_pills() {
        let mut t = Theme::dark();
        assert_eq!(btn_radius(&t), RADIUS);
        t.glass = Some(crate::theme::GlassTier::Lite);
        assert_eq!(btn_radius(&t), px(999.));
        assert_eq!(on_primary(&t), t.g.on_primary);
    }
}
