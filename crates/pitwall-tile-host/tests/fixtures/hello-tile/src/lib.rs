//! The spike's hello tile: a counter with a button, plus two spike-only
//! modes driven by events: a synthetic tree of N nodes (canonical-ABI and
//! diff measurements) and an endless loop (epoch interruption).

use std::cell::RefCell;

wit_bindgen::generate!({
    path: "../../../wit",
    world: "tile",
});

use pitwall::tile_spike::log::{log, Level};
// `Frame` and `Event` come from the world's `use` (generated at the top).
use pitwall::tile_spike::types::{Button, Element, Length, Node, Style, TextSize, Token};

struct State {
    clicks: u32,
    /// 0: the hello tile; otherwise a synthetic tree of this many nodes.
    synthetic: u32,
    cached: Option<Frame>,
    use_cache: bool,
}

thread_local! {
    static STATE: RefCell<State> = const {
        RefCell::new(State { clicks: 0, synthetic: 0, cached: None, use_cache: false })
    };
}

const CLICK: u32 = 1;

struct Tile;

impl Guest for Tile {
    fn render() -> Frame {
        STATE.with_borrow_mut(|s| {
            if s.synthetic == 0 {
                return hello(s.clicks);
            }
            if s.use_cache {
                if s.cached.is_none() {
                    s.cached = Some(synthetic(s.synthetic, s.clicks));
                }
                return s.cached.clone().unwrap();
            }
            synthetic(s.synthetic, s.clicks)
        })
    }

    fn on_event(event: Event) -> bool {
        match event {
            Event::Click(CLICK) => {
                STATE.with_borrow_mut(|s| s.clicks += 1);
                true
            }
            #[cfg(feature = "heavy")]
            Event::Click(2) => {
                log(Level::Info, &heavy::work());
                false
            }
            Event::Click(other) => {
                log(Level::Warn, &format!("unknown handler {other}"));
                false
            }
            Event::Configure(c) => {
                STATE.with_borrow_mut(|s| {
                    s.synthetic = c.nodes;
                    s.use_cache = c.cached;
                    s.cached = None;
                });
                log(Level::Info, &format!("synthetic tree of {} nodes", c.nodes));
                true
            }
            Event::Spin => {
                let mut n: u64 = 0;
                loop {
                    n = std::hint::black_box(n.wrapping_add(1));
                }
            }
        }
    }
}

fn node(parent: Option<u32>, key: impl Into<String>, element: Element, style: Vec<Style>) -> Node {
    Node { parent, key: key.into(), element, style }
}

fn hello(clicks: u32) -> Frame {
    let nodes = vec![
        node(
            None,
            "root",
            Element::Box,
            vec![
                Style::FlexCol,
                Style::Gap(Length::Px(8.0)),
                Style::Padding(Length::Px(12.0)),
                Style::Background(Token::Surface),
            ],
        ),
        node(
            Some(0),
            "title",
            Element::Text("Hello from a tile".into()),
            vec![Style::TextSize(TextSize::Lg), Style::Color(Token::Text)],
        ),
        node(
            Some(0),
            "count",
            Element::Text(format!("Clicked {clicks} times")),
            vec![Style::Color(Token::Text2)],
        ),
        node(
            Some(0),
            "button",
            Element::Button(Button { label: "Click me".into(), on_click: CLICK }),
            vec![],
        ),
    ];
    Frame { nodes }
}

/// A list of `n` nodes shaped like a real tile: rows of (box, icon-ish
/// text, title text, detail text, button) under one root, with a style list
/// on each. `version` changes the text of one row in 50, as a refresh would.
fn synthetic(n: u32, version: u32) -> Frame {
    let mut nodes = Vec::with_capacity(n as usize);
    nodes.push(node(None, "root", Element::Box, vec![Style::FlexCol, Style::Gap(Length::Px(2.0))]));
    let mut row = 0u32;
    while (nodes.len() as u32) < n {
        let row_ix = nodes.len() as u32;
        nodes.push(node(
            Some(0),
            format!("r{row}"),
            Element::Box,
            vec![
                Style::FlexRow,
                Style::Gap(Length::Px(6.0)),
                Style::Padding(Length::Px(4.0)),
                Style::Height(Length::Px(24.0)),
                Style::Background(if row % 2 == 0 { Token::Surface } else { Token::Surface2 }),
            ],
        ));
        let changed = row % 50 == version % 50;
        let children: [(&str, Element, Vec<Style>); 4] = [
            ("dot", Element::Text("●".into()), vec![Style::Color(Token::Good)]),
            (
                "title",
                Element::Text(format!("Item {row}")),
                vec![Style::Color(Token::Text), Style::Grow(1.0), Style::Truncate],
            ),
            (
                "detail",
                Element::Text(if changed { format!("updated {version}") } else { format!("{} KB", row * 3) }),
                vec![Style::Color(Token::Text3), Style::TextSize(TextSize::Sm)],
            ),
            (
                "open",
                Element::Button(Button { label: "Open".into(), on_click: 1000 + row }),
                vec![],
            ),
        ];
        for (key, element, style) in children {
            if nodes.len() as u32 >= n {
                break;
            }
            nodes.push(node(Some(row_ix), key, element, style));
        }
        row += 1;
    }
    Frame { nodes }
}

/// `--features heavy`: code the size of a real tile's (a JSON API client
/// with a regex filter), to measure compile time on more than a toy. Not
/// committed built; see ../build-hello-tile.sh.
#[cfg(feature = "heavy")]
mod heavy {
    pub fn work() -> String {
        let re = regex::Regex::new(r"(?i)^(fix|feat)\((\w+)\): (.+)$").unwrap();
        let v: serde_json::Value =
            serde_json::from_str(r#"{"items":[{"title":"feat(ui): a"},{"title":"fix(core): b"}]}"#).unwrap();
        let hits: Vec<String> = v["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i["title"].as_str())
            .filter_map(|t| re.captures(t).map(|c| c[2].to_string()))
            .collect();
        let when = chrono::DateTime::parse_from_rfc3339("2026-11-26T12:00:00Z").unwrap();
        format!("{hits:?} at {}", when.format("%A %e %B"))
    }
}

export!(Tile);
