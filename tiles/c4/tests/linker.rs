//! The Linker: zoom links and before/after pairs (hla.md §3.4, FR-12,
//! FR-13, frd.md §3.4).

mod common;

use c4_tile::index::{title_matches, Index, Pair, ZoomTarget};

fn box_index(index: &Index, id: &str, name: &str) -> usize {
    let s = index.get(id).unwrap().structure.as_ref().unwrap();
    s.boxes.iter().position(|b| b.name == name).unwrap_or_else(|| panic!("no box {name}"))
}

#[test]
fn the_specs_own_hla_links_its_levels() {
    let index = common::index_of(&["c4-tile/hla.md"]);
    let context = "c4-tile/hla.md#1-context";
    let containers = "c4-tile/hla.md#2-containers";
    let components = "c4-tile/hla.md#3-components-of-tilewasm";
    let c4 = box_index(&index, context, "C4 tile");
    assert_eq!(index.zoom_target(context, c4, &[]), ZoomTarget::One(containers.into()));
    let wasm = box_index(&index, containers, "tile.wasm");
    assert_eq!(index.zoom_target(containers, wasm, &[context]), ZoomTarget::One(components.into()));
    // "Pitwall" is a boundary in the container diagram too.
    let pitwall = box_index(&index, context, "Pitwall");
    assert!(index.is_detailed(context, pitwall));
    // Boxes nothing details.
    let engineer = box_index(&index, context, "Engineer");
    assert!(!index.is_detailed(context, engineer));
    let scanner = box_index(&index, components, "Scanner");
    assert!(!index.is_detailed(components, scanner));
}

#[test]
fn the_zoom_path_is_never_a_target() {
    let index = common::index_of(&["c4-tile/hla.md"]);
    let containers = "c4-tile/hla.md#2-containers";
    // The container diagram's "pitwall-tile-host" box: nothing details it.
    let host = box_index(&index, containers, "pitwall-tile-host");
    assert_eq!(index.zoom_target(containers, host, &[]), ZoomTarget::None);
    let wasm = box_index(&index, containers, "tile.wasm");
    let components = "c4-tile/hla.md#3-components-of-tilewasm";
    assert_eq!(index.zoom_target(containers, wasm, &[components]), ZoomTarget::None);
}

#[test]
fn same_file_then_same_folder_then_a_menu() {
    let index = common::index_of(&[
        "shop/docs/architecture.md",
        "shop/docs/orders/hla.md",
        "shop/docs/orders/cache-lld.md",
        "shop/docs/legacy/orders.md",
    ]);
    // Context "Shop": detailed in architecture.md (same file) and in the
    // LLD's diagrams; the same file wins.
    let context = "shop/docs/architecture.md#system-context";
    let shop = box_index(&index, context, "Shop");
    assert_eq!(
        index.zoom_target(context, shop, &[]),
        ZoomTarget::One("shop/docs/architecture.md#containers".into())
    );
    // From the LLD, "Orders API" is detailed by orders/hla.md (same folder)
    // and legacy/orders.md: the same folder wins.
    let after = "shop/docs/orders/cache-lld.md#after";
    let api = box_index(&index, after, "Orders API");
    assert_eq!(
        index.zoom_target(after, api, &[]),
        ZoomTarget::One("shop/docs/orders/hla.md#components".into())
    );
    // From architecture.md, neither is in the same file or folder: a menu.
    let containers = "shop/docs/architecture.md#containers";
    let api = box_index(&index, containers, "Orders API");
    assert_eq!(
        index.zoom_target(containers, api, &[]),
        ZoomTarget::Several(vec![
            "shop/docs/legacy/orders.md#legacy-orders".into(),
            "shop/docs/orders/hla.md#components".into(),
        ])
    );
}

#[test]
fn before_after_pairs() {
    let index = common::index_of(&["shop/docs/orders/cache-lld.md", "c4-tile/hla.md"]);
    assert_eq!(index.pairs(), [
        Pair {
            before: "shop/docs/orders/cache-lld.md#before".into(),
            after: "shop/docs/orders/cache-lld.md#after".into(),
        },
        Pair {
            before: "shop/docs/orders/cache-lld.md#1-current".into(),
            after: "shop/docs/orders/cache-lld.md#2-proposed-a-replica".into(),
        },
    ]);
    assert!(index.pair_of("shop/docs/orders/cache-lld.md#after").is_some());
    assert!(index.pair_of("c4-tile/hla.md#1-context").is_none());
}

#[test]
fn pairs_need_one_structure_diagram_per_side() {
    let mut index = Index::new();
    let block = "```mermaid\nflowchart LR\n  a --> b\n```\n";
    let seq = "```mermaid\nsequenceDiagram\n  a->>b: x\n```\n";
    index.set_file("two.md", &format!("# X\n## Before\n{block}{block}## After\n{block}"));
    index.set_file("seq.md", &format!("# X\n## Before\n{seq}## After\n{seq}"));
    index.set_file("mixed.md", &format!("# X\n## Before\n{block}## Proposed\n{block}"));
    index.set_file("nested.md", &format!("# X\n## Before\n### Deeper\n{block}## After\n{block}"));
    index.set_file("ok.md", &format!("# X\n## As is\n{block}## To be\n{block}"));
    index.set_file("ok2.md", &format!("## today\n{block}## TARGET state\n{block}"));
    let afters: Vec<&str> = index.pairs().iter().map(|p| p.after.as_str()).collect();
    assert_eq!(afters, ["ok.md#to-be", "ok2.md#target-state"]);
}

#[test]
fn heading_words() {
    assert!(title_matches("Before", "before"));
    assert!(title_matches("3.1 After: with a cache", "after"));
    assert!(title_matches("AS IS", "as is"));
    assert!(!title_matches("Afterwards", "after"));
    assert!(!title_matches("The before state", "before"));
}

#[test]
fn files_come_and_go() {
    let mut index = common::index_of(&["c4-tile/hla.md"]);
    let context = "c4-tile/hla.md#1-context";
    let c4 = box_index(&index, context, "C4 tile");
    let renamed = common::fixture("c4-tile/hla.md")
        .replace("C4 tile [Software System]\"]", "Other [Software System]\"]");
    index.set_file("c4-tile/hla.md", &renamed);
    assert!(!index.is_detailed(context, c4));
    index.remove_file("c4-tile/hla.md");
    assert!(index.entries().is_empty());
}
