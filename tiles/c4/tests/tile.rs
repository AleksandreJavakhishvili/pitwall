//! The C4 tile in Pitwall's tile runtime (plan.md phase B): `cargo test`
//! builds tile.wasm first (it needs `rustup target add wasm32-wasip2`),
//! then agents call `c4__show` through the harness as they would through
//! Pitwall's MCP server, and the user clicks, zooms and reads.

use std::time::Instant;

use pitwall_extension_test::{Called, DrawOp, Harness, Session};
use serde_json::json;

fn c4() -> Session {
    let mut tile = Harness::build(env!("CARGO_MANIFEST_DIR")).open("main");
    tile.resize("main", 900., 600.);
    tile
}

/// `c4__show` from `agent` with `item`.
fn show(tile: &mut Session, agent: &str, item: Option<&str>, mermaid: &str) -> Called {
    tile.call(agent, Some("show"), item, json!({ "mermaid": mermaid }))
}

fn ok(c: &Called) -> String {
    assert!(c.is_ok(), "the call failed: {c:?}");
    c.text().to_string()
}

/// The texts the diagram's canvas draws.
fn drawn(tile: &Session) -> Vec<String> {
    tile.canvas("diagram")
        .ops
        .iter()
        .filter_map(|o| match o {
            DrawOp::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

fn draws(tile: &Session, text: &str) -> bool {
    drawn(tile).iter().any(|t| t == text)
}

fn shows(tile: &Session, text: &str) -> bool {
    tile.find_text(text).is_some() || tile.text_snapshot().contains(&format!("\"{text}\""))
}

/// The shop's context diagram: the Shop box is detailed by the
/// containers.
const CONTEXT: &str = r#"flowchart TB
  buyer["Buyer [Person]<br/>orders things"]
  shop["Shop [Software System]<br/>sells things"]
  pay["Payments [External System]<br/>takes cards"]
  buyer -->|orders with| shop
  shop -->|charges with| pay
  class pay external
"#;

const CONTAINERS: &str = r#"flowchart LR
  buyer["Buyer [Person]"]
  subgraph shop["Shop [Software System]"]
    web["Web app [Container]<br/>pages"]
    api["Orders API [Container]<br/>takes orders"]
    db[("Orders DB [Database]<br/>orders")]
  end
  buyer -->|uses| web
  web -->|calls| api
  api -->|reads and writes| db
"#;

/// The containers again, with a cache added and the API changed.
const CONTAINERS_2: &str = r#"flowchart LR
  buyer["Buyer [Person]"]
  subgraph shop["Shop [Software System]"]
    web["Web app [Container]<br/>pages"]
    api["Orders API [Container]<br/>takes orders, caches prices"]
    cache["Price cache [Container]<br/>keeps prices"]
    db[("Orders DB [Database]<br/>orders")]
  end
  buyer -->|uses| web
  web -->|calls| api
  api -->|reads| cache
  api -->|reads and writes| db
  class cache new
  class api changed
"#;

const CHECKOUT: &str = "sequenceDiagram\n  participant web as Web app\n  participant api as Orders API\n  web->>api: POST /orders\n  api-->>web: 201 Created\n";

#[test]
fn an_agent_shows_a_diagram_and_it_is_drawn_with_its_reply() {
    let mut tile = c4();
    assert!(shows(&tile, "No diagrams yet"), "{}", tile.text_snapshot());
    let c = show(&mut tile, "api", Some("containers"), CONTAINERS);
    let text = ok(&c);
    assert!(text.contains("Showing “Buyer” in the C4 tile (item “containers”): 4 boxes, 3 arrows, 1 boundary."), "{text}");
    assert!(matches!(&c, Called::Replied { item: Some(i), .. } if i == "containers"), "{c:?}");
    for name in ["Web app", "Orders API", "Orders DB", "Shop [Software System]", "[Container]", "takes orders", "reads and writes"] {
        assert!(draws(&tile, name), "{name} not drawn: {:?}", drawn(&tile));
    }
    // A region per box, for clicks and screen readers.
    let regions: Vec<String> = tile.canvas("diagram").regions.iter().map(|r| r.label.clone()).collect();
    assert_eq!(regions, ["Buyer", "Web app", "Orders API", "Orders DB"]);
    // In the list, by agent, and kept.
    assert!(shows(&tile, "from api"), "{}", tile.text_snapshot());
    assert!(tile.storage().keys().any(|k| k.starts_with("d/")));
}

#[test]
fn a_title_names_it_and_a_call_without_an_item_gets_one() {
    let mut tile = c4();
    let c = tile.call("api", Some("show"), None, json!({ "mermaid": CONTAINERS, "title": "Shop containers" }));
    let text = ok(&c);
    assert!(text.contains("item “shop-containers”"), "{text}");
    // Again without an item: a second diagram, not a replacement.
    let c = tile.call("api", Some("show"), None, json!({ "mermaid": CONTEXT, "title": "Shop containers" }));
    assert!(ok(&c).contains("item “shop-containers-2”"), "{c:?}");
    // A fenced block is read as its Mermaid.
    let c = show(&mut tile, "api", Some("fenced"), &format!("```mermaid\n{CHECKOUT}```\n"));
    assert!(ok(&c).contains("2 participants, 2 messages"), "{c:?}");
}

#[test]
fn the_same_item_updates_in_place_and_offers_the_changes() {
    let mut tile = c4();
    ok(&show(&mut tile, "api", Some("containers"), CONTAINERS));
    assert!(tile.find_key("mode").is_none(), "nothing to compare yet");
    let text = ok(&show(&mut tile, "api", Some("containers"), CONTAINERS_2));
    assert!(text.contains("Updated “Buyer” in place"), "{text}");
    assert!(text.contains("Since the last show: 1 boxes added, 1 changed, 0 removed"), "{text}");
    // One diagram, drawn new; the classes in the legend.
    assert_eq!(tile.storage().keys().filter(|k| k.starts_with("d/")).count(), 1);
    assert!(draws(&tile, "Price cache"));
    assert!(shows(&tile, "New") && shows(&tile, "Changed"), "{}", tile.text_snapshot());
    // Changes since last show: the cache added, the API changed.
    tile.choose("mode", "changes");
    assert!(shows(&tile, "Added"), "{}", tile.text_snapshot());
    assert!(draws(&tile, "Price cache"));
    // B goes back.
    tile.focus(true).press("b");
    assert!(!shows(&tile, "Added"), "{}", tile.text_snapshot());
}

#[test]
fn a_removed_box_is_a_ghost_in_the_changes() {
    let mut tile = c4();
    ok(&show(&mut tile, "api", Some("containers"), CONTAINERS_2));
    ok(&show(&mut tile, "api", Some("containers"), CONTAINERS));
    assert!(!draws(&tile, "Price cache"));
    tile.choose("mode", "changes");
    // Back as a ghost, so the user sees what went.
    assert!(draws(&tile, "Price cache"), "{:?}", drawn(&tile));
    assert!(shows(&tile, "Removed"), "{}", tile.text_snapshot());
}

#[test]
fn bad_mermaid_changes_nothing_and_comes_back_with_the_line() {
    let mut tile = c4();
    ok(&show(&mut tile, "api", Some("containers"), CONTAINERS));
    let before = drawn(&tile);
    let bad = "flowchart LR\n  a[\"A [Container]\"]\n  a -->|calls b\n";
    let c = show(&mut tile, "api", Some("containers"), bad);
    let Called::Failed(text) = &c else { panic!("{c:?}") };
    assert!(text.starts_with("The diagram has problems; nothing was changed."), "{text}");
    assert!(text.contains("line 3, column"), "{text}");
    assert_eq!(drawn(&tile), before, "the drawing didn't change");
    // The user sees why, and the line, in the source.
    assert!(shows(&tile, "api's diagram “containers” wasn't drawn"), "{}", tile.text_snapshot());
    tile.click_key("notice0");
    let code = tile.code_view("problems");
    assert_eq!(code.marks.len(), 1, "line 3 marked");
    assert_eq!(code.marks[0].lines.start, 2);
    assert!(shows(&tile, "Line 3, column 16"), "{}", tile.text_snapshot());
    // The next good show of that item clears it.
    ok(&show(&mut tile, "api", Some("containers"), CONTAINERS_2));
    assert!(!shows(&tile, "wasn't drawn"), "{}", tile.text_snapshot());
}

#[test]
fn problems_in_a_fenced_block_count_lines_in_the_agents_text() {
    let mut tile = c4();
    let c = show(&mut tile, "api", Some("x"), "```mermaid\nflowchart LR\n  a -->|calls b\n```");
    assert!(c.text().contains("line 3, column"), "{c:?}");
}

#[test]
fn unsupported_types_and_too_large_diagrams_are_refused() {
    let mut tile = c4();
    let c = show(&mut tile, "api", Some("classes"), "classDiagram\n  class A\n");
    assert!(matches!(&c, Called::Failed(t) if t.contains("Class diagrams aren't drawn here") && t.contains("Nothing was changed")), "{c:?}");
    let c = show(&mut tile, "api", Some("c4"), "C4Context\n  Person(a, \"A\")\n");
    assert!(matches!(&c, Called::Failed(t) if t.contains("Mermaid's C4 syntax isn't drawn here")), "{c:?}");
    let mut big = String::from("flowchart LR\n");
    for i in 0..301 {
        big.push_str(&format!("  n{i}[\"Box {i}\"]\n"));
    }
    let c = show(&mut tile, "api", Some("big"), &big);
    assert!(matches!(&c, Called::Failed(t) if t.contains("Too large to draw here")), "{c:?}");
    let c = show(&mut tile, "api", Some("empty"), "   \n");
    assert!(matches!(&c, Called::Failed(t) if t.contains("has no diagram")), "{c:?}");
    assert!(shows(&tile, "No diagrams yet"), "nothing was added");
}

#[test]
fn skipped_parts_are_drawn_around_and_listed() {
    let mut tile = c4();
    let m = "flowchart LR\n  a[\"A [Container]\"] --> b[\"B [Container]\"]\n  click a callback\n  linkStyle 0 stroke:#f00\n";
    let text = ok(&show(&mut tile, "api", Some("skips"), m));
    assert!(text.contains("Not shown: line 3:"), "{text}");
    assert!(text.contains("line 4:"), "{text}");
    assert!(draws(&tile, "A") && draws(&tile, "B"));
    assert!(shows(&tile, "Not shown: 2"), "{}", tile.text_snapshot());
}

#[test]
fn a_detailed_box_opens_its_diagram_and_backspace_goes_up() {
    let mut tile = c4();
    ok(&show(&mut tile, "api", Some("containers"), CONTAINERS));
    let text = ok(&show(&mut tile, "api", Some("context"), CONTEXT));
    assert!(text.contains("1 of its boxes open another diagram you showed"), "{text}");
    let shop = tile.canvas("diagram").regions.iter().find(|r| r.label == "Shop").cloned().unwrap();
    assert_eq!(shop.tooltip.as_deref(), Some("Shop: click to open its diagram"));
    tile.click_region("Shop");
    assert!(draws(&tile, "Orders API"), "{:?}", drawn(&tile));
    // The breadcrumb: context, then containers.
    let snap = tile.text_snapshot();
    assert!(snap.contains("Shop") && snap.contains("Buyer"), "{snap}");
    tile.choose("path", "0");
    assert!(draws(&tile, "Payments"), "{:?}", drawn(&tile));
    tile.click_region("Shop");
    tile.focus(true).press("backspace");
    assert!(draws(&tile, "Payments"), "back up: {:?}", drawn(&tile));
}

#[test]
fn a_box_several_diagrams_detail_asks_which() {
    let mut tile = c4();
    ok(&show(&mut tile, "api", Some("containers"), CONTAINERS));
    ok(&show(&mut tile, "api", Some("containers-2"), CONTAINERS_2));
    ok(&show(&mut tile, "api", Some("context"), CONTEXT));
    // Two of its diagrams detail Shop: a menu to pick one.
    tile.click_region("Shop");
    assert!(shows(&tile, "Open which diagram?"), "{}", tile.text_snapshot());
    assert!(draws(&tile, "Payments"), "still the context");
    tile.click_key("t0");
    assert!(!shows(&tile, "Open which diagram?"));
    assert!(draws(&tile, "Orders API"), "{:?}", drawn(&tile));
}

#[test]
fn a_box_click_selects_it_and_the_footer_tells_about_it() {
    let mut tile = c4();
    ok(&show(&mut tile, "api", Some("checkout"), CHECKOUT));
    ok(&show(&mut tile, "api", Some("containers"), CONTAINERS));
    tile.click_region("Orders API");
    assert!(shows(&tile, "takes orders"), "{}", tile.text_snapshot());
    // The sequence with a participant of that Name.
    assert!(shows(&tile, "Orders API"), "{}", tile.text_snapshot());
    tile.click_key("seq0");
    assert_eq!(tile.canvas("diagram").regions.len(), 2, "the sequence's participants");
    assert!(draws(&tile, "POST /orders"), "{:?}", drawn(&tile));
}

#[test]
fn the_keys_move_the_selection_zoom_and_fit() {
    let mut tile = c4();
    ok(&show(&mut tile, "api", Some("containers"), CONTAINERS));
    let fitted = tile.canvas("diagram").transform;
    tile.focus(true).press("right");
    assert!(shows(&tile, "Buyer"), "{}", tile.text_snapshot());
    tile.press("right");
    assert!(tile.find_key("selected").is_some() && tile.text_snapshot().contains("Web app"), "{}", tile.text_snapshot());
    tile.press("+");
    assert!(tile.canvas("diagram").transform.scale > fitted.scale);
    tile.press("-").press("-");
    assert!(tile.canvas("diagram").transform.scale < fitted.scale);
    tile.press("f");
    assert_eq!(tile.canvas("diagram").transform, fitted);
    // The wheel with ⌘ zooms too.
    tile.wheel("diagram", 0., -10., 100., 100.);
}

#[test]
fn sequence_diagrams_are_drawn() {
    let mut tile = c4();
    let m = "sequenceDiagram\n  autonumber\n  actor u as User\n  participant web as Web app\n  participant api as Orders API\n  u->>web: order\n  activate web\n  loop until paid\n    web->>+api: POST /orders\n    api-->>-web: 201\n  end\n  alt card refused\n    web-xu: error\n  else paid\n    web--)u: receipt\n  end\n  Note over web,api: all over TLS\n  deactivate web\n";
    let text = ok(&show(&mut tile, "api", Some("checkout"), m));
    assert!(text.contains("3 participants, 5 messages"), "{text}");
    for t in ["User", "Web app", "Orders API", "1. order", "2. POST /orders", "loop", "[until paid]", "alt", "[else paid]", "all over TLS"] {
        assert!(draws(&tile, t), "{t}: {:?}", drawn(&tile));
    }
}

#[test]
fn the_drawing_is_theme_tokens_only_and_the_same_in_dark_and_light() {
    let mut tile = c4();
    ok(&show(&mut tile, "api", Some("containers"), CONTAINERS_2));
    let dark = tile.canvas("diagram");
    let ops = format!("{:?}", dark.ops);
    assert!(!ops.contains("Rgb"), "only theme tokens: {ops}");
    tile.light();
    let light = tile.canvas("diagram");
    assert_eq!(format!("{:?}", light.ops), ops, "Pitwall colours the tokens per theme");
}

#[test]
fn shown_diagrams_come_back_after_a_restart() {
    let mut tile = c4();
    ok(&tile.call("api", Some("show"), Some("containers"), json!({ "mermaid": CONTAINERS, "title": "Shop containers" })));
    ok(&show(&mut tile, "api", Some("context"), CONTEXT));
    tile.click_text("Shop containers");
    tile.reload();
    tile.resize("main", 900., 600.);
    assert!(draws(&tile, "Orders API"), "the view's diagram came back: {:?}", drawn(&tile));
    tile.restart();
    tile.resize("main", 900., 600.);
    assert!(shows(&tile, "from api"), "{}", tile.text_snapshot());
    // Updating after the restart still updates in place.
    let text = ok(&show(&mut tile, "api", Some("containers"), CONTAINERS_2));
    assert!(text.contains("Updated"), "{text}");
}

#[test]
fn a_fifty_first_diagram_closes_the_oldest_unopened() {
    // Fifty kept from before (Pitwall lets an agent make ten calls in ten
    // seconds), none opened for a long time.
    let mut h = Harness::build(env!("CARGO_MANIFEST_DIR"));
    for i in 0..50u64 {
        let item = json!({ "project": "", "agent_id": "api", "agent_name": "api", "item": format!("d{i}"), "title": format!("Box {i}"), "mermaid": format!("flowchart LR\n  a{i}[\"Box {i}\"]\n"), "shown_at": 1000 + i, "opened_at": 1000 + i });
        h = h.stored_json(&format!("d/k{i:02}"), &item);
    }
    let mut tile = h.open("main");
    tile.resize("main", 900., 600.);
    // The user opens Box 0 again: Box 1 is now the oldest unopened.
    tile.click_text("Box 0");
    assert!(draws(&tile, "Box 0"));
    let text = ok(&show(&mut tile, "api", Some("d50"), "flowchart LR\n  z[\"Box 50\"]\n"));
    assert!(text.contains("The oldest 1 diagrams of this project were closed to make room."), "{text}");
    assert_eq!(tile.storage().keys().filter(|k| k.starts_with("d/")).count(), 50);
    assert!(shows(&tile, "Box 0") && shows(&tile, "Box 50") && shows(&tile, "Box 2"), "{}", tile.text_snapshot());
    assert!(!tile.texts().iter().any(|t| t == "Box 1"), "{}", tile.text_snapshot());
}

#[test]
fn a_shown_diagram_can_be_closed_and_opened_by_its_item() {
    let mut tile = c4();
    ok(&show(&mut tile, "api", Some("containers"), CONTAINERS));
    ok(&show(&mut tile, "api", Some("context"), CONTEXT));
    // c4__open at a diagram the agent showed.
    let c = tile.call("api", None, None, json!({ "diagram": "containers" }));
    assert!(c.text().contains("Showing “Buyer”"), "{c:?}");
    assert!(draws(&tile, "Orders API"));
    // Another agent's item isn't its to open.
    let c = tile.call("web", None, None, json!({ "diagram": "containers" }));
    assert!(matches!(&c, Called::Failed(t) if t.contains("No diagram “containers” of yours")), "{c:?}");
    // Closed from the More menu: gone, the other one shows.
    tile.choose_menu_item("more", "Close this diagram");
    assert!(draws(&tile, "Payments"), "{:?}", drawn(&tile));
    assert_eq!(tile.storage().keys().filter(|k| k.starts_with("d/")).count(), 1);
    // The source, read-only.
    tile.choose_menu_item("more", "Show the source");
    assert!(tile.code_view("source").text.contains("charges with"));
}

/// NFR-2: parse, layout and first draw of 50, 150 and 300 boxes, one
/// `c4__show` each, in Wasmtime (the release tile.wasm). By hand:
/// `cargo test --release --test tile speed -- --ignored --nocapture`.
#[test]
#[ignore]
fn speed() {
    let mut tile = c4();
    for n in [50usize, 150, 300] {
        // Boundaries of ten boxes, chains inside, links to the next.
        let mut m = String::from("flowchart LR\n");
        for b in 0..n / 10 {
            m.push_str(&format!("  subgraph g{b}[\"Group {b} [Container]\"]\n"));
            for i in 0..10 {
                m.push_str(&format!("    n{b}_{i}[\"Box {b}.{i} [Component]<br/>does part {i}\"]\n"));
            }
            m.push_str("  end\n");
            for i in 0..9 {
                m.push_str(&format!("  n{b}_{i} -->|step| n{b}_{}\n", i + 1));
            }
            if b > 0 {
                m.push_str(&format!("  n{}_9 --> n{b}_0\n", b - 1));
            }
        }
        let start = Instant::now();
        let c = show(&mut tile, "api", Some(&format!("n{n}")), &m);
        let took = start.elapsed();
        assert!(c.is_ok(), "{c:?}");
        println!("{n} boxes: {took:?} (parse, layout, reply, first draw)");
    }
}

#[test]
fn an_update_without_a_title_keeps_its_title() {
    let mut tile = c4();
    ok(&tile.call("api", Some("show"), Some("containers"), json!({ "mermaid": CONTAINERS, "title": "Shop containers" })));
    let text = ok(&show(&mut tile, "api", Some("containers"), CONTAINERS_2));
    assert!(text.starts_with("Updated “Shop containers” in place"), "{text}");
    assert!(shows(&tile, "Shop containers"), "{}", tile.text_snapshot());
}
