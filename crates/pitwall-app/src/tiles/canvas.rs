//! A tile's `canvas` element painted with GPUI (docs/spec/tile-sdk/ui.md
//! §3.7): rectangles as quads (dashed outlines as paths), paths with
//! lyon's tessellation, text shaped in Pitwall's fonts, clips as content
//! masks, all through the canvas's transform. Its regions are drawn by
//! [`super::view`] as boxes over it, so they get tooltips, the hand cursor
//! and clicks the way other nodes do.

use gpui::{point, px, size, App, Bounds, ContentMask, FontStyle, Hsla, PathBuilder, Pixels, Point, SharedString, TextAlign, TextRun, Window};
use pitwall_tile_host::ui;

use super::render;
use crate::theme::Theme;

/// How far a press may move and still be a click on a region (pixels).
pub const CLICK_SLACK: f32 = 4.;

/// A paint's colour in theme `t`.
pub fn paint(p: &ui::Paint, t: &Theme) -> Hsla {
    match p {
        ui::Paint::Token(k) => render::color(*k, t),
        ui::Paint::Rgba(c) => gpui::rgba(*c).into(),
    }
}

/// A canvas transform with the node's origin folded in: canvas point →
/// window point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tf {
    pub scale: f32,
    pub dx: f32,
    pub dy: f32,
}

impl Tf {
    /// The transform of a canvas drawn at `origin` (its node's top-left).
    pub fn new(t: &ui::Transform, origin: Point<Pixels>) -> Tf {
        let scale = if t.scale.is_finite() && t.scale > 0. { t.scale } else { 1. };
        let finite = |v: f32| if v.is_finite() { v } else { 0. };
        Tf { scale, dx: f32::from(origin.x) + finite(t.dx), dy: f32::from(origin.y) + finite(t.dy) }
    }

    pub fn point(&self, p: &ui::Point) -> Point<Pixels> {
        point(px(p.x * self.scale + self.dx), px(p.y * self.scale + self.dy))
    }

    pub fn len(&self, v: f32) -> Pixels {
        px((v * self.scale).max(0.))
    }

    /// A canvas rectangle in window coordinates (negative sizes flipped).
    pub fn rect(&self, r: &ui::Rect) -> Bounds<Pixels> {
        let (x0, x1) = (r.x.min(r.x + r.width), r.x.max(r.x + r.width));
        let (y0, y1) = (r.y.min(r.y + r.height), r.y.max(r.y + r.height));
        Bounds::new(self.point(&ui::Point { x: x0, y: y0 }), size(self.len(x1 - x0), self.len(y1 - y0)))
    }
}

/// Each op's clip, in window coordinates: the clips pushed around it,
/// intersected (`None`: none). A `clip-pop` with no push is ignored.
pub fn clips(ops: &[ui::DrawOp], tf: &Tf) -> Vec<Option<Bounds<Pixels>>> {
    let mut stack: Vec<Bounds<Pixels>> = Vec::new();
    ops.iter()
        .map(|op| {
            match op {
                ui::DrawOp::ClipPush(r) => {
                    let b = tf.rect(r);
                    let b = stack.last().map_or(b, |top| top.intersect(&b));
                    stack.push(b);
                }
                ui::DrawOp::ClipPop => {
                    stack.pop();
                }
                _ => {}
            }
            stack.last().copied()
        })
        .collect()
}

/// Paint canvas `c` in `bounds` (its node's), in theme `t`.
pub fn draw(c: &ui::Canvas, bounds: Bounds<Pixels>, t: &Theme, window: &mut Window, cx: &mut App) {
    let tf = Tf::new(&c.transform, bounds.origin);
    let clips = clips(&c.ops, &tf);
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        for (op, clip) in c.ops.iter().zip(clips) {
            let mask = clip.map(|b| ContentMask { bounds: b.intersect(&bounds) });
            window.with_content_mask(mask, |window| draw_op(op, &tf, t, window, cx));
        }
    });
}

fn draw_op(op: &ui::DrawOp, tf: &Tf, t: &Theme, window: &mut Window, cx: &mut App) {
    match op {
        ui::DrawOp::Rect(r) => {
            let b = tf.rect(&r.rect);
            let radius = tf.len(r.radius).min(b.size.width / 2.).min(b.size.height / 2.);
            if let Some(f) = &r.fill {
                window.paint_quad(gpui::fill(b, paint(f, t)).corner_radii(radius));
            }
            if let Some(s) = r.stroke.as_ref().filter(|s| s.width > 0.) {
                if s.dash.is_empty() {
                    window.paint_quad(gpui::quad(b, radius, gpui::transparent_black(), tf.len(s.width), paint(&s.color, t), gpui::BorderStyle::Solid));
                } else {
                    // Dashed: the outline as a path, half the width inside.
                    let inset = tf.len(s.width) / 2.;
                    let b = Bounds::new(b.origin + point(inset, inset), size((b.size.width - inset * 2.).max(px(0.)), (b.size.height - inset * 2.).max(px(0.))));
                    let mut p = stroker(s, tf);
                    rounded_rect(&mut p, b, (radius - inset).max(px(0.)));
                    if let Ok(path) = p.build() {
                        window.paint_path(path, paint(&s.color, t));
                    }
                }
            }
        }
        ui::DrawOp::Path(p) => {
            if let Some(f) = &p.fill {
                let mut b = PathBuilder::fill();
                steps(&mut b, &p.steps, tf, true);
                if let Ok(path) = b.build() {
                    window.paint_path(path, paint(f, t));
                }
            }
            if let Some(s) = p.stroke.as_ref().filter(|s| s.width > 0.) {
                let mut b = stroker(s, tf);
                steps(&mut b, &p.steps, tf, false);
                if let Ok(path) = b.build() {
                    window.paint_path(path, paint(&s.color, t));
                }
            }
        }
        ui::DrawOp::Text(d) => text(d, tf, t, window, cx),
        ui::DrawOp::ClipPush(_) | ui::DrawOp::ClipPop => {}
    }
}

fn stroker(s: &ui::Stroke, tf: &Tf) -> PathBuilder {
    let b = PathBuilder::stroke(tf.len(s.width));
    let dash: Vec<Pixels> = s.dash.iter().map(|d| tf.len(d.max(0.))).collect();
    // A pattern of zeros would never advance.
    if dash.iter().any(|d| f32::from(*d) > 0.01) {
        b.dash_array(&dash)
    } else {
        b
    }
}

/// Add a path's steps; a fill closes each piece.
fn steps(b: &mut PathBuilder, steps: &[ui::PathStep], tf: &Tf, fill: bool) {
    let mut open = false;
    for s in steps {
        match s {
            ui::PathStep::MoveTo(p) => {
                if open && fill {
                    b.close();
                }
                b.move_to(tf.point(p));
                open = true;
            }
            // A line before any move starts there.
            ui::PathStep::LineTo(p) if !open => {
                b.move_to(tf.point(p));
                open = true;
            }
            ui::PathStep::LineTo(p) => b.line_to(tf.point(p)),
            ui::PathStep::QuadTo(q) if open => b.curve_to(tf.point(&q.to), tf.point(&q.control)),
            ui::PathStep::CubicTo(c) if open => b.cubic_bezier_to(tf.point(&c.to), tf.point(&c.control1), tf.point(&c.control2)),
            ui::PathStep::QuadTo(_) | ui::PathStep::CubicTo(_) => {}
            ui::PathStep::Close => {
                if open {
                    b.close();
                    open = false;
                }
            }
        }
    }
    if open && fill {
        b.close();
    }
}

/// A rectangle with round corners (cubic arcs), clockwise from the top.
fn rounded_rect(b: &mut PathBuilder, r: Bounds<Pixels>, radius: Pixels) {
    let (x0, y0, x1, y1) = (r.left(), r.top(), r.right(), r.bottom());
    let k = radius * 0.5523;
    b.move_to(point(x0 + radius, y0));
    b.line_to(point(x1 - radius, y0));
    b.cubic_bezier_to(point(x1, y0 + radius), point(x1 - radius + k, y0), point(x1, y0 + radius - k));
    b.line_to(point(x1, y1 - radius));
    b.cubic_bezier_to(point(x1 - radius, y1), point(x1, y1 - radius + k), point(x1 - radius + k, y1));
    b.line_to(point(x0 + radius, y1));
    b.cubic_bezier_to(point(x0, y1 - radius), point(x0 + radius - k, y1), point(x0, y1 - radius + k));
    b.line_to(point(x0, y0 + radius));
    b.cubic_bezier_to(point(x0 + radius, y0), point(x0, y0 + radius - k), point(x0 + radius - k, y0));
    b.close();
}

fn text(d: &ui::DrawText, tf: &Tf, t: &Theme, window: &mut Window, cx: &mut App) {
    let size = d.size * tf.scale;
    // Too small to read, or not a size.
    if !(size.is_finite() && (2.0..=400.).contains(&size)) || d.text.is_empty() {
        return;
    }
    let font = gpui::Font { family: render::family(d.font).into(), features: Default::default(), fallbacks: None, weight: render::weight(d.weight), style: FontStyle::Normal };
    let color = paint(&d.color, t);
    let one_line: String = d.text.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    let shape = |s: &str, window: &mut Window| {
        let s: SharedString = s.to_string().into();
        let run = TextRun { len: s.len(), font: font.clone(), color, background_color: None, underline: None, strikethrough: None };
        window.text_system().shape_line(s, px(size), &[run], None)
    };
    let mut line = shape(&one_line, window);
    if let Some(max) = d.max_width.filter(|m| m.is_finite() && *m > 0.) {
        let max = tf.len(max);
        if line.width > max {
            line = cut(&one_line, max, window, &shape);
        }
    }
    let line_height = px(size * 1.3);
    let at = tf.point(&d.at);
    let x = match d.align {
        ui::TextAlign::Start => at.x,
        ui::TextAlign::Center => at.x - line.width / 2.,
        ui::TextAlign::End => at.x - line.width,
    };
    let y = match d.baseline {
        ui::TextBaseline::Top => at.y,
        ui::TextBaseline::Middle => at.y - line_height / 2.,
        ui::TextBaseline::Bottom => at.y - line_height,
    };
    let _ = line.paint(point(x, y), line_height, TextAlign::Left, None, window, cx);
}

/// `text` cut with an ellipsis to fit `max`: the longest prefix that does.
fn cut(text: &str, max: Pixels, window: &mut Window, shape: &dyn Fn(&str, &mut Window) -> gpui::ShapedLine) -> gpui::ShapedLine {
    let ends: Vec<usize> = text.char_indices().map(|(i, _)| i).skip(1).chain(std::iter::once(text.len())).collect();
    let (mut lo, mut hi) = (0usize, ends.len());
    let mut best = shape("…", window);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let s = format!("{}…", text[..ends[mid]].trim_end());
        let l = shape(&s, window);
        if l.width <= max {
            best = l;
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    best
}

/// The region of `c` under node point `at` (the last listed wins), by
/// index.
pub fn region_at(c: &ui::Canvas, at: ui::Point) -> Option<usize> {
    let tf = Tf::new(&c.transform, Point::default());
    let p = point(px(at.x), px(at.y));
    c.regions.iter().rposition(|r| tf.rect(&r.rect).contains(&p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> ui::Rect {
        ui::Rect { x, y, width: w, height: h }
    }

    #[test]
    fn transforms_clips_and_regions() {
        let tf = Tf::new(&ui::Transform { scale: 2., dx: 10., dy: 20. }, point(px(100.), px(200.)));
        assert_eq!(tf.point(&ui::Point { x: 1., y: 2. }), point(px(112.), px(224.)));
        assert_eq!(tf.rect(&r(5., 5., -5., -5.)), Bounds::new(point(px(110.), px(220.)), size(px(10.), px(10.))));
        // Clips nest by intersection; a stray pop is ignored.
        let ops = vec![
            ui::DrawOp::ClipPop,
            ui::DrawOp::ClipPush(r(0., 0., 10., 10.)),
            ui::DrawOp::ClipPush(r(5., 5., 10., 10.)),
            ui::DrawOp::ClipPop,
            ui::DrawOp::ClipPop,
        ];
        let one = Tf::new(&ui::Transform { scale: 1., dx: 0., dy: 0. }, Point::default());
        let c = clips(&ops, &one);
        assert_eq!(c[0], None);
        assert_eq!(c[2], Some(Bounds::new(point(px(5.), px(5.)), size(px(5.), px(5.)))));
        assert_eq!(c[3], Some(Bounds::new(point(px(0.), px(0.)), size(px(10.), px(10.)))));
        assert_eq!(c[4], None);
        let region = |k: &str, x| ui::HitRegion { key: k.into(), rect: r(x, 0., 10., 10.), label: k.into(), tooltip: None, click: None, hover: None };
        let canvas = ui::Canvas { ops: vec![], transform: ui::Transform { scale: 2., dx: 0., dy: 0. }, regions: vec![region("a", 0.), region("b", 5.)] };
        assert_eq!(region_at(&canvas, ui::Point { x: 5., y: 5. }), Some(0));
        assert_eq!(region_at(&canvas, ui::Point { x: 15., y: 5. }), Some(1), "the last listed wins");
        assert_eq!(region_at(&canvas, ui::Point { x: 35., y: 5. }), None);
        assert_eq!(paint(&ui::Paint::Rgba(0xff0000ff), &Theme::dark()), gpui::rgba(0xff0000ff).into());
    }
}
