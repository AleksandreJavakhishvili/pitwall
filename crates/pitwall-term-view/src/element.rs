//! The GPUI element that draws a terminal grid.
//!
//! Per frame: fold the emulator's damage into line generations, rebuild only
//! the rows whose generation changed *and* whose content hash changed
//! (`runs::row_key`), shape each text run once as a whole string, and keep
//! the shaped glyphs keyed by content, so scrolling reuses rows that merely
//! moved. Glyphs are then placed on their cells by column, not by the
//! shaper's advances, so fallback fonts (Georgian, CJK, emoji) stay on the
//! grid. Box drawing and block elements are drawn as device-pixel-snapped
//! quads (`boxdraw`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as GridPoint};
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{CursorShape, Rgb};
use gpui::{
    fill, point, px, quad, size, App, BorderStyle, Bounds, ContentMask, Corners, Edges, Element, ElementId, Entity,
    Font, FontFeatures, FontId, FontStyle as GFontStyle, FontWeight, GlobalElementId, GlyphId, Hitbox, HitboxBehavior,
    Hsla, InspectorElementId, IntoElement, LayoutId, Pixels, Point, Rgba, SharedString, StrikethroughStyle, Style,
    UnderlineStyle, Window,
};

use crate::boxdraw::{self, Corner, Prim};
use crate::runs::{self, FontStyle, LineKind, RowRuns};
use crate::terminal::{TermSize, Terminal};
use crate::theme::{Palette, Rgba8, TermTheme};
use crate::view::{TerminalView, ViewMode};

/// Font settings.
#[derive(Clone, Debug, PartialEq)]
pub struct TermFont {
    /// Tried in order; the first installed one is used.
    pub families: Vec<SharedString>,
    pub size: f32,
    /// Row height as a multiple of the font's height (xterm.js: 1.15 in Pitwall).
    pub line_height: f32,
}

impl Default for TermFont {
    fn default() -> Self {
        TermFont {
            families: [
                "JetBrains Mono",
                "Menlo",
                "SF Mono",
                "Monaco",
                "DejaVu Sans Mono",
                "Consolas",
                "Liberation Mono",
            ]
            .into_iter()
            .map(SharedString::from)
            .collect(),
            size: 13.0,
            line_height: 1.15,
        }
    }
}

/// Make font files (TTF / OTF bytes) available to every terminal, e.g. the
/// JetBrains Mono the web UI bundles. Register before the first frame.
pub fn register_fonts(cx: &App, fonts: Vec<std::borrow::Cow<'static, [u8]>>) -> std::io::Result<()> {
    cx.text_system().add_fonts(fonts).map_err(|e| std::io::Error::other(e.to_string()))
}

/// Cell metrics for one font, size and scale factor (logical pixels).
#[derive(Clone, Debug)]
pub struct Metrics {
    pub font_size: f32,
    pub cell_w: f32,
    pub cell_h: f32,
    /// Baseline offset from the top of a row.
    pub baseline: f32,
    pub underline_y: f32,
    pub underline_thickness: f32,
    pub strike_y: f32,
    fonts: [Font; 4],
    primary: FontId,
}

impl Metrics {
    fn font(&self, s: FontStyle) -> &Font {
        &self.fonts[s.bold as usize | (s.italic as usize) << 1]
    }
}

/// Snap a logical coordinate to the device pixel grid.
pub(crate) fn snap(v: f32, scale: f32) -> f32 {
    (v * scale).round() / scale
}

fn hsla(c: Rgba8) -> Hsla {
    Rgba { r: c.r as f32 / 255.0, g: c.g as f32 / 255.0, b: c.b as f32 / 255.0, a: c.a as f32 / 255.0 }.into()
}

pub(crate) fn hsla_rgb(c: Rgb) -> Hsla {
    hsla(Rgba8::opaque(c))
}

/// The first of `families` that is installed. Each candidate is resolved
/// directly (no enumeration of all system fonts: that loads every font
/// into CoreText); the choice is remembered for the process.
fn pick_family(families: &[SharedString], cx: &App) -> SharedString {
    use std::sync::{Mutex, OnceLock};
    static CHOSEN: OnceLock<Mutex<HashMap<Vec<SharedString>, SharedString>>> = OnceLock::new();
    let memo = CHOSEN.get_or_init(Default::default);
    if let Some(f) = memo.lock().unwrap().get(families) {
        return f.clone();
    }
    let ts = cx.text_system();
    let chosen = families
        .iter()
        .find(|f| {
            let id = ts.resolve_font(&gpui::font((*f).clone()));
            ts.get_font_for_id(id).is_some_and(|got| got.family == **f)
        })
        .or(families.first())
        .cloned()
        .unwrap_or_else(|| "Menlo".into());
    memo.lock().unwrap().insert(families.to_vec(), chosen.clone());
    chosen
}

pub(crate) fn compute_metrics(font: &TermFont, font_size: f32, window: &mut Window, cx: &mut App) -> Metrics {
    let family = pick_family(&font.families, cx);
    let base = Font {
        family,
        features: FontFeatures::disable_ligatures(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: GFontStyle::Normal,
    };
    let fonts = [
        base.clone(),
        Font { weight: FontWeight::BOLD, ..base.clone() },
        Font { style: GFontStyle::Italic, ..base.clone() },
        Font { weight: FontWeight::BOLD, style: GFontStyle::Italic, ..base.clone() },
    ];
    let ts = cx.text_system();
    let id = ts.resolve_font(&base);
    let size_px = px(font_size);
    let cell_w = ts.advance(id, size_px, 'm').map(|s| f32::from(s.width)).unwrap_or(font_size * 0.6);
    let ascent = f32::from(ts.ascent(id, size_px));
    let descent = f32::from(ts.descent(id, size_px)).abs();
    let scale = window.scale_factor();
    let text_h = ascent + descent;
    // Whole device pixels per row, so rows (and box drawing) tile exactly.
    // xterm.js's rule (CharSizeService + RenderDimensions), so rows are as
    // tall as in the web UI: the font's height in whole CSS pixels, rounded
    // up to device pixels, times the line height, floored to device pixels.
    let cell_h = (((text_h.round() * scale).ceil() * font.line_height).floor() / scale).max(1.0 / scale);
    let baseline = snap((cell_h - text_h) / 2.0 + ascent, scale);
    let thickness = (font_size / 14.0).max(1.0 / scale);
    Metrics {
        font_size,
        cell_w,
        cell_h,
        baseline,
        underline_y: snap(baseline + descent * 0.4, scale),
        underline_thickness: snap(thickness, scale).max(1.0 / scale),
        strike_y: snap(baseline - ascent * 0.32, scale),
        fonts,
        primary: id,
    }
}

/// A glyph placed on the grid (x relative to the row's left edge).
#[derive(Clone, Debug)]
struct PlacedGlyph {
    font_id: FontId,
    id: GlyphId,
    x: f32,
    col: u16,
    emoji: bool,
}

#[derive(Clone, Debug)]
struct GlyphRun {
    color: Hsla,
    glyphs: Rc<[PlacedGlyph]>,
}

/// A row ready to paint.
struct ShapedRow {
    runs: RowRuns,
    glyphs: Vec<GlyphRun>,
}

struct CacheEntry {
    row: Rc<ShapedRow>,
    used: u64,
}

/// Counters for tests and benchmarks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderStats {
    pub frames: u64,
    /// Rows built and shaped.
    pub rows_shaped: u64,
    /// Damaged rows whose content turned out unchanged (or moved), reused.
    pub rows_reused: u64,
    /// Rows not damaged at all.
    pub rows_clean: u64,
    /// Text runs shaped (a changed row reuses its unchanged runs).
    pub runs_shaped: u64,
}

/// Per-view render state kept across frames.
#[derive(Default)]
pub(crate) struct RenderCache {
    /// Metrics by (families, size, line height, scale), in hundredths.
    metrics: HashMap<(Vec<SharedString>, u32, u32, u32), Metrics>,
    /// The metrics the cached rows were shaped with.
    shaped_with: Option<(u32, u32)>,
    palette_key: Option<(Box<[Rgb; 256]>, Rgb, Rgb)>,
    /// Viewport row → (generation, content key).
    rows: Vec<Option<(u64, u64)>>,
    by_key: HashMap<u64, CacheEntry>,
    /// Shaped text runs by content (text, style, cells), so a row that
    /// changed in one run (a spinner, a timer) shapes only that run again.
    runs: HashMap<u64, (Rc<[PlacedGlyph]>, u64)>,
    frame: u64,
    pub stats: RenderStats,
}

impl RenderCache {
    fn metrics(&mut self, font: &TermFont, font_size: f32, window: &mut Window, cx: &mut App) -> Metrics {
        let key = (
            font.families.clone(),
            (font_size * 100.0) as u32,
            (font.line_height * 100.0) as u32,
            (window.scale_factor() * 100.0) as u32,
        );
        if self.metrics.len() > 16 {
            self.metrics.clear();
        }
        self.metrics.entry(key).or_insert_with(|| compute_metrics(font, font_size, window, cx)).clone()
    }

    /// Drop shaped rows when the font size or scale they were shaped at changes.
    fn shape_at(&mut self, m: &Metrics, scale: f32) {
        let key = ((m.font_size * 100.0) as u32, (scale * 100.0) as u32);
        if self.shaped_with != Some(key) {
            self.shaped_with = Some(key);
            self.clear_rows();
        }
    }

    pub(crate) fn clear_rows(&mut self) {
        self.rows.clear();
        self.by_key.clear();
        self.runs.clear();
    }
}

/// Where the grid is on screen, for turning mouse positions into cells.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GridLayout {
    pub origin: Point<Pixels>,
    pub bounds: Bounds<Pixels>,
    pub cell_w: f32,
    pub cell_h: f32,
    pub cols: usize,
    pub rows: usize,
    /// First terminal row shown (tiles crop the top).
    pub first_row: usize,
    pub display_offset: usize,
    pub history: usize,
}

impl GridLayout {
    /// The grid cell under a window position (clamped into the grid) and
    /// whether it is in the cell's right half.
    pub fn cell_at(&self, pos: Point<Pixels>) -> (usize, usize, bool) {
        let x = f32::from(pos.x - self.origin.x) / self.cell_w;
        let y = f32::from(pos.y - self.origin.y) / self.cell_h;
        let col = (x.max(0.0) as usize).min(self.cols.saturating_sub(1));
        let row = (y.max(0.0) as usize).min(self.rows.saturating_sub(1));
        (col, row, x - x.floor() >= 0.5)
    }

    pub fn grid_point(&self, col: usize, row: usize) -> GridPoint {
        GridPoint::new(Line(row as i32 + self.first_row as i32 - self.display_offset as i32), Column(col))
    }

    /// The scrollbar's thumb (when there is scrollback).
    pub fn scrollbar(&self) -> Option<(Bounds<Pixels>, Bounds<Pixels>)> {
        if self.history == 0 {
            return None;
        }
        let b = self.bounds;
        let track = Bounds::new(point(b.right() - px(SCROLLBAR_W), b.top()), size(px(SCROLLBAR_W), b.size.height));
        let total = (self.history + self.rows) as f32;
        let h = f32::from(b.size.height);
        let thumb_h = (h * self.rows as f32 / total).max(24.0).min(h);
        let frac = 1.0 - self.display_offset as f32 / self.history as f32;
        let y = (h - thumb_h) * frac;
        let thumb =
            Bounds::new(point(track.left() + px(2.0), b.top() + px(y)), size(px(SCROLLBAR_W - 4.0), px(thumb_h)));
        Some((track, thumb))
    }
}

pub(crate) const SCROLLBAR_W: f32 = 10.0;

/// What the view tells the element about this frame.
#[derive(Clone)]
pub(crate) struct FrameParams {
    pub mode: ViewMode,
    pub font: TermFont,
    pub focused: bool,
    pub cursor_blink_on: bool,
    pub preedit: Option<String>,
    pub link: Option<(GridPoint, GridPoint)>,
    pub search_regex: Option<Rc<RefCell<alacritty_terminal::term::search::RegexSearch>>>,
    pub search_current: Option<(GridPoint, GridPoint)>,
    /// Keep room for the scrollbar (always, when it is enabled: the column
    /// count must not change when the thumb appears).
    pub reserve_scrollbar: bool,
    pub show_scrollbar: bool,
    /// Space around the grid: top, right, bottom, left.
    pub padding: [f32; 4],
}

pub(crate) struct TermElement {
    pub view: Option<Entity<TerminalView>>,
    pub terminal: Terminal,
    pub cache: Rc<RefCell<RenderCache>>,
    pub layout: Rc<RefCell<GridLayout>>,
    pub params: FrameParams,
}

struct CursorPaint {
    row: usize,
    col: usize,
    width: u8,
    shape: CursorShape,
    color: Hsla,
    accent: Hsla,
}

pub(crate) struct Prepaint {
    hitbox: Hitbox,
    metrics: Metrics,
    theme: TermTheme,
    origin: Point<Pixels>,
    rows: Vec<Rc<ShapedRow>>,
    /// Viewport rows to draw (tiles skip cropped ones).
    first_row: usize,
    cursor: Option<CursorPaint>,
    selection: Vec<(usize, usize, usize)>,
    matches: Vec<(usize, usize, usize, bool)>,
    link: Vec<(usize, usize, usize)>,
    background: Hsla,
    scrollbar: Option<Bounds<Pixels>>,
    scale: f32,
}

impl IntoElement for TermElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

/// Row spans covered by a range of grid points: `(viewport row, start col, end col exclusive)`.
fn spans(
    start: GridPoint,
    end: GridPoint,
    block: bool,
    display_offset: usize,
    rows: usize,
    cols: usize,
) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    let off = display_offset as i32;
    for line in start.line.0..=end.line.0 {
        let v = line + off;
        if v < 0 || v as usize >= rows {
            continue;
        }
        let (s, e) = if block {
            (start.column.0, end.column.0 + 1)
        } else {
            let s = if line == start.line.0 { start.column.0 } else { 0 };
            let e = if line == end.line.0 { end.column.0 + 1 } else { cols };
            (s, e)
        };
        if e > s {
            out.push((v as usize, s, e.min(cols)));
        }
    }
    out
}

impl Element for TermElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        style.flex_grow = 1.0;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let scale = window.scale_factor();
        let theme = self.terminal.theme();
        let [pad_t, pad_r, pad_b, pad_l] = self.params.padding;
        let inner_w = (f32::from(bounds.size.width) - pad_l - pad_r).max(1.0);
        let inner_h = (f32::from(bounds.size.height) - pad_t - pad_b).max(1.0);
        let mut cache = self.cache.borrow_mut();
        let base = cache.metrics(&self.params.font, self.params.font.size, window, cx);

        // Size: interactive views size the terminal to fit; tiles show it at
        // its own size, scaled to the tile width, bottom rows first.
        let metrics = match self.params.mode {
            ViewMode::Interactive => {
                let usable_w = inner_w - if self.params.reserve_scrollbar { SCROLLBAR_W } else { 0.0 };
                let cols = (usable_w / base.cell_w).floor().max(1.0) as u16;
                let rows = (inner_h / base.cell_h).floor().max(1.0) as u16;
                self.terminal.resize(TermSize { cols, rows, cell_width: base.cell_w, cell_height: base.cell_h });
                base
            }
            ViewMode::Tile { min_scale } => {
                let size = self.terminal.size();
                let natural_w = base.cell_w * size.cols as f32;
                let s = (inner_w / natural_w).clamp(min_scale, 1.0);
                // Quantised so tiles share glyph atlas entries.
                let fs = ((self.params.font.size * s) * 4.0).round() / 4.0;
                if (fs - self.params.font.size).abs() < 0.01 {
                    base
                } else {
                    cache.metrics(&self.params.font, fs, window, cx)
                }
            }
        };

        cache.shape_at(&metrics, scale);
        let origin = point(bounds.origin.x + px(pad_l), bounds.origin.y + px(pad_t));
        let mut st = self.terminal.lock();
        let gens = st.line_gens().to_vec();
        let term = st.term();
        let rows = term.screen_lines();
        let cols = term.columns();
        let display_offset = term.grid().display_offset();
        let history = term.history_size();
        let pal = Palette::new(&theme, Some(term.colors()));
        let pkey = (Box::new(pal.colors), pal.foreground, pal.background);
        if cache.palette_key.as_ref() != Some(&pkey) {
            cache.clear_rows();
            cache.palette_key = Some(pkey);
        }
        let visible_rows = ((inner_h / metrics.cell_h).floor() as usize).max(1);
        let first_row = rows.saturating_sub(visible_rows);
        if cache.rows.len() != rows {
            cache.rows = vec![None; rows];
        }
        cache.frame += 1;
        cache.stats.frames += 1;
        let frame = cache.frame;

        let mut shaped_rows = Vec::with_capacity(rows);
        for (v, &gen) in gens.iter().enumerate().take(rows) {
            let line = Line(v as i32 - display_offset as i32);
            let cached = cache.rows[v];
            let key = match cached {
                Some((g, k)) if g == gen && cache.by_key.contains_key(&k) => {
                    cache.stats.rows_clean += 1;
                    k
                }
                _ => {
                    let cells = &term.grid()[line][..];
                    let k = runs::row_key(cells);
                    if !cache.by_key.contains_key(&k) {
                        let built = runs::build_row(cells, &pal);
                        let cache = &mut *cache;
                        let glyphs = shape_row(&built, &metrics, &mut cache.runs, &mut cache.stats, frame, window);
                        cache.by_key.insert(k, CacheEntry { row: Rc::new(ShapedRow { runs: built, glyphs }), used: frame });
                        cache.stats.rows_shaped += 1;
                    } else {
                        cache.stats.rows_reused += 1;
                    }
                    k
                }
            };
            cache.rows[v] = Some((gen, key));
            let entry = cache.by_key.get_mut(&key).expect("row");
            entry.used = frame;
            shaped_rows.push(entry.row.clone());
        }
        // Keep what the last two frames used.
        cache.by_key.retain(|_, e| e.used + 1 >= frame);
        // Runs a little longer: a spinner's frames come round again.
        if cache.runs.len() > 4 * rows.max(16) {
            cache.runs.retain(|_, e| e.1 + 8 >= frame);
        }

        // Cursor.
        let mode = *term.mode();
        let interactive = matches!(self.params.mode, ViewMode::Interactive);
        let content = term.renderable_content();
        let cpoint = content.cursor.point;
        let cv = cpoint.line.0 + display_offset as i32;
        let mut cursor = None;
        let show = mode.contains(TermMode::SHOW_CURSOR) && cv >= 0 && (cv as usize) < rows;
        if show {
            let mut shape = term.cursor_style().shape;
            let blinking = term.cursor_style().blinking || interactive;
            if !self.params.focused {
                shape = CursorShape::HollowBlock;
            } else if blinking && !self.params.cursor_blink_on {
                shape = CursorShape::Hidden;
            }
            if self.params.preedit.is_some() {
                shape = CursorShape::Hidden;
            }
            let cell = &term.grid()[cpoint];
            let width = if cell.flags.contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR) { 2 } else { 1 };
            if shape != CursorShape::Hidden {
                cursor = Some(CursorPaint {
                    row: cv as usize,
                    col: cpoint.column.0,
                    width,
                    shape,
                    color: hsla_rgb(pal.cursor),
                    accent: hsla_rgb(theme.cursor_accent),
                });
            }
        }

        let selection = content
            .selection
            .map(|SelectionRange { start, end, is_block }| spans(start, end, is_block, display_offset, rows, cols))
            .unwrap_or_default();

        // Search matches in view.
        let mut matches = Vec::new();
        if let Some(regex) = &self.params.search_regex {
            let top = GridPoint::new(Line(-(display_offset as i32)), Column(0));
            let bottom = GridPoint::new(Line(rows as i32 - 1 - display_offset as i32), Column(cols - 1));
            let mut regex = regex.borrow_mut();
            let iter = alacritty_terminal::term::search::RegexIter::new(
                top,
                bottom,
                alacritty_terminal::index::Direction::Right,
                term,
                &mut regex,
            );
            for m in iter.take(500) {
                let current = self.params.search_current == Some((*m.start(), *m.end()));
                for (v, s, e) in spans(*m.start(), *m.end(), false, display_offset, rows, cols) {
                    matches.push((v, s, e, current));
                }
            }
        }
        let link = self.params.link.map(|(s, e)| spans(s, e, false, display_offset, rows, cols)).unwrap_or_default();
        drop(st);

        let layout = GridLayout {
            origin,
            bounds,
            cell_w: metrics.cell_w,
            cell_h: metrics.cell_h,
            cols,
            rows: rows.min(visible_rows),
            first_row,
            display_offset,
            history,
        };
        *self.layout.borrow_mut() = layout;
        let scrollbar =
            if self.params.show_scrollbar && interactive { layout.scrollbar().map(|(_, thumb)| thumb) } else { None };

        Prepaint {
            hitbox,
            background: hsla_rgb(pal.background),
            metrics,
            theme,
            origin,
            rows: shaped_rows,
            first_row,
            cursor,
            selection,
            matches,
            link,
            scrollbar,
            scale,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        p: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(view) = &self.view {
            let focus = view.read(cx).focus_ref().clone();
            window.handle_input(&focus, crate::view::TermInputHandler { view: view.clone() }, cx);
            if p.hitbox.is_hovered(window) {
                let style = view.read(cx).mouse_cursor_style();
                window.set_cursor_style(style, &p.hitbox);
            }
        }
        let m = &p.metrics;
        let s = p.scale;
        let ox = f32::from(p.origin.x);
        let oy = f32::from(p.origin.y);
        let col_x = |c: usize| snap(ox + c as f32 * m.cell_w, s);
        let row_y = |r: usize| oy + (r - p.first_row.min(r)) as f32 * m.cell_h;
        let visible = p.first_row..p.rows.len();

        // One layer for the whole grid: its primitives share a draw order, so
        // GPUI doesn't sort tens of thousands of glyphs into its bounds tree.
        // Within a layer quads draw before glyphs, which is the order needed
        // (backgrounds, selection, block cursor, then text).
        window.paint_layer(bounds, |window| {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                window.paint_quad(fill(bounds, p.background));

                // Backgrounds.
                for r in visible.clone() {
                    let y = row_y(r);
                    for bg in &p.rows[r].runs.backgrounds {
                        let x0 = col_x(bg.start as usize);
                        let x1 = col_x(bg.end as usize);
                        window.paint_quad(fill(
                            Bounds::new(point(px(x0), px(y)), size(px(x1 - x0), px(m.cell_h))),
                            hsla_rgb(bg.color),
                        ));
                    }
                }
                let band = |window: &mut Window, r: usize, c0: usize, c1: usize, color: Hsla| {
                    if r < p.first_row {
                        return;
                    }
                    let (x0, x1) = (col_x(c0), col_x(c1));
                    window.paint_quad(fill(
                        Bounds::new(point(px(x0), px(row_y(r))), size(px(x1 - x0), px(m.cell_h))),
                        color,
                    ));
                };
                for &(r, c0, c1, current) in &p.matches {
                    let c = if current { p.theme.search_current } else { p.theme.search_match };
                    band(window, r, c0, c1, hsla_rgb(c));
                }
                for &(r, c0, c1) in &p.selection {
                    band(window, r, c0, c1, hsla_rgb(p.theme.selection_background));
                }

                // Block cursor (text on it is drawn in the accent colour below).
                let block_cursor = p.cursor.as_ref().filter(|c| c.shape == CursorShape::Block);
                if let Some(c) = block_cursor {
                    band(window, c.row, c.col, c.col + c.width as usize, c.color);
                }

                // Text.
                for r in visible.clone() {
                    let baseline = row_y(r) + m.baseline;
                    for run in &p.rows[r].glyphs {
                        for g in run.glyphs.iter() {
                            let color = match block_cursor {
                                Some(c) if c.row == r && c.col == g.col as usize => c.accent,
                                _ => run.color,
                            };
                            let at = point(px(ox + g.x), px(baseline));
                            let _ = if g.emoji {
                                window.paint_emoji(at, g.font_id, g.id, px(m.font_size))
                            } else {
                                window.paint_glyph(at, g.font_id, g.id, px(m.font_size), color)
                            };
                        }
                    }
                    // Box drawing and blocks.
                    let light = ((m.cell_w * s / 8.0).round() as i32).max(1);
                    for b in &p.rows[r].runs.boxes {
                        let x0d = (col_x(b.col as usize) * s).round() as i32;
                        let x1d = (col_x(b.col as usize + 1) * s).round() as i32;
                        let y0d = (row_y(r) * s).round() as i32;
                        let y1d = ((row_y(r) + m.cell_h) * s).round() as i32;
                        let color = match block_cursor {
                            Some(c) if c.row == r && c.col == b.col as usize => c.accent,
                            _ => hsla(b.color),
                        };
                        paint_box(window, b.ch, x0d, y0d, x1d - x0d, y1d - y0d, light, s, color);
                    }
                    // Decorations.
                    for d in &p.rows[r].runs.decorations {
                        paint_decoration(window, d, col_x(d.start as usize), col_x(d.end as usize), row_y(r), m, s);
                    }
                }
                for &(r, c0, c1) in &p.link {
                    if r >= p.first_row {
                        let y = row_y(r) + m.underline_y;
                        window.paint_quad(fill(
                            Bounds::new(
                                point(px(col_x(c0)), px(y)),
                                size(px(col_x(c1) - col_x(c0)), px(m.underline_thickness)),
                            ),
                            hsla_rgb(p.theme.foreground),
                        ));
                    }
                }

                // Other cursor shapes.
                if let Some(c) = p.cursor.as_ref().filter(|c| c.shape != CursorShape::Block) {
                    if c.row >= p.first_row {
                        let x0 = col_x(c.col);
                        let x1 = col_x(c.col + c.width as usize);
                        let y = row_y(c.row);
                        let t = snap((m.cell_w / 7.0).max(1.0), s).max(1.0 / s);
                        let b = match c.shape {
                            CursorShape::Beam => Bounds::new(point(px(x0), px(y)), size(px(t * 2.0), px(m.cell_h))),
                            CursorShape::Underline => {
                                Bounds::new(point(px(x0), px(y + m.cell_h - t * 2.0)), size(px(x1 - x0), px(t * 2.0)))
                            }
                            _ => Bounds::new(point(px(x0), px(y)), size(px(x1 - x0), px(m.cell_h))),
                        };
                        if c.shape == CursorShape::HollowBlock {
                            window.paint_quad(quad(
                                b,
                                Corners::default(),
                                gpui::transparent_black(),
                                Edges::all(px(1.0)),
                                c.color,
                                BorderStyle::Solid,
                            ));
                        } else {
                            window.paint_quad(fill(b, c.color));
                        }
                    }
                }

                if let Some(thumb) = p.scrollbar {
                    let mut c = hsla_rgb(p.theme.foreground);
                    c.a = 0.35;
                    window.paint_quad(quad(thumb, Corners::all(px(3.0)), c, Edges::default(), c, BorderStyle::Solid));
                }
            })
        });

        // IME composition at the cursor.
        if let (Some(text), Some(_)) = (self.params.preedit.as_ref(), &self.view) {
            let layout = *self.layout.borrow();
            let st = self.terminal.lock();
            let cpoint = st.term().grid().cursor.point;
            drop(st);
            let r = (cpoint.line.0 + layout.display_offset as i32).max(0) as usize;
            let x = col_x(cpoint.column.0);
            let y = row_y(r);
            let run = gpui::TextRun {
                len: text.len(),
                font: m.font(FontStyle::default()).clone(),
                color: hsla_rgb(p.theme.foreground),
                background_color: Some(hsla_rgb(p.theme.background)),
                underline: Some(UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(hsla_rgb(p.theme.foreground)),
                    wavy: false,
                }),
                strikethrough: None,
            };
            let line = window.text_system().shape_line(text.clone().into(), px(m.font_size), &[run], None);
            let _ = line.paint_background(point(px(x), px(y)), px(m.cell_h), window, cx);
            let _ = line.paint(point(px(x), px(y)), px(m.cell_h), window, cx);
        }
    }
}

/// A text run's identity for shaping: its text, font style and cells
/// (colour is not part of it: glyphs don't depend on it).
fn run_key(run: &runs::TextRun) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    run.text.hash(&mut h);
    run.style.hash(&mut h);
    run.cells.hash(&mut h);
    h.finish()
}

/// Shape a row's text runs and place every glyph on its cell. Runs shaped
/// before (in `shaped`, stamped with the frame that last used them) are
/// reused.
fn shape_row(
    row: &RowRuns,
    m: &Metrics,
    shaped: &mut HashMap<u64, (Rc<[PlacedGlyph]>, u64)>,
    stats: &mut RenderStats,
    frame: u64,
    window: &mut Window,
) -> Vec<GlyphRun> {
    let mut out = Vec::with_capacity(row.texts.len());
    for run in &row.texts {
        let color = hsla(run.color);
        let key = run_key(run);
        if let Some((glyphs, used)) = shaped.get_mut(&key) {
            *used = frame;
            out.push(GlyphRun { color, glyphs: glyphs.clone() });
            continue;
        }
        let glyphs: Rc<[PlacedGlyph]> = shape_run(run, m, window).into();
        stats.runs_shaped += 1;
        shaped.insert(key, (glyphs.clone(), frame));
        out.push(GlyphRun { color, glyphs });
    }
    out
}

/// Shape one text run, every glyph on its cell.
fn shape_run(run: &runs::TextRun, m: &Metrics, window: &mut Window) -> Vec<PlacedGlyph> {
    {
        let color = hsla(run.color);
        let font = m.font(run.style).clone();
        let truns = [gpui::TextRun {
            len: run.text.len(),
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let shaped =
            window.text_system().shape_line(SharedString::from(run.text.clone()), px(m.font_size), &truns, None);
        // First glyph x of each cell, and the x where the next cell starts.
        let mut glyphs = Vec::with_capacity(run.text.len());
        let all: Vec<_> = shaped.runs.iter().flat_map(|r| r.glyphs.iter().map(move |g| (r.font_id, g))).collect();
        let width = f32::from(shaped.width);
        let mut i = 0;
        while i < all.len() {
            let (col, w) = run.cell_at(all[i].1.index);
            // Glyphs of the same cell (a cluster).
            let mut j = i + 1;
            while j < all.len() && run.cell_at(all[j].1.index).0 == col {
                j += 1;
            }
            let base_x = f32::from(all[i].1.position.x);
            let next_x = if j < all.len() { f32::from(all[j].1.position.x) } else { width };
            let advance = next_x - base_x;
            let span = w as f32 * m.cell_w;
            let foreign = all[i].0 != m.primary || all[i].1.is_emoji;
            // Glyphs from another font (Georgian, CJK, emoji) are centred on
            // their cells; the primary font's advance is the cell width.
            let shift = if foreign && advance > 0.0 { (span - advance) / 2.0 } else { 0.0 };
            let cell_x = col as f32 * m.cell_w;
            for &(font_id, g) in &all[i..j] {
                glyphs.push(PlacedGlyph {
                    font_id,
                    id: g.id,
                    x: cell_x + shift + (f32::from(g.position.x) - base_x),
                    col,
                    emoji: g.is_emoji,
                });
            }
            i = j;
        }
        glyphs
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_box(window: &mut Window, ch: char, x: i32, y: i32, w: i32, h: i32, light: i32, s: f32, color: Hsla) {
    let Some(prims) = boxdraw::prims(ch, w, h, light) else { return };
    let to = |v: i32| px(v as f32 / s);
    for prim in prims {
        match prim {
            Prim::Rect { x0, y0, x1, y1, alpha } => {
                let mut c = color;
                c.a *= alpha;
                window.paint_quad(fill(Bounds::new(point(to(x + x0), to(y + y0)), size(to(x1 - x0), to(y1 - y0))), c));
            }
            Prim::Arc { x0, y0, x1, y1, corner, thickness, radius } => {
                let cell = Bounds::new(point(to(x), to(y)), size(to(w), to(h)));
                let b = Bounds::new(point(to(x + x0), to(y + y0)), size(to(x1 - x0), to(y1 - y0)));
                let r = to(radius);
                let t = to(thickness);
                let (radii, edges) = match corner {
                    Corner::TopLeft => {
                        (Corners { top_left: r, ..Default::default() }, Edges { top: t, left: t, ..Default::default() })
                    }
                    Corner::TopRight => (
                        Corners { top_right: r, ..Default::default() },
                        Edges { top: t, right: t, ..Default::default() },
                    ),
                    Corner::BottomLeft => (
                        Corners { bottom_left: r, ..Default::default() },
                        Edges { bottom: t, left: t, ..Default::default() },
                    ),
                    Corner::BottomRight => (
                        Corners { bottom_right: r, ..Default::default() },
                        Edges { bottom: t, right: t, ..Default::default() },
                    ),
                };
                window.with_content_mask(Some(ContentMask { bounds: cell }), |window| {
                    window.paint_quad(quad(b, radii, gpui::transparent_black(), edges, color, BorderStyle::Solid));
                });
            }
        }
    }
}

fn paint_decoration(window: &mut Window, d: &runs::Decoration, x0: f32, x1: f32, row_y: f32, m: &Metrics, s: f32) {
    let color = hsla(d.color);
    let t = m.underline_thickness;
    let w = x1 - x0;
    let y = row_y + m.underline_y;
    match d.kind {
        LineKind::Strike => {
            window.paint_strikethrough(
                point(px(x0), px(row_y + m.strike_y)),
                px(w),
                &StrikethroughStyle { thickness: px(t), color: Some(color) },
            );
        }
        LineKind::Underline => {
            window.paint_underline(
                point(px(x0), px(y)),
                px(w),
                &UnderlineStyle { thickness: px(t), color: Some(color), wavy: false },
            );
        }
        LineKind::Double => {
            for dy in [0.0, t * 2.0] {
                window.paint_quad(fill(Bounds::new(point(px(x0), px(y + dy)), size(px(w), px(t))), color));
            }
        }
        LineKind::Curly => {
            window.paint_underline(
                point(px(x0), px(y)),
                px(w),
                &UnderlineStyle { thickness: px(t), color: Some(color), wavy: true },
            );
        }
        LineKind::Dotted | LineKind::Dashed => {
            let (on, off) = if d.kind == LineKind::Dotted { (t, t) } else { (t * 3.0, t * 2.0) };
            let mut x = x0;
            while x < x1 {
                let seg = on.min(x1 - x);
                window.paint_quad(fill(Bounds::new(point(px(snap(x, s)), px(y)), size(px(seg), px(t))), color));
                x += on + off;
            }
        }
    }
}
