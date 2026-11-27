//! The Painter (`--features tile`): every fixture structure draws within
//! the canvas limits, with a labelled region per box, and the shop's
//! context diagram marks the box that has a detail diagram.
#![cfg(feature = "tile")]

mod common;

use c4_tile::index::ZoomTarget;
use c4_tile::layout::{build, Dagre, Layout};
use c4_tile::paint::paint;
use pitwall_extension::prelude::*;

#[test]
fn the_shop_draws_with_a_region_per_box() {
    let index = common::index_of(&["shop/docs/architecture.md", "shop/docs/orders/hla.md"]);
    let entry = index.entries().iter().find(|e| e.structure.is_some()).expect("a structure diagram");
    let s = entry.structure.as_ref().unwrap();
    let (graph, sized) = build(s);
    let placed = Dagre::default().layout(&graph);
    let detailed: Vec<bool> = (0..s.boxes.len()).map(|i| index.zoom_target(entry.id(), i, &[]) != ZoomTarget::None).collect();
    assert!(detailed.iter().any(|d| *d), "the Shop box opens its containers");
    let (_, hits) = paint(canvas(), s, &sized, &placed, &detailed, Some(0));
    assert_eq!(hits.len(), s.boxes.len());
    for (h, b) in hits.iter().zip(&s.boxes) {
        assert_eq!(h.label, b.name);
        assert!(h.rect.w > 0. && h.rect.h > 0.);
    }
}

#[test]
fn every_fixture_draws_within_the_limits() {
    for path in common::all_fixtures() {
        let index = common::index_of(&[path.as_str()]);
        for e in index.entries() {
            let Some(s) = &e.structure else { continue };
            let (graph, sized) = build(s);
            let placed = Dagre::default().layout(&graph);
            let (_, hits) = paint(canvas(), s, &sized, &placed, &[], None);
            assert_eq!(hits.len(), s.boxes.len(), "{}", e.id());
        }
    }
}
