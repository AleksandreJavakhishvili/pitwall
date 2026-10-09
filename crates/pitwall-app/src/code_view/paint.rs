//! Drawing the code view and handling the pointer: geometry (panes,
//! margin, ruler, scrollbars), one display list per frame built from the
//! visible rows only, hit testing, and the overlays (Find, the context
//! menu, the comment hover and the read-only hint).

use crate::kit::HoverText as _;
use std::ops::Range;

use gpui::{
    anchored, canvas, deferred, div, fill, pattern_slash, point, prelude::*, px, quad, size,
    AnyElement, App, Bounds, Context, Corner, CursorStyle, DispatchPhase, Entity, FontWeight,
    Hitbox, HitboxBehavior, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PaintQuad, Pixels, Point, ScrollWheelEvent, ShapedLine, SharedString, TextRun, Window,
};

use super::rows::{Layout, Row, Side, FOLD_H, LINE_H};
use super::style::{mono_font, Colors, FONT_SIZE, LINE_HEIGHT};
use super::text::{word_at, Pos};
use super::{CodeView, CodeViewEvent, Drag, Menu, Selection};
use crate::theme::Theme;

/// Room kept above the first line while Find is open (Monaco's zone).
pub(super) const FIND_ZONE: f32 = 33.;
/// Extra horizontal room past the longest line.
pub(super) const CHAR_SLACK: f32 = 30.;
const RULER_W: f32 = 30.;
const VBAR_W: f32 = 14.;
const HBAR_H: f32 = 10.;
const MIN_THUMB: f32 = 20.;
const NUM_W: f32 = 38.;
const GLYPH_W: f32 = 19.;
const SIGN_W: f32 = 10.;
const TEXT_PAD: f32 = 2.;
const FIND_W: f32 = 419.;

#[derive(Debug, Clone)]
pub(super) struct PaneGeom {
    /// The version this pane shows (inline and file panes: `New`, with
    /// deleted rows from `Old`).
    pub side: Side,
    pub bounds: Bounds<Pixels>,
    pub unified: bool,
}

impl PaneGeom {
    fn gutter_w(&self) -> Pixels {
        px(if self.unified {
            NUM_W * 2. + GLYPH_W + SIGN_W
        } else {
            NUM_W + GLYPH_W + SIGN_W
        })
    }

    fn text_left(&self) -> Pixels {
        self.bounds.left() + self.gutter_w() + px(TEXT_PAD)
    }

    fn text_bounds(&self) -> Bounds<Pixels> {
        let l = self.bounds.left() + self.gutter_w();
        Bounds::from_corners(point(l, self.bounds.top()), self.bounds.bottom_right())
    }

    fn gutter_bounds(&self) -> Bounds<Pixels> {
        Bounds::new(
            self.bounds.origin,
            size(self.gutter_w(), self.bounds.size.height),
        )
    }

    /// Glyph column (the comment margin), relative to the pane.
    fn glyph_x(&self) -> Pixels {
        px(if self.unified { NUM_W } else { 0. })
    }

    fn num_right(&self) -> Pixels {
        self.glyph_x() + px(GLYPH_W + NUM_W)
    }
}

#[derive(Debug, Clone)]
pub(super) struct Geom {
    pub bounds: Bounds<Pixels>,
    pub panes: Vec<PaneGeom>,
    pub ruler: Bounds<Pixels>,
}

impl Geom {
    pub fn text_h(&self) -> Pixels {
        self.bounds.size.height
    }

    pub fn text_w(&self) -> Pixels {
        self.panes
            .iter()
            .map(|p| p.text_bounds().size.width - px(TEXT_PAD))
            .fold(px(f32::MAX), |a, b| a.min(b))
            .max(px(0.))
    }

    fn pane_for(&self, side: Side) -> Option<&PaneGeom> {
        self.panes
            .iter()
            .find(|p| p.side == side)
            .or_else(|| self.panes.first())
    }

    /// The caret's window position (for the read-only hint).
    pub fn caret_point(&self, v: &CodeView, s: Selection) -> Option<Point<Pixels>> {
        let pane = self.pane_for(s.side)?;
        let row = v.rows.row_of(s.side, s.head.line)?;
        let y = self.bounds.top() + v.top_pad() + px(v.rows.top(row)) - v.scroll_y;
        Some(point(pane.text_left() - v.scroll_x + px(8.), y))
    }

    fn vbar(&self) -> Bounds<Pixels> {
        let last = self.panes.last().map_or(self.bounds, |p| p.bounds);
        Bounds::new(
            point(last.right() - px(VBAR_W), self.bounds.top()),
            size(px(VBAR_W), self.bounds.size.height),
        )
    }

    fn hbar(&self, pane: &PaneGeom) -> Bounds<Pixels> {
        let t = pane.text_bounds();
        Bounds::new(
            point(t.left(), t.bottom() - px(HBAR_H)),
            size((t.size.width - px(VBAR_W)).max(px(0.)), px(HBAR_H)),
        )
    }
}

fn geometry(v: &CodeView, bounds: Bounds<Pixels>) -> Geom {
    let ruler = Bounds::new(
        point(bounds.right() - px(RULER_W), bounds.top()),
        size(px(RULER_W), bounds.size.height),
    );
    let area = Bounds::from_corners(bounds.origin, point(ruler.left(), bounds.bottom()));
    let is_diff = v.content.as_ref().is_some_and(|c| c.is_diff);
    let panes = if is_diff && v.layout == Layout::Split {
        let half = area.size.width / 2.;
        vec![
            PaneGeom {
                side: Side::Old,
                bounds: Bounds::new(area.origin, size(half, area.size.height)),
                unified: false,
            },
            PaneGeom {
                side: Side::New,
                bounds: Bounds::new(
                    point(area.left() + half, area.top()),
                    size(area.size.width - half, area.size.height),
                ),
                unified: false,
            },
        ]
    } else {
        vec![PaneGeom {
            side: Side::New,
            bounds: area,
            unified: is_diff,
        }]
    };
    Geom {
        bounds,
        panes,
        ruler,
    }
}

// ── text measuring ──────────────────────────────────────────────────────────

fn plain_run(len: usize, color: Hsla) -> TextRun {
    TextRun {
        len,
        font: mono_font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

fn shape_plain(window: &mut Window, text: &SharedString) -> ShapedLine {
    window.text_system().shape_line(
        text.clone(),
        FONT_SIZE,
        &[plain_run(text.len(), gpui::black())],
        None,
    )
}

/// Where column `col` of a line is drawn (from the text's left edge).
pub(super) fn x_for(window: &mut Window, text: &SharedString, col: usize) -> Pixels {
    shape_plain(window, text).x_for_index(col)
}

/// The column nearest to `x` (from the text's left edge).
pub(super) fn col_for(window: &mut Window, text: &SharedString, x: Pixels) -> usize {
    if x <= px(0.) {
        return 0;
    }
    shape_plain(window, text).closest_index_for_x(x)
}

fn shape_ui(window: &mut Window, text: &str, size_px: f32, color: Hsla) -> ShapedLine {
    let font = gpui::Font {
        // Monaco's widgets use the system font (review.css `--rv-sys`).
        family: ".SystemUIFont".into(),
        features: Default::default(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: gpui::FontStyle::Normal,
    };
    let text: SharedString = text.to_string().into();
    window.text_system().shape_line(
        text.clone(),
        px(size_px),
        &[TextRun {
            len: text.len(),
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    )
}

// ── the display list ────────────────────────────────────────────────────────

#[derive(Default)]
struct Layer {
    mask: Option<Bounds<Pixels>>,
    under: Vec<PaintQuad>,
    lines: Vec<(ShapedLine, Point<Pixels>)>,
    over: Vec<PaintQuad>,
}

/// Areas and the pointer each shows (the last that contains it wins).
type Zones = Vec<(Bounds<Pixels>, CursorStyle)>;

#[derive(Default)]
pub(super) struct Frame {
    layers: Vec<Layer>,
    cursor: Option<(Bounds<Pixels>, CursorStyle, Zones)>,
}

impl Frame {
    fn paint(self, hitbox: &Hitbox, window: &mut Window, cx: &mut App) {
        for layer in self.layers {
            let draw = |window: &mut Window, cx: &mut App| {
                for q in layer.under {
                    window.paint_quad(q);
                }
                for (line, origin) in &layer.lines {
                    let _ = line.paint(*origin, LINE_HEIGHT, window, cx);
                }
                for q in layer.over {
                    window.paint_quad(q);
                }
            };
            match layer.mask {
                Some(bounds) => {
                    window.with_content_mask(Some(gpui::ContentMask { bounds }), |w| draw(w, cx))
                }
                None => draw(window, cx),
            }
        }
        // The pointer: a text cursor over text, a hand over what is clickable.
        if let Some((_, default, zones)) = self.cursor {
            let m = window.mouse_position();
            let style = zones
                .iter()
                .find(|(b, _)| b.contains(&m))
                .map_or(default, |(_, s)| *s);
            window.set_cursor_style(style, hitbox);
        }
    }
}

fn col_x(line: &ShapedLine, col: usize) -> Pixels {
    line.x_for_index(col.min(line.len()))
}

impl CodeView {
    /// Lay out the visible rows and build what to paint.
    fn prepare(
        &mut self,
        bounds: Bounds<Pixels>,
        c: &Colors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Frame {
        let geom = geometry(self, bounds);
        self.geom = Some(geom.clone());
        self.apply_pending_reveal();
        self.clamp_scroll();
        let mut frame = Frame::default();
        let Some(content) = self.content.clone() else {
            return frame;
        };
        let focused = self.focus.is_focused(window);
        let top_pad = self.top_pad();
        let first = self.rows.row_at(f32::from(self.scroll_y - top_pad));
        let last = self
            .rows
            .row_at(f32::from(self.scroll_y - top_pad + bounds.size.height));
        let row_y = |r: usize| bounds.top() + top_pad + px(self.rows.top(r)) - self.scroll_y;
        let char_w = shape_plain(window, &SharedString::from("0000000000")).width / 10.;
        let mut widest = self.max_width;
        let mut zones: Vec<(Bounds<Pixels>, CursorStyle)> = Vec::new();
        // The caret's bracket pair and indent block (an empty selection).
        let caret = self.sel.filter(|s| s.is_empty());
        let plain = super::highlight::Lang::for_path(&content.path).is_none();
        let bracket_marks: Vec<(Side, super::text::Pos)> = caret
            .map(|s| {
                super::brackets::matching(
                    content.doc(s.side),
                    |l| content.spans(s.side, l),
                    plain,
                    s.head,
                )
                .into_iter()
                .map(|p| (s.side, p))
                .collect()
            })
            .unwrap_or_default();
        let active_guide = caret.and_then(|s| {
            super::brackets::active_guide(content.doc(s.side), s.head.line)
                .map(|(k, lines)| (s.side, k, lines))
        });

        for pane in &geom.panes {
            let tb = pane.text_bounds();
            let gb = pane.gutter_bounds();
            let text_x = pane.text_left() - self.scroll_x;
            let mut text = Layer {
                mask: Some(tb),
                ..Layer::default()
            };
            let mut gutter = Layer {
                mask: Some(gb),
                ..Layer::default()
            };
            let mut labels = Layer {
                mask: Some(pane.bounds),
                ..Layer::default()
            };
            zones.push((tb, CursorStyle::IBeam));
            if self.commentable && pane.side == Side::New {
                zones.push((gb, CursorStyle::PointingHand));
            }
            for r in first..=last.min(self.rows.len().saturating_sub(1)) {
                let Some(row) = self.rows.rows.get(r) else {
                    break;
                };
                let y = row_y(r);
                match row {
                    Row::Fold { hidden, .. } => {
                        let bar = Bounds::new(
                            point(pane.bounds.left(), y),
                            size(pane.bounds.size.width, px(FOLD_H - 4.)),
                        );
                        labels.under.push(fill(bar, c.fold_bg));
                        // Inset shadows top and bottom (Monaco's look).
                        labels.under.push(fill(
                            Bounds::new(bar.origin, size(bar.size.width, px(1.))),
                            c.widget_border,
                        ));
                        labels.under.push(fill(
                            Bounds::new(
                                point(bar.left(), bar.bottom() - px(1.)),
                                size(bar.size.width, px(1.)),
                            ),
                            c.widget_border,
                        ));
                        let label = shape_ui(
                            window,
                            &format!(
                                "{hidden} hidden line{}",
                                if *hidden == 1 { "" } else { "s" }
                            ),
                            13.,
                            c.fold_text,
                        );
                        let lx = pane.bounds.left() + (pane.bounds.size.width - label.width) / 2.;
                        labels.lines.push((label, point(lx, y + px(1.))));
                        let icon = shape_ui(window, "⇕", 13., c.fold_text);
                        let ix = pane.bounds.left() + pane.glyph_x() + px(GLYPH_W + 8.);
                        labels.lines.push((icon, point(ix, y + px(1.))));
                        zones.push((bar, CursorStyle::PointingHand));
                    }
                    Row::Line { old, new, changed } => {
                        // Which version's line this pane shows on this row.
                        let at: Option<(Side, usize)> = if pane.unified || !content.is_diff {
                            new.map(|l| (Side::New, l)).or(old.map(|l| (Side::Old, l)))
                        } else {
                            row.line(pane.side).map(|l| (pane.side, l))
                        };
                        let row_b =
                            Bounds::new(point(tb.left(), y), size(tb.size.width, px(LINE_H)));
                        let Some((side, line)) = at else {
                            // Filler opposite inserted/deleted lines.
                            text.under.push(fill(row_b, pattern_slash(c.fill, 2., 6.)));
                            continue;
                        };
                        let doc = content.doc(side);
                        let Some(l) = doc.line(line) else { continue };
                        let marks = if *changed {
                            match side {
                                Side::Old => content.diff.old_marks.get(&line),
                                Side::New => content.diff.new_marks.get(&line),
                            }
                        } else {
                            None
                        };
                        let (line_tint, char_tint, gut_tint) = match side {
                            Side::Old => (c.del_line, c.del_char, c.del_gutter),
                            Side::New => (c.ins_line, c.ins_char, c.ins_gutter),
                        };
                        // A brand new or deleted file: everything is changed.
                        let whole = content.is_diff
                            && (content.old_text.is_empty() != content.new_text.is_empty());
                        let changed = *changed || whole;
                        if changed {
                            text.under.push(fill(row_b, line_tint));
                            if marks.is_some_and(|m| m.full) && !whole {
                                text.under.push(fill(row_b, char_tint));
                            }
                            // The empty side of a new or deleted file: one
                            // 3 px bar where the other side's text goes
                            // (`.rv-empty-range`).
                            let empty = match side {
                                Side::Old => content.diff.empty_old,
                                Side::New => content.diff.empty_new,
                            };
                            if empty && line == 0 {
                                text.under.push(fill(
                                    Bounds::new(point(text_x, y), size(px(3.), px(LINE_H))),
                                    char_tint,
                                ));
                            }
                        }
                        if side == Side::New
                            && self.comments.contains_key(&line)
                            && self.commentable
                        {
                            text.under.push(fill(row_b, c.commented));
                        }
                        // The shaped line, coloured by the highlighter.
                        let shaped = shape_colored(window, l, content.spans(side, line), c);
                        widest = widest.max(shaped.width);
                        // Changed characters.
                        if let Some(m) = marks.filter(|m| !m.full) {
                            for rg in &m.ranges {
                                let (a, b) = (l.from_source(rg.start), l.from_source(rg.end));
                                let x0 = text_x + col_x(&shaped, a);
                                let x1 = text_x + col_x(&shaped, b);
                                // An insertion on the other side: a 3 px bar.
                                let w = (x1 - x0).max(px(3.));
                                text.under.push(fill(
                                    Bounds::new(point(x0, y), size(w, px(LINE_H))),
                                    char_tint,
                                ));
                            }
                        }
                        // Indent guides (the caret's block in the active colour).
                        let levels = super::brackets::levels(doc, line);
                        for k in 0..levels {
                            let gx = text_x + char_w * (k * doc.tab_size) as f32;
                            let active = active_guide.as_ref().is_some_and(|(sd, ak, lines)| {
                                *sd == side && *ak == k && lines.contains(&line)
                            });
                            text.under.push(fill(
                                Bounds::new(point(gx, y), size(px(1.), px(LINE_H))),
                                if active { c.guide_active } else { c.guide },
                            ));
                        }
                        // Matching brackets: `--rv-bm-bg` with a 1 px
                        // `--rv-bm-border` outline inside.
                        for (_, p) in bracket_marks
                            .iter()
                            .filter(|(sd, p)| *sd == side && p.line == line)
                        {
                            let x0 = text_x + col_x(&shaped, p.col);
                            let x1 = text_x + col_x(&shaped, p.col + 1);
                            text.under.push(gpui::quad(
                                Bounds::new(point(x0, y), size(x1 - x0, px(LINE_H))),
                                px(0.),
                                c.bracket_bg,
                                px(1.),
                                c.bracket_border,
                                gpui::BorderStyle::Solid,
                            ));
                        }
                        // Find matches.
                        if let Some(f) = self.find.as_ref().filter(|f| f.side == side) {
                            let ms = &f.found.matches;
                            let start = ms.partition_point(|m| m.line < line);
                            for (i, m) in ms[start..]
                                .iter()
                                .enumerate()
                                .take_while(|(_, m)| m.line == line)
                            {
                                let x0 = text_x + col_x(&shaped, m.cols.start);
                                let x1 = text_x + col_x(&shaped, m.cols.end);
                                let color = if f.current == Some(start + i) {
                                    c.find_cur
                                } else {
                                    c.find
                                };
                                text.under.push(fill(
                                    Bounds::new(point(x0, y), size(x1 - x0, px(LINE_H))),
                                    color,
                                ));
                            }
                        }
                        // Selection and cursor.
                        if let Some(s) = self.sel.filter(|s| s.side == side) {
                            let (a, b) = s.range();
                            if !s.is_empty() && line >= a.line && line <= b.line {
                                let x0 = if line == a.line {
                                    text_x + col_x(&shaped, a.col)
                                } else {
                                    text_x
                                };
                                let x1 = if line == b.line {
                                    text_x + col_x(&shaped, b.col)
                                } else {
                                    tb.right()
                                };
                                let color = if focused { c.sel } else { c.sel_inactive };
                                text.under.push(fill(
                                    Bounds::new(
                                        point(x0, y),
                                        size((x1 - x0).max(px(0.)), px(LINE_H)),
                                    ),
                                    color,
                                ));
                            }
                            if focused
                                && s.head.line == line
                                && !(pane.unified && side == Side::Old)
                            {
                                let x = text_x + col_x(&shaped, s.head.col);
                                text.over.push(fill(
                                    Bounds::new(point(x, y), size(px(2.), px(LINE_H))),
                                    c.cursor,
                                ));
                            }
                        }
                        text.lines.push((shaped, point(text_x, y)));

                        // The margin.
                        let gx = pane.bounds.left();
                        if changed {
                            let full_gut =
                                Bounds::new(point(gx, y), size(pane.gutter_w(), px(LINE_H)));
                            if pane.unified && side == Side::New {
                                // Inline insert: the original's column red, the rest green.
                                gutter.under.push(fill(
                                    Bounds::new(point(gx, y), size(px(NUM_W), px(LINE_H))),
                                    c.del_gutter,
                                ));
                                gutter.under.push(fill(
                                    Bounds::new(
                                        point(gx + px(NUM_W), y),
                                        size(pane.gutter_w() - px(NUM_W), px(LINE_H)),
                                    ),
                                    c.ins_gutter,
                                ));
                            } else {
                                gutter.under.push(fill(full_gut, gut_tint));
                            }
                            let sign = shape_ui(
                                window,
                                if side == Side::New { "+" } else { "−" },
                                12.,
                                c.text,
                            );
                            gutter.lines.push((
                                sign,
                                point(gx + pane.gutter_w() - px(SIGN_W) - px(1.), y + px(1.)),
                            ));
                        }
                        let active = self
                            .sel
                            .is_some_and(|s| s.side == side && s.head.line == line);
                        let num_color = if active {
                            c.gutter_active
                        } else {
                            c.gutter_text
                        };
                        let num = shape_number(window, line + 1, num_color);
                        if pane.unified {
                            // Original number, then the modified's.
                            let (o, n) = match (old, new) {
                                (Some(o), Some(n)) => (Some(*o), Some(*n)),
                                (Some(o), None) => (Some(*o), None),
                                (None, n) => (None, *n),
                            };
                            if let Some(o) = o {
                                let s = shape_number(window, o + 1, c.gutter_text);
                                let x = gx + px(NUM_W) - s.width - px(2.);
                                gutter.lines.push((s, point(x, y)));
                            }
                            if let Some(n) = n {
                                let color = if self
                                    .sel
                                    .is_some_and(|s| s.side == Side::New && s.head.line == n)
                                {
                                    c.gutter_active
                                } else {
                                    c.gutter_text
                                };
                                let s = shape_number(window, n + 1, color);
                                let x = gx + pane.num_right() - s.width - px(2.);
                                gutter.lines.push((s, point(x, y)));
                            }
                        } else {
                            let x = gx + pane.num_right() - num.width - px(2.);
                            gutter.lines.push((num, point(x, y)));
                        }
                        // The comment glyph: a mark on commented lines, "+" on the hovered one.
                        if self.commentable && side == Side::New {
                            let g = point(gx + pane.glyph_x(), y);
                            if self.comments.contains_key(&line) {
                                gutter.over.push(quad(
                                    Bounds::new(g + point(px(6.), px(5.)), size(px(8.), px(8.))),
                                    px(2.),
                                    c.text,
                                    px(0.),
                                    gpui::transparent_black(),
                                    Default::default(),
                                ));
                            } else if self.hover_line == Some(line) {
                                gutter.over.push(quad(
                                    Bounds::new(g + point(px(2.), px(2.)), size(px(16.), px(16.))),
                                    px(4.),
                                    c.glyph_add_bg,
                                    px(0.),
                                    gpui::transparent_black(),
                                    Default::default(),
                                ));
                                let plus = shape_ui(window, "+", 12., c.glyph_add_fg);
                                let px_ = g.x + px(2.) + (px(16.) - plus.width) / 2.;
                                gutter.lines.push((plus, point(px_, y + px(1.))));
                            }
                        }
                    }
                }
            }
            frame.layers.push(text);
            frame.layers.push(gutter);
            frame.layers.push(labels);
        }
        self.max_width = widest;

        // Between the panes: a line and a soft shadow.
        let mut chrome = Layer {
            mask: Some(bounds),
            ..Layer::default()
        };
        if geom.panes.len() == 2 {
            let x = geom.panes[1].bounds.left();
            chrome.under.push(fill(
                Bounds::new(point(x, bounds.top()), size(px(1.), bounds.size.height)),
                c.widget_border,
            ));
        }
        self.overview_and_bars(&geom, c, &mut chrome);
        zones.push((geom.ruler, CursorStyle::Arrow));
        zones.push((geom.vbar(), CursorStyle::Arrow));
        frame.layers.push(chrome);
        frame.cursor = Some((bounds, CursorStyle::Arrow, {
            zones.reverse();
            zones
        }));
        let _ = cx;
        frame
    }

    fn overview_and_bars(&self, geom: &Geom, c: &Colors, layer: &mut Layer) {
        let r = geom.ruler;
        layer.under.push(fill(r, c.bg));
        layer.under.push(fill(
            Bounds::new(r.origin, size(px(1.), r.size.height)),
            c.widget_border,
        ));
        let total = px(self.rows.height()) + self.top_pad();
        let view_h = geom.text_h();
        let scale = r.size.height / total.max(view_h);
        let half = (r.size.width - px(1.)) / 2.;
        // Changed rows: deletions in the left half, insertions in the right.
        let mut run: Option<(Side, usize, usize)> = None;
        let flush = |run: Option<(Side, usize, usize)>, layer: &mut Layer| {
            if let Some((side, a, b)) = run {
                let y0 = px(self.rows.top(a)) + self.top_pad();
                let y1 = px(self.rows.top(b + 1)) + self.top_pad();
                let h = ((y1 - y0) * scale).max(px(2.));
                let (x, color) = match side {
                    Side::Old => (r.left() + px(1.), c.ov_del),
                    Side::New => (r.left() + px(1.) + half, c.ov_ins),
                };
                layer.under.push(fill(
                    Bounds::new(point(x, r.top() + y0 * scale), size(half, h)),
                    color,
                ));
            }
        };
        for side in [Side::Old, Side::New] {
            for (i, row) in self.rows.rows.iter().enumerate() {
                let hit =
                    matches!(row, Row::Line { changed: true, .. }) && row.line(side).is_some();
                match (&mut run, hit) {
                    (Some((s, _, b)), true) if *s == side && *b + 1 == i => *b = i,
                    (_, true) => {
                        flush(run.take(), layer);
                        run = Some((side, i, i));
                    }
                    (_, false) => flush(run.take(), layer),
                }
            }
            flush(run.take(), layer);
        }
        // What is in view.
        let vy = r.top() + self.scroll_y * scale;
        let vh = (view_h * scale).min(r.size.height);
        let dragging = matches!(self.drag, Some(Drag::Ruler { .. }));
        layer.over.push(fill(
            Bounds::new(
                point(r.left() + px(1.), vy),
                size(r.size.width - px(1.), vh),
            ),
            if dragging { c.slider_active } else { c.slider },
        ));

        // Overlay scrollbars, while the pointer is in (or dragging).
        let show = self.hovered || self.drag.is_some();
        let max = self.max_scroll_y();
        let track = geom.vbar();
        if show && max > px(0.) {
            let content = total;
            let thumb_h = (track.size.height * (view_h / content)).max(px(MIN_THUMB));
            let y = track.top() + (track.size.height - thumb_h) * (self.scroll_y / max);
            // The lane: the cursor, and Find's matches.
            let lane = |line_top: Pixels| {
                track.top()
                    + (line_top + px(LINE_H / 2.)) * (track.size.height / content.max(view_h))
            };
            if let Some(f) = self.find.as_ref() {
                for m in f.found.matches.iter().take(2000) {
                    if let Some(row) = self.rows.row_of(f.side, m.line) {
                        let ty = lane(px(self.rows.top(row)) + self.top_pad());
                        layer.under.push(fill(
                            Bounds::new(
                                point(track.left() + px(2.), ty - px(1.)),
                                size(px(VBAR_W - 4.), px(2.)),
                            ),
                            c.lane_find,
                        ));
                    }
                }
            }
            if let Some(s) = self.sel {
                if let Some(row) = self.rows.row_of(s.side, s.head.line) {
                    let ty = lane(px(self.rows.top(row)) + self.top_pad());
                    layer.under.push(fill(
                        Bounds::new(point(track.left(), ty - px(1.)), size(px(VBAR_W), px(2.))),
                        c.lane_cursor,
                    ));
                }
            }
            let active = matches!(self.drag, Some(Drag::VBar { .. }));
            layer.over.push(fill(
                Bounds::new(point(track.left(), y), size(track.size.width, thumb_h)),
                if active {
                    c.slider_active
                } else {
                    c.slider_hover
                },
            ));
        }
        let max_x = (self.max_width + px(CHAR_SLACK) - geom.text_w()).max(px(0.));
        if show && max_x > px(0.) {
            for pane in &geom.panes {
                let hb = geom.hbar(pane);
                let content = self.max_width + px(CHAR_SLACK);
                let w = (hb.size.width * (geom.text_w() / content)).max(px(MIN_THUMB));
                let x = hb.left() + (hb.size.width - w) * (self.scroll_x / max_x);
                let active = matches!(self.drag, Some(Drag::HBar));
                layer.over.push(fill(
                    Bounds::new(point(x, hb.top()), size(w, hb.size.height)),
                    if active {
                        c.slider_active
                    } else {
                        c.slider_hover
                    },
                ));
            }
        }
    }

    // ── the pointer ─────────────────────────────────────────────────────

    fn hit(&self, p: Point<Pixels>) -> Hit {
        let Some(g) = self.geom.as_ref() else {
            return Hit::None;
        };
        if !g.bounds.contains(&p) {
            return Hit::None;
        }
        if g.ruler.contains(&p) {
            return Hit::Ruler;
        }
        if (self.hovered || self.drag.is_some())
            && g.vbar().contains(&p)
            && self.max_scroll_y() > px(0.)
        {
            return Hit::VBar;
        }
        let Some((pi, pane)) = g
            .panes
            .iter()
            .enumerate()
            .find(|(_, pn)| pn.bounds.contains(&p))
        else {
            return Hit::None;
        };
        let max_x = (self.max_width + px(CHAR_SLACK) - g.text_w()).max(px(0.));
        if self.hovered && max_x > px(0.) && g.hbar(pane).contains(&p) {
            return Hit::HBar;
        }
        let y = p.y - g.bounds.top() - self.top_pad() + self.scroll_y;
        if y < px(0.) || y >= px(self.rows.height()) {
            return Hit::None;
        }
        let r = self.rows.row_at(f32::from(y));
        let in_gutter = p.x < pane.bounds.left() + pane.gutter_w();
        Hit::Row {
            pane: pi,
            row: r,
            in_gutter,
            on_number: in_gutter
                && p.x >= pane.bounds.left() + pane.num_right() - px(NUM_W)
                && p.x < pane.bounds.left() + pane.num_right(),
            on_glyph: in_gutter
                && p.x >= pane.bounds.left() + pane.glyph_x()
                && p.x < pane.bounds.left() + pane.glyph_x() + px(GLYPH_W),
        }
    }

    /// The line a row shows in pane `pane` (for selecting and commenting).
    fn row_line(&self, pane: &PaneGeom, row: usize) -> Option<(Side, usize)> {
        let content = self.content.as_ref()?;
        let r = self.rows.rows.get(row)?;
        if pane.unified || !content.is_diff {
            // Inline: deleted rows can't be selected (as CodeMirror's widgets).
            r.line(Side::New).map(|l| (Side::New, l))
        } else {
            r.line(pane.side).map(|l| (pane.side, l))
        }
    }

    fn pos_at(
        &self,
        pane: &PaneGeom,
        side: Side,
        line: usize,
        x: Pixels,
        window: &mut Window,
    ) -> Pos {
        let text = self.doc(side).lines[line].text.clone();
        let col = col_for(window, &text, x - pane.text_left() + self.scroll_x);
        Pos::new(line, col)
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.readonly_at = None;
        if self.menu.take().is_some() {
            cx.notify();
            if e.button == MouseButton::Left {
                return;
            }
        }
        let Some(g) = self.geom.clone() else { return };
        let hit = self.hit(e.position);
        match (e.button, hit) {
            (MouseButton::Left, Hit::Ruler) => {
                let r = g.ruler;
                let total = (px(self.rows.height()) + self.top_pad()).max(g.text_h());
                let scale = r.size.height / total;
                let vy = r.top() + self.scroll_y * scale;
                let vh = g.text_h() * scale;
                let grab = if e.position.y >= vy && e.position.y <= vy + vh {
                    e.position.y - vy
                } else {
                    self.scroll_y = (e.position.y - r.top()) / scale - g.text_h() / 2.;
                    self.clamp_scroll();
                    vh / 2.
                };
                self.drag = Some(Drag::Ruler { grab });
            }
            (MouseButton::Left, Hit::VBar) => {
                let track = g.vbar();
                let (y, h) = self.vthumb(&g);
                if e.position.y >= y && e.position.y <= y + h {
                    self.drag = Some(Drag::VBar {
                        grab: e.position.y - y,
                    });
                } else {
                    // A click on the track pages toward it.
                    let dir = if e.position.y < y { -1. } else { 1. };
                    self.scroll_y += g.text_h() * dir;
                    self.clamp_scroll();
                    let _ = track;
                }
            }
            (MouseButton::Left, Hit::HBar) => {
                self.drag = Some(Drag::HBar);
                self.drag_hbar(e.position, &g);
            }
            (
                button,
                Hit::Row {
                    pane,
                    row,
                    in_gutter,
                    on_number,
                    on_glyph,
                },
            ) => {
                let pane_g = g.panes[pane].clone();
                if let Some(Row::Fold { region, .. }) = self.rows.rows.get(row).cloned() {
                    if button == MouseButton::Left {
                        self.expanded.insert(region);
                        self.rebuild_rows();
                    }
                    cx.notify();
                    return;
                }
                let at = self.row_line(&pane_g, row);
                if button == MouseButton::Right {
                    if let Some((side, line)) = at {
                        let p = self.pos_at(&pane_g, side, line, e.position.x, window);
                        let inside = self.sel.is_some_and(|s| {
                            let (a, b) = s.range();
                            s.side == side && p >= a && p <= b
                        });
                        if !inside {
                            self.sel = Some(Selection::caret(side, p));
                        }
                    }
                    let line = self
                        .sel
                        .filter(|s| s.side == Side::New)
                        .map(|s| s.head.line);
                    self.menu = Some(Menu {
                        at: e.position,
                        line,
                        active: None,
                    });
                    cx.notify();
                    return;
                }
                if button != MouseButton::Left {
                    return;
                }
                let Some((side, line)) = at else {
                    cx.notify();
                    return;
                };
                if in_gutter {
                    // A click on the number puts the cursor on the line (⌘C then
                    // copies it); any click in the margin comments.
                    if on_number || on_glyph || !self.commentable {
                        self.sel = Some(Selection::caret(side, Pos::new(line, 0)));
                    }
                    if self.commentable && side == Side::New {
                        cx.emit(CodeViewEvent::Comment { line });
                    }
                    cx.notify();
                    return;
                }
                let p = self.pos_at(&pane_g, side, line, e.position.x, window);
                let text = self.doc(side).lines[line].text.clone();
                self.goal_x = None;
                match e.click_count {
                    2 => {
                        let w = word_at(&text, p.col);
                        self.sel = Some(Selection {
                            side,
                            anchor: Pos::new(line, w.start),
                            head: Pos::new(line, w.end),
                        });
                    }
                    n if n >= 3 => {
                        let doc = self.doc(side);
                        let head = if line + 1 < doc.line_count() {
                            Pos::new(line + 1, 0)
                        } else {
                            Pos::new(line, text.len())
                        };
                        self.sel = Some(Selection {
                            side,
                            anchor: Pos::new(line, 0),
                            head,
                        });
                    }
                    _ => {
                        let extend = e.modifiers.shift && self.sel.is_some_and(|s| s.side == side);
                        self.sel = Some(match (extend, self.sel) {
                            (true, Some(s)) => Selection {
                                side,
                                anchor: s.anchor,
                                head: p,
                            },
                            _ => Selection::caret(side, p),
                        });
                        self.drag = Some(Drag::Select { side });
                    }
                }
                if let Some(f) = self.find.as_mut() {
                    f.current = None;
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn vthumb(&self, g: &Geom) -> (Pixels, Pixels) {
        let track = g.vbar();
        let total = px(self.rows.height()) + self.top_pad();
        let max = self.max_scroll_y();
        let h = (track.size.height * (g.text_h() / total.max(g.text_h()))).max(px(MIN_THUMB));
        let y = if max > px(0.) {
            track.top() + (track.size.height - h) * (self.scroll_y / max)
        } else {
            track.top()
        };
        (y, h)
    }

    fn drag_hbar(&mut self, p: Point<Pixels>, g: &Geom) {
        let Some(pane) = g.panes.last() else { return };
        let hb = g.hbar(pane);
        let max_x = (self.max_width + px(CHAR_SLACK) - g.text_w()).max(px(0.));
        let ratio = ((p.x - hb.left()) / hb.size.width).clamp(0., 1.);
        self.scroll_x = max_x * ratio;
    }

    fn mouse_move(
        &mut self,
        e: &MouseMoveEvent,
        hovered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(g) = self.geom.clone() else { return };
        match self.drag {
            Some(Drag::Select { side }) if e.pressed_button == Some(MouseButton::Left) => {
                // Past the top or bottom: scroll along.
                if e.position.y < g.bounds.top() {
                    self.scroll_y -= px(LINE_H);
                } else if e.position.y > g.bounds.bottom() {
                    self.scroll_y += px(LINE_H);
                }
                self.clamp_scroll();
                let Some(pane) = g
                    .panes
                    .iter()
                    .find(|p| p.side == side || p.unified || g.panes.len() == 1)
                    .cloned()
                else {
                    return;
                };
                let y = (e
                    .position
                    .y
                    .clamp(g.bounds.top(), g.bounds.bottom() - px(1.))
                    - g.bounds.top()
                    - self.top_pad()
                    + self.scroll_y)
                    .max(px(0.));
                let mut r = self.rows.row_at(f32::from(y));
                // The nearest row with a line on this side (fillers, folds).
                let mut line = None;
                for d in 0..self.rows.len() {
                    if let Some(l) = self
                        .rows
                        .rows
                        .get(r.saturating_sub(d))
                        .and_then(|x| x.line(side))
                    {
                        line = Some(l);
                        r = r.saturating_sub(d);
                        break;
                    }
                }
                let _ = r;
                if let Some(line) = line {
                    let p = self.pos_at(&pane, side, line, e.position.x, window);
                    if let Some(s) = self.sel.as_mut() {
                        if s.head != p {
                            s.head = p;
                            cx.notify();
                        }
                    }
                }
                return;
            }
            Some(Drag::VBar { grab }) if e.pressed_button == Some(MouseButton::Left) => {
                let track = g.vbar();
                let (_, h) = self.vthumb(&g);
                let room = (track.size.height - h).max(px(1.));
                let ratio = ((e.position.y - grab - track.top()) / room).clamp(0., 1.);
                self.scroll_y = self.max_scroll_y() * ratio;
                cx.notify();
                return;
            }
            Some(Drag::Ruler { grab }) if e.pressed_button == Some(MouseButton::Left) => {
                let r = g.ruler;
                let total = (px(self.rows.height()) + self.top_pad()).max(g.text_h());
                let scale = r.size.height / total;
                self.scroll_y = (e.position.y - grab - r.top()) / scale;
                self.clamp_scroll();
                cx.notify();
                return;
            }
            Some(Drag::HBar) if e.pressed_button == Some(MouseButton::Left) => {
                self.drag_hbar(e.position, &g);
                cx.notify();
                return;
            }
            Some(_) => {
                self.drag = None;
                cx.notify();
            }
            None => {}
        }
        if !hovered {
            return;
        }
        // Hover: "+" on the line under the pointer, the comment's text on its mark.
        let (mut line, mut comment) = (None, None);
        if let Hit::Row {
            pane,
            row,
            on_glyph,
            ..
        } = self.hit(e.position)
        {
            let pane_g = &g.panes[pane];
            if self.commentable {
                if let Some((Side::New, l)) = self.row_line(pane_g, row) {
                    if pane_g.side == Side::New {
                        line = Some(l);
                        if on_glyph && self.comments.contains_key(&l) {
                            let y = g.bounds.top() + self.top_pad() + px(self.rows.top(row))
                                - self.scroll_y;
                            comment = Some((
                                l,
                                point(
                                    pane_g.bounds.left() + pane_g.glyph_x() + px(GLYPH_W + 4.),
                                    y + px(LINE_H / 2.),
                                ),
                            ));
                        }
                    }
                }
            }
        }
        if line != self.hover_line || comment.map(|c| c.0) != self.hover_comment.map(|c| c.0) {
            self.hover_line = line;
            self.hover_comment = comment;
            cx.notify();
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }

    fn scroll_wheel(&mut self, e: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let mut d = e.delta.pixel_delta(LINE_HEIGHT);
        if e.modifiers.shift && d.x == px(0.) {
            d = point(d.y, px(0.));
        }
        self.scroll_y -= d.y;
        self.scroll_x -= d.x;
        self.clamp_scroll();
        self.readonly_at = None;
        self.hover_comment = None;
        cx.notify();
    }
}

#[derive(Debug, Clone, Copy)]
enum Hit {
    None,
    Ruler,
    VBar,
    HBar,
    Row {
        pane: usize,
        row: usize,
        in_gutter: bool,
        on_number: bool,
        on_glyph: bool,
    },
}

fn shape_number(window: &mut Window, n: usize, color: Hsla) -> ShapedLine {
    let s: SharedString = n.to_string().into();
    window
        .text_system()
        .shape_line(s.clone(), FONT_SIZE, &[plain_run(s.len(), color)], None)
}

/// A line with its syntax colours (spans are source offsets).
fn shape_colored(
    window: &mut Window,
    l: &super::text::Line,
    spans: &[(Range<usize>, super::highlight::Style)],
    c: &Colors,
) -> ShapedLine {
    let len = l.len();
    let mut runs: Vec<TextRun> = Vec::with_capacity(spans.len() * 2 + 1);
    let mut at = 0;
    for (r, tok) in spans {
        let a = l.from_source(r.start).max(at);
        let b = l.from_source(r.end).min(len);
        if b <= a {
            continue;
        }
        if a > at {
            runs.push(plain_run(a - at, c.text));
        }
        let mut run = plain_run(b - a, tok.tok.map_or(c.text, |t| c.token(t)));
        if tok.italic {
            run.font.style = gpui::FontStyle::Italic;
        }
        if tok.bold {
            run.font.weight = gpui::FontWeight::BOLD;
        }
        runs.push(run);
        at = b;
    }
    if at < len || runs.is_empty() {
        runs.push(plain_run(len - at, c.text));
    }
    window
        .text_system()
        .shape_line(l.text.clone(), FONT_SIZE, &runs, None)
}

/// The canvas that draws a view and routes the pointer to it.
pub(super) fn canvas_for(view: Entity<CodeView>, colors: Colors) -> impl IntoElement {
    let v2 = view.clone();
    canvas(
        move |bounds, window, cx| {
            let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
            let frame = view.update(cx, |v, cx| v.prepare(bounds, &colors, window, cx));
            (hitbox, frame)
        },
        move |_, (hitbox, frame), window, cx| {
            frame.paint(&hitbox, window, cx);
            let view = v2;
            {
                let (view, hitbox) = (view.clone(), hitbox.clone());
                window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                        view.update(cx, |v, cx| v.mouse_down(e, window, cx));
                    }
                });
            }
            {
                let (view, hitbox) = (view.clone(), hitbox.clone());
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Bubble {
                        let hovered = hitbox.is_hovered(window);
                        view.update(cx, |v, cx| v.mouse_move(e, hovered, window, cx));
                    }
                });
            }
            {
                let view = view.clone();
                window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble {
                        view.update(cx, |v, cx| v.mouse_up(e, cx));
                    }
                });
            }
            window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                    view.update(cx, |v, cx| v.scroll_wheel(e, cx));
                    cx.stop_propagation();
                }
            });
        },
    )
    .size_full()
}

// ── overlays ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MenuItem {
    Comment,
    Copy,
}

/// The editor's context menu: "Add review comment" (where commenting
/// works), then "Copy".
pub(super) fn menu_items(comment: bool) -> Vec<MenuItem> {
    if comment {
        vec![MenuItem::Comment, MenuItem::Copy]
    } else {
        vec![MenuItem::Copy]
    }
}

fn ui_text() -> gpui::Div {
    div().font_family(".SystemUIFont")
}

pub(super) fn overlays(
    v: &mut CodeView,
    c: &Colors,
    t: &Theme,
    window: &mut Window,
    cx: &mut Context<CodeView>,
) -> Vec<AnyElement> {
    let mut out = Vec::new();
    let Some(g) = v.geom.clone() else { return out };
    let origin = g.bounds.origin;

    // Find, at the top right of its pane.
    if let Some(f) = v.find.as_ref() {
        let pane = g
            .pane_for(f.side)
            .cloned()
            .unwrap_or_else(|| g.panes[0].clone());
        let avail = pane.bounds.size.width - px(28.);
        let w = px(FIND_W).min(avail.max(px(160.)));
        let narrow = f32::from(avail) < FIND_W - 69.;
        let left = pane.bounds.right() - origin.x - w - px(VBAR_W);
        let total = f.found.matches.len();
        let count = if f.query.is_empty() || f.found.error.is_some() || total == 0 {
            "No results".to_string()
        } else {
            format!(
                "{} of {}{}",
                f.current.map_or("?".to_string(), |i| (i + 1).to_string()),
                total,
                if f.found.capped { "+" } else { "" }
            )
        };
        let none = !f.query.is_empty() && total == 0;
        let toggle = |id: &'static str, label: &'static str, on: bool, tip: &'static str| {
            div()
                .id(id)
                .px(px(3.))
                .h(px(20.))
                .flex()
                .items_center()
                .rounded(px(3.))
                .text_size(px(12.))
                .cursor_pointer()
                .border_1()
                .border_color(if on {
                    c.focus
                } else {
                    gpui::transparent_black()
                })
                .when(on, |d| d.bg(c.opt_bg))
                .hover_probed(|s| s.bg(c.slider_hover))
                .child(label)
                .tooltip(move |_, cx| tooltip(tip, cx))
        };
        let q = f.query.clone();
        let in_sel = f.within.is_some();
        let field = f.field.clone();
        let focused = field.read(cx).is_focused(window);
        let button = |id: &'static str, label: &'static str, tip: &'static str, enabled: bool| {
            div()
                .id(id)
                .w(px(22.))
                .h(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .text_size(px(14.))
                .when(!enabled, |d| d.opacity(0.4))
                .when(enabled, |d| {
                    d.cursor_pointer().hover_probed(|s| s.bg(c.slider_hover))
                })
                .child(label)
                .tooltip(move |_, cx| tooltip(tip, cx))
        };
        // `.rv-find`: slides down into place (`transition: transform 0.2s
        // linear` from `translateY(calc(-100% - 10px))`).
        out.push(
            crate::kit::motion::enter("find-in", crate::kit::motion::Fx::FIND_SLIDE, ui_text()
                .id("find")
                .occlude()
                .absolute()
                .top(px(0.))
                .left(left)
                .w(w)
                .h(px(FIND_ZONE))
                .px(px(4.))
                .flex()
                .items_center()
                .gap(px(3.))
                .bg(c.widget_bg)
                .border_1()
                .border_color(c.widget_border)
                .rounded_b(px(4.))
                .shadow_md()
                .text_color(c.widget_fg)
                .text_size(px(12.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex_1()
                        .min_w(px(60.))
                        .h(px(24.))
                        .px(px(4.))
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .bg(c.input_bg)
                        .border_1()
                        .border_color(if focused { c.focus } else { c.widget_border })
                        .rounded(px(2.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.))
                                .line_height(px(18.))
                                .child(field),
                        )
                        .child(toggle("find-case", "Aa", q.case, "Match Case").on_click(
                            cx.listener(|v, _, _, cx| v.set_query(|q| q.case = !q.case, true, cx)),
                        ))
                        .child(
                            toggle("find-word", "ab", q.word, "Match Whole Word")
                                .underline()
                                .on_click(cx.listener(|v, _, _, cx| {
                                    v.set_query(|q| q.word = !q.word, true, cx)
                                })),
                        )
                        .child(
                            toggle("find-re", ".*", q.regex, "Use Regular Expression").on_click(
                                cx.listener(|v, _, _, cx| {
                                    v.set_query(|q| q.regex = !q.regex, true, cx)
                                }),
                            ),
                        ),
                )
                .when(!narrow, |d| {
                    d.child(
                        div()
                            .min_w(px(68.))
                            .px(px(4.))
                            .text_size(px(12.))
                            .when(none, |d| d.text_color(c.error))
                            .child(count),
                    )
                })
                .child(
                    button("find-prev", "↑", "Previous Match (⇧Enter)", total > 0).on_click(
                        cx.listener(|v, _, w, cx| v.find_previous(&super::FindPrevious, w, cx)),
                    ),
                )
                .child(
                    button("find-next", "↓", "Next Match (Enter)", total > 0)
                        .on_click(cx.listener(|v, _, w, cx| v.find_next(&super::FindNext, w, cx))),
                )
                .child(
                    button("find-insel", "≡", "Find in Selection", true)
                        .when(in_sel, |d| d.bg(c.opt_bg).border_1().border_color(c.focus))
                        .on_click(cx.listener(|v, _, _, cx| v.toggle_in_selection(cx))),
                )
                .child(
                    button("find-close", "×", "Close (Escape)", true)
                        .on_click(cx.listener(|v, _, w, cx| v.close_find(w, cx))),
                )
                ).into_any_element(),
        );
    }

    // The comment's text, next to its mark.
    if let Some((line, at)) = v.hover_comment {
        if let Some(text) = v.comments.get(&line) {
            out.push(
                ui_text()
                    .absolute()
                    .left(at.x - origin.x)
                    .top(at.y - origin.y - px(LINE_H / 2. + 1.))
                    .max_w(px(500.))
                    .px(px(8.))
                    .py(px(3.))
                    .bg(c.widget_bg)
                    .border_1()
                    .border_color(c.widget_border)
                    .rounded(px(3.))
                    .shadow_md()
                    .text_color(c.widget_fg)
                    .text_size(px(13.))
                    .line_height(px(19.))
                    .child(text.clone())
                    .into_any_element(),
            );
        }
    }

    // "Read-only…" where the cursor is.
    if let Some(at) = v.readonly_at {
        out.push(
            ui_text()
                .absolute()
                .left((at.x - origin.x - px(6.)).max(px(0.)))
                .top((at.y - origin.y - px(26.)).max(px(0.)))
                .px(px(8.))
                .py(px(2.))
                .bg(c.widget_bg)
                .border_1()
                .border_color(c.focus)
                .text_color(c.widget_fg)
                .text_size(px(12.))
                .child(v.readonly_text.clone())
                .into_any_element(),
        );
    }

    // The context menu.
    if let Some(menu) = v.menu.as_ref() {
        let items = menu_items(menu.line.is_some() && v.commentable);
        let mut list = ui_text()
            .id("code-menu")
            .occlude()
            .min_w(px(180.))
            .py(px(4.))
            .bg(c.menu_bg)
            .text_color(c.menu_fg)
            .text_size(px(13.))
            .rounded(px(5.))
            .border_1()
            .border_color(c.widget_border)
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|v, _, _, cx| {
                v.menu = None;
                cx.notify();
            }));
        for (i, item) in items.iter().enumerate() {
            if i > 0 && items[i - 1] == MenuItem::Comment {
                list = list.child(div().my(px(4.)).mx(px(8.)).h(px(1.)).bg(c.menu_sep));
            }
            let label = match item {
                MenuItem::Comment => "Add review comment",
                MenuItem::Copy => "Copy",
            };
            let active = menu.active == Some(i);
            list = list.child(
                div()
                    .id(("code-menu-item", i))
                    .mx(px(4.))
                    .px(px(10.))
                    .h(px(24.))
                    .flex()
                    .items_center()
                    .rounded(px(3.))
                    .cursor_pointer()
                    .when(active, |d| d.bg(c.menu_sel))
                    .on_hover(cx.listener(move |v, h: &bool, _, cx| {
                        if let Some(m) = v.menu.as_mut() {
                            m.active = h.then_some(i);
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |v, _, _, cx| v.run_menu(i, cx)))
                    .child(label),
            );
        }
        out.push(
            deferred(
                anchored()
                    .position(menu.at)
                    .anchor(Corner::TopLeft)
                    .snap_to_window()
                    .child(list),
            )
            .with_priority(10)
            .into_any_element(),
        );
    }
    let _ = t;
    out
}

/// A tooltip (the kit's).
pub(crate) fn tooltip(text: &'static str, cx: &mut App) -> gpui::AnyView {
    crate::kit::tooltip_view(text, cx)
}
