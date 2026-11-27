//! C4 reading of labels: Name [Kind], purpose separators, classes, shapes
//! and levels (FR-5, FR-11; mermaid-lld.md §5).

mod common;

use c4_tile::c4::{self, level, name_key, Level, Shape, Structure};
use common::{flowchart, lines};

fn structure(text: &str) -> Structure {
    c4::read(&flowchart(text))
}

#[test]
fn each_purpose_separator() {
    let s = structure(&lines(&[
        "flowchart LR",
        "  a[\"Scanner [Component]<br/>finds Mermaid blocks in Markdown\"]",
        "  b[\"Scanner [Component]<br>finds Mermaid blocks in Markdown\"]",
        "  c[\"Scanner [Component]<br />finds Mermaid blocks in Markdown\"]",
        "  d[\"Scanner [Component]",
        "  finds Mermaid blocks in Markdown\"]",
    ]));
    for b in &s.boxes {
        assert_eq!(b.name, "Scanner", "{}", b.id);
        assert_eq!(b.kind.as_ref().unwrap().text, "Component");
        assert_eq!(b.purpose.as_deref(), Some("finds Mermaid blocks in Markdown"), "{}", b.id);
    }
}

#[test]
fn kind_on_its_own_line_and_missing_parts() {
    let s = structure(&lines(&[
        "flowchart LR",
        "  a[\"Scanner<br/>[Component]<br/>finds blocks<br/>in Markdown\"]",
        "  b[\"Just a name\"]",
        "  c",
        "  d[\"Name [Kind]\"]",
        "  e[\"[Kind] only\"]",
    ]));
    let got: Vec<(&str, Option<&str>, Option<&str>)> = s
        .boxes
        .iter()
        .map(|b| (b.name.as_str(), b.kind.as_ref().map(|k| k.text.as_str()), b.purpose.as_deref()))
        .collect();
    assert_eq!(got, [
        ("Scanner", Some("Component"), Some("finds blocks in Markdown")),
        ("Just a name", None, None),
        ("c", None, None),
        ("Name", Some("Kind"), None),
        ("[Kind] only", None, None),
    ]);
}

#[test]
fn kinds_with_technology() {
    let k = c4::Kind::parse("Container: Rust, Axum");
    assert_eq!((k.base.as_str(), k.technology.as_deref()), ("container", Some("Rust, Axum")));
    assert_eq!(c4::Kind::parse("Software System").base, "software system");
}

#[test]
fn shapes_follow_the_kind_then_the_hint() {
    let s = structure(&lines(&[
        "flowchart LR",
        "  a[\"User [Person]\"]",
        "  b[\"Orders DB [Container: PostgreSQL]\"]",
        "  c[(\"Cache [Container: Redis]\")]",
        "  d[\"Events [Container: Queue]\"]",
        "  e[\"Files [Store]\"]",
        "  f[(\"Plain cylinder\")]",
        "  g((\"Person-ish [Person]\"))",
        "  h[\"Web [Container]\"]",
    ]));
    let shapes: Vec<Shape> = s.boxes.iter().map(|b| b.shape).collect();
    use Shape::*;
    assert_eq!(shapes, [Person, Box, Cylinder, Pipe, Cylinder, Cylinder, Person, Box]);
}

#[test]
fn classes_with_meaning() {
    let s = structure(&lines(&[
        "flowchart LR",
        "  a:::new",
        "  b",
        "  c[\"Mail [External System]\"]",
        "  d",
        "  class b changed",
        "  class d external,mine",
    ]));
    let marks: Vec<(bool, bool, bool)> =
        s.boxes.iter().map(|b| (b.marks.new, b.marks.changed, b.marks.external)).collect();
    assert_eq!(marks, [
        (true, false, false),
        (false, true, false),
        (false, false, true),
        (false, false, true),
    ]);
    assert_eq!(s.boxes[3].classes, ["external", "mine"]);
}

#[test]
fn boundaries_and_arrow_ends() {
    let s = structure(
        "flowchart LR\n  u --> sys\n  subgraph sys[\"Pitwall [Software System]\"]\n    a\n  end",
    );
    assert_eq!(s.boundaries[0].name, "Pitwall");
    assert_eq!(s.boundaries[0].kind.as_ref().unwrap().text, "Software System");
    assert_eq!(s.end_name(s.arrows[0].to), "Pitwall");
}

#[test]
fn name_keys() {
    assert_eq!(name_key("  C4   tile [Software System] "), "c4 tile");
    assert_eq!(name_key("tile.wasm"), "tile.wasm");
}

#[test]
fn level_from_title_and_headings_first() {
    let s = structure("flowchart LR\n  a[\"A [Component]\"]");
    assert_eq!(level(&s, Some("Shop containers"), &[]), Level::Container);
    assert_eq!(level(&s, None, &["Doc", "1. Context"]), Level::Context);
    // The nearest heading that names a level wins.
    assert_eq!(level(&s, None, &["Containers", "Before"]), Level::Container);
    assert_eq!(level(&s, None, &["System context", "Components of x"]), Level::Component);
}

#[test]
fn level_from_kinds() {
    let comp = structure("flowchart LR\n  a[\"A [Container]\"]\n  b[\"B [Component]\"]");
    let cont = structure("flowchart LR\n  a[\"A [Container: Rust]\"]\n  b[\"B [Person]\"]");
    let ctx = structure("flowchart LR\n  a[\"A [Software System]\"]\n  b[\"B [Person]\"]");
    let plain = structure("flowchart LR\n  a --> b");
    assert_eq!(level(&comp, None, &["Notes"]), Level::Component);
    assert_eq!(level(&cont, None, &[]), Level::Container);
    assert_eq!(level(&ctx, None, &[]), Level::Context);
    assert_eq!(level(&plain, None, &[]), Level::Context);
}

#[test]
fn levels_of_the_fixtures() {
    let index = common::index_of(&["c4-tile/hla.md", "shop/docs/architecture.md"]);
    let got: Vec<(&str, Level)> =
        index.entries().iter().filter_map(|e| Some((e.id(), e.level?))).collect();
    assert_eq!(got, [
        ("c4-tile/hla.md#1-context", Level::Context),
        ("c4-tile/hla.md#2-containers", Level::Container),
        ("c4-tile/hla.md#3-components-of-tilewasm", Level::Component),
        ("shop/docs/architecture.md#system-context", Level::Context),
        ("shop/docs/architecture.md#containers", Level::Container),
    ]);
}
