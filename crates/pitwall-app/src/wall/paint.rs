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
    FontStyle, FontWeight, Hsla, Pixels, Point, ShapedLine, StrikethroughStyle, TextRun,
    UnderlineStyle, Window,
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

/// The monospace family until the app embeds JetBrains Mono (phase 1).
pub const MONO: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else if cfg!(target_os = "windows") {
    "Consolas"
} else {
    "DejaVu Sans Mono"
};

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
    let h = f32::from(ts.ascent(id, font_size)) + f32::from(ts.descent(id, font_size)).abs();
    let h = if h > 0. {
        h
    } else {
        f32::from(font_size) * 1.2
    };
    (w, (h.ceil() * LINE_HEIGHT).floor())
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
    let base = font(MONO);
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
    };
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
            let x = left + seg.col as f32 * cell_w;
            if x >= right_limit {
                break;
            }
            let mut runs = Vec::with_capacity(seg.pieces.len());
            let mut col = seg.col as f32;
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
                col += p.cells as f32;
                let mut f = base.clone();
                if st.bold {
                    f.weight = FontWeight::BOLD;
                }
                if st.italic {
                    f.style = FontStyle::Italic;
                }
                runs.push(TextRun {
                    len: p.len,
                    font: f,
                    color: st.fg,
                    background_color: None,
                    underline: (st.underline > 0).then_some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(st.fg),
                        wavy: st.underline == 3,
                    }),
                    strikethrough: st.strike.then_some(StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(st.fg),
                    }),
                });
            }
            // Narrow characters sit on the cell grid; a wide or clustered
            // one is drawn as the font draws it.
            let force = (!seg.single).then_some(px(cell_w));
            let shaped = ts.shape_line(seg.text.clone(), font_size, &runs, force);
            out.lines.push((point(px(x), px(y)), shaped));
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
    fn term_colours_are_shared_per_mode() {
        assert!(Arc::ptr_eq(
            &term_colors(Mode::Dark),
            &term_colors(Mode::Dark)
        ));
        assert_ne!(term_colors(Mode::Dark).bg, term_colors(Mode::Light).bg);
    }
}
