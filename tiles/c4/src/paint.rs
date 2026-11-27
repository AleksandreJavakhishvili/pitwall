//! The Painter (hla.md §3.8, phase B): a laid-out structure diagram as
//! canvas draw operations, with a clickable region per box. Theme colours
//! only, text in pixels so it zooms, dashed boundaries and dotted arrows.
//! Behind the `tile` feature: it needs the Tile SDK.

use pitwall_extension::prelude::*;

use crate::c4::{Shape as BoxShape, Structure};
use crate::layout::{
    Placed, Point, Rect, SizedBox, BOX_PAD_X, BOX_PAD_Y, CYLINDER_CAP, NAME_LINE, NAME_PX,
    PERSON_HEAD, SMALL_LINE, SMALL_PX, TITLE_BAND, TITLE_PX,
};
use crate::mermaid::flowchart::{Head, LineStyle};

/// Around the whole drawing, in canvas units.
pub const MARGIN: f32 = 24.;

/// A box the user can click: its index in the structure, its key, where it
/// is and its accessible name.
#[derive(Debug, Clone, PartialEq)]
pub struct HitBox {
    pub index: usize,
    pub key: String,
    pub rect: Rect,
    pub label: String,
}

/// Draws `s` as `placed` lays it out on `canvas`: boundaries, then arrows,
/// then boxes. `detailed[i]` marks box `i` as having a detail diagram (a
/// corner mark, FR-12); `selected` is outlined. Returns the canvas and its
/// boxes, for regions.
pub fn paint(
    mut canvas: Canvas,
    s: &Structure,
    sized: &[SizedBox],
    placed: &Placed,
    detailed: &[bool],
    selected: Option<usize>,
) -> (Canvas, Vec<HitBox>) {
    // Boundaries, outer first.
    for (b, r) in s.boundaries.iter().zip(&placed.clusters) {
        canvas = canvas
            .draw(Shape::rect(f(r.x), f(r.y), f(r.w), f(r.h)).stroke(Token::LineStrong, 1.).dash([6., 4.]).radius(10.))
            .draw(
                Label::new(title(&b.name, b.kind.as_ref().map(|k| k.text.as_str())), f(r.x) + 10., f(r.y) + f(TITLE_BAND) / 2.)
                    .size(f(TITLE_PX))
                    .semibold()
                    .color(Token::Text2)
                    .middle()
                    .max_width(f(r.w) - 20.),
            );
    }
    // Arrows.
    for (a, e) in s.arrows.iter().zip(&placed.edges) {
        if a.style == LineStyle::Invisible || e.points.len() < 2 {
            continue;
        }
        let mut line = Shape::path().move_to(f(e.points[0].x), f(e.points[0].y));
        for p in &e.points[1..] {
            line = line.line_to(f(p.x), f(p.y));
        }
        let width = if a.style == LineStyle::Thick { 2.5 } else { 1.5 };
        let mut line = line.stroke(Token::Text3, width);
        if a.style == LineStyle::Dotted {
            line = line.dash([3., 3.]);
        }
        canvas = canvas.draw(line);
        let n = e.points.len();
        if a.end_head != Head::None {
            canvas = canvas.draw(head(e.points[n - 2], e.points[n - 1]));
        }
        if a.start_head != Head::None {
            canvas = canvas.draw(head(e.points[1], e.points[0]));
        }
        if let (Some(label), Some(r)) = (&a.label, &e.label) {
            canvas = canvas
                .draw(Shape::rect(f(r.x), f(r.y), f(r.w), f(r.h)).fill(Token::Surface).radius(4.))
                .draw(Label::new(label.clone(), f(r.x + r.w / 2.), f(r.y + r.h / 2.)).size(f(SMALL_PX)).color(Token::Text2).center().middle().max_width(f(r.w)));
        }
    }
    // Boxes.
    let mut hits = Vec::new();
    for (i, ((b, sz), r)) in s.boxes.iter().zip(sized).zip(&placed.nodes).enumerate() {
        let on = selected == Some(i);
        let fill = if on {
            Token::DoneSoft
        } else if b.marks.external {
            Token::Surface3
        } else {
            Token::Surface2
        };
        let (stroke, width) = if on { (Token::Done, 2.) } else { (Token::Line, 1.) };
        let (x, y, w, h) = (f(r.x), f(r.y), f(r.w), f(r.h));
        let mut top = y;
        match b.shape {
            BoxShape::Person => {
                let d = f(PERSON_HEAD) - 4.;
                canvas = canvas.draw(circle(x + w / 2., y + d / 2., d / 2.).fill(fill).stroke(stroke, width));
                top = y + f(PERSON_HEAD);
                canvas = canvas.draw(Shape::rect(x, top, w, y + h - top).fill(fill).stroke(stroke, width).radius(12.));
            }
            BoxShape::Cylinder => {
                let cap = f(CYLINDER_CAP);
                canvas = canvas.draw(cylinder(x, y, w, h, cap).fill(fill).stroke(stroke, width));
                // The top's front rim.
                canvas = canvas.draw(Shape::path().move_to(x, y + cap / 2.).cubic_to(x, y + cap, x + w, y + cap, x + w, y + cap / 2.).stroke(stroke, width));
                top = y + cap;
            }
            BoxShape::Pipe => {
                canvas = canvas.draw(Shape::rect(x, y, w, h).fill(fill).stroke(stroke, width).radius(h / 2.));
            }
            BoxShape::Box => {
                let mut rect = Shape::rect(x, y, w, h).fill(fill).stroke(stroke, width).radius(8.);
                if b.marks.external && !on {
                    rect = rect.dash([4., 3.]);
                }
                canvas = canvas.draw(rect);
            }
        }
        let inner = w - 2. * f(BOX_PAD_X);
        let mut at = top + f(BOX_PAD_Y);
        canvas = canvas.draw(Label::new(sz.name.clone(), x + w / 2., at + f(NAME_LINE) / 2.).size(f(NAME_PX)).semibold().center().middle().max_width(inner));
        at += f(NAME_LINE);
        if let Some(kind) = &sz.kind {
            canvas = canvas.draw(Label::new(kind.clone(), x + w / 2., at + f(SMALL_LINE) / 2.).size(f(SMALL_PX)).color(Token::Text3).center().middle().max_width(inner));
            at += f(SMALL_LINE);
        }
        for line in &sz.purpose {
            canvas = canvas.draw(Label::new(line.clone(), x + w / 2., at + f(SMALL_LINE) / 2.).size(f(SMALL_PX)).color(Token::Text2).center().middle().max_width(inner));
            at += f(SMALL_LINE);
        }
        if detailed.get(i).copied().unwrap_or(false) {
            // The zoom mark: a small magnifier in the top-right corner.
            let (mx, my) = (x + w - 12., top + 11.);
            canvas = canvas
                .draw(circle(mx - 1.5, my - 1.5, 3.5).stroke(Token::Text3, 1.2))
                .draw(Shape::line(mx + 1., my + 1., mx + 4., my + 4.).stroke(Token::Text3, 1.2));
        }
        hits.push(HitBox { index: i, key: b.id.clone(), rect: *r, label: b.name.clone() });
    }
    (canvas, hits)
}

/// The whole drawing's bounds, with the margin.
pub fn bounds(placed: &Placed) -> (f32, f32, f32, f32) {
    (-MARGIN, -MARGIN, f(placed.width) + 2. * MARGIN, f(placed.height) + 2. * MARGIN)
}

fn f(v: f64) -> f32 {
    v as f32
}

fn title(name: &str, kind: Option<&str>) -> String {
    match kind {
        Some(k) => format!("{name} [{k}]"),
        None => name.to_string(),
    }
}

/// An arrowhead at `to`, coming from `from`.
fn head(from: Point, to: Point) -> Shape {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let (ux, uy) = (f(dx / len), f(dy / len));
    let (px_, py_) = (-uy, ux);
    let (x, y) = (f(to.x), f(to.y));
    let (l, w) = (9., 4.5);
    Shape::path()
        .move_to(x, y)
        .line_to(x - ux * l + px_ * w, y - uy * l + py_ * w)
        .line_to(x - ux * l - px_ * w, y - uy * l - py_ * w)
        .close()
        .fill(Token::Text3)
}

/// A circle as four cubic curves.
fn circle(cx: f32, cy: f32, r: f32) -> Shape {
    let k = 0.552_284_8 * r;
    Shape::path()
        .move_to(cx + r, cy)
        .cubic_to(cx + r, cy + k, cx + k, cy + r, cx, cy + r)
        .cubic_to(cx - k, cy + r, cx - r, cy + k, cx - r, cy)
        .cubic_to(cx - r, cy - k, cx - k, cy - r, cx, cy - r)
        .cubic_to(cx + k, cy - r, cx + r, cy - k, cx + r, cy)
        .close()
}

/// A cylinder's outline: an elliptical top and bottom `cap` high.
fn cylinder(x: f32, y: f32, w: f32, h: f32, cap: f32) -> Shape {
    let half = cap / 2.;
    Shape::path()
        .move_to(x, y + half)
        .cubic_to(x, y, x + w, y, x + w, y + half)
        .line_to(x + w, y + h - half)
        .cubic_to(x + w, y + h, x, y + h, x, y + h - half)
        .close()
}
