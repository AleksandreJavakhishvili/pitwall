//! Unsupported types and partial support (FR-18, FR-20; mermaid-lld.md §2).

mod common;

use c4_tile::mermaid::{Diagram, FallbackReason};

#[test]
fn every_type_of_frd_6_2_falls_back() {
    let index = common::index_of(&["unsupported.md"]);
    let got: Vec<(&str, FallbackReason, String)> = index
        .entries()
        .iter()
        .map(|e| match &e.diagram {
            Diagram::Fallback(f) => (f.type_word.as_str(), f.reason, f.note()),
            other => panic!("{}: not a fallback: {other:?}", e.id()),
        })
        .collect();
    use FallbackReason::*;
    let s = |t: &'static str, r, n: &str| (t, r, n.to_string());
    assert_eq!(got, [
        s("classDiagram", Unsupported, "Class diagrams aren't drawn here"),
        s("stateDiagram-v2", Unsupported, "State diagrams aren't drawn here"),
        s("erDiagram", Unsupported, "Entity relationship diagrams aren't drawn here"),
        s("gantt", Unsupported, "Gantt charts aren't drawn here"),
        s("pie", Unsupported, "Pie charts aren't drawn here"),
        s("mindmap", Unsupported, "Mindmaps aren't drawn here"),
        s("timeline", Unsupported, "Timelines aren't drawn here"),
        s(
            "C4Context",
            MermaidC4,
            "Mermaid's C4 syntax isn't drawn here; write C4 diagrams as flowcharts",
        ),
        s("zenDiagram", Unsupported, "'zenDiagram' diagrams aren't drawn here"),
        s("", Empty, "The block is empty"),
    ]);
    assert!(index.entries().iter().all(|e| e.problems().is_empty()));
}

#[test]
fn every_mermaid_c4_word_falls_back() {
    for word in ["C4Context", "C4Container", "C4Component", "C4Dynamic", "C4Deployment"] {
        let f = common::fallback(&format!("{word}\n  title x"));
        assert_eq!(f.reason, FallbackReason::MermaidC4, "{word}");
    }
}

#[test]
fn the_tile_sdk_state_diagram_falls_back() {
    let index = common::index_of(&["tile-sdk/diagrams.md"]);
    let e = index.get("tile-sdk/diagrams.md#23-lifecycle").unwrap();
    assert!(matches!(&e.diagram, Diagram::Fallback(f) if f.type_word == "stateDiagram-v2"));
}

#[test]
fn unclosed_frontmatter_is_a_problem() {
    let f = common::fallback("---\ntitle: x\nflowchart LR\n  a");
    assert_eq!(f.reason, FallbackReason::Problems);
    assert_eq!(f.problems[0].message, "frontmatter has no closing '---'");
}

#[test]
fn partial_support_lists_what_isnt_shown_and_still_draws() {
    let d = c4_tile::mermaid::parse(&common::lines(&[
        "flowchart LR",
        "  a[[Sub]] --> b",
        "  click a href \"https://example.com\"",
        "  linkStyle 0 stroke:red",
    ]));
    let Diagram::Flowchart(f) = &d else { panic!("{d:?}") };
    assert_eq!(f.edges.len(), 1);
    let what: Vec<(u32, &str)> = d.skipped().iter().map(|s| (s.line, s.what.as_str())).collect();
    assert_eq!(what, [
        (2, "shape '[[…' drawn as a box"),
        (3, "'click' line"),
        (4, "'linkStyle' line"),
    ]);
}
