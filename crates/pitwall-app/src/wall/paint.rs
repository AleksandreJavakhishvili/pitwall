//! Drawing a tile's [`Screen`]: the one seam where `pitwall-term-view`'s
//! view-only wall-tile mode plugs in later ([`screen_view`]).
//!
//! As the React Wall: the terminal at its real PTY size, scaled to the tile
//! width (never below 0.42, wider screens crop on the right), anchored
//! bottom-left 8 px / 6 px in so the bottom rows stay visible. Scaling is a
//! smaller font on the same cell grid. Rows above the tile are not shaped;
//! unchanged rows come from GPUI's line cache, and a tile that got no frame
//! is not painted again at all (it is a cached view).

use std::sync::{Arc, OnceLock};

use gpui::{
    canvas, fill, font, outline, point, prelude::*, px, size, App, BorderStyle, Bounds, Font,
    FontFeatures, FontStyle, FontWeight, Hsla, Pixels, Point, ShapedLine, SharedString,
    StrikethroughStyle, TextRun, UnderlineStyle, Window,
};

use super::screen::{resolve, Row, Screen, TermColors};
use crate::theme::Mode;

/// Never scale below this; wider terminals are cropped on the right.
pub const MIN_SCALE: f32 = 0.42;
/// `.wall-scale`: left 8 px, bottom 6 px.
const INSET_LEFT: f32 = 8.;
const INSET_BOTTOM: f32 = 6.;
/// xterm's line height (`LINE_HEIGHT` in screenStyle.ts).
const LINE_HEIGHT: f32 = 1.15;
/// The terminals' font size by default (`DEFAULT_FONT`, src/layout/density.ts).
pub const DEFAULT_FONT: f32 = 13.;

pub use crate::kit::MONO_FONT as MONO;

/// The terminal colours of a theme mode (built once).
pub fn term_colors(mode: Mode) -> Arc<TermColors> {
    static DARK: OnceLock<Arc<TermColors>> = OnceLock::new();
    static LIGHT: OnceLock<Arc<TermColors>> = OnceLock::new();
    match mode {
        Mode::Dark => DARK.get_or_init(|| Arc::new(TermColors::dark())).clone(),
        Mode::Light => LIGHT.get_or_init(|| Arc::new(TermColors::light())).clone(),
    }
}

/// How to draw a screen.
#[derive(Clone)]
pub struct ScreenOpts {
    pub font_size: f32,
    pub colors: Arc<TermColors>,
    /// Draw the cursor (only when the agent's own terminal would show it).
    pub show_cursor: bool,
}

/// The scale for a screen `natural` wide in a box `avail` wide.
pub fn scale_for(avail: f32, natural: f32) -> f32 {
    if natural <= 0. {
        return 1.;
    }
    (avail / natural).clamp(MIN_SCALE, 1.)
}

/// The element drawing `screen` view-only, filling its parent.
pub fn screen_view(screen: &Screen, opts: ScreenOpts) -> impl IntoElement {
    let lines: Vec<Arc<Row>> = screen.lines.clone();
    let (cols, rows) = (screen.cols, screen.rows);
    let cursor = screen.cursor.filter(|_| opts.show_cursor);
    canvas(
        move |bounds, window, cx| prepaint(bounds, &lines, cols, rows, cursor, &opts, window, cx),
        |_, p, window, cx| {
            for (b, c) in p.backgrounds {
                window.paint_quad(fill(b, c));
            }
            for (origin, line) in p.lines {
                let _ = line.paint(origin, p.line_h, window, cx);
            }
            for (b, c) in p.decorations {
                window.paint_quad(fill(b, c));
            }
            if let Some((b, c)) = p.cursor {
                window.paint_quad(outline(b, c, BorderStyle::Solid));
            }
        },
    )
    .size_full()
}

struct Prepainted {
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    line_h: Pixels,
    cursor: Option<(Bounds<Pixels>, Hsla)>,
    /// Underlines GPUI can't draw (double, dotted, dashed).
    decorations: Vec<(Bounds<Pixels>, Hsla)>,
}

/// CSS `text-decoration` styles GPUI's text runs lack, as rectangles
/// (`x`, `width`) along a cell span `w` wide from `x0`, `t` thick:
/// `double` two lines one thickness apart, `dotted` square dots one
/// thickness long with equal gaps, `dashed` dashes three long, two apart
/// (Chrome's proportions). The `dy` of each piece is its offset below the
/// underline position. Styles 1 (single) and 3 (wavy) are GPUI's own.
pub fn underline_pieces(style: u8, x0: f32, w: f32, t: f32) -> Vec<(f32, f32, f32)> {
    let dashes = |on: f32, off: f32| {
        let mut out = Vec::new();
        let mut x = x0;
        while x < x0 + w {
            out.push((x, on.min(x0 + w - x), 0.));
            x += on + off;
        }
        out
    };
    match style {
        2 => vec![(x0, w, 0.), (x0, w, 2. * t)],
        4 => dashes(t, t),
        5 => dashes(3. * t, 2. * t),
        _ => Vec::new(),
    }
}

/// Cell metrics at `font_size`: (cell width, row height) as xterm rounds
/// them at 1× (character height up to a whole pixel, times the line height).
fn metrics(family: &Font, font_size: Pixels, window: &Window) -> (f32, f32) {
    let ts = window.text_system();
    let id = ts.resolve_font(family);
    let w = ts
        .advance(id, font_size, 'm')
        .map(|s| f32::from(s.width))
        .unwrap_or(f32::from(font_size) * 0.6);
    // Chrome rounds ascent and descent to whole pixels for `line-height:
    // normal`; xterm then rounds the cell in device pixels.
    let h = f32::from(ts.ascent(id, font_size)).round()
        + f32::from(ts.descent(id, font_size)).abs().round();
    let h = if h > 0. {
        h
    } else {
        f32::from(font_size) * 1.2
    };
    (w, cell_height(h, window.scale_factor()))
}

/// xterm's cell height for a character `h` px high at `dpr` (RenderDimensions).
pub fn cell_height(h: f32, dpr: f32) -> f32 {
    ((h * dpr).ceil() * LINE_HEIGHT).floor() / dpr
}

#[allow(clippy::too_many_arguments)]
fn prepaint(
    bounds: Bounds<Pixels>,
    lines: &[Arc<Row>],
    cols: u16,
    rows: u16,
    cursor: Option<(u16, u16)>,
    opts: &ScreenOpts,
    window: &mut Window,
    _cx: &mut App,
) -> Prepainted {
    // Like the React Wall: no programming ligatures (`=>` stays two cells).
    let mut base = font(MONO);
    base.features = FontFeatures::disable_ligatures();
    let (cw, ch) = metrics(&base, px(opts.font_size), window);
    let s = scale_for(f32::from(bounds.size.width), cw * cols as f32);
    let font_size = px(opts.font_size * s);
    let (cell_w, line_h) = (cw * s, ch * s);
    let left = f32::from(bounds.left()) + INSET_LEFT;
    let bottom = f32::from(bounds.bottom()) - INSET_BOTTOM;
    let top_limit = f32::from(bounds.top());
    let right_limit = f32::from(bounds.right());
    let c = &opts.colors;
    let mut out = Prepainted {
        backgrounds: Vec::new(),
        lines: Vec::new(),
        line_h: px(line_h),
        cursor: None,
        decorations: Vec::new(),
    };
    // Where GPUI puts an underline: the baseline (the text centred in the
    // line box) plus 0.618 of the descent.
    let fid = window.text_system().resolve_font(&base);
    let (asc, desc) = (
        f32::from(window.text_system().ascent(fid, font_size)),
        f32::from(window.text_system().descent(fid, font_size)).abs(),
    );
    let under_dy = (line_h - (asc + desc)) / 2. + asc + desc * 0.618;
    // React scales the whole tile: a 1 px line scales with it.
    let thick = s.max(0.5);
    let ts = window.text_system().clone();
    for r in (0..rows).rev() {
        let y = bottom - (rows - r) as f32 * line_h;
        if y + line_h < top_limit {
            break;
        }
        if let Some((cc, cr)) = cursor {
            if cr == r {
                let col = cc.min(cols.saturating_sub(1)) as f32;
                out.cursor = Some((
                    Bounds::new(
                        point(px(left + col * cell_w), px(y)),
                        size(px(cell_w), px(line_h)),
                    ),
                    c.cursor,
                ));
            }
        }
        let Some(row) = lines.get(r as usize) else {
            continue;
        };
        for seg in &row.segments {
            if left + seg.col as f32 * cell_w >= right_limit {
                break;
            }
            // One shaped line per stretch of one font face: gpui 0.2.2 on
            // macOS draws every run of a line in the first run's face when
            // the faces are bundled fonts (bold would come out regular).
            let force = (!seg.single).then_some(px(cell_w));
            let mut col = seg.col as f32;
            let mut start_col = col;
            let mut start_byte = 0usize;
            let mut byte = 0usize;
            let mut runs: Vec<TextRun> = Vec::new();
            let flush =
                |runs: &mut Vec<TextRun>, from: usize, to: usize, at: f32, out: &mut Prepainted| {
                    if runs.is_empty() {
                        return;
                    }
                    let text: SharedString = seg.text[from..to].to_string().into();
                    let shaped = ts.shape_line(text, font_size, runs, force);
                    out.lines
                        .push((point(px(left + at * cell_w), px(y)), shaped));
                    runs.clear();
                };
            for p in &seg.pieces {
                let st = resolve(p.fg, p.bg, p.attrs, c);
                if let Some(bg) = st.bg {
                    out.backgrounds.push((
                        Bounds::new(
                            point(px(left + col * cell_w), px(y)),
                            size(px(p.cells as f32 * cell_w), px(line_h)),
                        ),
                        bg,
                    ));
                }
                let mut f = base.clone();
                if st.bold {
                    // `font-weight: bold` (JetBrains Mono Bold is bundled).
                    f.weight = FontWeight::BOLD;
                }
                if st.italic {
                    f.style = FontStyle::Italic;
                }
                if runs.last().is_some_and(|r: &TextRun| r.font != f) {
                    flush(&mut runs, start_byte, byte, start_col, &mut out);
                    start_byte = byte;
                    start_col = col;
                }
                runs.push(TextRun {
                    len: p.len,
                    font: f,
                    color: st.fg,
                    background_color: None,
                    underline: matches!(st.underline, 1 | 3).then_some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(st.fg),
                        wavy: st.underline == 3,
                    }),
                    strikethrough: st.strike.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(st.fg),
                    }),
                });
                for (x, w, dy) in
                    underline_pieces(st.underline, left + col * cell_w, p.cells as f32 * cell_w, thick)
                {
                    out.decorations.push((
                        Bounds::new(point(px(x), px(y + under_dy + dy)), size(px(w), px(thick))),
                        st.fg,
                    ));
                }
                byte += p.len;
                col += p.cells as f32;
            }
            flush(&mut runs, start_byte, byte, start_col, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_fits_the_width_within_limits() {
        assert_eq!(scale_for(800., 400.), 1., "never enlarged");
        assert!((scale_for(300., 600.) - 0.5).abs() < 1e-6);
        assert_eq!(
            scale_for(100., 1000.),
            MIN_SCALE,
            "cropped below the minimum"
        );
        assert_eq!(scale_for(100., 0.), 1.);
    }

    #[test]
    fn underline_styles_gpui_lacks_are_drawn() {
        // Single and wavy are GPUI's.
        assert!(underline_pieces(1, 0., 20., 1.).is_empty());
        assert!(underline_pieces(3, 0., 20., 1.).is_empty());
        // Double: two full lines, one thickness apart.
        assert_eq!(underline_pieces(2, 5., 20., 1.), vec![(5., 20., 0.), (5., 20., 2.)]);
        // Dotted: dot, gap, dot… within the span.
        let d = underline_pieces(4, 0., 5., 1.);
        assert_eq!(d.iter().map(|p| p.0).collect::<Vec<_>>(), vec![0., 2., 4.]);
        // Dashed: 3 on, 2 off; the last dash is cut at the span's end.
        let d = underline_pieces(5, 0., 8., 1.);
        assert_eq!(d, vec![(0., 3., 0.), (5., 3., 0.)]);
        let d = underline_pieces(5, 0., 7., 1.);
        assert_eq!(d[1], (5., 2., 0.));
    }

    #[test]
    fn cells_round_like_xterm() {
        assert_eq!(cell_height(17., 2.), 19.5);
        assert_eq!(cell_height(17., 1.), 19.);
    }

    #[test]
    fn term_colours_are_shared_per_mode() {
        assert!(Arc::ptr_eq(
            &term_colors(Mode::Dark),
            &term_colors(Mode::Dark)
        ));
        assert_ne!(term_colors(Mode::Dark).bg, term_colors(Mode::Light).bg);
    }
}
