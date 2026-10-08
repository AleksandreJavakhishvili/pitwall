//! Terminal colours: Pitwall's dark and light terminal themes (the same values
//! the web UI gives xterm.js), xterm's 256-colour palette, and the rules that
//! turn a cell's colours and flags into what is drawn (bold-as-bright, dim,
//! inverse, hidden).

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

/// A colour with alpha, as the renderer paints it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const fn opaque(c: Rgb) -> Self {
        Rgba8 { r: c.r, g: c.g, b: c.b, a: 0xff }
    }
    pub const fn with_alpha(self, a: u8) -> Self {
        Rgba8 { a, ..self }
    }
    pub const fn rgb(self) -> Rgb {
        Rgb { r: self.r, g: self.g, b: self.b }
    }
}

/// `0xRRGGBB` → [`Rgb`].
pub const fn hex(v: u32) -> Rgb {
    Rgb { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 }
}

/// A terminal colour theme: default colours plus the 16 ANSI colours. Colours
/// 16–255 are xterm's cube and grey ramp (see [`TermTheme::palette`]).
#[derive(Clone, Debug, PartialEq)]
pub struct TermTheme {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    /// Text drawn on top of a block cursor.
    pub cursor_accent: Rgb,
    pub selection_background: Rgb,
    /// Search matches other than the current one.
    pub search_match: Rgb,
    /// The current search match.
    pub search_current: Rgb,
    /// black, red, green, yellow, blue, magenta, cyan, white, then the bright ones.
    pub ansi: [Rgb; 16],
    /// Draw bold text in the bright variant of colours 0–7 (xterm's default).
    pub bold_is_bright: bool,
    /// Opacity of dim (SGR 2) text, 0–255; xterm.js uses 50 %.
    pub dim_alpha: u8,
}

impl TermTheme {
    /// Pitwall's dark terminal theme (`src/terminal/registry.ts` DARK).
    pub fn pitwall_dark() -> Self {
        TermTheme {
            background: hex(0x0d0f12),
            foreground: hex(0xd9dde3),
            cursor: hex(0xe8ebef),
            cursor_accent: hex(0x0d0f12),
            selection_background: hex(0x3a4250),
            search_match: hex(0x5a4a1a),
            search_current: hex(0x8a6a10),
            ansi: [
                hex(0x1b1f25),
                hex(0xff6b6b),
                hex(0x5fd38d),
                hex(0xf2c35b),
                hex(0x6aa6ff),
                hex(0xc792ea),
                hex(0x5fc9d3),
                hex(0xd9dde3),
                hex(0x5b636f),
                hex(0xff8787),
                hex(0x7fe0a5),
                hex(0xffd479),
                hex(0x8cbcff),
                hex(0xd7aefb),
                hex(0x7fdde5),
                hex(0xf4f6f8),
            ],
            bold_is_bright: true,
            dim_alpha: 0x80,
        }
    }

    /// Pitwall's light terminal theme (`src/terminal/registry.ts` LIGHT).
    pub fn pitwall_light() -> Self {
        TermTheme {
            background: hex(0xfbfbfc),
            foreground: hex(0x1d2128),
            cursor: hex(0x1d2128),
            cursor_accent: hex(0xfbfbfc),
            selection_background: hex(0xc9d6ea),
            search_match: hex(0xf5e3a3),
            search_current: hex(0xf0c040),
            ansi: [
                hex(0x1d2128),
                hex(0xc92a2a),
                hex(0x2b8a3e),
                hex(0xa96800),
                hex(0x1c64d6),
                hex(0x8b3fc4),
                hex(0x0b7f8a),
                hex(0xd5d9de),
                hex(0x6b7280),
                hex(0xe03131),
                hex(0x37a24f),
                hex(0xc27c00),
                hex(0x3b7ee8),
                hex(0xa35ad8),
                hex(0x1898a4),
                hex(0xffffff),
            ],
            bold_is_bright: true,
            dim_alpha: 0x80,
        }
    }

    /// xterm's 256 colours: the theme's 16, the 6×6×6 cube, then 24 greys.
    pub fn palette(&self) -> [Rgb; 256] {
        let mut out = [Rgb::default(); 256];
        out[..16].copy_from_slice(&self.ansi);
        const STEPS: [u8; 6] = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
        for i in 0..216 {
            out[16 + i] = Rgb { r: STEPS[i / 36], g: STEPS[(i / 6) % 6], b: STEPS[i % 6] };
        }
        for i in 0..24 {
            let v = 8 + 10 * i as u8;
            out[232 + i] = Rgb { r: v, g: v, b: v };
        }
        out
    }
}

impl Default for TermTheme {
    fn default() -> Self {
        Self::pitwall_dark()
    }
}

/// Colours in effect for one frame: the theme, overridden by whatever the
/// program set with OSC 4 / 10 / 11 / 12.
#[derive(Clone, Debug)]
pub struct Palette {
    pub colors: [Rgb; 256],
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub bold_is_bright: bool,
    pub dim_alpha: u8,
}

impl Palette {
    pub fn new(theme: &TermTheme, overrides: Option<&Colors>) -> Self {
        let mut colors = theme.palette();
        let mut foreground = theme.foreground;
        let mut background = theme.background;
        let mut cursor = theme.cursor;
        if let Some(o) = overrides {
            for (i, c) in colors.iter_mut().enumerate() {
                if let Some(v) = o[i] {
                    *c = v;
                }
            }
            foreground = o[NamedColor::Foreground].unwrap_or(foreground);
            background = o[NamedColor::Background].unwrap_or(background);
            cursor = o[NamedColor::Cursor].unwrap_or(cursor);
        }
        Palette {
            colors,
            foreground,
            background,
            cursor,
            bold_is_bright: theme.bold_is_bright,
            dim_alpha: theme.dim_alpha,
        }
    }

    /// A colour as the terminal stores it; `bold` applies bold-as-bright.
    pub fn resolve(&self, c: Color, bold: bool) -> Rgb {
        match c {
            Color::Spec(rgb) => rgb,
            Color::Indexed(i) => {
                let i = if bold && self.bold_is_bright && i < 8 { i + 8 } else { i };
                self.colors[i as usize]
            }
            Color::Named(n) => self.named(n, bold),
        }
    }

    fn named(&self, n: NamedColor, bold: bool) -> Rgb {
        let i = n as usize;
        if i < 16 {
            let i = if bold && self.bold_is_bright && i < 8 { i + 8 } else { i };
            return self.colors[i];
        }
        match n {
            NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            NamedColor::DimForeground => self.foreground,
            // Dim* named colours (alacritty's own dim mapping): the normal colour.
            other => {
                let base = (other as usize).saturating_sub(NamedColor::DimBlack as usize);
                self.colors[base.min(7)]
            }
        }
    }

    /// The (foreground, background) a cell is drawn with, after inverse,
    /// dim and hidden. `None` background = the terminal's default (not drawn).
    pub fn cell_colors(&self, fg: Color, bg: Color, flags: Flags) -> (Rgba8, Option<Rgb>) {
        let bold = flags.contains(Flags::BOLD);
        let mut f = self.resolve(fg, bold);
        let default_bg = matches!(bg, Color::Named(NamedColor::Background));
        let mut b = if default_bg { None } else { Some(self.resolve(bg, false)) };
        if flags.contains(Flags::INVERSE) {
            let old_b = b.unwrap_or(self.background);
            b = Some(f);
            f = old_b;
        }
        let mut fg = Rgba8::opaque(f);
        if flags.contains(Flags::DIM) {
            fg = fg.with_alpha(self.dim_alpha);
        }
        if flags.contains(Flags::HIDDEN) {
            fg = fg.with_alpha(0);
        }
        (fg, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_matches_xterm() {
        let p = TermTheme::pitwall_dark().palette();
        assert_eq!(p[1], hex(0xff6b6b));
        assert_eq!(p[16], hex(0x000000));
        assert_eq!(p[17], hex(0x00005f));
        assert_eq!(p[196], hex(0xff0000));
        assert_eq!(p[231], hex(0xffffff));
        assert_eq!(p[232], hex(0x080808));
        assert_eq!(p[255], hex(0xeeeeee));
    }

    #[test]
    fn bold_bright_inverse_dim_hidden() {
        let pal = Palette::new(&TermTheme::pitwall_dark(), None);
        let red = Color::Named(NamedColor::Red);
        let bg = Color::Named(NamedColor::Background);
        let fg = Color::Named(NamedColor::Foreground);
        assert_eq!(pal.cell_colors(red, bg, Flags::BOLD).0, Rgba8::opaque(hex(0xff8787)));
        assert_eq!(pal.cell_colors(Color::Indexed(1), bg, Flags::BOLD).0, Rgba8::opaque(hex(0xff8787)));
        // Truecolor is never brightened.
        let tc = Color::Spec(hex(0x102030));
        assert_eq!(pal.cell_colors(tc, bg, Flags::BOLD).0, Rgba8::opaque(hex(0x102030)));
        // Inverse of defaults: background-coloured text on a foreground block.
        let (f, b) = pal.cell_colors(fg, bg, Flags::INVERSE);
        assert_eq!(f, Rgba8::opaque(hex(0x0d0f12)));
        assert_eq!(b, Some(hex(0xd9dde3)));
        assert_eq!(pal.cell_colors(fg, bg, Flags::DIM).0.a, 0x80);
        assert_eq!(pal.cell_colors(fg, bg, Flags::HIDDEN).0.a, 0);
        assert_eq!(pal.cell_colors(fg, bg, Flags::empty()).1, None);
    }
}
