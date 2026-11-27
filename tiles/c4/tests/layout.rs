//! Layout with dagre behind the `Layout` trait (layout-lld.md §4, §6):
//! every fixture lays out with no overlapping boxes and every box inside its
//! boundary.

mod common;

use c4_tile::c4::Structure;
use c4_tile::layout::{build, check, size_box, Dagre, Layout, LayoutGraph, Placed};
use c4_tile::layout::{BOX_MAX_W, BOX_MIN_W};

/// Every structure diagram of every fixture, by id.
fn structures() -> Vec<(String, Structure)> {
    let paths = common::all_fixtures();
    let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
    let index = common::index_of(&paths);
    index
        .entries()
        .iter()
        .filter_map(|e| Some((e.id().to_string(), e.structure.clone()?)))
        .collect()
}

fn lay_out(engine: &dyn Layout, s: &Structure) -> (LayoutGraph, Placed) {
    let (graph, _) = build(s);
    let placed = engine.layout(&graph);
    (graph, placed)
}

#[test]
fn every_fixture_lays_out_cleanly_with_dagre() {
    let all = structures();
    assert!(all.len() >= 20, "fixtures: {}", all.len());
    let mut failures = Vec::new();
    for (id, s) in &all {
        let (graph, placed) = lay_out(&Dagre::default(), s);
        assert_eq!(placed.nodes.len(), graph.nodes.len());
        assert_eq!(placed.clusters.len(), graph.clusters.len());
        assert_eq!(placed.edges.len(), graph.edges.len());
        for problem in check(&graph, &placed) {
            failures.push(format!("{id}: {problem}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn boxes_keep_their_sizes_and_edges_have_points() {
    for (id, s) in structures() {
        let (graph, placed) = lay_out(&Dagre::default(), &s);
        for (n, r) in graph.nodes.iter().zip(&placed.nodes) {
            assert!((n.w - r.w).abs() < 0.01 && (n.h - r.h).abs() < 0.01, "{id}");
            assert!(r.x >= 0.0 && r.y >= 0.0 && r.right() <= placed.width, "{id}");
        }
        for (e, p) in graph.edges.iter().zip(&placed.edges) {
            assert!(p.points.len() >= 2, "{id}: {e:?}");
            assert_eq!(e.label.is_some(), p.label.is_some(), "{id}");
        }
    }
}

#[test]
fn arrows_to_a_boundary_stop_at_its_frame() {
    let index = common::index_of(&["tile-sdk/diagrams.md"]);
    let s = index.get("tile-sdk/diagrams.md#21-where-it-fits").unwrap().structure.clone().unwrap();
    let (graph, placed) = lay_out(&Dagre::default(), &s);
    // `Host --> T`: from the Host boundary to a box in Core.
    let host = s.boundaries.iter().position(|b| b.id == "Host").unwrap();
    let frame = placed.clusters[host];
    let i = s
        .arrows
        .iter()
        .position(|a| a.from == c4_tile::c4::End::Boundary(host))
        .expect("an arrow from Host");
    let start = placed.edges[i].points[0];
    let on_edge = |v: f64, a: f64, b: f64| (v - a).abs() < 1.0 || (v - b).abs() < 1.0;
    assert!(
        on_edge(start.x, frame.x, frame.right()) || on_edge(start.y, frame.y, frame.bottom()),
        "{start:?} not on {frame:?}"
    );
    assert!(check(&graph, &placed).is_empty());
}

#[test]
fn directions_change_the_main_axis() {
    let index = common::index_of(&["shop/docs/architecture.md"]);
    let tb = common::structure(&index, "shop/docs/architecture.md#system-context");
    let (_, placed) = lay_out(&Dagre::default(), &tb);
    // Customer → Shop runs down.
    assert!(placed.nodes[1].y > placed.nodes[0].bottom());
    let mut lr = tb.clone();
    lr.direction = c4_tile::mermaid::flowchart::Direction::LeftRight;
    let (_, placed) = lay_out(&Dagre::default(), &lr);
    assert!(placed.nodes[1].x > placed.nodes[0].right());
}

#[test]
fn box_sizes() {
    let index = common::index_of(&["c4-tile/hla.md"]);
    let s = common::structure(&index, "c4-tile/hla.md#3-components-of-tilewasm");
    for b in &s.boxes {
        let sized = size_box(b);
        assert!(sized.w >= BOX_MIN_W && sized.w <= BOX_MAX_W, "{}: {}", b.name, sized.w);
        assert!(sized.purpose.len() <= 2);
    }
    let person = index.get("c4-tile/hla.md#1-context").unwrap().structure.clone().unwrap();
    let engineer = size_box(&person.boxes[0]);
    let c4 = size_box(&person.boxes[1]);
    assert!(engineer.h > c4.h - 1.0, "the person shape adds a head");
}

#[test]
fn layout_is_deterministic() {
    for (id, s) in structures() {
        let a = lay_out(&Dagre::default(), &s).1;
        let b = lay_out(&Dagre::default(), &s).1;
        assert_eq!(a, b, "{id}");
    }
}
