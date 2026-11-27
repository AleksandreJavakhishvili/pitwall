//! The hello tile: the host's test fixture for `pitwall:tile@0.1.0`, written
//! with the guest SDK (`pitwall-extension`). A counter kept in `storage`, a
//! greeting from `settings`, a text field, a timer, content from views, and
//! test-only buttons (under the key `test`) that the host's tests click to
//! hang, crash or overflow it (see `tests/host.rs`). Its `gallery` tile
//! shows every component ([`gallery`], [`widgets`]). Agents open it with
//! content (`hello__open`) and show messages in it with its declared
//! `show_message` tool, one per agent's item (appended to, or read from a
//! file, and undone by the user), read them back with `current_messages`
//! and send one with `send_message` (an `act`, answered at once or
//! later).

use std::collections::BTreeMap;

use pitwall_extension::prelude::*;
use pitwall_extension::tools;
use pitwall_extension::tools::{Call, Presented, Problem, Reply, ToolError};

mod gallery;
mod widgets;

#[derive(Default)]
struct Hello {
    clicks: u32,
    ticks: u32,
    typed: String,
    content: String,
    huge: bool,
    hoard: Vec<Vec<u8>>,
    gallery: gallery::Gallery,
    /// Agents' messages by `<agent-id>/<item>`: (from whom, the text).
    messages: Presented<(String, String)>,
    /// Messages sent with `send_message`, and calls given up.
    sent: Vec<String>,
    cancelled: u32,
    widgets: widgets::Widgets,
    /// The component a gallery view shows alone (its content
    /// `{"component": "<name>"}`), by view id.
    alone: std::collections::HashMap<String, String>,
}

/// `show_message`'s input (its schema is in extension.json).
#[derive(serde::Deserialize)]
struct Message {
    /// Absent when the call names a file instead (`from_file`).
    message: Option<String>,
}

#[tools]
impl Hello {
    /// Show a message under the agent's item (the same item replaces it).
    #[tool(kind = "present")]
    fn show_message(&mut self, call: Call<Message>, cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
        // A file named instead of inline text: its text is the message.
        let text = match call.large_input() {
            Some(read) => read?.trim().to_string(),
            None => call.input().message.clone().unwrap_or_default().trim().to_string(),
        };
        if text.is_empty() {
            return Err(ToolError::problems("Nothing was shown: the message is empty.", [Problem::new("`message` has no text")]));
        }
        let from = call.caller().agent_name.clone();
        if call.is_append() {
            self.messages.append(&call, |m| (from.clone(), format!("{}{text}", m.map(|(_, t)| t.as_str()).unwrap_or(""))));
        } else {
            self.messages.present(&call, (from, text));
        }
        cx.notify();
        Ok(Reply::text(format!("Shown in Hello ({} messages)", self.messages.len())))
    }

    /// The messages shown now, with the user's edits. Tries to notify,
    /// which a read call may not.
    #[tool(kind = "read")]
    fn current_messages(&mut self, _call: Call<Empty>, _cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
        let all: BTreeMap<&str, &str> = self.messages.iter().map(|(k, (_, t))| (k, t.as_str())).collect();
        let notify = match notify::send("read", "a read call") {
            Ok(()) => "ok",
            Err(_) => "denied",
        };
        Reply::json(format!("{} messages (notify: {notify})", all.len()), &all)
    }

    /// Send a message: counted as sent, at once or after `wait_ms`.
    #[tool(kind = "act")]
    fn send_message(&mut self, call: Call<Send>, cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
        let notify = match notify::send("sent", &call.input().text) {
            Ok(()) => "ok",
            Err(_) => "denied",
        };
        let by = if call.by_user() { "the user".to_string() } else { call.caller().agent_name.clone() };
        let Some(ms) = call.input().wait_ms else {
            self.sent.push(call.input().text.clone());
            cx.notify();
            return Ok(Reply::text(format!("Sent “{}” for {by} (notify: {notify})", call.input().text)));
        };
        let (origin, text) = (call.origin(), call.input().text.clone());
        cx.after(Duration::from_millis(ms), move |t: &mut Hello, cx| {
            t.sent.push(text.clone());
            tools::reply(origin, Ok(Reply::text(format!("Sent “{text}” later"))));
            cx.notify();
        });
        Ok(Reply::later())
    }
}

/// An input with nothing in it.
#[derive(serde::Deserialize)]
struct Empty {}

/// `send_message`'s input.
#[derive(serde::Deserialize)]
struct Send {
    text: String,
    wait_ms: Option<u64>,
}

impl Tile for Hello {
    fn new(cx: &mut Cx<Self>) -> Self {
        let clicks = match storage::get("clicks") {
            Ok(Some(b)) if b.len() == 4 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            _ => 0,
        };
        log::info(format!("hello {} {} starts with {clicks} clicks", cx.extension(), cx.version()));
        if let Err(e) = ui::put_image("dot", gallery::DOT_PNG) {
            log::warn(format!("put-image: {e}"));
        }
        Hello { clicks, ..Hello::default() }
    }

    fn render(&mut self, view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
        if self.huge {
            return huge();
        }
        if view == "gallery" {
            if let Some(name) = self.alone.get(view).cloned() {
                let demo = self.widgets.demo(&name, cx);
                return div().id("alone").p(px(16.0)).bg(Token::Surface).child(div().id(name).self_align(pitwall_extension::Align::Start).child(demo)).into_any();
            }
            let mut g = std::mem::take(&mut self.gallery);
            let el = g.render(&self.widgets, cx);
            self.gallery = g;
            return el;
        }
        let greeting = settings::text("greeting").unwrap_or_else(|| "Hello".to_string());
        // A key of the tile's and one Pitwall keeps (dropped by the host).
        let j = cx.listener(|t: &mut Hello, _: &KeyPress, cx| {
            t.clicks += 1;
            cx.notify();
        });
        cx.bind_key("j", j);
        let w = cx.listener(|_: &mut Hello, _: &KeyPress, _| {});
        cx.bind_key("cmd-w", w);
        v_flex()
            .gap(px(8.0))
            .p(px(12.0))
            .bg(Token::Surface)
            .child(text(format!("{greeting} from a tile ({view})")).id("title").text_lg().text_color(Token::Text))
            .child(text(format!("Clicked {} times", self.clicks)).id("count").text_color(Token::Text2))
            .child(text(format!("Ticks: {}", self.ticks)).id("ticks").text_color(Token::Text3))
            .child(text(format!("Typed: {}", self.typed)).id("typed").text_color(Token::Text3))
            .child(text(format!("Content: {}", self.content)).id("content").text_color(Token::Text3))
            .children(self.messages.iter().map(|(k, (from, m))| text(format!("{from}: {m}")).id(format!("message-{k}")).text_color(Token::Text)))
            .child(text(format!("Sent: {}; cancelled: {}", self.sent.join(", "), self.cancelled)).id("sent").text_color(Token::Text3))
            .child(button("Click me").id("button").primary().icon(Icon::Plus).on_click(cx.listener(|t: &mut Hello, _, cx| {
                t.clicks += 1;
                if storage::set("clicks", t.clicks.to_le_bytes()).is_ok() {
                    cx.notify();
                }
            })))
            .child(input("field").placeholder("Type here").on_change(cx.listener(|t: &mut Hello, v: &str, cx| {
                t.typed = v.to_string();
                cx.notify();
            })))
            .child(tests(cx))
            .into_any()
    }

    fn on_content(&mut self, view: &ViewInfo, _cx: &mut Cx<Self>) {
        self.content = view.content.as_ref().and_then(|c| c.json_text()).unwrap_or_default().to_string();
        if view.tile == "gallery" {
            match json_string(&self.content, "component") {
                Some(name) => {
                    // Shown alone, a popover or a sheet is open, for its screenshot.
                    self.widgets.popover_open |= name == "popover";
                    self.widgets.asking |= name == "confirm-sheet";
                    self.alone.insert(view.id.clone(), name);
                }
                None => {
                    self.alone.remove(&view.id);
                }
            }
        }
    }

    fn on_view_closed(&mut self, view: &str, _cx: &mut Cx<Self>) {
        self.alone.remove(view);
    }

    fn on_event(&mut self, event: &Event, cx: &mut Cx<Self>) {
        match event {
            // The user's Undo on an agent's message.
            Event::UndoPresent(call) => {
                if self.messages.undo(*call) {
                    cx.notify();
                }
            }
            Event::ToolCancelled(_) => {
                self.cancelled += 1;
                cx.notify();
            }
            Event::Theme(_) | Event::SettingsChanged | Event::GrantsChanged(_) => cx.notify(),
            _ => {}
        }
    }

    fn save_state(&self) -> Option<SavedState> {
        Some(SavedState::bytes(self.clicks.to_le_bytes().to_vec()))
    }
}

/// The test-only buttons.
fn tests(cx: &mut Cx<Hello>) -> AnyElement {
    let row = h_flex()
        .id("test")
        .gap_1()
        .child(button("Spin").id("spin").small().on_click(cx.listener(|_, _, _| {
            let mut n: u64 = 0;
            loop {
                n = std::hint::black_box(n.wrapping_add(1));
            }
        })))
        .child(button("Huge").id("huge").small().on_click(cx.listener(|t: &mut Hello, _, cx| {
            t.huge = true;
            cx.notify();
        })))
        .child(button("Print").id("print").small().on_click(cx.listener(|_, _, _| {
            println!("hello from stdout");
            eprintln!("hello from stderr");
            log::debug("a debug line");
        })))
        .child(button("Timer").id("timer").small().on_click(cx.listener(|_, _, cx: &mut Cx<Hello>| {
            cx.after(Duration::from_millis(100), |t, cx| {
                t.ticks += 1;
                cx.notify();
            });
        })))
        .child(button("Toast").id("toast").small().on_click(cx.listener(|_, _, _| ui::toast("Saved", Tone::Good))))
        .child(button("Panic").id("panic").small().on_click(cx.listener(|_, _, _| panic!("boom"))))
        .child(button("Grow").id("grow").small().on_click(cx.listener(|t: &mut Hello, _, _| {
            // 1 MiB at a time until the memory limit traps.
            loop {
                t.hoard.push(vec![1u8; 1 << 20]);
            }
        })))
        .child(button("Edit").id("edit").small().on_click(cx.listener(|t: &mut Hello, _, cx| {
            // The user edits every agent's message: Pitwall refuses to
            // replace them from now on.
            let all: Vec<(String, (String, String))> = t.messages.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
            for (k, (from, m)) in all {
                if let Some(call) = t.messages.edit_by_user(&k, (from, format!("{m} (edited)"))) {
                    tools::user_edited(call);
                }
            }
            cx.notify();
        })))
        .child(button("Many").id("many").small().on_click(cx.listener(|_, _, _| {
            // Two keys in one host call.
            let got = storage::get_many(&["clicks", "nope"]).map(|v| v.iter().map(Option::is_some).collect::<Vec<_>>());
            log::info(format!("get-many: {got:?}"));
        })));
    #[cfg(feature = "heavy")]
    let row = row.child(button("Heavy").id("heavy").small().on_click(cx.listener(|_, _, _| log::info(heavy::work()))));
    row.into_any()
}

/// The string `key` of a flat JSON object (the fixture has no JSON parser).
fn json_string(json: &str, key: &str) -> Option<String> {
    let rest = &json[json.find(&format!("\"{key}\""))? + key.len() + 2..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

/// One node more than a frame may have.
fn huge() -> AnyElement {
    div().children((0..20_000).map(|i| text("").id(i.to_string()))).into_any()
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

pitwall_extension::export!(Hello);
