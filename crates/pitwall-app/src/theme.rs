//! Colour and size tokens, ported from `src/styles/tokens.css` (built
//! in-house: Zed's `theme` crate is GPL-3.0). Dark is the default; light
//! applies when the system is light (Settings → Appearance comes in phase 6).
//! Keep the values in step with tokens.css until the Tauri app is retired.

use gpui::{px, rgb, rgba, App, Global, Hsla, Pixels, WindowAppearance};

use pitwall_proto::Status;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

impl Mode {
    pub fn for_appearance(a: WindowAppearance) -> Mode {
        match a {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Mode::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Mode::Dark,
        }
    }
}

/// The tokens one theme needs (names as in tokens.css).
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub mode: Mode,
    pub bg: Hsla,
    pub surface: Hsla,
    pub surface_2: Hsla,
    pub surface_3: Hsla,
    pub raised: Hsla,
    pub term_bg: Hsla,
    pub line: Hsla,
    pub line_strong: Hsla,
    pub text: Hsla,
    pub text_2: Hsla,
    pub text_3: Hsla,
    pub text_4: Hsla,
    pub focus: Hsla,
    pub green: Hsla,
    pub green_soft: Hsla,
    pub finish: Hsla,
    pub finish_soft: Hsla,
    pub amber: Hsla,
    pub amber_soft: Hsla,
    pub red: Hsla,
    pub red_soft: Hsla,
    pub flag: Hsla,
}

/// Sizes (`--radius*`, `--topbar-h`, `--sidebar-w`, `--right-w`).
pub const RADIUS_SM: Pixels = px(4.);
pub const RADIUS: Pixels = px(6.);
pub const RADIUS_LG: Pixels = px(10.);
pub const TOPBAR_H: Pixels = px(44.);
pub const SIDEBAR_W: Pixels = px(268.);
pub const RIGHT_W: Pixels = px(320.);

/// `#rrggbb` + alpha (CSS `rgba(r, g, b, a)`).
fn a(hex: u32, alpha: f32) -> Hsla {
    rgba((hex << 8) | (alpha * 255.0).round() as u32).into()
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl Theme {
    pub fn dark() -> Theme {
        Theme {
            mode: Mode::Dark,
            bg: c(0x0a0b0d),
            surface: c(0x0f1114),
            surface_2: c(0x15181c),
            surface_3: c(0x1b1f24),
            raised: c(0x171a1f),
            term_bg: c(0x0d0f12),
            line: c(0x1e2227),
            line_strong: c(0x2a2f36),
            text: c(0xe6e8eb),
            text_2: c(0xa2a8b1),
            text_3: c(0x6c737d),
            text_4: c(0x474d55),
            focus: a(0xe6e8eb, 0.45),
            green: c(0x3fd07f),
            green_soft: a(0x3fd07f, 0.13),
            finish: c(0x4fe3b8),
            finish_soft: a(0x4fe3b8, 0.12),
            amber: c(0xffb224),
            amber_soft: a(0xffb224, 0.12),
            red: c(0xff5f5f),
            red_soft: a(0xff5f5f, 0.13),
            flag: c(0xf4f5f7),
        }
    }

    pub fn light() -> Theme {
        Theme {
            mode: Mode::Light,
            bg: c(0xeef0f2),
            surface: c(0xf7f8f9),
            surface_2: c(0xeef0f2),
            surface_3: c(0xe4e7ea),
            raised: c(0xffffff),
            term_bg: c(0xfbfbfc),
            line: c(0xdfe2e6),
            line_strong: c(0xcdd1d6),
            text: c(0x15181c),
            text_2: c(0x4f5661),
            text_3: c(0x7d848e),
            text_4: c(0xa9aeb5),
            focus: a(0x15181c, 0.4),
            green: c(0x14a052),
            green_soft: a(0x14a052, 0.11),
            finish: c(0x0e9f7e),
            finish_soft: a(0x0e9f7e, 0.1),
            amber: c(0xd98600),
            amber_soft: a(0xed9600, 0.13),
            red: c(0xd63a3a),
            red_soft: a(0xd63a3a, 0.1),
            flag: c(0x15181c),
        }
    }

    pub fn for_mode(mode: Mode) -> Theme {
        match mode {
            Mode::Dark => Theme::dark(),
            Mode::Light => Theme::light(),
        }
    }

    /// A status glyph's colour (`.glyph[data-status=…]` in agents.css).
    pub fn status_color(&self, s: Status) -> Hsla {
        match s {
            Status::Working => self.green,
            Status::Blocked => self.amber,
            Status::Done => self.flag,
            Status::Idle => self.text_3,
            Status::Unknown | Status::Exited | Status::Stopped => self.text_4,
        }
    }
}

impl Global for Theme {}

/// The current theme (set at start and when the system appearance changes).
pub fn theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_match_tokens_css() {
        // A few spot checks against src/styles/tokens.css.
        let css = include_str!("../../../src/styles/tokens.css");
        for hex in [
            "#0a0b0d", "#3fd07f", "#ffb224", "#eef0f2", "#14a052", "#d98600",
        ] {
            assert!(
                css.contains(hex),
                "{hex} is gone from tokens.css: update theme.rs"
            );
        }
        assert_eq!(Theme::dark().bg, c(0x0a0b0d));
        assert_eq!(Theme::light().text, c(0x15181c));
    }

    #[test]
    fn alpha_tokens_keep_their_alpha() {
        let t = Theme::dark();
        assert!((t.green_soft.a - 0.13).abs() < 0.01);
        assert!((t.focus.a - 0.45).abs() < 0.01);
    }

    #[test]
    fn the_system_appearance_picks_the_mode() {
        assert_eq!(
            Mode::for_appearance(WindowAppearance::VibrantLight),
            Mode::Light
        );
        assert_eq!(Mode::for_appearance(WindowAppearance::Dark), Mode::Dark);
        assert_eq!(Theme::for_mode(Mode::Light).mode, Mode::Light);
    }

    #[test]
    fn statuses_have_colours() {
        let t = Theme::dark();
        assert_eq!(t.status_color(Status::Blocked), t.amber);
        assert_eq!(t.status_color(Status::Working), t.green);
    }
}
