//! Layout with `rust-sugiyama`, kept to compare with dagre (layout-lld.md
//! §7). It has no compound graphs, so each boundary's inside is laid out
//! first and the boundary then counts as one big node one level up. Edges
//! are straight lines between box borders; labels sit at their middle.

use rust_sugiyama::configure::Config;

use super::dagre::finish;
use super::{End, Layout, LayoutGraph, Placed, PlacedEdge, Point, Rect, CLUSTER_PAD, TITLE_BAND};
use crate::mermaid::flowchart::Direction;

#[derive(Debug, Clone)]
pub struct Sugiyama {
    pub spacing: f64,
    pub margin: f64,
}

impl Default for Sugiyama {
    fn default() -> Self {
        Sugiyama { spacing: 50.0, margin: 16.0 }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Item {
    Node(usize),
    Cluster(usize),
}

impl Sugiyama {
    /// Lays out the items directly inside `level` (`None`: the top), with
    /// clusters already sized; returns each item's rect, relative to the
    /// level's origin, and the level's size.
    fn level(
        &self,
        g: &LayoutGraph,
        level: Option<usize>,
        sizes: &[(f64, f64)],
    ) -> (Vec<(Item, Rect)>, f64, f64) {
        let mut items: Vec<Item> = (0..g.nodes.len())
            .filter(|&n| g.nodes[n].parent == level)
            .map(Item::Node)
            .collect();
        items.extend(
            (0..g.clusters.len()).filter(|&c| g.clusters[c].parent == level).map(Item::Cluster),
        );
        let size = |item: Item| match item {
            Item::Node(n) => (g.nodes[n].w, g.nodes[n].h),
            Item::Cluster(c) => sizes[c],
        };
        let horizontal = matches!(g.direction, Direction::LeftRight | Direction::RightLeft);
        let vertices: Vec<(u32, (f64, f64))> = items
            .iter()
            .enumerate()
            .map(|(i, &item)| {
                let (w, h) = size(item);
                (i as u32, if horizontal { (h, w) } else { (w, h) })
            })
            .collect();
        // The item at this level that holds an edge end.
        let holder = |end: End| -> Option<u32> {
            items.iter().position(|&item| match (item, end) {
                (Item::Node(a), End::Node(b)) => a == b,
                (Item::Cluster(c), End::Node(n)) => g.node_within(n, c),
                (Item::Cluster(c), End::Cluster(k)) => g.cluster_within(k, c),
                (Item::Node(_), End::Cluster(_)) => false,
            })
            .map(|i| i as u32)
        };
        let mut edges: Vec<(u32, u32)> = Vec::new();
        for e in &g.edges {
            if let (Some(a), Some(b)) = (holder(e.from), holder(e.to)) {
                if a != b && !edges.contains(&(a, b)) {
                    edges.push((a, b));
                }
            }
        }
        let config = Config { vertex_spacing: self.spacing, ..Config::default() };
        let layouts = rust_sugiyama::from_vertices_and_edges(&vertices, &edges, &config);
        let mut rects = vec![Rect::default(); items.len()];
        let mut offset = 0.0;
        let mut height: f64 = 0.0;
        // Components side by side, in the order they come.
        for (coords, _, _) in layouts {
            let placed: Vec<(usize, Rect)> = coords
                .iter()
                .map(|&(i, (x, y))| {
                    let (w, h) = vertices[i].1;
                    (i, Rect { x: x - w / 2.0, y: y - h / 2.0, w, h })
                })
                .collect();
            let min_x = placed.iter().map(|p| p.1.x).fold(f64::INFINITY, f64::min);
            let min_y = placed.iter().map(|p| p.1.y).fold(f64::INFINITY, f64::min);
            let max_x = placed.iter().map(|p| p.1.right()).fold(f64::NEG_INFINITY, f64::max);
            for (i, mut r) in placed {
                r.x += offset - min_x;
                r.y -= min_y;
                height = height.max(r.bottom());
                rects[i] = r;
            }
            offset += max_x - min_x + self.spacing;
        }
        let width = (offset - self.spacing).max(0.0);
        let (mut width, mut height) = (width, height);
        if horizontal {
            for r in &mut rects {
                *r = Rect { x: r.y, y: r.x, w: r.h, h: r.w };
            }
            std::mem::swap(&mut width, &mut height);
        }
        for r in &mut rects {
            match g.direction {
                Direction::BottomUp => r.y = height - r.bottom(),
                Direction::RightLeft => r.x = width - r.right(),
                _ => {}
            }
        }
        (items.into_iter().zip(rects).collect(), width, height)
    }
}

/// Where a straight line from `from`'s centre towards `to` leaves `from`.
fn border_point(from: Rect, to: Point) -> Point {
    let c = Point { x: from.x + from.w / 2.0, y: from.y + from.h / 2.0 };
    let (dx, dy) = (to.x - c.x, to.y - c.y);
    if dx == 0.0 && dy == 0.0 {
        return c;
    }
    let sx = if dx != 0.0 { (from.w / 2.0) / dx.abs() } else { f64::INFINITY };
    let sy = if dy != 0.0 { (from.h / 2.0) / dy.abs() } else { f64::INFINITY };
    let s = sx.min(sy);
    Point { x: c.x + dx * s, y: c.y + dy * s }
}

impl Layout for Sugiyama {
    fn name(&self) -> &'static str {
        "rust-sugiyama"
    }

    fn layout(&self, g: &LayoutGraph) -> Placed {
        // Innermost clusters first: each gets its size and its items'
        // rects relative to its inside.
        let mut order: Vec<usize> = (0..g.clusters.len()).collect();
        order.sort_by_key(|&c| std::cmp::Reverse(g.depth(c)));
        let mut sizes = vec![(0.0, 0.0); g.clusters.len()];
        let mut inner: Vec<Vec<(Item, Rect)>> = vec![Vec::new(); g.clusters.len()];
        for c in order {
            let (items, w, h) = self.level(g, Some(c), &sizes);
            let w = (w + 2.0 * CLUSTER_PAD).max(g.clusters[c].title_w + 2.0 * CLUSTER_PAD);
            sizes[c] = (w, h + 2.0 * CLUSTER_PAD + TITLE_BAND);
            inner[c] = items;
        }
        let (top, _, _) = self.level(g, None, &sizes);
        let mut nodes = vec![Rect::default(); g.nodes.len()];
        let mut clusters = vec![Rect::default(); g.clusters.len()];
        // Place recursively: an item's rect, then its inside.
        let mut stack: Vec<(Item, Rect)> = top;
        while let Some((item, rect)) = stack.pop() {
            match item {
                Item::Node(n) => nodes[n] = rect,
                Item::Cluster(c) => {
                    clusters[c] = rect;
                    let (ox, oy) = (rect.x + CLUSTER_PAD, rect.y + CLUSTER_PAD + TITLE_BAND);
                    for &(child, r) in &inner[c] {
                        stack.push((child, Rect { x: r.x + ox, y: r.y + oy, ..r }));
                    }
                }
            }
        }
        let rect_of = |end: End| match end {
            End::Node(n) => nodes[n],
            End::Cluster(c) => clusters[c],
        };
        let edges = g
            .edges
            .iter()
            .map(|e| {
                let (a, b) = (rect_of(e.from), rect_of(e.to));
                let centre = |r: Rect| Point { x: r.x + r.w / 2.0, y: r.y + r.h / 2.0 };
                let start = border_point(a, centre(b));
                let end = border_point(b, centre(a));
                let label = e.label.map(|(w, h)| Rect {
                    x: (start.x + end.x) / 2.0 - w / 2.0,
                    y: (start.y + end.y) / 2.0 - h / 2.0,
                    w,
                    h,
                });
                PlacedEdge { points: vec![start, end], label }
            })
            .collect();
        finish(nodes, clusters, edges, self.margin)
    }
}
