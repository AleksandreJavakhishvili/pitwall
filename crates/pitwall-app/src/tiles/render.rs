//! A tile's element tree in Pitwall's own terms (docs/spec/tile-sdk/ui.md
//! §3.3): its colour tokens as the theme's colours, its lengths, text
//! sizes, fonts, radii and flex styles as GPUI styles, its buttons as the
//! kit's, and its key bindings as keystrokes. Pure functions over the WIT
//! types; [`super::view`] builds the elements with them.

use gpui::{px, relative, rems, AbsoluteLength, DefiniteLength, Div, FontWeight, Hsla, Keystroke, Length, Pixels, Styled};
use pitwall_tile_host::ui;

use crate::kit::BtnKind;
use crate::theme::{Theme, RADIUS, RADIUS_LG, RADIUS_SM};

/// A token's colour in theme `t`.
pub fn color(token: ui::Token, t: &Theme) -> Hsla {
    use ui::Token::*;
    match token {
        Bg => t.bg,
        Surface => t.surface,
        Surface2 => t.surface_2,
        Surface3 => t.surface_3,
        Raised => t.raised,
        Line => t.line,
        LineStrong => t.line_strong,
        Text => t.text,
        Text2 => t.text_2,
        Text3 => t.text_3,
        Text4 => t.text_4,
        Focus => t.focus,
        Good => t.green,
        GoodSoft => t.green_soft,
        Warn => t.amber,
        WarnSoft => t.amber_soft,
        Bad => t.red,
        BadSoft => t.red_soft,
        Done => t.finish,
        DoneSoft => t.finish_soft,
        DiffAddBg => t.diff_add_bg,
        DiffDelBg => t.diff_del_bg,
    }
}

/// Pitwall's text sizes, in px: the ones its own tiles use.
pub fn text_px(size: ui::TextSize) -> f32 {
    match size {
        ui::TextSize::Xs => 11.,
        ui::TextSize::Sm => 12.,
        ui::TextSize::Md => 13.,
        ui::TextSize::Lg => 16.,
        ui::TextSize::Xl => 20.,
    }
}

/// A text weight.
pub fn weight(w: ui::Weight) -> FontWeight {
    match w {
        ui::Weight::Normal => FontWeight::NORMAL,
        ui::Weight::Medium => FontWeight::MEDIUM,
        ui::Weight::Semibold => FontWeight::SEMIBOLD,
        ui::Weight::Bold => FontWeight::BOLD,
    }
}

/// A font's family.
pub fn family(f: ui::Font) -> &'static str {
    match f {
        ui::Font::Ui => crate::kit::UI_FONT,
        ui::Font::Mono => crate::kit::MONO_FONT,
    }
}

/// A length.
pub fn length(l: &ui::Length) -> Length {
    match *l {
        ui::Length::Px(v) => px(finite(v)).into(),
        ui::Length::Rem(v) => rems(finite(v)).into(),
        ui::Length::Fraction(v) => relative(finite(v).clamp(0., 1.)).into(),
        ui::Length::Auto => Length::Auto,
    }
}

/// A length that can't be `auto` (padding, gaps): `auto` is none.
pub fn definite(l: &ui::Length) -> DefiniteLength {
    match *l {
        ui::Length::Px(v) => px(finite(v).max(0.)).into(),
        ui::Length::Rem(v) => rems(finite(v).max(0.)).into(),
        ui::Length::Fraction(v) => relative(finite(v).clamp(0., 1.)),
        ui::Length::Auto => px(0.).into(),
    }
}

/// A border width: px or rem; fractions and `auto` are none.
pub fn absolute(l: &ui::Length) -> AbsoluteLength {
    match *l {
        ui::Length::Px(v) => px(finite(v).max(0.)).into(),
        ui::Length::Rem(v) => rems(finite(v).max(0.)).into(),
        _ => px(0.).into(),
    }
}

fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.
    }
}

/// A corner radius.
pub fn radius(r: ui::Radius) -> Pixels {
    match r {
        ui::Radius::Square => px(0.),
        ui::Radius::Sm => RADIUS_SM,
        ui::Radius::Md => RADIUS,
        ui::Radius::Lg => RADIUS_LG,
        ui::Radius::Full => px(999.),
    }
}

/// A button's kind and size as the kit's: a small ghost button is the
/// kit's small button; a small primary or danger one is drawn at the small
/// height (`shrink`).
pub fn button_kind(k: ui::ButtonKind, size: ui::ButtonSize) -> (BtnKind, bool) {
    let small = size == ui::ButtonSize::Small;
    match k {
        ui::ButtonKind::Primary => (BtnKind::Primary, small),
        ui::ButtonKind::Ghost if small => (BtnKind::Small, false),
        ui::ButtonKind::Ghost => (BtnKind::Ghost, false),
        ui::ButtonKind::Danger => (BtnKind::Danger, small),
        ui::ButtonKind::Link => (BtnKind::Link, false),
    }
}

/// A hover group's name in GPUI, apart from the app's own groups.
pub fn group(name: &str) -> gpui::SharedString {
    format!("tile-group-{name}").into()
}

/// What a node's styles say that its children inherit: text colour, size,
/// font and weight (and italic, underline).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inherit {
    pub color: Hsla,
    pub size: f32,
    pub font: ui::Font,
    pub weight: FontWeight,
}

impl Inherit {
    /// What the tile's root starts from: the body text.
    pub fn root(t: &Theme) -> Inherit {
        Inherit { color: t.text, size: text_px(ui::TextSize::Md), font: ui::Font::Ui, weight: FontWeight::NORMAL }
    }

    /// With `styles` applied.
    pub fn with(mut self, styles: &[ui::Style], t: &Theme) -> Inherit {
        for s in styles {
            match s {
                ui::Style::Color(c) => self.color = color(*c, t),
                ui::Style::TextSize(z) => self.size = text_px(*z),
                ui::Style::Font(f) => self.font = *f,
                ui::Style::Weight(w) => self.weight = weight(*w),
                _ => {}
            }
        }
        self
    }
}

/// The flags a node's styles set that aren't GPUI styles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Flags {
    /// Lay out in a row (the default is a column).
    pub row: bool,
    pub hover_bg: Option<ui::Token>,
    pub cursor: Option<ui::Cursor>,
    pub overflow_x: Option<ui::Overflow>,
    pub overflow_y: Option<ui::Overflow>,
    /// It is a hover group of this name.
    pub group: Option<String>,
    /// Hidden until the pointer is over the hover group of this name.
    pub group_hover: Option<String>,
}

/// Apply `styles` to `d` (a flex column unless the styles say a row), and
/// what they set that the caller handles (hover, cursor, overflow).
pub fn apply(mut d: Div, styles: &[ui::Style], t: &Theme) -> (Div, Flags) {
    let mut f = Flags::default();
    d = d.flex().flex_col();
    for s in styles {
        d = match s {
            ui::Style::FlexRow => {
                f.row = true;
                d.flex_row()
            }
            ui::Style::FlexCol => {
                f.row = false;
                d.flex_col()
            }
            ui::Style::Wrap => d.flex_wrap(),
            ui::Style::Grow(g) => d.flex_grow(finite(*g).max(0.)),
            ui::Style::Shrink(g) => d.flex_shrink(finite(*g).max(0.)),
            ui::Style::Gap(l) => d.gap(definite(l)),
            ui::Style::Padding(e) => d.pt(definite(&e.top)).pr(definite(&e.right)).pb(definite(&e.bottom)).pl(definite(&e.left)),
            ui::Style::Margin(e) => d.mt(length(&e.top)).mr(length(&e.right)).mb(length(&e.bottom)).ml(length(&e.left)),
            ui::Style::Width(l) => d.w(length(l)),
            ui::Style::Height(l) => d.h(length(l)),
            ui::Style::MinWidth(l) => d.min_w(length(l)),
            ui::Style::MaxWidth(l) => d.max_w(length(l)),
            ui::Style::MinHeight(l) => d.min_h(length(l)),
            ui::Style::MaxHeight(l) => d.max_h(length(l)),
            ui::Style::AlignItems(a) => match a {
                ui::Align::Start => d.items_start(),
                ui::Align::Center => d.items_center(),
                ui::Align::End => d.items_end(),
                ui::Align::Stretch => d.items_stretch(),
                ui::Align::Baseline => d.items_baseline(),
            },
            ui::Style::Justify(j) => match j {
                ui::Justify::Start => d.justify_start(),
                ui::Justify::Center => d.justify_center(),
                ui::Justify::End => d.justify_end(),
                ui::Justify::SpaceBetween => d.justify_between(),
                ui::Justify::SpaceAround => d.justify_around(),
            },
            ui::Style::SelfAlign(a) => match a {
                ui::Align::Start => d.self_start(),
                ui::Align::Center => d.self_center(),
                ui::Align::End => d.self_end(),
                ui::Align::Stretch => d.self_stretch(),
                ui::Align::Baseline => d.self_baseline(),
            },
            ui::Style::OverflowX(o) => {
                f.overflow_x = Some(*o);
                if *o == ui::Overflow::Hidden {
                    d.overflow_x_hidden()
                } else {
                    d
                }
            }
            ui::Style::OverflowY(o) => {
                f.overflow_y = Some(*o);
                if *o == ui::Overflow::Hidden {
                    d.overflow_y_hidden()
                } else {
                    d
                }
            }
            ui::Style::Border(e) => {
                let b = &mut d.style().border_widths;
                b.top = Some(absolute(&e.top));
                b.right = Some(absolute(&e.right));
                b.bottom = Some(absolute(&e.bottom));
                b.left = Some(absolute(&e.left));
                d
            }
            ui::Style::BorderColor(c) => d.border_color(color(*c, t)),
            ui::Style::Radius(r) => d.rounded(radius(*r)),
            ui::Style::Background(c) => d.bg(color(*c, t)),
            ui::Style::Color(c) => d.text_color(color(*c, t)),
            ui::Style::Opacity(o) => d.opacity(finite(*o).clamp(0., 1.)),
            ui::Style::TextSize(z) => d.text_size(px(text_px(*z))),
            ui::Style::Font(fo) => d.font_family(family(*fo)),
            ui::Style::Weight(w) => d.font_weight(weight(*w)),
            ui::Style::Italic => d.italic(),
            ui::Style::Underline => d.underline(),
            ui::Style::Truncate => d.truncate(),
            ui::Style::LineClamp(n) => d.line_clamp((*n).max(1) as usize),
            // Copying text from a tile isn't drawn yet: no effect.
            ui::Style::Selectable => d,
            ui::Style::HoverBackground(c) => {
                f.hover_bg = Some(*c);
                d
            }
            ui::Style::Cursor(c) => {
                f.cursor = Some(*c);
                d
            }
            ui::Style::Group(g) => {
                f.group = Some(g.clone());
                d
            }
            ui::Style::VisibleOnGroupHover(g) => {
                f.group_hover = Some(g.clone());
                d
            }
            // Dragging and dropping are the view's (`view::interact`).
            ui::Style::Drag(_) | ui::Style::DropTarget(_) => d,
        };
    }
    (d, f)
}

/// The keys of Pitwall's own that a tile's keymap never gets (ui.md §3.5;
/// the host drops them from frames already): the palette, ⌘⏎, ⌘W, ⌘1–9,
/// Esc, Tab and the rest of the main screen's and the menu's keys.
pub fn reserved(keys: &str) -> bool {
    pitwall_tile_host::keys::reserved(keys)
}

/// Whether `pressed` is binding `keys` as a tile writes it (`j`, `enter`,
/// `cmd-r`, `shift-tab`, `/`): `cmd` is ⌘ on macOS and Ctrl elsewhere
/// (`mac`).
pub fn matches(keys: &str, pressed: &Keystroke, mac: bool) -> bool {
    let k = keys.trim().to_ascii_lowercase();
    if k.is_empty() {
        return false;
    }
    // The key is after the last `-` that has something before it ("-" and
    // "cmd--" name the minus key).
    let (mods, key) = match k.rfind('-') {
        Some(i) if i > 0 && i + 1 < k.len() => (&k[..i], &k[i + 1..]),
        Some(i) if i > 0 && i + 1 == k.len() && k[..i].ends_with('-') => (&k[..i - 1], "-"),
        _ => ("", k.as_str()),
    };
    let (mut cmd, mut ctrl, mut alt, mut shift) = (false, false, false, false);
    for m in mods.split('-').filter(|m| !m.is_empty()) {
        match m {
            "cmd" if mac => cmd = true,
            "cmd" | "ctrl" | "control" => ctrl = true,
            "alt" | "opt" | "option" => alt = true,
            "shift" => shift = true,
            "super" | "win" => cmd = true,
            _ => return false,
        }
    }
    let m = &pressed.modifiers;
    let key_same = pressed.key.eq_ignore_ascii_case(key) || pressed.key_char.as_deref() == Some(key);
    // A typed symbol ("/", "?") may come with Shift held; it matches
    // when the binding names no Shift.
    let shift_ok = m.shift == shift || (!shift && pressed.key_char.as_deref() == Some(key) && key.len() == 1 && !key.as_bytes()[0].is_ascii_alphanumeric());
    key_same && m.platform == cmd && m.control == ctrl && m.alt == alt && shift_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(key: &str, f: impl FnOnce(&mut gpui::Modifiers)) -> Keystroke {
        let mut k = Keystroke { key: key.into(), key_char: Some(key.into()), ..Default::default() };
        f(&mut k.modifiers);
        k
    }

    /// A tile's keys as written match the keystrokes; ⌘ is Ctrl off macOS;
    /// Pitwall's own keys are reserved.
    #[test]
    fn a_keymap_matches_keystrokes() {
        assert!(matches("j", &ks("j", |_| {}), true));
        assert!(!matches("j", &ks("k", |_| {}), true));
        assert!(!matches("j", &ks("j", |m| m.platform = true), true));
        assert!(matches("cmd-r", &ks("r", |m| m.platform = true), true));
        assert!(matches("cmd-r", &ks("r", |m| m.control = true), false));
        assert!(!matches("cmd-r", &ks("r", |m| m.control = true), true));
        assert!(matches("shift-tab", &ks("tab", |m| m.shift = true), true));
        assert!(matches("/", &ks("/", |_| {}), true));
        assert!(matches("cmd--", &ks("-", |m| m.platform = true), true));
        assert!(!matches("hyper-x", &ks("x", |_| {}), true));
        for r in ["cmd-enter", "cmd-w", "cmd-3", "escape", "Tab", "cmd-r"] {
            assert!(reserved(r), "{r}");
        }
        assert!(!reserved("cmd-s") && !reserved("j"));
    }

    /// Every key the main screen and the menu bind for the whole window is
    /// in the reserved list the docs publish, on this system's keys.
    #[test]
    fn the_apps_own_keys_are_reserved() {
        let mut all = crate::main_screen::bindings();
        all.extend(crate::menu::bindings());
        let mac = cfg!(target_os = "macos");
        for b in &all {
            let Some(ks) = b.keystrokes().first().map(|k| k.inner().clone()) else { continue };
            // Contexts other than the window's own (lists, menus) aren't a
            // tile's business: only bindings with none or the main screen.
            if b.predicate().is_some_and(|p| !p.to_string().contains("MainScreen")) {
                continue;
            }
            let m = &ks.modifiers;
            let mut keys = String::new();
            if (mac && m.platform) || (!mac && m.control) {
                keys.push_str("cmd-");
            } else if m.control {
                keys.push_str("ctrl-");
            }
            if mac && m.control && m.platform {
                keys.push_str("ctrl-");
            }
            if m.alt {
                keys.push_str("alt-");
            }
            if m.shift {
                keys.push_str("shift-");
            }
            keys.push_str(&ks.key);
            // Shifted symbols off macOS (ctrl-!) stand for cmd-shift-1.
            if !mac && ks.key.len() == 1 && !ks.key.as_bytes()[0].is_ascii_alphanumeric() && !"=+-,".contains(ks.key.as_str()) {
                continue;
            }
            assert!(reserved(&keys), "{keys} ({ks:?}) is Pitwall's but not reserved");
        }
    }

    /// Every token has a colour of the theme; lengths keep within reason.
    #[test]
    fn tokens_and_lengths() {
        let t = Theme::dark();
        assert_eq!(color(ui::Token::Text, &t), t.text);
        assert_eq!(color(ui::Token::Bad, &t), t.red);
        assert_eq!(length(&ui::Length::Px(f32::NAN)), Length::Definite(px(0.).into()));
        assert_eq!(definite(&ui::Length::Auto), px(0.).into());
        assert_eq!(text_px(ui::TextSize::Md), 13.);
        let i = Inherit::root(&t).with(&[ui::Style::Color(ui::Token::Text3), ui::Style::TextSize(ui::TextSize::Lg)], &t);
        assert_eq!((i.color, i.size), (t.text_3, 16.));
    }
}
