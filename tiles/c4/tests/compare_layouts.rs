//! dagre against rust-sugiyama on the same fixtures (layout-lld.md §7).
//! Run with `cargo test --release --features sugiyama --test compare_layouts
//! -- --nocapture` to see the numbers.
#![cfg(feature = "sugiyama")]

mod common;

use std::time::Instant;

use c4_tile::layout::{
    build, check, Dagre, End, Layout, LayoutCluster, LayoutEdge, LayoutGraph, LayoutNode, Placed,
    Point, Rect, Sugiyama,
};
use c4_tile::mermaid::flowchart::Direction;

fn graphs() -> Vec<(String, LayoutGraph)> {
    let paths = common::all_fixtures();
    let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
    let index = common::index_of(&paths);
    index
        .entries()
        .iter()
        .filter_map(|e| Some((e.id().to_string(), build(e.structure.as_ref()?).0)))
        .collect()
}

/// A C4-like graph with `n` boxes: boundaries of 10 boxes, chains inside,
/// links between neighbouring boundaries, every third box labelled.
fn synthetic(n: usize) -> LayoutGraph {
    let groups = n / 10;
    let clusters = (0..groups).map(|_| LayoutCluster { parent: None, title_w: 120.0 }).collect();
    let nodes =
        (0..n).map(|i| LayoutNode { w: 180.0, h: 70.0, parent: Some(i / 10) }).collect();
    let mut edges = Vec::new();
    for i in 0..n {
        let label = (i % 3 == 0).then_some((90.0, 18.0));
        if i % 10 != 9 {
            edges.push(LayoutEdge { from: End::Node(i), to: End::Node(i + 1), label });
        }
        if i % 10 == 0 && i + 10 < n {
            edges.push(LayoutEdge { from: End::Node(i), to: End::Node(i + 15), label });
        }
    }
    LayoutGraph { direction: Direction::TopDown, nodes, clusters, edges }
}

fn segments(points: &[Point]) -> Vec<(Point, Point)> {
    points.windows(2).map(|w| (w[0], w[1])).collect()
}

fn cross(a: (Point, Point), b: (Point, Point)) -> bool {
    let orient =
        |p: Point, q: Point, r: Point| (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x);
    let d1 = orient(b.0, b.1, a.0);
    let d2 = orient(b.0, b.1, a.1);
    let d3 = orient(a.0, a.1, b.0);
    let d4 = orient(a.0, a.1, b.1);
    d1 * d2 < -1e-6 && d3 * d4 < -1e-6
}

fn hits(seg: (Point, Point), r: &Rect) -> bool {
    let inner = Rect { x: r.x + 2.0, y: r.y + 2.0, w: r.w - 4.0, h: r.h - 4.0 };
    (0..=20).any(|i| {
        let t = i as f64 / 20.0;
        inner.contains_point(Point {
            x: seg.0.x + (seg.1.x - seg.0.x) * t,
            y: seg.0.y + (seg.1.y - seg.0.y) * t,
        })
    })
}

#[derive(Default, Debug)]
struct Quality {
    problems: usize,
    crossings: usize,
    through_boxes: usize,
    labels_on_boxes: usize,
    area: f64,
}

fn quality(g: &LayoutGraph, p: &Placed) -> Quality {
    let mut q = Quality {
        problems: check(g, p).len(),
        area: p.width * p.height,
        ..Quality::default()
    };
    let segs: Vec<Vec<(Point, Point)>> = p.edges.iter().map(|e| segments(&e.points)).collect();
    for i in 0..segs.len() {
        for j in i + 1..segs.len() {
            q.crossings +=
                segs[i].iter().filter(|a| segs[j].iter().any(|b| cross(**a, *b))).count();
        }
        let ends = [g.edges[i].from, g.edges[i].to];
        for (n, r) in p.nodes.iter().enumerate() {
            if !ends.contains(&End::Node(n)) && segs[i].iter().any(|s| hits(*s, r)) {
                q.through_boxes += 1;
            }
        }
        if let Some(label) = p.edges[i].label {
            q.labels_on_boxes += p.nodes.iter().filter(|r| r.overlaps(&label, 0.5)).count();
        }
    }
    q
}

fn time(engine: &dyn Layout, g: &LayoutGraph, runs: u32) -> f64 {
    let start = Instant::now();
    for _ in 0..runs {
        std::hint::black_box(engine.layout(g));
    }
    start.elapsed().as_secs_f64() * 1000.0 / runs as f64
}

#[test]
fn compare() {
    let engines: [&dyn Layout; 2] = [&Dagre::default(), &Sugiyama::default()];
    let fixtures = graphs();
    println!("{} fixture diagrams", fixtures.len());
    for engine in engines {
        let mut total = Quality::default();
        let mut ms = 0.0;
        for (id, g) in &fixtures {
            let placed = engine.layout(g);
            let q = quality(g, &placed);
            if q.problems > 0 {
                println!("  {}: {id}: {:?}", engine.name(), check(g, &placed));
            }
            total.problems += q.problems;
            total.crossings += q.crossings;
            total.through_boxes += q.through_boxes;
            total.labels_on_boxes += q.labels_on_boxes;
            total.area += q.area;
            ms += time(engine, g, 5);
        }
        println!(
            "{:14} problems {:3}  crossings {:3}  edges through boxes {:3}  labels on boxes {:3}  \
             area {:8.0} kpx²  time {:7.2} ms for all",
            engine.name(),
            total.problems,
            total.crossings,
            total.through_boxes,
            total.labels_on_boxes,
            total.area / 1000.0,
            ms
        );
        for n in [50, 150, 300] {
            let g = synthetic(n);
            let runs = if n == 300 { 1 } else { 3 };
            let q = quality(&g, &engine.layout(&g));
            println!(
                "{:14} {n:3} boxes: {:8.1} ms, problems {}, crossings {}",
                engine.name(),
                time(engine, &g, runs),
                q.problems,
                q.crossings
            );
        }
    }
}
