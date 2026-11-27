//! Drawing: a [`canvas()`] element with shapes, lines and text the tile
//! places itself, clickable [`Region`]s over them, and a [`Viewport`] for
//! zoom and pan.
//!
//! ```ignore
//! use pitwall_extension::prelude::*;
//! use pitwall_extension::canvas::{Label, Region, Shape};
//!
//! canvas()
//!     .size_full()
//!     .transform(self.viewport)
//!     .draw(Shape::rect(40., 40., 160., 64.).fill(Token::Surface2).stroke(Token::Line, 1.).radius(6.))
//!     .draw(Label::new("API", 120., 72.).center().middle().semibold())
//!     .draw(Shape::path().move_to(200., 72.).line_to(280., 72.).stroke(Token::Text3, 1.5).dash([4., 3.]))
//!     .region(Region::new("api", 40., 40., 160., 64., "API").on_click(cx.listener(|this, _, cx| this.open("api", cx))))
//!     .on_pointer(cx.listener(|this, p, cx| if this.viewport.on_pointer(p) { cx.notify() }))
//!     .on_wheel(cx.listener(|this, p, cx| if this.viewport.on_pointer(p) { cx.notify() }))
//! ```
//!
//! Coordinates are logical pixels in the canvas's own space; the canvas's
//! [`Viewport`] maps them to the node (zoom and pan), so a tile draws its
//! diagram once in its own coordinates and moves the viewport. Events
//! ([`Pointer`], [`Click`](crate::events::Click)) come in the node's
//! coordinates: [`Viewport::to_canvas`] turns them back.
//!
//! Colours are [`Paint`]s: a theme [`Token`] (best: the drawing follows
//! light and dark) or a fixed colour. Text is shaped by Pitwall in its own
//! fonts (Inter for the UI font): to size boxes around labels before
//! drawing, measure with the same font's advance widths (the C4 tile keeps
//! a table of Inter's, generated at build time).

use crate::bindings::pitwall::tile::types as wit;
use crate::element::{builder, AnyElement, Handler, Interactive, IntoElement, Kind};
use crate::events::{Click, Hover, MouseButton, Pointer, PointerKind};
use crate::style::{StyleSet, Styled, Token};

/// A colour in a canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Paint {
    /// A theme colour (follows light and dark).
    Token(Token),
    /// A fixed colour, `0xRRGGBBAA`, the same in both themes.
    Rgba(u32),
}

impl Paint {
    /// A fixed colour from its red, green and blue (0–255), opaque.
    pub fn rgb(r: u8, g: u8, b: u8) -> Paint {
        Paint::Rgba(u32::from_be_bytes([r, g, b, 0xff]))
    }

    fn wit(self) -> wit::Paint {
        match self {
            Paint::Token(t) => wit::Paint::Token(t.wit()),
            Paint::Rgba(c) => wit::Paint::Rgba(c),
        }
    }
}

impl From<Token> for Paint {
    fn from(t: Token) -> Paint {
        Paint::Token(t)
    }
}

fn point(x: f32, y: f32) -> wit::Point {
    wit::Point { x, y }
}

/// A rectangle or a path to draw: [`Shape::rect`], [`Shape::path`].
#[derive(Debug, Clone)]
pub struct Shape {
    rect: Option<wit::Rect>,
    radius: f32,
    steps: Vec<wit::PathStep>,
    fill: Option<Paint>,
    stroke: Option<(Paint, f32)>,
    dash: Vec<f32>,
}

impl Shape {
    /// A rectangle.
    pub fn rect(x: f32, y: f32, width: f32, height: f32) -> Shape {
        Shape { rect: Some(wit::Rect { x, y, width, height }), radius: 0., steps: Vec::new(), fill: None, stroke: None, dash: Vec::new() }
    }

    /// A path: start it with [`Shape::move_to`].
    pub fn path() -> Shape {
        Shape { rect: None, radius: 0., steps: Vec::new(), fill: None, stroke: None, dash: Vec::new() }
    }

    /// A straight line from (x1, y1) to (x2, y2): give it a
    /// [`stroke`](Shape::stroke).
    pub fn line(x1: f32, y1: f32, x2: f32, y2: f32) -> Shape {
        Shape::path().move_to(x1, y1).line_to(x2, y2)
    }

    /// Start a new piece of the path at (x, y).
    pub fn move_to(mut self, x: f32, y: f32) -> Shape {
        self.steps.push(wit::PathStep::MoveTo(point(x, y)));
        self
    }

    /// A straight line to (x, y).
    pub fn line_to(mut self, x: f32, y: f32) -> Shape {
        self.steps.push(wit::PathStep::LineTo(point(x, y)));
        self
    }

    /// A quadratic curve to (x, y) bending towards (cx, cy).
    pub fn quad_to(mut self, cx: f32, cy: f32, x: f32, y: f32) -> Shape {
        self.steps.push(wit::PathStep::QuadTo(wit::QuadTo { control: point(cx, cy), to: point(x, y) }));
        self
    }

    /// A cubic curve to (x, y) with control points (c1x, c1y), (c2x, c2y).
    pub fn cubic_to(mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) -> Shape {
        self.steps.push(wit::PathStep::CubicTo(wit::CubicTo { control1: point(c1x, c1y), control2: point(c2x, c2y), to: point(x, y) }));
        self
    }

    /// A line back to the piece's start.
    pub fn close(mut self) -> Shape {
        self.steps.push(wit::PathStep::Close);
        self
    }

    /// Fill it.
    pub fn fill(mut self, paint: impl Into<Paint>) -> Shape {
        self.fill = Some(paint.into());
        self
    }

    /// Draw its outline (or the path's line) `width` pixels wide.
    pub fn stroke(mut self, paint: impl Into<Paint>, width: f32) -> Shape {
        self.stroke = Some((paint.into(), width));
        self
    }

    /// Dash the line: lengths drawn and left out, in turn (`[4., 3.]`; at
    /// most 16).
    pub fn dash(mut self, pattern: impl IntoIterator<Item = f32>) -> Shape {
        self.dash = pattern.into_iter().collect();
        self
    }

    /// Round a rectangle's corners.
    pub fn radius(mut self, r: f32) -> Shape {
        self.radius = r;
        self
    }

    fn wit(self) -> wit::DrawOp {
        let stroke = self.stroke.map(|(p, width)| wit::Stroke { color: p.wit(), width, dash: self.dash });
        let fill = self.fill.map(Paint::wit);
        match self.rect {
            Some(rect) => wit::DrawOp::Rect(wit::DrawRect { rect, fill, stroke, radius: self.radius }),
            None => wit::DrawOp::Path(wit::DrawPath { steps: self.steps, fill, stroke }),
        }
    }
}

/// One line of text to draw, shaped by Pitwall in its fonts. By default
/// 13 px (Pitwall's body size), the UI font, the theme's text colour,
/// starting at its point with the point at the line's top.
#[derive(Debug, Clone)]
pub struct Label(wit::DrawText);

impl Label {
    /// `text` at (x, y).
    pub fn new(text: impl Into<String>, x: f32, y: f32) -> Label {
        Label(wit::DrawText {
            text: text.into(),
            at: point(x, y),
            size: 13.,
            color: wit::Paint::Token(wit::Token::Text),
            font: wit::Font::Ui,
            weight: wit::Weight::Normal,
            align: wit::TextAlign::Start,
            baseline: wit::TextBaseline::Top,
            max_width: None,
        })
    }

    /// Its size in pixels (scaled with the viewport). Pitwall's sizes are
    /// 11, 12, 13 (body), 16 and 20.
    pub fn size(mut self, px: f32) -> Label {
        self.0.size = px;
        self
    }

    /// Its colour.
    pub fn color(mut self, paint: impl Into<Paint>) -> Label {
        self.0.color = paint.into().wit();
        self
    }

    /// The monospace font.
    pub fn mono(mut self) -> Label {
        self.0.font = wit::Font::Mono;
        self
    }

    /// Medium weight: labels.
    pub fn medium(mut self) -> Label {
        self.0.weight = wit::Weight::Medium;
        self
    }

    /// Semibold: names, headings.
    pub fn semibold(mut self) -> Label {
        self.0.weight = wit::Weight::Semibold;
        self
    }

    /// Bold.
    pub fn bold(mut self) -> Label {
        self.0.weight = wit::Weight::Bold;
        self
    }

    /// Centred on its point, across.
    pub fn center(mut self) -> Label {
        self.0.align = wit::TextAlign::Center;
        self
    }

    /// Ending at its point.
    pub fn end(mut self) -> Label {
        self.0.align = wit::TextAlign::End;
        self
    }

    /// Its line's middle at its point.
    pub fn middle(mut self) -> Label {
        self.0.baseline = wit::TextBaseline::Middle;
        self
    }

    /// Its line's bottom at its point.
    pub fn bottom(mut self) -> Label {
        self.0.baseline = wit::TextBaseline::Bottom;
        self
    }

    /// At most this wide (in canvas coordinates): longer text is cut with
    /// an ellipsis.
    pub fn max_width(mut self, w: f32) -> Label {
        self.0.max_width = Some(w);
        self
    }
}

/// Something a canvas draws: a [`Shape`] or a [`Label`].
#[derive(Debug, Clone)]
pub struct DrawOp(pub(crate) wit::DrawOp);

impl From<Shape> for DrawOp {
    fn from(s: Shape) -> DrawOp {
        DrawOp(s.wit())
    }
}

impl From<Label> for DrawOp {
    fn from(l: Label) -> DrawOp {
        DrawOp(wit::DrawOp::Text(l.0))
    }
}

/// A clickable area over a canvas's drawing (a box of a diagram). Its
/// label is its accessible name, read by screen readers, and its tooltip
/// unless it has another.
#[derive(Debug, Clone)]
pub struct Region {
    pub(crate) key: String,
    pub(crate) rect: wit::Rect,
    pub(crate) label: String,
    pub(crate) tooltip: Option<String>,
    pub(crate) click: Option<(u32, u64)>,
    pub(crate) hover: Option<(u32, u64)>,
}

impl Region {
    /// The area (x, y, width, height) in canvas coordinates, with `key`
    /// (unique in the canvas) and accessible name `label`.
    pub fn new(key: impl Into<String>, x: f32, y: f32, width: f32, height: f32, label: impl Into<String>) -> Region {
        Region { key: key.into(), rect: wit::Rect { x, y, width, height }, label: label.into(), tooltip: None, click: None, hover: None }
    }

    /// Text shown when the pointer rests on it (instead of its label).
    pub fn tooltip(mut self, text: impl Into<String>) -> Region {
        self.tooltip = Some(text.into());
        self
    }

    /// Call `handler` when it is clicked ([`Click::at`] is from the canvas
    /// node's corner). A press that dragged (to pan) isn't a click.
    pub fn on_click(mut self, handler: Handler<Click>) -> Region {
        self.click = Some((handler.slot, handler.generation));
        self
    }

    /// Call `handler` when the pointer enters or leaves it.
    pub fn on_hover(mut self, handler: Handler<Hover>) -> Region {
        self.hover = Some((handler.slot, handler.generation));
        self
    }
}

/// What a canvas holds before the frame is built.
#[derive(Debug, Clone)]
pub(crate) struct CanvasSpec {
    pub ops: Vec<wit::DrawOp>,
    pub transform: wit::Transform,
    pub regions: Vec<Region>,
}

/// A drawing. Made by [`canvas()`].
#[derive(Debug, Clone)]
pub struct Canvas(AnyElement);
builder!(Canvas);

impl Canvas {
    fn spec(&mut self) -> &mut CanvasSpec {
        match &mut self.0.kind {
            Kind::Canvas(s) => s,
            _ => unreachable!("a canvas"),
        }
    }

    /// Draw `op` (a [`Shape`] or a [`Label`]) over what is drawn so far.
    pub fn draw(mut self, op: impl Into<DrawOp>) -> Canvas {
        self.spec().ops.push(op.into().0);
        self
    }

    /// Draw each of `ops`, in order.
    pub fn draws<I>(mut self, ops: I) -> Canvas
    where
        I: IntoIterator,
        I::Item: Into<DrawOp>,
    {
        self.spec().ops.extend(ops.into_iter().map(|o| o.into().0));
        self
    }

    /// Draw only inside (x, y, width, height) until [`Canvas::clip_pop`].
    pub fn clip_push(mut self, x: f32, y: f32, width: f32, height: f32) -> Canvas {
        self.spec().ops.push(wit::DrawOp::ClipPush(wit::Rect { x, y, width, height }));
        self
    }

    /// End the newest [`Canvas::clip_push`].
    pub fn clip_pop(mut self) -> Canvas {
        self.spec().ops.push(wit::DrawOp::ClipPop);
        self
    }

    /// Add a clickable area; where areas overlap, the one added last wins.
    pub fn region(mut self, r: Region) -> Canvas {
        self.spec().regions.push(r);
        self
    }

    /// Map the canvas's coordinates to the node with `viewport` (zoom and
    /// pan).
    pub fn transform(mut self, viewport: Viewport) -> Canvas {
        self.spec().transform = wit::Transform { scale: viewport.scale, dx: viewport.x, dy: viewport.y };
        self
    }
}

/// A drawing: shapes, lines and text placed by the tile, with clickable
/// regions over them. Give it a size (`.size_full()`, `.w(..).h(..)` or
/// `.flex_1()`). At most 20 000 draw ops and 5 000 regions.
pub fn canvas() -> Canvas {
    let spec = CanvasSpec { ops: Vec::new(), transform: wit::Transform { scale: 1., dx: 0., dy: 0. }, regions: Vec::new() };
    Canvas(AnyElement::new_kind(Kind::Canvas(Box::new(spec))))
}

/// A zoom and a pan: how a canvas's coordinates map to its node. Keep one
/// in the tile's state, pass it to [`Canvas::transform`], and feed it the
/// canvas's pointer and wheel events with [`Viewport::on_pointer`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// The zoom (1.0: canvas pixels are node pixels).
    pub scale: f32,
    /// Where the canvas's origin is, from the node's left edge.
    pub x: f32,
    /// Where the canvas's origin is, from the node's top edge.
    pub y: f32,
    /// The smallest zoom [`Viewport::zoom_at`] allows.
    pub min_scale: f32,
    /// The largest zoom.
    pub max_scale: f32,
    /// Where a drag to pan started (node coordinates), while one is on.
    drag: Option<(f32, f32)>,
    /// The drag went further than a click's slack.
    dragged: bool,
}

impl Default for Viewport {
    fn default() -> Viewport {
        Viewport { scale: 1., x: 0., y: 0., min_scale: 0.1, max_scale: 8., drag: None, dragged: false }
    }
}

impl Viewport {
    /// A canvas point in node coordinates.
    pub fn to_view(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.scale + self.x, y * self.scale + self.y)
    }

    /// A node point (an event's `at`) in canvas coordinates.
    pub fn to_canvas(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.x) / self.scale, (y - self.y) / self.scale)
    }

    /// Zoom by `factor` (2.0: twice as large) keeping the node point
    /// (x, y) still, within the zoom limits.
    pub fn zoom_at(&mut self, factor: f32, x: f32, y: f32) {
        if !factor.is_finite() || factor <= 0. {
            return;
        }
        let (cx, cy) = self.to_canvas(x, y);
        self.scale = (self.scale * factor).clamp(self.min_scale, self.max_scale);
        self.x = x - cx * self.scale;
        self.y = y - cy * self.scale;
    }

    /// Move the drawing by (dx, dy) node pixels.
    pub fn pan(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
    }

    /// Fit the canvas rectangle (x, y, width, height) into a node of size
    /// (view_width, view_height) with `margin` pixels around, centred, no
    /// larger than `max_scale` (and at most 1.0 unless it already is
    /// above).
    #[allow(clippy::too_many_arguments)]
    pub fn fit(&mut self, x: f32, y: f32, width: f32, height: f32, view_width: f32, view_height: f32, margin: f32) {
        let (aw, ah) = ((view_width - 2. * margin).max(1.), (view_height - 2. * margin).max(1.));
        let s = (aw / width.max(1.)).min(ah / height.max(1.)).clamp(self.min_scale, self.max_scale.min(1.0_f32.max(self.min_scale)));
        self.scale = s;
        self.x = (view_width - width * s) / 2. - x * s;
        self.y = (view_height - height * s) / 2. - y * s;
    }

    /// Zoom and pan as Pitwall's own canvases do, from a canvas's
    /// [`on_pointer`](crate::Interactive::on_pointer) and
    /// [`on_wheel`](crate::Interactive::on_wheel) events: drag with the
    /// main button to pan, scroll to pan, ⌘ or Ctrl + scroll and pinch to
    /// zoom at the pointer. Whether the view changed (call
    /// [`Cx::notify`](crate::Cx::notify) then).
    pub fn on_pointer(&mut self, p: &Pointer) -> bool {
        match p.kind {
            PointerKind::Down(MouseButton::Left) => {
                self.drag = Some((p.at.x, p.at.y));
                self.dragged = false;
                false
            }
            PointerKind::Move => {
                let Some((x0, y0)) = self.drag else { return false };
                let (dx, dy) = (p.at.x - x0, p.at.y - y0);
                if !self.dragged && dx.abs() + dy.abs() < 4. {
                    return false;
                }
                self.dragged = true;
                self.pan(dx, dy);
                self.drag = Some((p.at.x, p.at.y));
                true
            }
            PointerKind::Up(MouseButton::Left) => {
                self.drag = None;
                false
            }
            PointerKind::Wheel { dx, dy } => {
                if p.modifiers.platform || p.modifiers.ctrl {
                    self.zoom_at((-dy / 200.).exp(), p.at.x, p.at.y);
                } else {
                    self.pan(-dx, -dy);
                }
                true
            }
            PointerKind::Pinch(d) => {
                self.zoom_at(1. + d, p.at.x, p.at.y);
                true
            }
            _ => false,
        }
    }

    /// Whether the last press dragged further than a click's slack (a
    /// region's click after a pan isn't sent anyway; this is for clicks on
    /// the canvas itself).
    pub fn dragged(&self) -> bool {
        self.dragged
    }
}

impl From<Viewport> for wit::Transform {
    fn from(v: Viewport) -> wit::Transform {
        wit::Transform { scale: v.scale, dx: v.x, dy: v.y }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Modifiers;

    #[test]
    fn the_viewport_zooms_at_the_pointer_and_pans_by_dragging() {
        let mut v = Viewport::default();
        v.zoom_at(2., 100., 50.);
        assert_eq!(v.to_canvas(100., 50.), (100., 50.), "the point under the pointer stays");
        assert_eq!(v.scale, 2.);
        let p = |kind, x, y| Pointer { kind, at: crate::events::Point { x, y }, modifiers: Modifiers::default() };
        assert!(!v.on_pointer(&p(PointerKind::Down(MouseButton::Left), 10., 10.)));
        assert!(!v.on_pointer(&p(PointerKind::Move, 11., 11.)), "within a click's slack");
        assert!(v.on_pointer(&p(PointerKind::Move, 30., 10.)));
        assert!(v.dragged());
        assert_eq!((v.x, v.y), (-80., -50.));
        v.on_pointer(&p(PointerKind::Up(MouseButton::Left), 30., 10.));
        assert!(!v.on_pointer(&p(PointerKind::Move, 60., 10.)), "no button held");
        assert!(v.on_pointer(&p(PointerKind::Pinch(1.), 0., 0.)));
        assert_eq!(v.scale, 4.);
        v.fit(0., 0., 1000., 500., 500., 500., 0.);
        assert_eq!(v.scale, 0.5);
        assert_eq!(v.to_view(500., 250.), (250., 250.), "centred");
    }
}
