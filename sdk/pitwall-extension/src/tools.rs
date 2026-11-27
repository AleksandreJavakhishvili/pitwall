//! Tools a tile offers to agents (show-only: `show` and `present`).
//!
//! Every tile can be opened by an agent with no code: Pitwall offers
//! `<extension-id>__open` to every agent it starts, with the tile's
//! `"input"` schema, and the content arrives in [`Tile::on_content`]
//! (with [`Content::call`] and [`Content::caller`] set). A tile may also
//! declare its own show tools on its manifest entry (`"tools"`, kind
//! `show` or `present`, with an input schema) and write one method per
//! tool in a `#[tools]` impl:
//!
//! ```ignore
//! use pitwall_extension::prelude::*;
//! use pitwall_extension::tools::{Call, Reply, ToolError, Problem};
//!
//! #[derive(serde::Deserialize)]
//! struct ShowMessage { item: Option<String>, message: String }
//!
//! #[pitwall_extension::tools]
//! impl Board {
//!     /// Show a message to the user.
//!     #[tool(kind = "present")]
//!     fn show_message(&mut self, call: Call<ShowMessage>, _cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
//!         if call.input().message.trim().is_empty() {
//!             return Err(ToolError::problems("Nothing was shown.", [Problem::new("`message` is empty")]));
//!         }
//!         self.messages.insert(call.key(), (call.input().message.clone(), call.origin()));
//!         Ok(Reply::text("Shown."))
//!     }
//! }
//! ```
//!
//! Pitwall lists each declared tool to agents as
//! `<extension-id>__<name>` and checks the input against its schema
//! before the tile sees it. A call may only change what the calling agent
//! showed (items are per agent: key them by [`Call::key`]); when the user
//! edits an item an agent showed, call [`user_edited`] with its
//! [`Call::origin`], and Pitwall refuses that agent's later calls for the
//! same item. Replies carry no tile data: a short text (≤ 4 KiB) and
//! problems in the agent's own input.
//!
//! [`Tile::on_content`]: crate::Tile::on_content
//! [`Content::call`]: crate::Content::call
//! [`Content::caller`]: crate::Content::caller

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::bindings::pitwall::tile::{agent_tools as wit, types};
use crate::{Cx, Tile};

/// The agent that called, as Pitwall identified it from the calling
/// process (never what the agent says about itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    /// The agent's id in Pitwall.
    pub agent_id: String,
    /// Its name as the user sees it (`api`).
    pub agent_name: String,
    /// Its project folder, when it has one.
    pub project: Option<String>,
}

impl From<types::Caller> for Caller {
    fn from(c: types::Caller) -> Caller {
        Caller { agent_id: c.agent_id, agent_name: c.agent_name, project: c.project }
    }
}

/// A tool's kind, as the manifest declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Opens or focuses the tile.
    Show,
    /// Puts content into the tile, or changes content the same agent put
    /// there.
    Present,
}

/// A call of one of the tile's declared tools, with its input as `I`
/// (derive `serde::Deserialize` for it, matching the tool's schema).
#[derive(Debug, Clone)]
pub struct Call<I> {
    id: u64,
    tool: String,
    view: String,
    item: Option<String>,
    caller: Caller,
    input: I,
}

impl<I> Call<I> {
    /// The input, as the tool's schema describes it.
    pub fn input(&self) -> &I {
        &self.input
    }

    /// The input, owned.
    pub fn into_input(self) -> I {
        self.input
    }

    /// The tool called (its name in the manifest).
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// The view the call went to.
    pub fn view(&self) -> &str {
        &self.view
    }

    /// The agent's id for the item it shows (`item` in the call), if it
    /// gave one.
    pub fn item(&self) -> Option<&str> {
        self.item.as_deref()
    }

    /// Who called.
    pub fn caller(&self) -> &Caller {
        &self.caller
    }

    /// The call's id, also the origin of what it shows: keep it with the
    /// item, for [`user_edited`].
    pub fn origin(&self) -> u64 {
        self.id
    }

    /// A key for what this call shows that is unique per agent and item:
    /// `<agent-id>/<item>` (`<agent-id>/` without an item). Key items by
    /// it so two agents never replace each other's.
    pub fn key(&self) -> String {
        format!("{}/{}", self.caller.agent_id, self.item.as_deref().unwrap_or(""))
    }
}

/// A call before its input is read ([`RawCall::typed`]); what the code
/// `#[tools]` writes takes.
#[derive(Debug, Clone)]
pub struct RawCall {
    id: u64,
    tool: String,
    view: String,
    item: Option<String>,
    caller: Caller,
    json: Option<String>,
}

impl RawCall {
    /// A call as a test of the tile's own code makes one.
    pub fn new(id: u64, tool: &str, view: &str, item: Option<&str>, caller: Caller, json: Option<&str>) -> RawCall {
        RawCall { id, tool: tool.into(), view: view.into(), item: item.map(String::from), caller, json: json.map(String::from) }
    }

    /// The call with its input read as `I`; input that isn't `I` is an
    /// error for the agent.
    pub fn typed<I: DeserializeOwned>(self) -> Result<Call<I>, ToolError> {
        let json = self.json.as_deref().unwrap_or("{}");
        let input = serde_json::from_str(json).map_err(|e| ToolError::plain(format!("The input isn't what {} takes: {e}. Nothing was changed.", self.tool)))?;
        Ok(Call { id: self.id, tool: self.tool, view: self.view, item: self.item, caller: self.caller, input })
    }

    /// The tool called.
    pub fn tool(&self) -> &str {
        &self.tool
    }
}

/// What a call did, for the agent: a short text, optionally JSON of the
/// same, and the item id it was shown under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    text: String,
    json: Option<String>,
    item: Option<String>,
}

impl Reply {
    /// A reply in words ("Showing “auth-flow”."); at most 4 KiB.
    pub fn text(text: impl Into<String>) -> Reply {
        Reply { text: text.into(), json: None, item: None }
    }

    /// A reply with JSON (at most 256 KiB) and its text.
    pub fn json<S: Serialize + ?Sized>(text: impl Into<String>, value: &S) -> Result<Reply, ToolError> {
        let json = serde_json::to_string(value).map_err(|e| ToolError::plain(format!("The answer couldn't be written: {e}")))?;
        Ok(Reply { text: text.into(), json: Some(json), item: None })
    }

    /// The item id it was shown under (when the tile chose one).
    pub fn item(mut self, item: impl Into<String>) -> Reply {
        self.item = Some(item.into());
        self
    }
}

/// A problem in the agent's input, at a line and column when known
/// (from 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub message: String,
}

impl Problem {
    /// A problem at line `line`, column `column`.
    pub fn at(line: u32, column: u32, message: impl Into<String>) -> Problem {
        Problem { line: Some(line), column: Some(column), message: message.into() }
    }

    /// A problem without a place.
    pub fn new(message: impl Into<String>) -> Problem {
        Problem { line: None, column: None, message: message.into() }
    }
}

/// Why a call changed nothing: a message, then each problem. The agent
/// reads it as an error and can fix its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError {
    pub message: String,
    pub problems: Vec<Problem>,
}

impl ToolError {
    /// An error in words.
    pub fn plain(message: impl Into<String>) -> ToolError {
        ToolError { message: message.into(), problems: Vec::new() }
    }

    /// An error with problems in the input ("The diagram has errors;
    /// nothing was changed.", then each).
    pub fn problems(message: impl Into<String>, problems: impl IntoIterator<Item = Problem>) -> ToolError {
        ToolError { message: message.into(), problems: problems.into_iter().collect() }
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        for p in &self.problems {
            match (p.line, p.column) {
                (Some(l), Some(c)) => write!(f, "\nline {l}, column {c}: {}", p.message)?,
                (Some(l), None) => write!(f, "\nline {l}: {}", p.message)?,
                _ => write!(f, "\n{}", p.message)?,
            }
        }
        Ok(())
    }
}

fn output(result: Result<Reply, ToolError>) -> wit::ToolOutput {
    match result {
        Ok(r) => wit::ToolOutput::Ok(wit::ToolOk { text: r.text, json: r.json, item: r.item }),
        Err(e) => wit::ToolOutput::Error(wit::ToolError {
            message: e.message,
            problems: e.problems.into_iter().map(|p| wit::Problem { line: p.line, column: p.column, message: p.message }).collect(),
        }),
    }
}

/// Answer agent call `call` (a [`Content::call`](crate::Content::call)
/// of the open every tile has; declared tools are answered with what
/// their method returns). Only while the event that delivered it is
/// handled: a later reply is dropped, and without one Pitwall tells the
/// agent what opened and where.
pub fn reply(call: u64, result: Result<Reply, ToolError>) {
    wit::reply(call, &output(result));
}

/// The user changed what call `call` showed (they typed in an agent's
/// draft): from now on, Pitwall refuses that agent's calls for the same
/// item, so an agent never overwrites the user's edits.
pub fn user_edited(call: u64) {
    wit::user_edited(call);
}

/// A tile's declared tools, written by `#[pitwall_extension::tools]` from
/// its `#[tool(kind = …)]` methods.
pub trait Tools: Tile {
    /// The tools by name and kind (they must match the manifest's
    /// `tools`).
    const TOOLS: &'static [(&'static str, Kind)];

    /// Run tool `name`; `None` when the tile has no such tool.
    fn call_tool(&mut self, name: &str, call: RawCall, cx: &mut Cx<Self>) -> Option<Result<Reply, ToolError>>;
}

/// The function that runs a tile's tools, if it has any.
#[doc(hidden)]
pub type Dispatch<T> = fn(&mut T, &str, RawCall, &mut Cx<T>) -> Option<Result<Reply, ToolError>>;

fn dispatch<T: Tools>(t: &mut T, name: &str, call: RawCall, cx: &mut Cx<T>) -> Option<Result<Reply, ToolError>> {
    t.call_tool(name, call, cx)
}

/// `export!`'s way of finding out whether the tile has a `#[tools]` impl
/// (the first trait applies when it does, the second otherwise).
#[doc(hidden)]
pub struct __Probe<T>(pub std::marker::PhantomData<T>);

#[doc(hidden)]
pub trait __HasTools<T> {
    fn __dispatch(&self) -> Option<Dispatch<T>>;
}

impl<T: Tools> __HasTools<T> for __Probe<T> {
    fn __dispatch(&self) -> Option<Dispatch<T>> {
        Some(dispatch::<T>)
    }
}

#[doc(hidden)]
pub trait __NoTools<T> {
    fn __dispatch(&self) -> Option<Dispatch<T>>;
}

impl<T: Tile> __NoTools<T> for &__Probe<T> {
    fn __dispatch(&self) -> Option<Dispatch<T>> {
        None
    }
}

/// A call of a declared tool arrived in `content` for view `view`: run it
/// and reply. Whether it showed something.
pub(crate) fn run<T: Tile>(tile: &mut T, cx: &mut Cx<T>, tools: Option<Dispatch<T>>, view: &str, content: &crate::Content) -> bool {
    let (Some(call), Some(tool)) = (content.call(), content.tool()) else { return false };
    let caller = content.caller().cloned().unwrap_or(Caller { agent_id: String::new(), agent_name: String::new(), project: None });
    let raw = RawCall { id: call, tool: tool.to_string(), view: view.to_string(), item: content.id().map(String::from), caller, json: content.json_text().map(String::from) };
    let result = match tools.and_then(|d| d(tile, tool, raw, cx)) {
        Some(r) => r,
        None => Err(ToolError::plain(format!("This tile has no tool “{tool}” (its #[tools] impl and its manifest disagree)."))),
    };
    let ok = result.is_ok();
    reply(call, result);
    ok
}
