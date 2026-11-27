//! The flowchart parser: every flowchart row of frd.md §6.1, and errors with
//! line, column and message (mermaid-lld.md §3).

mod common;

use c4_tile::mermaid::flowchart::{Direction, End, Head, LineStyle, NodeShape};
use c4_tile::mermaid::{Fallback, FallbackReason, Flowchart};
use common::{fallback, flowchart, lines, problems};

fn node<'a>(f: &'a Flowchart, id: &str) -> &'a c4_tile::mermaid::flowchart::Node {
    f.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| panic!("no node {id}"))
}

fn ids(f: &Flowchart, end: End) -> String {
    match end {
        End::Node(i) => f.nodes[i].id.clone(),
        End::Boundary(i) => format!("[{}]", f.boundaries[i].id),
    }
}

fn edges(f: &Flowchart) -> Vec<(String, String)> {
    f.edges.iter().map(|e| (ids(f, e.from), ids(f, e.to))).collect()
}

// Row: diagram types.

#[test]
fn flowchart_and_graph_headers() {
    assert_eq!(flowchart("flowchart LR\n  a --> b").edges.len(), 1);
    assert_eq!(flowchart("graph TD;\n  a-->b;").edges.len(), 1);
    assert_eq!(flowchart("flowchart\n  a").direction, Direction::TopDown);
    assert_eq!(flowchart("graph TD; a-->b; b-->c").edges.len(), 2);
}

// Row: direction.

#[test]
fn directions() {
    for (word, dir) in [
        ("TB", Direction::TopDown),
        ("TD", Direction::TopDown),
        ("BT", Direction::BottomUp),
        ("LR", Direction::LeftRight),
        ("RL", Direction::RightLeft),
    ] {
        assert_eq!(flowchart(&format!("flowchart {word}\n  a")).direction, dir, "{word}");
    }
}

#[test]
fn direction_inside_a_subgraph_is_read() {
    let f = flowchart("flowchart LR\n  subgraph s\n    direction TB\n    a --> b\n  end");
    assert_eq!(f.boundaries[0].direction, Some(Direction::TopDown));
    assert_eq!(f.direction, Direction::LeftRight);
}

// Row: nodes.

#[test]
fn every_shape() {
    let f = flowchart(&lines(&[
        "flowchart TB",
        "  a[rect]",
        "  b(round)",
        "  c([stadium])",
        "  d[(cylinder)]",
        "  e((circle))",
        "  f{diamond}",
        "  g{{hexagon}}",
        "  h[/para/]",
        "  i[\\alt\\]",
        "  j>flag]",
        "  k",
    ]));
    let shapes: Vec<(&str, NodeShape, &str)> =
        f.nodes.iter().map(|n| (n.id.as_str(), n.shape, n.lines[0].as_str())).collect();
    assert_eq!(shapes, [
        ("a", NodeShape::Rect, "rect"),
        ("b", NodeShape::Round, "round"),
        ("c", NodeShape::Stadium, "stadium"),
        ("d", NodeShape::Cylinder, "cylinder"),
        ("e", NodeShape::Circle, "circle"),
        ("f", NodeShape::Diamond, "diamond"),
        ("g", NodeShape::Hexagon, "hexagon"),
        ("h", NodeShape::Parallelogram, "para"),
        ("i", NodeShape::ParallelogramAlt, "alt"),
        ("j", NodeShape::Flag, "flag"),
        ("k", NodeShape::Plain, "k"),
    ]);
    assert!(f.skipped.is_empty());
}

#[test]
fn shapes_outside_the_subset_are_plain_boxes_and_noted() {
    let f = flowchart("flowchart TB\n  a[[sub]]\n  b(((double)))\n  c@{ shape: doc }");
    assert_eq!(node(&f, "a").shape, NodeShape::Other);
    assert_eq!(node(&f, "b").lines, ["double"]);
    let what: Vec<&str> = f.skipped.iter().map(|s| s.what.as_str()).collect();
    assert_eq!(what, [
        "shape '[[…' drawn as a box",
        "shape '(((…' drawn as a box",
        "shape syntax '@{…}'",
    ]);
}

#[test]
fn quoted_labels_and_line_breaks() {
    let f = flowchart(&lines(&[
        "flowchart LR",
        "  a[\"Scanner [Component]<br/>finds blocks\"]",
        "  b[\"Web (app) {x}\"]",
        "  c[\"line one",
        "  line two\"]",
        "  d[one<br>two<br />three]",
    ]));
    assert_eq!(node(&f, "a").lines, ["Scanner [Component]", "finds blocks"]);
    assert_eq!(node(&f, "b").lines, ["Web (app) {x}"]);
    assert_eq!(node(&f, "c").lines, ["line one", "line two"]);
    assert_eq!(node(&f, "d").lines, ["one", "two", "three"]);
}

#[test]
fn ids_with_hyphens_dots_and_spaces_before_shapes() {
    let f = flowchart("flowchart LR\n  pitwall-app[App] --> tile.wasm [Wasm]\n  a-->b");
    let s = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(edges(&f), [s("pitwall-app", "tile.wasm"), s("a", "b")]);
    assert_eq!(node(&f, "tile.wasm").lines, ["Wasm"]);
}

#[test]
fn a_later_shape_sets_the_label() {
    let f = flowchart("flowchart LR\n  a --> b\n  b[\"Bee\"]");
    assert_eq!(node(&f, "b").lines, ["Bee"]);
    assert_eq!(node(&f, "b").line, 2);
}

// Row: edges.

#[test]
fn every_arrow() {
    let f = flowchart(&lines(&[
        "flowchart LR",
        "  a --> b",
        "  a --- b",
        "  a -.-> b",
        "  a -.- b",
        "  a ==> b",
        "  a === b",
        "  a --x b",
        "  a --o b",
        "  a <--> b",
        "  a ---> b",
        "  a ~~~ b",
        "  a x--x b",
    ]));
    let got: Vec<(LineStyle, Head, Head)> =
        f.edges.iter().map(|e| (e.style, e.start_head, e.end_head)).collect();
    use Head::{Arrow, Circle, Cross, None as No};
    use LineStyle::*;
    assert_eq!(got, [
        (Solid, No, Arrow),
        (Solid, No, No),
        (Dotted, No, Arrow),
        (Dotted, No, No),
        (Thick, No, Arrow),
        (Thick, No, No),
        (Solid, No, Cross),
        (Solid, No, Circle),
        (Solid, Arrow, Arrow),
        (Solid, No, Arrow),
        (Invisible, No, No),
        (Solid, Cross, Cross),
    ]);
    assert!(f.edges.iter().all(|e| ids(&f, e.from) == "a" && ids(&f, e.to) == "b"));
}

#[test]
fn edge_labels_in_both_forms() {
    let f = flowchart(&lines(&[
        "flowchart LR",
        "  a -->|reads files| b",
        "  a -- reads files --> b",
        "  a -->|\"quoted | pipe\"| b",
        "  a -. maybe .-> b",
        "  a == must ==> b",
        "  a -- \"event (click, key, …)\" --> b",
        "  a --> | spaced | b",
        "  a---|open|b",
    ]));
    let labels: Vec<Option<&str>> = f.edges.iter().map(|e| e.label.as_deref()).collect();
    assert_eq!(labels, [
        Some("reads files"),
        Some("reads files"),
        Some("quoted | pipe"),
        Some("maybe"),
        Some("must"),
        Some("event (click, key, …)"),
        Some("spaced"),
        Some("open"),
    ]);
    assert_eq!(f.edges[3].style, LineStyle::Dotted);
    assert_eq!(f.edges[3].end_head, Head::Arrow);
    assert_eq!(f.edges[4].style, LineStyle::Thick);
    assert_eq!(f.edges[7].end_head, Head::None);
}

#[test]
fn chains_and_ampersands() {
    let f = flowchart("flowchart LR\n  a --> b --> c\n  d & e --> f & g");
    let s = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(edges(&f), [
        s("a", "b"),
        s("b", "c"),
        s("d", "f"),
        s("d", "g"),
        s("e", "f"),
        s("e", "g"),
    ]);
}

#[test]
fn chains_with_labels_and_shapes() {
    let f = flowchart("flowchart LR\n  a[A] -->|one| b(B) -- two --> c{C}");
    assert_eq!(edges(&f).len(), 2);
    assert_eq!(f.edges[1].label.as_deref(), Some("two"));
    assert_eq!(node(&f, "c").shape, NodeShape::Diamond);
}

// Row: subgraphs.

#[test]
fn subgraph_title_forms() {
    let f = flowchart(&lines(&[
        "flowchart LR",
        "  subgraph one [The one]",
        "  a",
        "  end",
        "  subgraph two[\"Pitwall [Software System]\"]",
        "  b",
        "  end",
        "  subgraph three",
        "  c",
        "  end",
        "  subgraph Plain title words",
        "  d",
        "  end",
        "  subgraph \"Quoted\"",
        "  e",
        "  end",
    ]));
    let got: Vec<(&str, &str)> =
        f.boundaries.iter().map(|b| (b.id.as_str(), b.title.as_str())).collect();
    assert_eq!(got, [
        ("one", "The one"),
        ("two", "Pitwall [Software System]"),
        ("three", "three"),
        ("Plain title words", "Plain title words"),
        ("Quoted", "Quoted"),
    ]);
    for (i, id) in ["a", "b", "c", "d", "e"].iter().enumerate() {
        assert_eq!(node(&f, id).boundary, Some(i));
    }
}

#[test]
fn nesting_and_membership() {
    let f = flowchart(&lines(&[
        "flowchart LR",
        "  x --> a",
        "  subgraph outer",
        "    a",
        "    subgraph inner",
        "      b --> c",
        "    end",
        "    c --> d",
        "  end",
        "  d --> e",
    ]));
    assert_eq!(f.boundaries[1].parent, Some(0));
    assert_eq!(node(&f, "x").boundary, None);
    // First mentioned at the top, then inside a subgraph: it moves in.
    assert_eq!(node(&f, "a").boundary, Some(0));
    assert_eq!(node(&f, "b").boundary, Some(1));
    // First mentioned inside the inner subgraph: stays there.
    assert_eq!(node(&f, "c").boundary, Some(1));
    assert_eq!(node(&f, "d").boundary, Some(0));
    assert_eq!(node(&f, "e").boundary, None);
}

#[test]
fn edges_to_a_subgraph_id() {
    let f = flowchart(
        "flowchart LR\n  user --> sys\n  subgraph sys [System]\n    a\n  end\n  sys --> other",
    );
    assert_eq!(edges(&f), [("user".into(), "[sys]".into()), ("[sys]".into(), "other".into())]);
    assert!(f.nodes.iter().all(|n| n.id != "sys"));
}

// Row: classes.

#[test]
fn class_defs_and_applications() {
    let f = flowchart(&lines(&[
        "flowchart LR",
        "  a:::new --> b",
        "  c:::new,changed",
        "  classDef new stroke-dasharray: 4 2",
        "  classDef changed,external fill:#eee",
        "  class b, c external",
        "  subgraph s",
        "  d",
        "  end",
        "  class s new",
    ]));
    assert_eq!(f.class_defs, ["new", "changed", "external"]);
    assert_eq!(node(&f, "a").classes, ["new"]);
    assert_eq!(node(&f, "b").classes, ["external"]);
    assert_eq!(node(&f, "c").classes, ["new", "changed", "external"]);
    assert_eq!(f.boundaries[0].classes, ["new"]);
}

// Row: ignored and listed under "Not shown".

#[test]
fn ignored_parts_are_listed() {
    let f = flowchart(&lines(&[
        "%%{init: {\"theme\": \"dark\"}}%%",
        "flowchart LR",
        "  a[\"fa:fa-car Car\"] --> b[\"`**bold**`\"]",
        "  c[\"<img src='x.png'> pic\"]",
        "  style a fill:#f9f",
        "  linkStyle 0 stroke:#f00",
        "  click a callback",
        "  accTitle: Title",
        "  %%{init: {}}%%",
    ]));
    let got: Vec<(u32, &str)> = f.skipped.iter().map(|s| (s.line, s.what.as_str())).collect();
    assert_eq!(got, [
        (1, "init directive"),
        (3, "icon in a label"),
        (3, "Markdown string in a label"),
        (4, "image in a label"),
        (5, "'style' line"),
        (6, "'linkStyle' line"),
        (7, "'click' line"),
        (8, "'accTitle' line"),
        (9, "init directive"),
    ]);
    assert_eq!(node(&f, "a").lines, ["Car"]);
    assert_eq!(node(&f, "b").lines, ["**bold**"]);
    assert_eq!(f.nodes.len(), 3);
}

// Rows: comments and frontmatter.

#[test]
fn comments() {
    let f = flowchart("%% before\nflowchart LR\n  %% a comment\n  a --> b %% trailing\n%%last");
    assert_eq!(edges(&f), [("a".into(), "b".into())]);
}

#[test]
fn frontmatter_title() {
    let f = flowchart(&lines(&[
        "---",
        "title: Shop containers",
        "config:",
        "  theme: dark",
        "---",
        "flowchart LR",
        "  a",
    ]));
    assert_eq!(f.title.as_deref(), Some("Shop containers"));
}

#[test]
fn entities_in_labels() {
    let f = flowchart("flowchart TD\n  G[\"Move into &lt;data&gt;/extensions/ #quot;x#quot;\"]");
    assert_eq!(node(&f, "G").lines, ["Move into <data>/extensions/ \"x\""]);
}

// Errors: line and column in the Markdown file.

fn md(body: &str) -> String {
    format!("# Doc\n\nText.\n\n```mermaid\n{body}\n```\n")
}

#[test]
fn error_bad_direction() {
    assert_eq!(problems(&md("flowchart XY\n  a --> b")), [(
        6,
        11,
        "expected a direction (TB, TD, BT, LR or RL), found 'XY'".to_string()
    )]);
    assert_eq!(problems(&md("flowchart LR\n  subgraph s\n    direction up\n  end")), [(
        8,
        15,
        "expected a direction (TB, TD, BT, LR or RL), found 'up'".to_string()
    )]);
}

#[test]
fn error_junk_after_a_vertex() {
    assert_eq!(problems(&md("flowchart LR\n  a --> b c")), [(
        7,
        11,
        "expected an arrow, '&' or the end of the line, found 'c'".to_string()
    )]);
}

#[test]
fn error_missing_node_id() {
    assert_eq!(problems(&md("flowchart LR\n  --> b")), [(
        7,
        3,
        "expected a node id, found '-->'".to_string()
    )]);
    assert_eq!(problems(&md("flowchart LR\n  a -->")), [(
        7,
        8,
        "expected a node id, found the end of the line".to_string()
    )]);
}

#[test]
fn error_unclosed_label_and_quote() {
    assert_eq!(problems(&md("flowchart LR\n  a[label\n  b")), [(
        7,
        10,
        "unclosed label: expected ']'".to_string()
    )]);
    assert_eq!(problems(&md("flowchart LR\n  a([label)\n")), [(
        7,
        12,
        "unclosed label: expected '])'".to_string()
    )]);
    assert_eq!(problems(&md("flowchart LR\n  a[\"x\n  b --> c")), [(
        7,
        5,
        "unclosed quote".to_string()
    )]);
}

#[test]
fn error_edge_labels() {
    assert_eq!(problems(&md("flowchart LR\n  a -->|yes b")), [(
        7,
        14,
        "unclosed edge label: expected '|'".to_string()
    )]);
    assert_eq!(problems(&md("flowchart LR\n  a -- yes b")), [(
        7,
        5,
        "edge label with no arrow: expected '-->'".to_string()
    )]);
    assert_eq!(problems(&md("flowchart LR\n  a == yes b")), [(
        7,
        5,
        "edge label with no arrow: expected '==>'".to_string()
    )]);
}

#[test]
fn error_subgraph_ends() {
    assert_eq!(problems(&md("flowchart LR\n  a\n  end")), [(
        8,
        3,
        "'end' without an open subgraph".to_string()
    )]);
    assert_eq!(problems(&md("flowchart LR\n  subgraph s1 [One]\n    a\n")), [(
        7,
        3,
        "subgraph 'One' has no matching 'end'".to_string()
    )]);
}

#[test]
fn error_classes() {
    assert_eq!(problems(&md("flowchart LR\n  classDef")), [(
        7,
        11,
        "expected a class name".to_string()
    )]);
    assert_eq!(problems(&md("flowchart LR\n  class a")), [(
        7,
        9,
        "expected node ids and a class name".to_string()
    )]);
    assert_eq!(problems(&md("flowchart LR\n  a::: --> b")), [(
        7,
        8,
        "expected a class name".to_string()
    )]);
}

#[test]
fn several_errors_in_one_block_and_the_rest_still_checked() {
    let got = problems(&md("flowchart LR\n  a --> b c\n  d --> e\n  f[x\n  end"));
    let lines: Vec<(u32, u32)> = got.iter().map(|p| (p.0, p.1)).collect();
    assert_eq!(lines, [(7, 11), (9, 6), (10, 3)]);
}

#[test]
fn errors_in_an_indented_fence_map_to_the_file() {
    let markdown = "# Doc\n\n- Item:\n\n  ```mermaid\n  flowchart LR\n    a --> b c\n  ```\n";
    assert_eq!(problems(markdown), [(
        7,
        13,
        "expected an arrow, '&' or the end of the line, found 'c'".to_string()
    )]);
}

#[test]
fn errors_give_the_problems_fallback() {
    let f: Fallback = fallback("flowchart LR\n  a --> b c");
    assert_eq!(f.reason, FallbackReason::Problems);
    assert_eq!(f.type_word, "flowchart");
    assert_eq!(f.note(), "This diagram has problems");
}

// Limits (hla.md §8, FR-21).

#[test]
fn too_many_boxes_or_arrows() {
    let boxes: String = (0..301).map(|i| format!("  n{i}\n")).collect();
    let f = fallback(&format!("flowchart LR\n{boxes}"));
    assert_eq!((f.reason, f.note().as_str()), (FallbackReason::TooLarge, "Too large to draw here"));
    let arrows: String = (0..601).map(|i| format!("  a --> b{}\n", i % 10)).collect();
    assert_eq!(fallback(&format!("flowchart LR\n{arrows}")).reason, FallbackReason::TooLarge);
    let ok: String = (0..300).map(|i| format!("  n{i}\n")).collect();
    assert_eq!(flowchart(&format!("flowchart LR\n{ok}")).nodes.len(), 300);
}

#[test]
fn too_deep() {
    let open: String = (0..17).map(|i| format!("subgraph s{i}\n")).collect();
    let close: String = "end\n".repeat(17);
    let f = fallback(&format!("flowchart LR\n{open}  a\n{close}"));
    assert_eq!(f.reason, FallbackReason::TooLarge);
    assert_eq!(f.problems[0].message, "nested deeper than 16 subgraphs");
    assert_eq!(f.problems[0].line, 18);
}
