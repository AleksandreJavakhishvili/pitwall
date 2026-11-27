//! The hello tile: the host's test fixture for `pitwall:tile@0.1.0`, written
//! against the raw WIT bindings (no SDK). A counter kept in `storage`, a
//! greeting from `settings`, a text field, a timer, and test-only actions
//! the host's tests trigger by handler number (see `tests/host.rs`).

use std::cell::RefCell;

wit_bindgen::generate!({
    path: "../../../wit",
    world: "tile",
});

use exports::pitwall::tile::guest::Guest;
use pitwall::tile::types::{
    Button, ButtonKind, Edges, Element, Event, Frame, Icon, InitContext, Input, Length, Listen, Listener, Node, Style,
    TextSize, Token, Tone,
};
use pitwall::tile::{log, settings, storage, tasks, ui};

/// Handlers, by number.
const CLICK: u32 = 1;
const SPIN: u32 = 2;
const HUGE: u32 = 3;
const PRINT: u32 = 4;
const TIMER: u32 = 5;
const TOAST: u32 = 6;
const PANIC: u32 = 7;
const GROW: u32 = 8;
#[cfg(feature = "heavy")]
const HEAVY: u32 = 9;
const TYPED: u32 = 10;

#[derive(Default)]
struct State {
    clicks: u32,
    ticks: u32,
    typed: String,
    content: String,
    huge: bool,
    hoard: Vec<Vec<u8>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

struct Tile;

impl Guest for Tile {
    fn init(cx: InitContext) -> Result<(), String> {
        let clicks = match storage::get("clicks") {
            Ok(Some(b)) if b.len() == 4 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            Ok(_) => 0,
            Err(e) => return Err(format!("storage: {e:?}")),
        };
        STATE.with_borrow_mut(|s| s.clicks = clicks);
        log::info(&format!("hello {} {} starts with {clicks} clicks", cx.extension, cx.version));
        Ok(())
    }

    fn render(view: String) -> Frame {
        STATE.with_borrow(|s| if s.huge { huge() } else { hello(&view, s) })
    }

    fn handle(event: Event) -> bool {
        match event {
            Event::Click(c) => match c.handler {
                CLICK => {
                    let clicks = STATE.with_borrow_mut(|s| {
                        s.clicks += 1;
                        s.clicks
                    });
                    storage::set("clicks", &clicks.to_le_bytes()).is_ok()
                }
                SPIN => {
                    let mut n: u64 = 0;
                    loop {
                        n = std::hint::black_box(n.wrapping_add(1));
                    }
                }
                HUGE => {
                    STATE.with_borrow_mut(|s| s.huge = true);
                    true
                }
                PRINT => {
                    println!("hello from stdout");
                    eprintln!("hello from stderr");
                    log::debug("a debug line");
                    false
                }
                TIMER => {
                    if let Err(e) = tasks::timer(100, false) {
                        log::error(&format!("timer: {e:?}"));
                    }
                    false
                }
                TOAST => {
                    ui::toast("Saved", Tone::Good);
                    false
                }
                PANIC => panic!("boom"),
                GROW => {
                    // 1 MiB at a time until the memory limit traps.
                    STATE.with_borrow_mut(|s| loop {
                        s.hoard.push(vec![1u8; 1 << 20]);
                    })
                }
                #[cfg(feature = "heavy")]
                HEAVY => {
                    log::info(&heavy::work());
                    false
                }
                other => {
                    log::warn(&format!("unknown handler {other}"));
                    false
                }
            },
            Event::InputChanged(c) if c.handler == TYPED => {
                STATE.with_borrow_mut(|s| s.typed = c.value);
                true
            }
            Event::Timer(_) => {
                STATE.with_borrow_mut(|s| s.ticks += 1);
                true
            }
            Event::ViewOpened(v) => {
                if let Some(json) = v.content.and_then(|c| c.json) {
                    STATE.with_borrow_mut(|s| s.content = json);
                }
                true
            }
            Event::ViewContent(v) => {
                STATE.with_borrow_mut(|s| s.content = v.content.json.unwrap_or_default());
                true
            }
            Event::SettingsChanged => true,
            _ => false,
        }
    }

    fn save_state() -> Vec<u8> {
        STATE.with_borrow(|s| s.clicks.to_le_bytes().to_vec())
    }
}

fn node(parent: Option<u32>, key: &str, element: Element, style: Vec<Style>) -> Node {
    Node { parent, key: key.into(), element, style, on: vec![], focusable: false, tooltip: None, label: None }
}

fn edges(px: f32) -> Edges {
    Edges { top: Length::Px(px), right: Length::Px(px), bottom: Length::Px(px), left: Length::Px(px) }
}

fn hello(view: &str, s: &State) -> Frame {
    let greeting = match settings::get("greeting") {
        Some(settings::SettingValue::Text(t)) => t,
        _ => "Hello".to_string(),
    };
    let nodes = vec![
        node(None, "root", Element::Box, vec![Style::FlexCol, Style::Gap(Length::Px(8.0)), Style::Padding(edges(12.0)), Style::Background(Token::Surface)]),
        node(Some(0), "title", Element::Text(format!("{greeting} from a tile ({view})")), vec![Style::TextSize(TextSize::Lg), Style::Color(Token::Text)]),
        node(Some(0), "count", Element::Text(format!("Clicked {} times", s.clicks)), vec![Style::Color(Token::Text2)]),
        node(Some(0), "ticks", Element::Text(format!("Ticks: {}", s.ticks)), vec![Style::Color(Token::Text3)]),
        node(Some(0), "typed", Element::Text(format!("Typed: {}", s.typed)), vec![Style::Color(Token::Text3)]),
        node(Some(0), "content", Element::Text(format!("Content: {}", s.content)), vec![Style::Color(Token::Text3)]),
        Node {
            on: vec![Listener { on: Listen::Click, handler: CLICK }],
            ..node(Some(0), "button", Element::Button(Button { label: "Click me".into(), kind: ButtonKind::Primary, icon: Some(Icon::Named("plus".into())), disabled: false }), vec![])
        },
        Node {
            on: vec![Listener { on: Listen::Change, handler: TYPED }],
            ..node(Some(0), "field", Element::Input(Input { value: None, revision: 0, selection: None, placeholder: Some("Type here".into()), multiline: false, rows: None, disabled: false }), vec![])
        },
    ];
    Frame { nodes, focus: None, keymap: vec![] }
}

/// One node more than a frame may have.
fn huge() -> Frame {
    let mut nodes = vec![node(None, "root", Element::Box, vec![])];
    for i in 0..20_000 {
        nodes.push(node(Some(0), &i.to_string(), Element::Text(String::new()), vec![]));
    }
    Frame { nodes, focus: None, keymap: vec![] }
}

/// `--features heavy`: code the size of a real tile's (a JSON API client
/// with a regex filter), for compile-time measurements on more than a toy.
/// Not committed built; see ../build-hello-tile.sh.
#[cfg(feature = "heavy")]
mod heavy {
    pub fn work() -> String {
        let re = regex::Regex::new(r"(?i)^(fix|feat)\((\w+)\): (.+)$").unwrap();
        let v: serde_json::Value = serde_json::from_str(r#"{"items":[{"title":"feat(ui): a"},{"title":"fix(core): b"}]}"#).unwrap();
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
