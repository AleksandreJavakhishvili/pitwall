//! Layout with the `dagre` crate, a port of dagre.js (layout-lld.md §4).

use dagre::graph::{Graph, GraphOptions};
use dagre::{EdgeLabel, LayoutOptions, NodeLabel, RankDir};

use super::{
    End, Layout, LayoutGraph, Placed, PlacedEdge, Point, Rect, CLUSTER_PAD, TITLE_BAND,
};
use crate::mermaid::flowchart::Direction;

/// dagre with the tile's spacing.
#[derive(Debug, Clone)]
pub struct Dagre {
    pub nodesep: f64,
    pub ranksep: f64,
    pub edgesep: f64,
    pub margin: f64,
}

impl Default for Dagre {
    fn default() -> Self {
        Dagre { nodesep: 40.0, ranksep: 60.0, edgesep: 20.0, margin: 16.0 }
    }
}

fn node_id(i: usize) -> String {
    format!("n{i}")
}

fn cluster_id(i: usize) -> String {
    format!("c{i}")
}

fn anchor_id(i: usize) -> String {
    format!("a{i}")
}

fn sized(w: f64, h: f64) -> NodeLabel {
    NodeLabel { width: w, height: h, ..NodeLabel::default() }
}

fn rect_of(label: &NodeLabel) -> Rect {
    let (x, y) = (label.x.unwrap_or(0.0), label.y.unwrap_or(0.0));
    Rect { x: x - label.width / 2.0, y: y - label.height / 2.0, w: label.width, h: label.height }
}

impl Layout for Dagre {
    fn name(&self) -> &'static str {
        "dagre"
    }

    fn layout(&self, g: &LayoutGraph) -> Placed {
        let mut dg: Graph<NodeLabel, EdgeLabel> =
            Graph::with_options(GraphOptions { directed: true, multigraph: true, compound: true });
        let has_children: Vec<bool> = (0..g.clusters.len())
            .map(|c| {
                g.nodes.iter().any(|n| n.parent == Some(c))
                    || g.clusters.iter().any(|k| k.parent == Some(c))
            })
            .collect();
        for (i, n) in g.nodes.iter().enumerate() {
            dg.set_node(node_id(i), Some(sized(n.w, n.h)));
        }
        for (c, cluster) in g.clusters.iter().enumerate() {
            // An empty boundary is a leaf: as big as its title.
            let label = if has_children[c] {
                NodeLabel::default()
            } else {
                sized(cluster.title_w + 2.0 * CLUSTER_PAD, TITLE_BAND + 2.0 * CLUSTER_PAD)
            };
            dg.set_node(cluster_id(c), Some(label));
        }
        for (i, n) in g.nodes.iter().enumerate() {
            if let Some(c) = n.parent {
                dg.set_parent(&node_id(i), Some(&cluster_id(c)));
            }
        }
        for (c, cluster) in g.clusters.iter().enumerate() {
            if let Some(p) = cluster.parent {
                dg.set_parent(&cluster_id(c), Some(&cluster_id(p)));
            }
        }
        // dagre can't attach edges to a parent node: an arrow to a boundary
        // goes to a hidden anchor inside it.
        let end_id =|dg: &mut Graph<NodeLabel, EdgeLabel>, end: End| match end {
            End::Node(i) => node_id(i),
            End::Cluster(c) if !has_children[c] => cluster_id(c),
            End::Cluster(c) => {
                let id = anchor_id(c);
                if dg.node(&id).is_none() {
                    dg.set_node(id.clone(), Some(sized(1.0, 1.0)));
                    dg.set_parent(&id, Some(&cluster_id(c)));
                }
                id
            }
        };
        let mut edge_ends = Vec::new();
        for (i, e) in g.edges.iter().enumerate() {
            let (v, w) = (end_id(&mut dg, e.from), end_id(&mut dg, e.to));
            let (width, height) = e.label.unwrap_or((0.0, 0.0));
            let label = EdgeLabel { width, height, ..EdgeLabel::default() };
            dg.set_edge(v.clone(), w.clone(), Some(label), Some(&format!("e{i}")));
            edge_ends.push((v, w));
        }
        let rankdir = match g.direction {
            Direction::TopDown => RankDir::TB,
            Direction::BottomUp => RankDir::BT,
            Direction::LeftRight => RankDir::LR,
            Direction::RightLeft => RankDir::RL,
        };
        let options = LayoutOptions {
            rankdir,
            nodesep: self.nodesep,
            ranksep: self.ranksep,
            edgesep: self.edgesep,
            ..LayoutOptions::default()
        };
        dagre::layout(&mut dg, Some(options));

        let nodes: Vec<Rect> = (0..g.nodes.len())
            .map(|i| dg.node(&node_id(i)).map(rect_of).unwrap_or_default())
            .collect();
        let mut clusters: Vec<Rect> = (0..g.clusters.len())
            .map(|c| dg.node(&cluster_id(c)).map(rect_of).unwrap_or_default())
            .collect();
        let anchors: Vec<(usize, Rect)> = (0..g.clusters.len())
            .filter_map(|c| Some((c, rect_of(dg.node(&anchor_id(c))?))))
            .collect();
        grow_clusters(g, &nodes, &anchors, &mut clusters, &has_children);
        let mut edges: Vec<PlacedEdge> = g
            .edges
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let (v, w) = &edge_ends[i];
                let label = dg.edge(v, w, Some(&format!("e{i}")));
                let points = label
                    .map(|l| l.points.iter().map(|p| Point { x: p.x, y: p.y }).collect())
                    .unwrap_or_default();
                let label_rect = e.label.and_then(|(lw, lh)| {
                    let l = label?;
                    Some(Rect { x: l.x? - lw / 2.0, y: l.y? - lh / 2.0, w: lw, h: lh })
                });
                PlacedEdge { points, label: label_rect }
            })
            .collect();
        for (e, placed) in g.edges.iter().zip(edges.iter_mut()) {
            trim_to_clusters(g, e.from, e.to, &clusters, &nodes, &mut placed.points);
        }
        finish(nodes, clusters, edges, self.margin)
    }
}

/// Grows each boundary around its members (and the anchors of arrows to it)
/// and adds padding and the title band, innermost first (layout-lld.md §4).
pub(super) fn grow_clusters(
    g: &LayoutGraph,
    nodes: &[Rect],
    anchors: &[(usize, Rect)],
    clusters: &mut [Rect],
    has_children: &[bool],
) {
    let mut order: Vec<usize> = (0..g.clusters.len()).collect();
    order.sort_by_key(|&c| std::cmp::Reverse(g.depth(c)));
    for c in order {
        if !has_children[c] {
            continue;
        }
        let mut rect: Option<Rect> = None;
        let members =
            (0..g.nodes.len()).filter(|&n| g.nodes[n].parent == Some(c)).map(|n| nodes[n]);
        let children = (0..g.clusters.len())
            .filter(|&k| g.clusters[k].parent == Some(c))
            .map(|k| clusters[k]);
        let anchors = anchors.iter().filter(|a| a.0 == c).map(|a| a.1);
        for r in members.chain(children).chain(anchors) {
            rect = Some(rect.map_or(r, |acc| acc.union(&r)));
        }
        let mut r = rect.unwrap_or(clusters[c]);
        r.x -= CLUSTER_PAD;
        r.w += 2.0 * CLUSTER_PAD;
        r.y -= CLUSTER_PAD + TITLE_BAND;
        r.h += 2.0 * CLUSTER_PAD + TITLE_BAND;
        let min_w = g.clusters[c].title_w + 2.0 * CLUSTER_PAD;
        if r.w < min_w {
            r.x -= (min_w - r.w) / 2.0;
            r.w = min_w;
        }
        clusters[c] = r;
    }
}

/// Cuts an edge where it enters a boundary it ends at, so the arrowhead
/// sits on the frame.
pub(super) fn trim_to_clusters(
    g: &LayoutGraph,
    from: End,
    to: End,
    clusters: &[Rect],
    nodes: &[Rect],
    points: &mut Vec<Point>,
) {
    let inside = |end: End, c: usize| match end {
        End::Node(n) => g.node_within(n, c) && clusters[c].contains(&nodes[n], 0.5),
        End::Cluster(k) => g.cluster_within(k, c),
    };
    if let End::Cluster(c) = to {
        if !inside(from, c) {
            cut(points, clusters[c]);
        }
    }
    if let End::Cluster(c) = from {
        if !inside(to, c) {
            points.reverse();
            cut(points, clusters[c]);
            points.reverse();
        }
    }
}

/// Keeps a polyline up to where it first enters `rect` from outside.
fn cut(points: &mut Vec<Point>, rect: Rect) {
    for i in 0..points.len().saturating_sub(1) {
        let (a, b) = (points[i], points[i + 1]);
        if !rect.contains_point(a) && rect.contains_point(b) {
            let (mut lo, mut hi) = (0.0, 1.0);
            for _ in 0..32 {
                let mid = (lo + hi) / 2.0;
                let p = Point { x: a.x + (b.x - a.x) * mid, y: a.y + (b.y - a.y) * mid };
                if rect.contains_point(p) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            let p = Point { x: a.x + (b.x - a.x) * hi, y: a.y + (b.y - a.y) * hi };
            points.truncate(i + 1);
            points.push(p);
            return;
        }
    }
}

/// Moves everything so the drawing starts at (`margin`, `margin`).
pub(super) fn finish(
    mut nodes: Vec<Rect>,
    mut clusters: Vec<Rect>,
    mut edges: Vec<PlacedEdge>,
    margin: f64,
) -> Placed {
    let mut bounds: Option<Rect> = None;
    let mut add = |r: Rect| bounds = Some(bounds.map_or(r, |b| b.union(&r)));
    nodes.iter().chain(clusters.iter()).for_each(|r| add(*r));
    for e in &edges {
        e.points.iter().for_each(|p| add(Rect { x: p.x, y: p.y, w: 0.0, h: 0.0 }));
        e.label.iter().for_each(|r| add(*r));
    }
    let b = bounds.unwrap_or_default();
    let (dx, dy) = (margin - b.x, margin - b.y);
    let shift = |r: &mut Rect| {
        r.x += dx;
        r.y += dy;
    };
    nodes.iter_mut().chain(clusters.iter_mut()).for_each(shift);
    for e in &mut edges {
        for p in &mut e.points {
            p.x += dx;
            p.y += dy;
        }
        e.label.iter_mut().for_each(shift);
    }
    Placed { nodes, clusters, edges, width: b.w + 2.0 * margin, height: b.h + 2.0 * margin }
}
