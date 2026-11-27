//! Sizing and layout of structure diagrams behind the [`Layout`] trait
//! (layout-lld.md §2–§4, §6).

mod dagre;
#[cfg(feature = "sugiyama")]
mod sugiyama;

pub use self::dagre::Dagre;
#[cfg(feature = "sugiyama")]
pub use self::sugiyama::Sugiyama;

use crate::c4::{self, Shape, Structure};
use crate::mermaid::flowchart::Direction;
use crate::text::{text_width, wrap, Weight};

/// Slack on measured widths (hla.md §3.7).
const SLACK: f64 = 1.08;
pub const NAME_PX: f64 = 14.0;
pub const NAME_LINE: f64 = 18.0;
pub const SMALL_PX: f64 = 12.0;
pub const SMALL_LINE: f64 = 16.0;
pub const BOX_PAD_X: f64 = 12.0;
pub const BOX_PAD_Y: f64 = 10.0;
pub const BOX_MIN_W: f64 = 120.0;
pub const BOX_MAX_W: f64 = 240.0;
pub const PERSON_HEAD: f64 = 24.0;
pub const CYLINDER_CAP: f64 = 12.0;
pub const PIPE_ENDS: f64 = 16.0;
pub const LABEL_MAX_W: f64 = 200.0;
pub const LABEL_H: f64 = 18.0;
pub const TITLE_PX: f64 = 13.0;
pub const TITLE_BAND: f64 = 24.0;
pub const CLUSTER_PAD: f64 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    /// Whether the two overlap by more than `eps` on both axes.
    pub fn overlaps(&self, other: &Rect, eps: f64) -> bool {
        self.x + eps < other.right()
            && other.x + eps < self.right()
            && self.y + eps < other.bottom()
            && other.y + eps < self.bottom()
    }

    /// Whether `other` lies inside, give or take `eps`.
    pub fn contains(&self, other: &Rect, eps: f64) -> bool {
        other.x + eps >= self.x
            && other.y + eps >= self.y
            && other.right() <= self.right() + eps
            && other.bottom() <= self.bottom() + eps
    }

    pub fn contains_point(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }

    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let (right, bottom) = (self.right().max(other.right()), self.bottom().max(other.bottom()));
        Rect { x, y, w: right - x, h: bottom - y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// An edge end in a [`LayoutGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Node(usize),
    Cluster(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutNode {
    pub w: f64,
    pub h: f64,
    pub parent: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutCluster {
    pub parent: Option<usize>,
    /// Width of the title, so the frame is at least that wide.
    pub title_w: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutEdge {
    pub from: End,
    pub to: End,
    /// The label's size, if it has one.
    pub label: Option<(f64, f64)>,
}

/// What a layout engine gets: sizes and nesting, no text.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutGraph {
    pub direction: Direction,
    pub nodes: Vec<LayoutNode>,
    pub clusters: Vec<LayoutCluster>,
    pub edges: Vec<LayoutEdge>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedEdge {
    pub points: Vec<Point>,
    pub label: Option<Rect>,
}

/// What a layout engine returns, index for index with the graph.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub nodes: Vec<Rect>,
    pub clusters: Vec<Rect>,
    pub edges: Vec<PlacedEdge>,
    pub width: f64,
    pub height: f64,
}

/// A layout engine (hla.md §3.5).
pub trait Layout {
    fn name(&self) -> &'static str;
    fn layout(&self, graph: &LayoutGraph) -> Placed;
}

/// A box's text as it will be drawn, and its size.
#[derive(Debug, Clone, PartialEq)]
pub struct SizedBox {
    pub name: String,
    pub kind: Option<String>,
    pub purpose: Vec<String>,
    pub w: f64,
    pub h: f64,
}

/// Sizes a box from its Name, Kind and purpose (layout-lld.md §2).
pub fn size_box(b: &c4::C4Box) -> SizedBox {
    let inner = BOX_MAX_W - 2.0 * BOX_PAD_X;
    let name_lines = wrap(&b.name, NAME_PX, Weight::SemiBold, inner / SLACK, 1);
    let name = name_lines.into_iter().next().unwrap_or_default();
    let kind = b.kind.as_ref().map(|k| format!("[{}]", k.text));
    let purpose = b
        .purpose
        .as_deref()
        .map(|p| wrap(p, SMALL_PX, Weight::Regular, inner / SLACK, 2))
        .unwrap_or_default();
    let mut widest = text_width(&name, NAME_PX, Weight::SemiBold);
    let mut h = 2.0 * BOX_PAD_Y + NAME_LINE;
    for line in kind.iter().chain(purpose.iter()) {
        widest = widest.max(text_width(line, SMALL_PX, Weight::Regular));
        h += SMALL_LINE;
    }
    let mut w = (widest * SLACK + 2.0 * BOX_PAD_X).clamp(BOX_MIN_W, BOX_MAX_W);
    match b.shape {
        Shape::Person => h += PERSON_HEAD,
        Shape::Cylinder => h += CYLINDER_CAP,
        Shape::Pipe => w += PIPE_ENDS,
        Shape::Box => {}
    }
    SizedBox { name, kind, purpose, w, h }
}

/// Builds the layout graph of a structure diagram: a node per box, a cluster
/// per boundary, an edge per arrow, index for index.
pub fn build(s: &Structure) -> (LayoutGraph, Vec<SizedBox>) {
    let sized: Vec<SizedBox> = s.boxes.iter().map(size_box).collect();
    let nodes = s
        .boxes
        .iter()
        .zip(&sized)
        .map(|(b, sz)| LayoutNode { w: sz.w, h: sz.h, parent: b.boundary })
        .collect();
    let clusters = s
        .boundaries
        .iter()
        .map(|b| {
            let title = match &b.kind {
                Some(k) => format!("{} [{}]", b.name, k.text),
                None => b.name.clone(),
            };
            let title_w = text_width(&title, TITLE_PX, Weight::SemiBold) * SLACK;
            LayoutCluster { parent: b.parent, title_w }
        })
        .collect();
    let end = |e: c4::End| match e {
        c4::End::Box(i) => End::Node(i),
        c4::End::Boundary(i) => End::Cluster(i),
    };
    let edges = s
        .arrows
        .iter()
        .map(|a| LayoutEdge {
            from: end(a.from),
            to: end(a.to),
            label: a.label.as_deref().map(|l| {
                let w = (text_width(l, SMALL_PX, Weight::Regular) * SLACK + 8.0).min(LABEL_MAX_W);
                (w, LABEL_H)
            }),
        })
        .collect();
    (LayoutGraph { direction: s.direction, nodes, clusters, edges }, sized)
}

impl LayoutGraph {
    /// Whether cluster `inner` is `outer` or nested in it.
    pub fn cluster_within(&self, inner: usize, outer: usize) -> bool {
        let mut at = Some(inner);
        while let Some(c) = at {
            if c == outer {
                return true;
            }
            at = self.clusters[c].parent;
        }
        false
    }

    /// Whether node `n` is inside cluster `c`, directly or not.
    pub fn node_within(&self, n: usize, c: usize) -> bool {
        self.nodes[n].parent.is_some_and(|p| self.cluster_within(p, c))
    }

    pub fn depth(&self, c: usize) -> usize {
        let mut depth = 0;
        let mut at = self.clusters[c].parent;
        while let Some(p) = at {
            depth += 1;
            at = self.clusters[p].parent;
        }
        depth
    }
}

/// Checks a layout (layout-lld.md §6): no boxes overlap, boxes and nested
/// boundaries sit inside their boundary, sibling boundaries and boxes
/// outside a boundary don't overlap it. Returns what's wrong.
pub fn check(g: &LayoutGraph, p: &Placed) -> Vec<String> {
    const EPS: f64 = 0.5;
    let mut problems = Vec::new();
    for i in 0..p.nodes.len() {
        for j in i + 1..p.nodes.len() {
            if p.nodes[i].overlaps(&p.nodes[j], EPS) {
                problems.push(format!("boxes {i} and {j} overlap"));
            }
        }
        if let Some(c) = g.nodes[i].parent {
            if !p.clusters[c].contains(&p.nodes[i], EPS) {
                problems.push(format!("box {i} is outside boundary {c}"));
            }
        }
    }
    for c in 0..p.clusters.len() {
        if let Some(parent) = g.clusters[c].parent {
            if !p.clusters[parent].contains(&p.clusters[c], EPS) {
                problems.push(format!("boundary {c} is outside boundary {parent}"));
            }
        }
        for d in c + 1..p.clusters.len() {
            let nested = g.cluster_within(c, d) || g.cluster_within(d, c);
            if !nested && p.clusters[c].overlaps(&p.clusters[d], EPS) {
                problems.push(format!("boundaries {c} and {d} overlap"));
            }
        }
        for n in 0..p.nodes.len() {
            if !g.node_within(n, c) && p.clusters[c].overlaps(&p.nodes[n], EPS) {
                problems.push(format!("box {n} overlaps boundary {c}"));
            }
        }
    }
    problems
}
