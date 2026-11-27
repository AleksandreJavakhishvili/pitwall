//! The sequence parser: every sequence item of frd.md §6.1 (mermaid-lld.md
//! §4).

mod common;

use c4_tile::mermaid::sequence::{BranchKind, FrameKind, MessageHead, NotePlacement, Step};
use c4_tile::mermaid::{Diagram, FallbackReason};
use common::{fallback, lines, problems, sequence};

fn names(s: &c4_tile::mermaid::Sequence) -> Vec<(&str, &str, bool)> {
    s.participants.iter().map(|p| (p.id.as_str(), p.label.as_str(), p.actor)).collect()
}

#[test]
fn participants_actors_and_aliases() {
    let s = sequence(&lines(&[
        "sequenceDiagram",
        "  participant agent as Coding agent",
        "  actor U as User",
        "  participant host",
        "  participant Alice Smith",
        "  agent->>Bob: hi",
    ]));
    assert_eq!(names(&s), [
        ("agent", "Coding agent", false),
        ("U", "User", true),
        ("host", "host", false),
        ("Alice Smith", "Alice Smith", false),
        ("Bob", "Bob", false),
    ]);
}

#[test]
fn every_message_arrow() {
    let s = sequence(&lines(&[
        "sequenceDiagram",
        "  A->B: a",
        "  A-->B: b",
        "  A->>B: c",
        "  A-->>B: d",
        "  A-xB: e",
        "  A--xB: f",
        "  A-)B: g",
        "  A--)B: h",
    ]));
    let got: Vec<(bool, MessageHead, &str)> = s
        .steps
        .iter()
        .map(|step| match step {
            Step::Message { dotted, head, text, .. } => (*dotted, *head, text.as_str()),
            other => panic!("{other:?}"),
        })
        .collect();
    use MessageHead::*;
    assert_eq!(got, [
        (false, None, "a"),
        (true, None, "b"),
        (false, Arrow, "c"),
        (true, Arrow, "d"),
        (false, Cross, "e"),
        (true, Cross, "f"),
        (false, Async, "g"),
        (true, Async, "h"),
    ]);
}

#[test]
fn message_text_keeps_colons_and_brackets() {
    let s = sequence(&lines(&[
        "sequenceDiagram",
        "  W->>G: handle(event) [deadline 1 s]",
        "  G->>W: host call: x",
        "  A->>B:",
    ]));
    let texts: Vec<&str> = s
        .steps
        .iter()
        .filter_map(|st| match st {
            Step::Message { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["handle(event) [deadline 1 s]", "host call: x", ""]);
}

#[test]
fn activations_with_plus_minus_and_keywords() {
    let s = sequence(&lines(&[
        "sequenceDiagram",
        "  web->>+routes: POST",
        "  routes-->>-web: 201",
        "  activate web",
        "  deactivate web",
    ]));
    match &s.steps[0] {
        Step::Message { activate_target, deactivate_source, .. } => {
            assert!(*activate_target && !*deactivate_source)
        }
        other => panic!("{other:?}"),
    }
    match &s.steps[1] {
        Step::Message { deactivate_source, from, .. } => assert!(*deactivate_source && *from == 1),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.steps[2], Step::Activate { participant: 0, line: 4 });
    assert_eq!(s.steps[3], Step::Deactivate { participant: 0, line: 5 });
}

#[test]
fn notes() {
    let s = sequence(&lines(&[
        "sequenceDiagram",
        "  Note left of A: one",
        "  note right of B: two",
        "  Note over A,B: three",
        "  Note over A: four",
    ]));
    let got: Vec<(NotePlacement, usize, Option<usize>, &str)> = s
        .steps
        .iter()
        .map(|st| match st {
            Step::Note { placement, first, second, text, .. } => {
                (*placement, *first, *second, text.as_str())
            }
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(got, [
        (NotePlacement::LeftOf, 0, None, "one"),
        (NotePlacement::RightOf, 1, None, "two"),
        (NotePlacement::Over, 0, Some(1), "three"),
        (NotePlacement::Over, 0, None, "four"),
    ]);
}

#[test]
fn every_frame_and_branch() {
    let s = sequence(&lines(&[
        "sequenceDiagram",
        "  loop every 2 s",
        "    A->>B: poll",
        "  end",
        "  alt ok",
        "    B-->>A: yes",
        "  else no",
        "    B-->>A: no",
        "  end",
        "  opt cached",
        "    A->>A: read",
        "  end",
        "  par one",
        "    A->>B: x",
        "  and two",
        "    A->>C: y",
        "  end",
        "  critical connect",
        "    A->>B: c",
        "  option timeout",
        "    A->>A: retry",
        "  end",
        "  break failed",
        "    A->>B: stop",
        "  end",
        "  rect rgb(200, 220, 255)",
        "    A->>B: z",
        "  end",
    ]));
    let frames: Vec<String> = s
        .steps
        .iter()
        .filter_map(|st| match st {
            Step::FrameStart { kind, label, .. } => Some(format!("start {kind:?} {label}")),
            Step::FrameBranch { kind, label, .. } => Some(format!("branch {kind:?} {label}")),
            Step::FrameEnd { .. } => Some("end".to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(frames, [
        "start Loop every 2 s",
        "end",
        "start Alt ok",
        "branch Else no",
        "end",
        "start Opt cached",
        "end",
        "start Par one",
        "branch And two",
        "end",
        "start Critical connect",
        "branch Option timeout",
        "end",
        "start Break failed",
        "end",
        "start Rect ",
        "end",
    ]);
    assert_eq!(s.skipped.iter().map(|k| k.what.as_str()).collect::<Vec<_>>(), ["'rect' colour"]);
    assert!(matches!(s.steps[5], Step::FrameBranch { kind: BranchKind::Else, line: 7, .. }));
    assert!(matches!(s.steps.last(), Some(Step::FrameEnd { line: 28 })));
    assert!(matches!(s.steps[0], Step::FrameStart { kind: FrameKind::Loop, line: 2, .. }));
}

#[test]
fn autonumber_and_boxes() {
    let s = sequence(&lines(&[
        "sequenceDiagram",
        "  autonumber",
        "  box Aqua Pitwall",
        "    participant app",
        "    participant host",
        "  end",
        "  box rgb(1,2,3) Repo",
        "    participant repo",
        "  end",
        "  participant agent",
        "  app->>repo: read",
    ]));
    assert!(s.autonumber);
    let boxes: Vec<&str> = s.boxes.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(boxes, ["Aqua Pitwall", "Repo"]);
    let groups: Vec<Option<usize>> = s.participants.iter().map(|p| p.box_group).collect();
    assert_eq!(groups, [Some(0), Some(0), Some(1), None]);
}

#[test]
fn title_comments_and_skipped_lines() {
    let s = sequence(&lines(&[
        "---",
        "title: Front",
        "---",
        "sequenceDiagram",
        "  %% comment",
        "  title Checkout",
        "  accTitle: x",
        "  link A: Docs @ https://example.com",
        "  create participant N",
        "  A->>N: hi;",
    ]));
    assert_eq!(s.title.as_deref(), Some("Checkout"));
    let what: Vec<&str> = s.skipped.iter().map(|k| k.what.as_str()).collect();
    assert_eq!(what, ["'accTitle' line", "'link' line", "'create'"]);
    assert_eq!(s.steps.len(), 1);
}

#[test]
fn fixtures_parse() {
    let index =
        common::index_of(&["c4-tile/hla.md", "tile-sdk/diagrams.md", "shop/docs/orders/hla.md"]);
    let sequences: Vec<&str> = index
        .entries()
        .iter()
        .filter(|e| matches!(e.diagram, Diagram::Sequence(_)))
        .map(|e| e.id())
        .collect();
    assert_eq!(sequences, [
        "c4-tile/hla.md#39-file-change-to-redraw",
        "c4-tile/hla.md#5-v2-agent-tools",
        "shop/docs/orders/hla.md#checkout-flow",
        "tile-sdk/diagrams.md#24-threading-with-gpui",
    ]);
    let entry = index.get("c4-tile/hla.md#39-file-change-to-redraw").unwrap();
    let Diagram::Sequence(s) = &entry.diagram else { unreachable!() };
    assert_eq!(s.participants.len(), 8);
    assert_eq!(s.participants[2].label, "pitwall-tile-host");
}

fn md(body: &str) -> String {
    format!("# Doc\n\n```mermaid\nsequenceDiagram\n{body}\n```\n")
}

#[test]
fn errors() {
    let one = |body: &str| problems(&md(body));
    let p = |line, column, message: &str| vec![(line, column, message.to_string())];
    assert_eq!(one("  participant"), p(5, 3, "expected a participant name"));
    assert_eq!(one("  A->>B hello"), p(5, 14, "expected ':' and the message text"));
    assert_eq!(one("  ->>B: x"), p(5, 3, "expected a participant name"));
    assert_eq!(one("  Note under A: x"), p(5, 3, "expected 'left of', 'right of' or 'over'"));
    assert_eq!(one("  else nope"), p(5, 3, "'else' outside 'alt'"));
    assert_eq!(one("  loop x\n    and y\n  end"), p(6, 5, "'and' outside 'par'"));
    assert_eq!(one("  alt x\n    option y\n  end"), p(6, 5, "'option' outside 'critical'"));
    assert_eq!(one("  end"), p(5, 3, "'end' without an open frame"));
    assert_eq!(
        one("  A->>B: x\n  loop forever\n    A->>B: y"),
        p(6, 3, "'loop' has no matching 'end'")
    );
    assert_eq!(one("  box Group\n    participant A"), p(5, 3, "'box' has no matching 'end'"));
    assert_eq!(one("  deactivate A"), p(5, 3, "'A' is not active"));
    assert_eq!(one("  A-->>-B: x"), p(5, 3, "'A' is not active"));
    assert_eq!(one("  hello there"), p(5, 3, "not a sequence statement: 'hello'"));
}

#[test]
fn too_many_steps() {
    let steps: String = (0..601).map(|i| format!("  A->>B: {i}\n")).collect();
    assert_eq!(fallback(&format!("sequenceDiagram\n{steps}")).reason, FallbackReason::TooLarge);
}
