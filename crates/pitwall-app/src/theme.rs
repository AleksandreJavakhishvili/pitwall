//! Colour, size and motion tokens and the appearance every window follows,
//! ported from `src/styles/tokens.css`, `glass.css` and `motion.css` (built
//! in-house: Zed's `theme` crate is GPL-3.0). Keep the values in step with
//! the CSS until the Tauri app is retired.
//!
//! - [`Theme`]: one mode's tokens, a global. With Glass on it also carries
//!   the glass tier and tokens, so widgets (pill buttons, chrome surfaces)
//!   follow without asking anything else.
//! - [`Appearance`]: Settings → Appearance and Tiles (theme, look, motion,
//!   density, font) resolved against what the OS asks for, a global.
//! - [`chrome`] / [`float`] / [`panel`]: the theme re-pointed at the glass
//!   surfaces for chrome (top bar, sidebar, right panel, strip), floating
//!   surfaces (dialogs, menus, toasts) and Liquid Glass slabs. Terminals,
//!   Wall tiles and code views keep the plain theme: they stay opaque.
//! - [`backdrop`] for the window's root background, [`motion_on`] before
//!   animating anything ([`crate::kit::motion`] has the durations,
//!   easings and animated elements).

use gpui::{
    linear_color_stop, linear_gradient, point, px, rgb, rgba, App, Background,
    BoxShadow, Global, Hsla, Pixels, Window, WindowAppearance,
};
use serde::{Deserialize, Serialize};

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
    /// `--amber-glow`: the soft glow of blocked rows, panes and the count.
    pub amber_glow: Hsla,
    pub flag_dark: Hsla,
    /// `--diff-add-bg` / `--diff-add-ln` / `--diff-del-bg` / `--diff-del-ln`.
    pub diff_add_bg: Hsla,
    pub diff_add_ln: Hsla,
    pub diff_del_bg: Hsla,
    pub diff_del_ln: Hsla,
    /// `--backdrop` behind modals and drawers (Flat; see [`Theme::scrim`]).
    pub backdrop: Hsla,
    /// `--shadow`'s drop part (the 1px ring is a border in GPUI).
    pub shadow: Hsla,
    /// The glass tier shown (`None`: Flat).
    pub glass: Option<GlassTier>,
    /// glass.css `--g-*` for this mode (used only while [`Theme::glass`] is set).
    pub g: GlassTokens,
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
            amber_glow: a(0xffb224, 0.38),
            flag_dark: c(0x2a2e35),
            diff_add_bg: a(0x3fd07f, 0.10),
            diff_add_ln: a(0x3fd07f, 0.16),
            diff_del_bg: a(0xff5f5f, 0.10),
            diff_del_ln: a(0xff5f5f, 0.16),
            backdrop: a(0x040506, 0.62),
            shadow: a(0x000000, 0.55),
            glass: None,
            g: GlassTokens::for_mode(Mode::Dark),
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
            amber_glow: a(0xed9600, 0.35),
            flag_dark: c(0xd4d8dd),
            diff_add_bg: a(0x14a052, 0.09),
            diff_add_ln: a(0x14a052, 0.15),
            diff_del_bg: a(0xd63a3a, 0.08),
            diff_del_ln: a(0xd63a3a, 0.14),
            backdrop: a(0x1e2228, 0.28),
            shadow: a(0x141923, 0.18),
            glass: None,
            g: GlassTokens::for_mode(Mode::Light),
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

/// The current theme (set at start, by Settings and when the system
/// appearance changes). Dark Flat before anything set it.
pub fn theme(cx: &App) -> &Theme {
    if let Some(t) = cx.try_global::<Theme>() {
        return t;
    }
    static FALLBACK: std::sync::OnceLock<Theme> = std::sync::OnceLock::new();
    FALLBACK.get_or_init(Theme::dark)
}


// ───────────────────────────────────────────── appearance

// ───────────────────────────────────────────── preferences (ui.json values)

/// Settings → Appearance → Theme (`ui.theme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemePref {
    pub const ALL: [ThemePref; 3] = [ThemePref::System, ThemePref::Dark, ThemePref::Light];

    pub fn label(self) -> &'static str {
        match self {
            ThemePref::System => "System",
            ThemePref::Dark => "Dark",
            ThemePref::Light => "Light",
        }
    }

    /// The mode shown, given the OS appearance (`resolveScheme`).
    pub fn resolve(self, os: WindowAppearance) -> Mode {
        match self {
            ThemePref::System => Mode::for_appearance(os),
            ThemePref::Dark => Mode::Dark,
            ThemePref::Light => Mode::Light,
        }
    }

    /// The native window appearance to force (`None`: follow the OS).
    pub fn forced(self) -> Option<Mode> {
        match self {
            ThemePref::System => None,
            ThemePref::Dark => Some(Mode::Dark),
            ThemePref::Light => Some(Mode::Light),
        }
    }
}

/// Settings → Appearance → Look (`ui.look`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Look {
    #[default]
    Flat,
    Glass,
}

impl Look {
    pub const ALL: [Look; 2] = [Look::Flat, Look::Glass];
}

/// Settings → Tiles → Density (`ui.density`; `src/layout/density.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    Comfortable,
    #[default]
    Compact,
    Dense,
}

/// Terminal cells of the smallest tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cells {
    pub cols: u16,
    pub rows: u16,
}

/// The base terminal font (`ui.fontSize`): default, limits, floor.
pub const DEFAULT_FONT: u8 = 13;
pub const FONT_MIN: u8 = 9;
pub const FONT_MAX: u8 = 22;
/// Tiles shrink their font to this before folding into chips.
pub const FONT_FLOOR: u8 = 10;
/// Estimated cell size per px of font (JetBrains Mono, line height 1.15).
pub const CELL_PER_PX: (f64, f64) = (0.6, 1.32);
/// Pane chrome around a terminal (header, insets, scrollbar).
pub const PANE_CHROME: (f64, f64) = (28.0, 44.0);

pub fn clamp_font(n: i32) -> u8 {
    n.clamp(FONT_MIN as i32, FONT_MAX as i32) as u8
}

impl Density {
    pub const ALL: [Density; 3] = [Density::Comfortable, Density::Compact, Density::Dense];

    pub fn label(self) -> &'static str {
        match self {
            Density::Comfortable => "Comfortable",
            Density::Compact => "Compact",
            Density::Dense => "Dense",
        }
    }

    /// Smallest readable terminal at this density (`DENSITY_CELLS`).
    pub fn cells(self) -> Cells {
        match self {
            Density::Comfortable => Cells { cols: 80, rows: 20 },
            Density::Compact => Cells { cols: 60, rows: 12 },
            Density::Dense => Cells { cols: 40, rows: 8 },
        }
    }

    /// Pixel size (w, h) of a pane holding the minimum cells at `font`
    /// (`minTilePx`).
    pub fn min_tile_px(self, font: f32) -> (f32, f32) {
        // In f64 like the JS it ports (same rounding at the ceil).
        let c = self.cells();
        let f = font as f64;
        (
            (c.cols as f64 * f * CELL_PER_PX.0 + PANE_CHROME.0).ceil() as f32,
            (c.rows as f64 * f * CELL_PER_PX.1 + PANE_CHROME.1).ceil() as f32,
        )
    }

    /// Below this a pane folds into a chip (`foldMin`).
    pub fn fold_min(self, base: u8) -> (f32, f32) {
        self.min_tile_px(base.min(FONT_FLOOR) as f32)
    }

    /// The largest whole font ≤ `base` (not below the floor) at which a
    /// `w`×`h` box still holds the minimum cells (`autoTileFont`).
    pub fn auto_tile_font(self, w: f32, h: f32, base: u8) -> u8 {
        let min = self.cells();
        let floor = base.min(FONT_FLOOR);
        let mut f = base;
        while f > floor {
            let cols = (w as f64 / (f as f64 * CELL_PER_PX.0)).floor();
            let rows = (h as f64 / (f as f64 * CELL_PER_PX.1)).floor();
            if cols >= min.cols as f64 && rows >= min.rows as f64 {
                return f;
            }
            f -= 1;
        }
        floor
    }
}

// ───────────────────────────────────────────── glass tiers

/// The window material this desktop offers (`HostInfo::glass`, plus Liquid
/// Glass on macOS 26+). `PITWALL_GLASS=liquid|vibrancy|mica|lite` overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlassOffer {
    /// macOS 26+: `NSGlassEffectView` behind GPUI's content.
    Liquid,
    /// Older macOS: `NSVisualEffectView` (GPUI's `Blurred`).
    Vibrancy,
    /// Windows 11: the Mica system backdrop behind a transparent window.
    Mica,
    /// Nothing native (Linux, Windows 10): Pitwall paints "Glass lite".
    None,
}

/// What a window shows when Glass is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlassTier {
    Liquid,
    Native,
    Mica,
    Lite,
}

pub const GLASS_ENV: &str = "PITWALL_GLASS";

/// `PITWALL_GLASS` (`glass_override` in `pitwall_core::host`, plus `liquid`).
pub fn parse_glass_override(v: Option<&str>) -> Option<GlassOffer> {
    match v? {
        "liquid" => Some(GlassOffer::Liquid),
        "vibrancy" => Some(GlassOffer::Vibrancy),
        "mica" => Some(GlassOffer::Mica),
        "lite" | "none" => Some(GlassOffer::None),
        _ => None,
    }
}

impl GlassOffer {
    /// Decided once per run: the override, else the best this OS can draw.
    /// An override the OS can't draw degrades (Liquid → Vibrancy on macOS
    /// before 26, anything native → lite elsewhere).
    pub fn detect() -> GlassOffer {
        let wanted = parse_glass_override(std::env::var(GLASS_ENV).ok().as_deref());
        Self::degrade(wanted.unwrap_or_else(crate::platform::glass::default_offer))
    }

    fn degrade(self) -> GlassOffer {
        match self {
            GlassOffer::Liquid if !crate::platform::glass::liquid_available() => {
                if cfg!(target_os = "macos") {
                    GlassOffer::Vibrancy
                } else {
                    GlassOffer::None
                }
            }
            GlassOffer::Vibrancy if !cfg!(target_os = "macos") => GlassOffer::None,
            GlassOffer::Mica if !cfg!(windows) => GlassOffer::None,
            other => other,
        }
    }

    /// How Settings names Glass here (`glassLabel`).
    pub fn label(self) -> &'static str {
        match self {
            GlassOffer::Liquid => "Glass · Liquid",
            GlassOffer::Vibrancy => "Glass · native",
            GlassOffer::Mica => "Glass · Mica",
            GlassOffer::None => "Glass lite",
        }
    }

    /// The tooltip on the Glass button (`GLASS_HINT`).
    pub fn hint(self) -> &'static str {
        match self {
            GlassOffer::Liquid => {
                "Translucent chrome over macOS Liquid Glass. Terminals stay solid."
            }
            GlassOffer::Vibrancy => {
                "Translucent chrome over the window's native material. Terminals stay solid."
            }
            GlassOffer::Mica => "Translucent chrome over Windows 11 Mica. Terminals stay solid.",
            GlassOffer::None => {
                "Tinted chrome over Pitwall's own backdrop, no blur (no window material on this desktop). Terminals stay solid."
            }
        }
    }

    pub fn tier(self) -> GlassTier {
        match self {
            GlassOffer::Liquid => GlassTier::Liquid,
            GlassOffer::Vibrancy => GlassTier::Native,
            GlassOffer::Mica => GlassTier::Mica,
            GlassOffer::None => GlassTier::Lite,
        }
    }
}

impl GlassTier {
    /// The window has a native material behind it (the UI only tints).
    pub fn is_native(self) -> bool {
        !matches!(self, GlassTier::Lite)
    }
}

/// What the OS asks for (macOS accessibility display options, Windows
/// client-area animation, GNOME's enable-animations).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OsPrefs {
    pub reduce_transparency: bool,
    pub increase_contrast: bool,
    pub reduce_motion: bool,
}

// ───────────────────────────────────────────── tokens

/// glass.css `--g-*` tokens for one mode.
#[derive(Debug, Clone, PartialEq)]
pub struct GlassTokens {
    pub surface: Hsla,
    pub surface_2: Hsla,
    pub surface_3: Hsla,
    pub raised: Hsla,
    /// Surfaces floating over terminals (dialogs, menus, toasts).
    pub float: Hsla,
    pub well: Hsla,
    pub line: Hsla,
    pub line_strong: Hsla,
    pub highlight: Hsla,
    pub on_primary: Hsla,
    /// `--text-3` on glass (small text stays ≥ 4.5:1).
    pub text_3: Hsla,
    pub backdrop: Hsla,
    pub shadow: Hsla,
    /// Lite: the painted backdrop (`--g-base` with the `--g-atmo` colours
    /// folded in; GPUI draws two-stop linear gradients).
    pub base_top: Hsla,
    pub base_bottom: Hsla,
    /// Native tiers: the whisper of colour over the window material
    /// (`--g-atmo-native`).
    pub atmo_top: Hsla,
    pub atmo_bottom: Hsla,
    /// Primary button sheen (`--g-sheen`).
    pub sheen: Hsla,
}

impl GlassTokens {
    pub fn for_mode(mode: Mode) -> GlassTokens {
        match mode {
            Mode::Dark => GlassTokens {
                surface: a(0x0e1016, 0.36),
                surface_2: a(0xffffff, 0.045),
                surface_3: a(0xffffff, 0.085),
                raised: a(0x181b22, 0.72),
                float: a(0x161920, 0.78),
                well: a(0x000000, 0.22),
                line: a(0xffffff, 0.075),
                line_strong: a(0xffffff, 0.14),
                highlight: a(0xffffff, 0.07),
                on_primary: c(0x0b0d12),
                text_3: c(0x858c97),
                backdrop: a(0x06080c, 0.38),
                shadow: a(0x000000, 0.5),
                base_top: c(0x2b3a62),
                base_bottom: c(0x3a2232),
                atmo_top: a(0x34446e, 0.42),
                atmo_bottom: a(0x3c2434, 0.34),
                sheen: a(0x788cbe, 0.3),
            },
            Mode::Light => GlassTokens {
                surface: a(0xffffff, 0.5),
                surface_2: a(0x1e283c, 0.05),
                surface_3: a(0x1e283c, 0.085),
                raised: a(0xffffff, 0.76),
                float: a(0xffffff, 0.82),
                well: a(0xffffff, 0.55),
                line: a(0x1e283c, 0.09),
                line_strong: a(0x1e283c, 0.16),
                highlight: a(0xffffff, 0.7),
                on_primary: c(0xffffff),
                text_3: c(0x5f6670),
                backdrop: a(0x28303e, 0.16),
                shadow: a(0x1e283c, 0.2),
                base_top: c(0xc9d5ec),
                base_bottom: c(0xf0d9d4),
                atmo_top: a(0xdde4f2, 0.34),
                atmo_bottom: a(0xf3e0db, 0.3),
                sheen: a(0xffffff, 0.28),
            },
        }
    }

    /// Increase contrast: glass goes (almost) solid, hairlines get stronger.
    pub fn with_more_contrast(mut self, base: &Theme) -> GlassTokens {
        self.surface = base.surface.opacity(0.94);
        self.raised = base.raised;
        self.float = base.raised;
        self.line = base.line_strong;
        self.line_strong = base.text_4;
        self.text_3 = base.text_2;
        self
    }
}

/// A cubic Bézier easing (CSS `cubic-bezier(x1, y1, x2, y2)`).
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 + Clone {
    move |t: f32| {
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        let bez = |p1: f32, p2: f32, s: f32| {
            let u = 1.0 - s;
            3.0 * u * u * s * p1 + 3.0 * u * s * s * p2 + s * s * s
        };
        // Solve x(s) = t by bisection (monotonic in s for 0 ≤ x1, x2 ≤ 1).
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if bez(x1, x2, mid) < t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        bez(y1, y2, (lo + hi) / 2.0)
    }
}

// ───────────────────────────────────────────── the resolved state

/// The appearance every window shows (a global; promote into theme.rs).
#[derive(Debug, Clone)]
pub struct Appearance {
    pub pref: ThemePref,
    pub mode: Mode,
    pub look: Look,
    /// What this desktop offers (decided once per run).
    pub offer: GlassOffer,
    /// The tier shown, `None` when Flat shows (chosen, or Reduce
    /// transparency).
    pub glass: Option<GlassTier>,
    pub os: OsPrefs,
    /// The setting OR the OS: nothing animates.
    pub reduce_motion: bool,
    /// The Settings toggle alone.
    pub reduce_motion_setting: bool,
    pub density: Density,
    pub font_size: u8,
    pub glass_tokens: GlassTokens,
}

impl Global for Appearance {}

/// The inputs of [`Appearance::resolve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inputs {
    pub pref: ThemePref,
    pub look: Look,
    pub reduce_motion: bool,
    pub density: Density,
    pub font_size: u8,
}

impl Default for Inputs {
    fn default() -> Self {
        Inputs {
            pref: ThemePref::System,
            look: Look::Flat,
            reduce_motion: false,
            density: Density::Compact,
            font_size: DEFAULT_FONT,
        }
    }
}

impl Appearance {
    pub fn resolve(
        inputs: Inputs,
        os_appearance: WindowAppearance,
        offer: GlassOffer,
        os: OsPrefs,
    ) -> Appearance {
        let mode = inputs.pref.resolve(os_appearance);
        let base = Theme::for_mode(mode);
        let shows = inputs.look == Look::Glass && !os.reduce_transparency;
        let mut glass_tokens = GlassTokens::for_mode(mode);
        if os.increase_contrast {
            glass_tokens = glass_tokens.with_more_contrast(&base);
        }
        Appearance {
            pref: inputs.pref,
            mode,
            look: inputs.look,
            offer,
            glass: shows.then(|| offer.tier()),
            os,
            reduce_motion: inputs.reduce_motion || os.reduce_motion,
            reduce_motion_setting: inputs.reduce_motion,
            density: inputs.density,
            font_size: inputs.font_size,
            glass_tokens,
        }
    }

    /// The base theme for this appearance: `theme.rs` for the mode, with the
    /// few tokens glass.css changes everywhere (`--text-3`).
    pub fn theme(&self) -> Theme {
        let mut t = Theme::for_mode(self.mode);
        t.g = self.glass_tokens.clone();
        t.glass = self.glass;
        if self.glass.is_some() {
            t.text_3 = self.glass_tokens.text_3;
        }
        t
    }

    pub fn inputs(&self) -> Inputs {
        Inputs {
            pref: self.pref,
            look: self.look,
            reduce_motion: self.reduce_motion_setting,
            density: self.density,
            font_size: self.font_size,
        }
    }
}

/// The current appearance (Flat, dark, defaults before `settings::init`).
pub fn appearance(cx: &App) -> &Appearance {
    if let Some(a) = cx.try_global::<Appearance>() {
        return a;
    }
    static FALLBACK: std::sync::OnceLock<Appearance> = std::sync::OnceLock::new();
    FALLBACK.get_or_init(|| {
        Appearance::resolve(
            Inputs::default(),
            WindowAppearance::Dark,
            GlassOffer::None,
            OsPrefs::default(),
        )
    })
}

/// Animations may run (Reduce motion is off, here and in the OS).
pub fn motion_on(cx: &App) -> bool {
    !appearance(cx).reduce_motion
}

/// Chrome regions get their own Liquid Glass pieces (macOS 26+, Glass on).
/// `PITWALL_GLASS_REGIONS=0` keeps window-level glass only.
pub fn glass_regions(cx: &App) -> bool {
    appearance(cx).glass == Some(GlassTier::Liquid)
        && std::env::var("PITWALL_GLASS_REGIONS").as_deref() != Ok("0")
}

impl Theme {
    /// Glass is on (pill buttons, translucent chrome).
    pub fn is_glass(&self) -> bool {
        self.glass.is_some()
    }

    /// This theme with the chrome's glass surfaces (glass.css "chrome:
    /// translucent surface tokens"): top bar, tabs, sidebar, right panel,
    /// status strip. Flat: unchanged.
    pub fn chrome(&self) -> Theme {
        if self.glass.is_none() {
            return self.clone();
        }
        let g = &self.g;
        let mut out = self.clone();
        out.surface = g.surface;
        out.surface_2 = g.surface_2;
        out.surface_3 = g.surface_3;
        out.raised = g.raised;
        out.bg = g.well;
        out.line = g.line;
        out.line_strong = g.line_strong;
        out
    }

    /// Surfaces floating over terminals (dialogs, menus, toasts). The web
    /// view frosts these with `backdrop-filter`; GPUI 0.2.2 can't blur what
    /// the app itself draws (the native material only sees what is behind
    /// the window), so they are solid, as glass.css does for the lite tier
    /// (`--g-float: var(--raised)`), with the glass hairlines and pill buttons.
    pub fn float(&self) -> Theme {
        if self.glass.is_none() {
            return self.clone();
        }
        let mut out = self.chrome();
        out.surface = self.raised;
        out.raised = self.raised;
        out
    }

    /// `--backdrop` behind a modal or a drawer.
    pub fn scrim(&self) -> Hsla {
        if self.glass.is_some() {
            self.g.backdrop
        } else {
            self.backdrop
        }
    }

    /// `--shadow` (the drop part) of dialogs and menus.
    pub fn drop_shadow(&self) -> Vec<BoxShadow> {
        let (color, y, blur): (Hsla, Pixels, Pixels) = if self.glass.is_some() {
            (self.g.shadow, px(24.), px(60.))
        } else {
            (self.shadow, px(18.), px(50.))
        };
        vec![BoxShadow {
            color,
            offset: point(px(0.), y),
            blur_radius: blur,
            spread_radius: px(0.),
        }]
    }

    /// The whole `--shadow` of dialogs, menus and toasts: the drop and its
    /// 1 px ring outside the box (`0 0 0 1px var(--line-strong)`; a border
    /// would sit inside and move the content).
    pub fn float_shadow(&self) -> Vec<BoxShadow> {
        let mut s = self.drop_shadow();
        s.push(self.ring(if self.glass.is_some() {
            self.g.line_strong
        } else {
            self.line_strong
        }));
        s
    }

    /// `box-shadow: 0 0 0 1px <color>`: a 1 px ring outside the box.
    pub fn ring(&self, color: Hsla) -> BoxShadow {
        BoxShadow {
            color,
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(1.),
        }
    }

    /// `--amber-glow` as a soft outer glow (blocked rows, panes, the count).
    pub fn amber_glow(&self, blur: f32, spread: f32) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: self.amber_glow,
            offset: point(px(0.), px(0.)),
            blur_radius: px(blur),
            spread_radius: px(spread),
        }]
    }

    /// `--diff-add-bg` / `--diff-del-bg` (or the line-number tints).
    pub fn diff_bg(&self, add: bool, number: bool) -> Hsla {
        match (add, number) {
            (true, false) => self.diff_add_bg,
            (true, true) => self.diff_add_ln,
            (false, false) => self.diff_del_bg,
            (false, true) => self.diff_del_ln,
        }
    }
}

/// `t` for chrome (see [`Theme::chrome`]).
pub fn chrome(t: &Theme, _cx: &App) -> Theme {
    t.chrome()
}

/// `t` for a chrome panel drawn as its own glass piece
/// ([`crate::kit::chrome_panel`]): clear surfaces, the glass shows through.
pub fn panel(t: &Theme, cx: &App) -> Theme {
    let mut out = t.chrome();
    if glass_regions(cx) {
        // The slab is its own glass piece: no hairline of our own, and only
        // a light tint (`--g-surface`) so text isn't drawn on bare
        // transparency, where its antialiasing comes out hollow.
        out.line = gpui::transparent_black();
    }
    out
}

/// `t` for floating surfaces (see [`Theme::float`]).
pub fn float(t: &Theme, _cx: &App) -> Theme {
    t.float()
}

/// The window's root background: opaque `--bg` (Flat), the painted
/// backdrop (lite), or a light tint over the native material.
pub fn backdrop(cx: &App) -> Background {
    let ap = appearance(cx);
    match ap.glass {
        None => theme(cx).bg.into(),
        Some(GlassTier::Lite) => linear_gradient(
            180.,
            linear_color_stop(ap.glass_tokens.base_top, 0.),
            linear_color_stop(ap.glass_tokens.base_bottom, 1.),
        ),
        Some(_) => linear_gradient(
            180.,
            linear_color_stop(ap.glass_tokens.atmo_top, 0.),
            linear_color_stop(ap.glass_tokens.atmo_bottom, 1.),
        ),
    }
}

/// `--backdrop` behind a modal.
pub fn modal_backdrop(cx: &App) -> Hsla {
    theme(cx).scrim()
}

/// `--shadow` (the drop part).
pub fn shadow(cx: &App) -> Vec<BoxShadow> {
    theme(cx).drop_shadow()
}

/// The OS appearance (the app's, not a window's: a window forced dark still
/// reports dark).
pub fn os_appearance(cx: &App) -> WindowAppearance {
    cx.window_appearance()
}

/// Recompute the global from new inputs and what the OS says now; push the
/// theme and every window's material. Returns whether anything changed.
pub fn apply(inputs: Inputs, cx: &mut App) -> bool {
    let offer = cx
        .try_global::<Appearance>()
        .map(|a| a.offer)
        .unwrap_or_else(GlassOffer::detect);
    let next = Appearance::resolve(inputs, os_appearance(cx), offer, crate::platform::glass::os_prefs());
    let changed = cx
        .try_global::<Appearance>()
        .is_none_or(|cur| !same(cur, &next));
    crate::kit::motion::set_on(!next.reduce_motion);
    cx.set_global(next.theme());
    cx.set_global(next);
    for w in cx.windows() {
        let _ = w.update(cx, |_, window, cx| apply_window(window, cx));
    }
    cx.refresh_windows();
    changed
}

fn same(a: &Appearance, b: &Appearance) -> bool {
    a.mode == b.mode && a.glass == b.glass && a.os == b.os && a.inputs() == b.inputs()
}

/// The OS appearance or its accessibility options changed (a window's
/// observer, or a window coming back to the front).
pub fn os_changed(cx: &mut App) {
    let inputs = appearance(cx).inputs();
    apply(inputs, cx);
}

/// Give one window its material and native appearance (title bar).
pub fn apply_window(window: &mut Window, cx: &App) {
    let ap = appearance(cx);
    crate::platform::glass::apply(window, ap.glass, ap.pref.forced(), ap.mode);
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


    #[test]
    fn tokens_match_the_css() {
        let tokens = include_str!("../../../src/styles/tokens.css");
        for v in [
            "rgba(255, 178, 36, 0.38)",
            "#2a2e35",
            "rgba(4, 5, 6, 0.62)",
            "#d4d8dd",
        ] {
            assert!(
                tokens.contains(v),
                "{v} gone from tokens.css: update appearance.rs"
            );
        }
        let glass = include_str!("../../../src/styles/glass.css");
        for v in [
            "rgba(14, 16, 22, 0.36)",
            "rgba(22, 25, 32, 0.78)",
            "#858c97",
            "rgba(255, 255, 255, 0.82)",
            "#5f6670",
        ] {
            assert!(
                glass.contains(v),
                "{v} gone from glass.css: update appearance.rs"
            );
        }
        let motion = include_str!("../../../src/styles/motion.css");
        for v in [
            "cubic-bezier(0.2, 0.9, 0.3, 1.18)",
            "cubic-bezier(0.16, 1, 0.3, 1)",
            "m-open 0.2s",
        ] {
            assert!(
                motion.contains(v),
                "{v} gone from motion.css: update appearance.rs"
            );
        }
        let g = GlassTokens::for_mode(Mode::Dark);
        assert!((g.float.a - 0.78).abs() < 0.01);
        assert_eq!(GlassTokens::for_mode(Mode::Light).text_3, c(0x5f6670));
    }

    #[test]
    fn the_theme_preference_resolves_against_the_os() {
        assert_eq!(
            ThemePref::System.resolve(WindowAppearance::Light),
            Mode::Light
        );
        assert_eq!(
            ThemePref::System.resolve(WindowAppearance::VibrantDark),
            Mode::Dark
        );
        assert_eq!(ThemePref::Dark.resolve(WindowAppearance::Light), Mode::Dark);
        assert_eq!(ThemePref::Light.forced(), Some(Mode::Light));
        assert_eq!(ThemePref::System.forced(), None);
    }

    #[test]
    fn reduce_transparency_shows_flat_and_motion_follows_the_os() {
        let inputs = Inputs {
            look: Look::Glass,
            ..Inputs::default()
        };
        let os = OsPrefs::default();
        let a = Appearance::resolve(inputs, WindowAppearance::Dark, GlassOffer::Liquid, os);
        assert_eq!(a.glass, Some(GlassTier::Liquid));
        assert!(!a.reduce_motion);
        let os = OsPrefs {
            reduce_transparency: true,
            reduce_motion: true,
            ..os
        };
        let b = Appearance::resolve(inputs, WindowAppearance::Dark, GlassOffer::Liquid, os);
        assert_eq!(b.glass, None, "Reduce transparency → Flat");
        assert!(b.reduce_motion && !b.reduce_motion_setting);
        assert_eq!(b.inputs(), inputs, "the user's choices are kept");
        let flat = Appearance::resolve(
            Inputs::default(),
            WindowAppearance::Dark,
            GlassOffer::Liquid,
            OsPrefs::default(),
        );
        assert_eq!(flat.glass, None);
    }

    #[test]
    fn glass_changes_text_3_and_more_contrast_makes_it_solid() {
        let inputs = Inputs {
            look: Look::Glass,
            ..Inputs::default()
        };
        let a = Appearance::resolve(
            inputs,
            WindowAppearance::Dark,
            GlassOffer::None,
            OsPrefs::default(),
        );
        assert_eq!(a.glass, Some(GlassTier::Lite));
        assert_eq!(a.theme().text_3, c(0x858c97));
        let more = OsPrefs {
            increase_contrast: true,
            ..OsPrefs::default()
        };
        let b = Appearance::resolve(inputs, WindowAppearance::Dark, GlassOffer::None, more);
        assert_eq!(b.glass_tokens.float, Theme::dark().raised);
    }

    #[test]
    fn the_glass_override_is_parsed() {
        assert_eq!(
            parse_glass_override(Some("liquid")),
            Some(GlassOffer::Liquid)
        );
        assert_eq!(parse_glass_override(Some("lite")), Some(GlassOffer::None));
        assert_eq!(parse_glass_override(Some("mica")), Some(GlassOffer::Mica));
        assert_eq!(parse_glass_override(Some("bogus")), None);
        assert_eq!(parse_glass_override(None), None);
        assert_eq!(GlassOffer::Liquid.label(), "Glass · Liquid");
        assert_eq!(GlassOffer::None.label(), "Glass lite");
        if !cfg!(windows) {
            assert_eq!(GlassOffer::Mica.degrade(), GlassOffer::None);
        }
    }

    #[test]
    fn easings_match_css() {
        let spring = cubic_bezier(0.2, 0.9, 0.3, 1.18);
        assert_eq!(spring(0.0), 0.0);
        assert_eq!(spring(1.0), 1.0);
        // The spring overshoots past 1 before settling.
        assert!((0..100).map(|i| spring(i as f32 / 100.0)).any(|v| v > 1.0));
        let soft = cubic_bezier(0.16, 1.0, 0.3, 1.0);
        assert!(soft(0.5) > 0.85, "out-soft is front-loaded: {}", soft(0.5));
        let linear = cubic_bezier(0.0, 0.0, 1.0, 1.0);
        assert!((linear(0.3) - 0.3).abs() < 0.01);
    }

    #[test]
    fn density_policy_matches_density_ts() {
        assert_eq!(Density::Compact.cells(), Cells { cols: 60, rows: 12 });
        // As density.ts computes it (f64): 60 × 13 × 0.6 + 28 = 496;
        // 12 × 13 × 1.32 + 44 = 249.92 → 250.
        assert_eq!(Density::Compact.min_tile_px(13.0), (496.0, 250.0));
        assert_eq!(
            Density::Dense.fold_min(13),
            Density::Dense.min_tile_px(10.0)
        );
        // A big box keeps the base font; a small one shrinks to the floor.
        assert_eq!(Density::Compact.auto_tile_font(2000.0, 2000.0, 13), 13);
        assert_eq!(
            Density::Compact.auto_tile_font(100.0, 100.0, 13),
            FONT_FLOOR
        );
        assert_eq!(clamp_font(40), FONT_MAX);
        assert_eq!(clamp_font(2), FONT_MIN);
    }
}
