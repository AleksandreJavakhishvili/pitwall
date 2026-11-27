//! The Painter (hla.md §3.8, phase B): a laid-out structure diagram as
//! canvas draw operations, with a clickable region per box. Theme colours
//! only, text in pixels so it zooms, dashed boundaries and dotted arrows.
//! Behind the `tile` feature: it needs the Tile SDK.

use pitwall_extension::prelude::*;

use crate::c4::{Shape as BoxShape, Structure};
use crate::changes::Drawn;
use crate::layout::sequence::{self, Row, SeqPlaced, HEADER_H};
use crate::mermaid::sequence::{FrameKind, MessageHead, Sequence, Step};
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
    canvas: Canvas,
    s: &Structure,
    sized: &[SizedBox],
    placed: &Placed,
    detailed: &[bool],
    selected: Option<usize>,
) -> (Canvas, Vec<HitBox>) {
    paint_with(canvas, s, sized, placed, &Look { detailed, selected, changes: None })
}

/// How a structure is drawn beyond its own classes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Look<'a> {
    /// Box `i` has a detail diagram: the zoom mark (FR-12).
    pub detailed: &'a [bool],
    /// The selected box, outlined.
    pub selected: Option<usize>,
    /// The Changes view (FR-17, FR-40): how each box and arrow is drawn,
    /// instead of the `new` and `changed` classes.
    pub changes: Option<(&'a [Drawn], &'a [Drawn])>,
}

/// A box's fill, its outline and its outline's dashes (FR-8, FR-16).
fn box_colours(b: &crate::c4::C4Box, drawn: Option<Drawn>) -> (Token, Token, Option<[f32; 2]>) {
    match drawn {
        Some(Drawn::Added) => (Token::GoodSoft, Token::Good, None),
        Some(Drawn::Changed) => (Token::WarnSoft, Token::Warn, None),
        Some(Drawn::Removed) => (Token::Surface, Token::Text4, Some([4., 3.])),
        None if b.marks.new => (Token::GoodSoft, Token::Good, None),
        None if b.marks.changed => (Token::WarnSoft, Token::Warn, None),
        _ if b.marks.external => (Token::Surface3, Token::LineStrong, Some([4., 3.])),
        _ => (Token::Surface2, Token::LineStrong, None),
    }
}

/// [`paint`] with a [`Look`].
pub fn paint_with(mut canvas: Canvas, s: &Structure, sized: &[SizedBox], placed: &Placed, look: &Look) -> (Canvas, Vec<HitBox>) {
    let (detailed, selected) = (look.detailed, look.selected);
    let box_drawn = |i: usize| look.changes.and_then(|(b, _)| b.get(i).copied());
    let arrow_drawn = |i: usize| look.changes.and_then(|(_, a)| a.get(i).copied());
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
    for (ai, (a, e)) in s.arrows.iter().zip(&placed.edges).enumerate() {
        if a.style == LineStyle::Invisible || e.points.len() < 2 {
            continue;
        }
        let removed = arrow_drawn(ai) == Some(Drawn::Removed);
        let colour = match arrow_drawn(ai) {
            Some(Drawn::Added) => Token::Good,
            Some(Drawn::Changed) => Token::Warn,
            Some(Drawn::Removed) => Token::Text4,
            _ => Token::Text3,
        };
        let mut line = Shape::path().move_to(f(e.points[0].x), f(e.points[0].y));
        for p in &e.points[1..] {
            line = line.line_to(f(p.x), f(p.y));
        }
        let width = if a.style == LineStyle::Thick { 2.5 } else { 1.5 };
        let mut line = line.stroke(colour, width);
        if a.style == LineStyle::Dotted || removed {
            line = line.dash([3., 3.]);
        }
        canvas = canvas.draw(line);
        let n = e.points.len();
        if a.end_head != Head::None {
            canvas = canvas.draw(head(e.points[n - 2], e.points[n - 1], colour));
        }
        if a.start_head != Head::None {
            canvas = canvas.draw(head(e.points[1], e.points[0], colour));
        }
        if let (Some(label), Some(r)) = (&a.label, &e.label) {
            let text = if removed { Token::Text4 } else { Token::Text2 };
            canvas = canvas
                .draw(Shape::rect(f(r.x), f(r.y), f(r.w), f(r.h)).fill(Token::Surface).radius(4.))
                .draw(Label::new(label.clone(), f(r.x + r.w / 2.), f(r.y + r.h / 2.)).size(f(SMALL_PX)).color(text).center().middle().max_width(f(r.w)));
        }
    }
    // Boxes.
    let mut hits = Vec::new();
    for (i, ((b, sz), r)) in s.boxes.iter().zip(sized).zip(&placed.nodes).enumerate() {
        let on = selected == Some(i);
        let drawn = box_drawn(i);
        let ghost = drawn == Some(Drawn::Removed);
        let (fill, outline, dash) = box_colours(b, drawn);
        let fill = if on { Token::DoneSoft } else { fill };
        let (stroke, width) = if on { (Token::Done, 2.) } else { (outline, 1.) };
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
                if let (Some(d), false) = (dash, on) {
                    rect = rect.dash(d);
                }
                canvas = canvas.draw(rect);
            }
        }
        let inner = w - 2. * f(BOX_PAD_X);
        let mut at = top + f(BOX_PAD_Y);
        let (name_c, kind_c, purpose_c) = if ghost { (Token::Text3, Token::Text4, Token::Text4) } else { (Token::Text, Token::Text3, Token::Text2) };
        canvas = canvas.draw(Label::new(sz.name.clone(), x + w / 2., at + f(NAME_LINE) / 2.).size(f(NAME_PX)).semibold().color(name_c).center().middle().max_width(inner));
        at += f(NAME_LINE);
        if let Some(kind) = &sz.kind {
            canvas = canvas.draw(Label::new(kind.clone(), x + w / 2., at + f(SMALL_LINE) / 2.).size(f(SMALL_PX)).color(kind_c).center().middle().max_width(inner));
            at += f(SMALL_LINE);
        }
        for line in &sz.purpose {
            canvas = canvas.draw(Label::new(line.clone(), x + w / 2., at + f(SMALL_LINE) / 2.).size(f(SMALL_PX)).color(purpose_c).center().middle().max_width(inner));
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
fn head(from: Point, to: Point, colour: Token) -> Shape {
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
        .fill(colour)
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

/// Draws a sequence diagram as [`sequence::layout`] places it: `box`
/// groups, lifelines, frames, activations, messages, notes, then the
/// headers on top. `selected` outlines a participant. Returns the canvas
/// and a region per participant (its header), for the footer.
pub fn paint_sequence(mut canvas: Canvas, s: &Sequence, p: &SeqPlaced, selected: Option<usize>) -> (Canvas, Vec<HitBox>) {
    for g in &p.groups {
        let r = g.rect;
        canvas = canvas
            .draw(Shape::rect(f(r.x), f(r.y), f(r.w), f(r.h)).fill(Token::Surface2).stroke(Token::Line, 1.).radius(8.))
            .draw(Label::new(g.label.clone(), f(r.x) + 10., f(r.y) + 12.).size(f(TITLE_PX)).semibold().color(Token::Text2).middle().max_width(f(r.w) - 20.));
    }
    for h in &p.headers {
        let top = h.rect.y + HEADER_H;
        canvas = canvas.draw(Shape::line(f(h.x), f(top), f(h.x), f(p.bottom)).stroke(Token::LineStrong, 1.).dash([4., 4.]));
    }
    for fr in &p.frames {
        let r = fr.rect;
        let (x, y, w, h) = (f(r.x), f(r.y), f(r.w), f(r.h));
        canvas = canvas.draw(Shape::rect(x, y, w, h).stroke(Token::LineStrong, 1.).radius(4.));
        let word = sequence::frame_word(fr.kind);
        if fr.kind != FrameKind::Rect {
            let tag_w = word.len() as f32 * 7.5 + 16.;
            canvas = canvas
                .draw(Shape::path().move_to(x, y).line_to(x + tag_w, y).line_to(x + tag_w, y + 12.).line_to(x + tag_w - 6., y + 18.).line_to(x, y + 18.).close().fill(Token::Surface3))
                .draw(Label::new(word, x + 8., y + 9.).size(f(SMALL_PX)).semibold().color(Token::Text2).middle());
            if !fr.label.is_empty() {
                canvas = canvas.draw(Label::new(format!("[{}]", fr.label), x + tag_w + 8., y + 9.).size(f(SMALL_PX)).color(Token::Text2).middle().max_width(w - tag_w - 16.));
            }
        }
        for (by, label) in &fr.branches {
            let by = f(*by);
            canvas = canvas.draw(Shape::line(x, by, x + w, by).stroke(Token::LineStrong, 1.).dash([4., 3.]));
            if !label.is_empty() {
                canvas = canvas.draw(Label::new(format!("[{label}]"), x + 8., by + 10.).size(f(SMALL_PX)).color(Token::Text2).middle().max_width(w - 16.));
            }
        }
    }
    for a in &p.activations {
        canvas = canvas.draw(Shape::rect(f(a.x), f(a.y), f(a.w), f(a.h)).fill(Token::Surface3).stroke(Token::LineStrong, 1.));
    }
    for row in &p.rows {
        match row {
            Row::Message { step, from_x, to_x, y, y2, label_x, label_y, label } => {
                let Some(Step::Message { dotted, head: kind, .. }) = s.steps.get(*step) else { continue };
                let (fx, tx, ly, ly2) = (f(*from_x), f(*to_x), f(*y), f(*y2));
                let mut line = if fx == tx {
                    Shape::path().move_to(fx, ly).line_to(fx + 36., ly).line_to(fx + 36., ly2).line_to(fx + 2., ly2)
                } else {
                    Shape::line(fx, ly, tx, ly)
                }
                .stroke(Token::Text2, 1.5);
                if *dotted {
                    line = line.dash([4., 3.]);
                }
                canvas = canvas.draw(line);
                let (from, to) = if fx == tx {
                    (Point { x: *from_x + 36., y: *y2 }, Point { x: *from_x + 2., y: *y2 })
                } else {
                    (Point { x: *from_x, y: *y }, Point { x: *to_x, y: *y })
                };
                canvas = match kind {
                    MessageHead::Arrow => canvas.draw(head(from, to, Token::Text2)),
                    MessageHead::Async => canvas.draw(open_head(from, to)),
                    MessageHead::Cross => {
                        let (cx, cy) = (f(to.x) - (f(to.x) - f(from.x)).signum() * 6., f(to.y));
                        canvas.draw(Shape::line(cx - 4., cy - 4., cx + 4., cy + 4.).stroke(Token::Text2, 1.5)).draw(Shape::line(cx - 4., cy + 4., cx + 4., cy - 4.).stroke(Token::Text2, 1.5))
                    }
                    MessageHead::None => canvas,
                };
                let mut l = Label::new(label.clone(), f(*label_x), f(*label_y)).size(f(SMALL_PX)).color(Token::Text).middle();
                if fx != tx {
                    l = l.center().max_width((tx - fx).abs() - 8.);
                }
                canvas = canvas.draw(l);
            }
            Row::Note { rect, text, .. } => {
                let r = rect;
                canvas = canvas
                    .draw(Shape::rect(f(r.x), f(r.y), f(r.w), f(r.h)).fill(Token::WarnSoft).stroke(Token::Line, 1.).radius(4.))
                    .draw(Label::new(text.clone(), f(r.x + r.w / 2.), f(r.y + r.h / 2.)).size(f(SMALL_PX)).color(Token::Text).center().middle().max_width(f(r.w) - 8.));
            }
        }
    }
    let mut hits = Vec::new();
    for (i, (part, h)) in s.participants.iter().zip(&p.headers).enumerate() {
        let r = h.rect;
        let on = selected == Some(i);
        let (fill, stroke, width) = if on { (Token::DoneSoft, Token::Done, 2.) } else { (Token::Surface2, Token::LineStrong, 1.) };
        let (x, y, w) = (f(r.x), f(r.y), f(r.w));
        if part.actor {
            canvas = canvas.draw(circle(x + w / 2., y + 8., 6.).fill(fill).stroke(stroke, width));
            canvas = canvas.draw(Label::new(part.label.clone(), x + w / 2., y + 26.).size(13.).semibold().center().middle().max_width(w));
        } else {
            canvas = canvas
                .draw(Shape::rect(x, y, w, f(r.h)).fill(fill).stroke(stroke, width).radius(6.))
                .draw(Label::new(part.label.clone(), x + w / 2., y + f(r.h) / 2.).size(13.).semibold().center().middle().max_width(w - 8.));
        }
        hits.push(HitBox { index: i, key: format!("p-{}", part.id), rect: r, label: part.label.clone() });
    }
    (canvas, hits)
}

/// The bounds of a laid-out sequence diagram, with the margin.
pub fn sequence_bounds(p: &SeqPlaced) -> (f32, f32, f32, f32) {
    (-MARGIN, -MARGIN, f(p.width) + 2. * MARGIN, f(p.height) + 2. * MARGIN)
}

/// An open arrowhead (an async message): two strokes.
fn open_head(from: Point, to: Point) -> Shape {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let (ux, uy) = (f(dx / len), f(dy / len));
    let (px_, py_) = (-uy, ux);
    let (x, y) = (f(to.x), f(to.y));
    Shape::path().move_to(x - ux * 9. + px_ * 4.5, y - uy * 9. + py_ * 4.5).line_to(x, y).line_to(x - ux * 9. - px_ * 4.5, y - uy * 9. - py_ * 4.5).stroke(Token::Text2, 1.5)
}
