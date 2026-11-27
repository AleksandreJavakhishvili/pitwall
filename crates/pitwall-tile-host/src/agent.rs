//! Agents' show-only calls to a tile (docs/spec/tile-sdk/agent-tools.md
//! §11.2–11.7): the WIT types of `agent-tools`, what a call's answer is
//! ([`Answer`]), and the bookkeeping Pitwall does before a call reaches
//! the tile ([`ShowCalls`]): call ids, items per agent, the refusal on an
//! item the user edited, the inline input limit and the rate limit. The
//! app and the test harness both use [`ShowCalls`], so a test of a tile
//! sees the refusals the app makes.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub use crate::bindings::pitwall::tile::agent_tools::{Problem, ToolCall, ToolError, ToolKind, ToolOk, ToolOutput};
pub use crate::bindings::pitwall::tile::types::Caller;
use crate::ui::Content;

/// How long a `read` call may take, start to answer.
pub const READ_TIME: Duration = Duration::from_secs(10);
/// How long an `act` call may take once the user approved it.
pub const ACT_TIME: Duration = Duration::from_secs(120);
/// The largest file a `present` call may name (`from_file`).
pub const MAX_LARGE: usize = 16 << 20;
/// The most one `input-part` returns.
pub const MAX_PART: u32 = 1 << 20;
/// How often an item that appends arrive for is drawn again, at most.
pub const APPEND_EVERY: Duration = Duration::from_millis(250);

/// Large inputs (`from_file`) of calls on their way, by extension and
/// call: the app (or the harness) puts the file's text here before it
/// delivers the call, the tile reads it in parts (`input-part`) while it
/// handles it, and the text is let go when the call ends.
#[derive(Debug, Clone, Default)]
pub struct LargeInputs(Arc<Mutex<Texts>>);

/// The texts by (extension, call).
type Texts = HashMap<(String, u64), Arc<String>>;

impl LargeInputs {
    /// Keep `text` for call `call` of extension `ext`.
    pub fn put(&self, ext: &str, call: u64, text: String) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert((ext.to_string(), call), Arc::new(text));
    }

    /// The text of call `call`, if it named a file.
    pub fn get(&self, ext: &str, call: u64) -> Option<Arc<String>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).get(&(ext.to_string(), call)).cloned()
    }

    /// Call `call` ended: its text goes.
    pub fn forget(&self, ext: &str, call: u64) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(&(ext.to_string(), call));
    }
}

/// Up to `len` bytes of `text` from byte `offset` (at most [`MAX_PART`]),
/// both ends moved back to a character boundary.
pub fn part(text: &str, offset: u64, len: u32) -> String {
    let floor = |mut i: usize| {
        i = i.min(text.len());
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let start = floor(offset.min(text.len() as u64) as usize);
    let end = floor(start.saturating_add(len.min(MAX_PART) as usize));
    text[start..end].to_string()
}

/// A `read` or `act` call's answer within the limits: text and JSON up to
/// [`MAX_REPLY_JSON`] each (more is cut and says `truncated`), messages
/// as [`capped`].
pub fn capped_data(out: ToolOutput) -> ToolOutput {
    match out {
        ToolOutput::Ok(ok) => {
            let too_big = ok.json.as_ref().is_some_and(|j| j.len() > MAX_REPLY_JSON);
            let mut text = crate::host::cut(&ok.text, MAX_REPLY_JSON);
            if ok.text.len() > MAX_REPLY_JSON {
                text.push_str(" (truncated: more than 256 KiB of text)");
            }
            if too_big {
                text.push_str(" (its JSON was over 256 KiB and was left out: truncated; ask for less, with a range or a cursor)");
            }
            ToolOutput::Ok(ToolOk { text, json: ok.json.filter(|_| !too_big), item: ok.item.map(|i| crate::host::cut(&i, 200)) })
        }
        e => capped(e),
    }
}

impl ToolKind {
    /// Its word in manifests and the docs.
    pub fn word(self) -> &'static str {
        match self {
            ToolKind::Show => "show",
            ToolKind::Present => "present",
            ToolKind::Read => "read",
            ToolKind::Act => "act",
        }
    }

    /// The kind of that word.
    pub fn parse(word: &str) -> Option<ToolKind> {
        Some(match word {
            "show" => ToolKind::Show,
            "present" => ToolKind::Present,
            "read" => ToolKind::Read,
            "act" => ToolKind::Act,
            _ => return None,
        })
    }

    /// How long a call of this kind may take.
    pub fn time(self) -> Duration {
        match self {
            ToolKind::Act => ACT_TIME,
            ToolKind::Read => READ_TIME,
            _ => CALL_TIME,
        }
    }
}

/// The most JSON a call's input may carry inline (agent-tools.md §11.7).
pub const MAX_INPUT: usize = 1 << 20;
/// The most text a reply's `text` and messages carry (more is cut).
pub const MAX_REPLY_TEXT: usize = 4 << 10;
/// The most JSON a reply carries.
pub const MAX_REPLY_JSON: usize = 256 << 10;
/// Calls per agent per tile in [`RATE_WINDOW`].
pub const RATE_CALLS: usize = 10;
/// The window of the rate limit.
pub const RATE_WINDOW: Duration = Duration::from_secs(10);
/// How long a show-only call may take, start to answer.
pub const CALL_TIME: Duration = Duration::from_secs(10);

/// How a call delivered to the tile ended ([`crate::TileOutput::Answered`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// The tile answered with `agent-tools.reply`.
    Reply(ToolOutput),
    /// `handle` returned without a reply: Pitwall says what opened.
    Handled,
    /// It never reached the tile (its view closed, the content too
    /// large), with why.
    NotDelivered(String),
}

/// `out` within the reply limits: text and messages cut to
/// [`MAX_REPLY_TEXT`], JSON over [`MAX_REPLY_JSON`] dropped (the text
/// says so).
pub fn capped(out: ToolOutput) -> ToolOutput {
    let cut = |s: &str| crate::host::cut(s, MAX_REPLY_TEXT);
    match out {
        ToolOutput::Ok(ok) => {
            let too_big = ok.json.as_ref().is_some_and(|j| j.len() > MAX_REPLY_JSON);
            let text = if too_big { format!("{} (its JSON was over 256 KiB and was left out: truncated)", ok.text) } else { ok.text };
            ToolOutput::Ok(ToolOk { text: cut(&text), json: ok.json.filter(|_| !too_big), item: ok.item.map(|i| crate::host::cut(&i, 200)) })
        }
        ToolOutput::Error(e) => ToolOutput::Error(ToolError {
            message: cut(&e.message),
            problems: e.problems.into_iter().take(100).map(|p| Problem { message: cut(&p.message), ..p }).collect(),
        }),
    }
}

/// A tool error as the agent reads it: the message first, then each
/// problem as "line L, column C: message".
pub fn error_text(e: &ToolError) -> String {
    let mut s = e.message.trim().to_string();
    for p in &e.problems {
        let at = match (p.line, p.column) {
            (Some(l), Some(c)) => format!("line {l}, column {c}: "),
            (Some(l), None) => format!("line {l}: "),
            _ => String::new(),
        };
        s.push_str(&format!("\n{at}{}", p.message.trim()));
    }
    s
}

/// One show-only call, as Pitwall has it before the tile does.
#[derive(Debug, Clone, PartialEq)]
pub struct ShowCall {
    /// The declared tool (`None`: the open every tile has).
    pub tool: Option<String>,
    /// Which of the extension's tiles it goes to.
    pub tile: String,
    /// The agent's item id (`content.id`).
    pub item: Option<String>,
    /// The input for the tile, as JSON text (already checked against the
    /// schema; without Pitwall's own `where` and `of`).
    pub input: Option<String>,
    pub caller: Caller,
    /// `"mode": "append"`: the input adds to the item.
    pub append: bool,
    /// The size of the file it named (`from_file`), whose text is in
    /// [`LargeInputs`].
    pub large: Option<u64>,
}

/// Why Pitwall refused a call before the tile saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// The user changed that item since the agent showed it.
    Edited { item: String },
    /// More than [`RATE_CALLS`] in [`RATE_WINDOW`].
    Busy,
    /// Over [`MAX_INPUT`].
    TooLarge(usize),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Edited { item } => write!(f, "The user changed “{item}” since you showed it; show it under a new item id instead. Nothing was changed."),
            Refused::Busy => write!(f, "Busy: more than {RATE_CALLS} calls to this tile in {} seconds; try again in a few seconds. Nothing was changed.", RATE_WINDOW.as_secs()),
            Refused::TooLarge(n) => write!(f, "The input is {} KiB of JSON, over the 1 MiB a call may carry; send less. Nothing was changed.", n / 1024),
        }
    }
}

/// What an extension's calls so far left: their ids, which agent's item
/// each showed, the items the user edited since, and the recent calls
/// for the rate limit. One per running extension (the app keeps it while
/// the extension is installed, so a restart of the tile keeps the
/// edited marks).
#[derive(Debug, Default)]
pub struct ShowCalls {
    next: u64,
    /// Each call that carried an item: (agent, tile, item).
    items: BTreeMap<u64, (String, String, String)>,
    edited: BTreeSet<(String, String, String)>,
    recent: BTreeMap<(String, String), VecDeque<Instant>>,
}

impl ShowCalls {
    pub fn new() -> ShowCalls {
        ShowCalls::default()
    }

    /// Check call `c` at `now` and give it an id: the content to deliver
    /// (with `call`, `tool`, `caller` and the item as its id), or why not.
    pub fn begin(&mut self, c: &ShowCall, now: Instant) -> Result<(u64, Content), Refused> {
        let size = c.input.as_ref().map_or(0, String::len);
        if size > MAX_INPUT {
            return Err(Refused::TooLarge(size));
        }
        let agent = c.caller.agent_id.clone();
        if let Some(item) = &c.item {
            if self.edited.contains(&(agent.clone(), c.tile.clone(), item.clone())) {
                return Err(Refused::Edited { item: item.clone() });
            }
        }
        let id = self.next_id(&agent, &c.tile, now)?;
        if let Some(item) = &c.item {
            self.items.insert(id, (agent, c.tile.clone(), item.clone()));
        }
        let content = Content { id: c.item.clone(), json: c.input.clone(), bytes: None, call: Some(id), tool: c.tool.clone(), caller: Some(c.caller.clone()), append: c.append, large: c.large };
        Ok((id, content))
    }

    /// A `read` or `act` call of agent `agent` (`None`: the user) to tile
    /// `tile` at `now`: its id, or busy (the same rate limit as show
    /// calls).
    pub fn begin_tool(&mut self, agent: Option<&str>, tile: &str, input: &str, now: Instant) -> Result<u64, Refused> {
        if input.len() > MAX_INPUT {
            return Err(Refused::TooLarge(input.len()));
        }
        self.next_id(agent.unwrap_or(""), tile, now)
    }

    fn next_id(&mut self, agent: &str, tile: &str, now: Instant) -> Result<u64, Refused> {
        let recent = self.recent.entry((agent.to_string(), tile.to_string())).or_default();
        while recent.front().is_some_and(|t| now.saturating_duration_since(*t) >= RATE_WINDOW) {
            recent.pop_front();
        }
        if recent.len() >= RATE_CALLS {
            return Err(Refused::Busy);
        }
        recent.push_back(now);
        self.next += 1;
        Ok(self.next)
    }

    /// The tile said the user changed what call `call` showed
    /// (`agent-tools.user-edited`): that agent's later calls with the same
    /// item are refused. Whether it marked one (a call without an item
    /// marks nothing).
    pub fn user_edited(&mut self, call: u64) -> bool {
        match self.items.get(&call) {
            Some(key) => {
                self.edited.insert(key.clone());
                true
            }
            None => false,
        }
    }

    /// Whether agent `agent`'s item `item` in tile `tile` was edited by
    /// the user.
    pub fn is_edited(&self, agent: &str, tile: &str, item: &str) -> bool {
        self.edited.contains(&(agent.to_string(), tile.to_string(), item.to_string()))
    }

    /// The agent and item call `call` showed, if it carried an item.
    pub fn item_of(&self, call: u64) -> Option<(&str, &str)> {
        self.items.get(&call).map(|(a, _, i)| (a.as_str(), i.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller(id: &str) -> Caller {
        Caller { agent_id: id.into(), agent_name: id.into(), project: None }
    }

    fn call(agent: &str, item: Option<&str>) -> ShowCall {
        ShowCall { tool: Some("show".into()), tile: "main".into(), item: item.map(String::from), input: Some("{}".into()), caller: caller(agent), append: false, large: None }
    }

    #[test]
    fn items_are_per_agent_and_edits_refuse_them() {
        let mut s = ShowCalls::new();
        let now = Instant::now();
        let (a, content) = s.begin(&call("api", Some("flow")), now).unwrap();
        assert_eq!((content.call, content.id.as_deref(), content.tool.as_deref()), (Some(a), Some("flow"), Some("show")));
        assert!(s.user_edited(a));
        assert_eq!(s.begin(&call("api", Some("flow")), now), Err(Refused::Edited { item: "flow".into() }));
        // Another agent's item of the same id, and a new id: fine.
        assert!(s.begin(&call("web", Some("flow")), now).is_ok());
        assert!(s.begin(&call("api", Some("flow-2")), now).is_ok());
        assert!(Refused::Edited { item: "flow".into() }.to_string().contains("new item id"));
    }

    #[test]
    fn the_rate_and_the_size_are_limited() {
        let mut s = ShowCalls::new();
        let now = Instant::now();
        for _ in 0..RATE_CALLS {
            s.begin(&call("api", None), now).unwrap();
        }
        assert_eq!(s.begin(&call("api", None), now), Err(Refused::Busy));
        assert!(s.begin(&call("web", None), now).is_ok(), "per agent");
        assert!(s.begin(&call("api", None), now + RATE_WINDOW).is_ok(), "the window moves");
        let mut big = call("api", None);
        big.input = Some("x".repeat(MAX_INPUT + 1));
        assert!(matches!(s.begin(&big, now + RATE_WINDOW), Err(Refused::TooLarge(_))));
    }

    #[test]
    fn replies_are_capped_and_errors_read_in_lines() {
        let long = ToolOutput::Ok(ToolOk { text: "x".repeat(10_000), json: Some("1".repeat(MAX_REPLY_JSON + 1)), item: None });
        let ToolOutput::Ok(ok) = capped(long) else { panic!() };
        assert!(ok.text.len() <= MAX_REPLY_TEXT && ok.json.is_none());
        let e = ToolError { message: "The diagram has errors.".into(), problems: vec![Problem { line: Some(2), column: Some(5), message: "unknown arrow".into() }, Problem { line: None, column: None, message: "no nodes".into() }] };
        assert_eq!(error_text(&e), "The diagram has errors.\nline 2, column 5: unknown arrow\nno nodes");
    }
}
