//! Tools a tile offers to agents: `show` and `present` (show-only),
//! `read` and `act`.
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
//! **Undo.** The user can undo what an agent's `present` call put in the
//! tile (an Undo beside the host's "from <agent>" mark): the tile gets
//! [`Event::UndoPresent`](crate::Event::UndoPresent) with the call's
//! origin. Keep what agents show in a [`Presented`] and it restores the
//! value from before that call (or removes the item the call created).
//!
//! **Appends and files.** A `present` call may add to an item
//! (`"mode": "append"` with the same `item`: [`Call::is_append`]) and may
//! name a file in the agent's project instead of inline text
//! (`"from_file"`: read it with [`Call::large_input`]). Pitwall adds both
//! to every `present` tool's schema.
//!
//! **`read` and `act`.** A `read` tool returns what the tile has (with
//! the user's edits), as text and JSON up to 256 KiB each; an `act` tool
//! changes something outside the tile (a file, the network). The user
//! decides in Pitwall whether an agent may call them: a `read` until they
//! allow that agent always, an `act` each time unless they did. While a
//! `show`, `present` or `read` call runs, the tile's acting calls (`net`,
//! `show`, `notify`, `agents.queue-add`) return `denied`. An `act` that
//! waits (on the network) returns [`Reply::later`] and calls [`reply`]
//! when it is done, within 120 s.
//!
//! ```ignore
//! #[pitwall_extension::tools]
//! impl Board {
//!     /// The messages the user sees now, with their edits.
//!     #[tool(kind = "read")]
//!     fn current_messages(&mut self, _call: Call<Empty>, _cx: &mut Cx<Self>) -> Result<Reply, ToolError> {
//!         Reply::json(format!("{} messages", self.messages.len()), &self.messages)
//!     }
//! }
//! ```
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
    /// Returns what the tile has now, with the user's edits.
    Read,
    /// Has effects outside the tile (a file, the network).
    Act,
}

impl From<wit::ToolKind> for Kind {
    fn from(k: wit::ToolKind) -> Kind {
        match k {
            wit::ToolKind::Show => Kind::Show,
            wit::ToolKind::Present => Kind::Present,
            wit::ToolKind::Read => Kind::Read,
            wit::ToolKind::Act => Kind::Act,
        }
    }
}

/// What every call carries besides its input.
#[derive(Debug, Clone)]
struct Meta {
    id: u64,
    tool: String,
    kind: Kind,
    view: Option<String>,
    item: Option<String>,
    caller: Option<Caller>,
    append: bool,
    large: Option<u64>,
}

/// A call of one of the tile's declared tools, with its input as `I`
/// (derive `serde::Deserialize` for it, matching the tool's schema).
#[derive(Debug, Clone)]
pub struct Call<I> {
    meta: Meta,
    input: I,
}

const USER: &Caller = &Caller { agent_id: String::new(), agent_name: String::new(), project: None };

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
        &self.meta.tool
    }

    /// Its kind.
    pub fn kind(&self) -> Kind {
        self.meta.kind
    }

    /// The view the call went to (a `show` or `present` call), or the open
    /// view of the tool's tile (a `read` or `act` call; empty when none is
    /// open: they never open one).
    pub fn view(&self) -> &str {
        self.meta.view.as_deref().unwrap_or("")
    }

    /// The agent's id for the item it shows (`item` in the call), if it
    /// gave one.
    pub fn item(&self) -> Option<&str> {
        self.meta.item.as_deref()
    }

    /// Who called (empty fields when the user ran it: [`Call::by_user`]).
    pub fn caller(&self) -> &Caller {
        self.meta.caller.as_ref().unwrap_or(USER)
    }

    /// The user ran it themselves (the command palette, or `pitwall
    /// extension run` in one of Pitwall's own terminals), not an agent.
    pub fn by_user(&self) -> bool {
        self.meta.caller.is_none()
    }

    /// The call's id, also the origin of what it shows: keep it with the
    /// item, for [`user_edited`] and Undo ([`Presented`]).
    pub fn origin(&self) -> u64 {
        self.meta.id
    }

    /// A key for what this call shows that is unique per agent and item:
    /// `<agent-id>/<item>` (`<agent-id>/` without an item). Key items by
    /// it so two agents never replace each other's.
    pub fn key(&self) -> String {
        format!("{}/{}", self.caller().agent_id, self.meta.item.as_deref().unwrap_or(""))
    }

    /// The call adds to its item (`"mode": "append"`): the input is the
    /// part to add. Pitwall draws the item again at most 4 times a second
    /// while appends arrive.
    pub fn is_append(&self) -> bool {
        self.meta.append
    }

    /// The call named a file in the agent's project instead of inline
    /// text (`"from_file"`): its size in bytes.
    pub fn large_size(&self) -> Option<u64> {
        self.meta.large
    }

    /// The whole text of the file the call named (`"from_file"`, at most
    /// 16 MiB), read in parts from Pitwall; `None` when it named none.
    /// Only while the call is handled.
    pub fn large_input(&self) -> Option<Result<String, ToolError>> {
        let size = self.meta.large?;
        Some(large_input(self.meta.id, size))
    }
}

/// The text of the file call `call` named (`size` bytes, from
/// [`Content::large`](crate::Content::large)), read in parts.
pub fn large_input(call: u64, size: u64) -> Result<String, ToolError> {
    let mut out = String::with_capacity(size as usize);
    while (out.len() as u64) < size {
        let part = wit::input_part(call, out.len() as u64, 1 << 20).map_err(|e| ToolError::plain(format!("The file couldn't be read: {e}. Nothing was changed.")))?;
        if part.is_empty() {
            break;
        }
        out.push_str(&part);
    }
    Ok(out)
}

/// A call before its input is read ([`RawCall::typed`]); what the code
/// `#[tools]` writes takes.
#[derive(Debug, Clone)]
pub struct RawCall {
    meta: Meta,
    json: Option<String>,
}

impl RawCall {
    /// A `show` or `present` call as a test of the tile's own code makes
    /// one.
    pub fn new(id: u64, tool: &str, view: &str, item: Option<&str>, caller: Caller, json: Option<&str>) -> RawCall {
        let meta = Meta { id, tool: tool.into(), kind: Kind::Present, view: Some(view.into()), item: item.map(String::from), caller: Some(caller), append: false, large: None };
        RawCall { meta, json: json.map(String::from) }
    }

    /// A `read` or `act` call as a test of the tile's own code makes one
    /// (`caller` `None`: the user ran it).
    pub fn tool_call(id: u64, tool: &str, kind: Kind, caller: Option<Caller>, json: &str) -> RawCall {
        let meta = Meta { id, tool: tool.into(), kind, view: None, item: None, caller, append: false, large: None };
        RawCall { meta, json: Some(json.into()) }
    }

    /// The call with its input read as `I`; input that isn't `I` is an
    /// error for the agent.
    pub fn typed<I: DeserializeOwned>(self) -> Result<Call<I>, ToolError> {
        let json = self.json.as_deref().unwrap_or("{}");
        let input = serde_json::from_str(json).map_err(|e| ToolError::plain(format!("The input isn't what {} takes: {e}. Nothing was changed.", self.meta.tool)))?;
        Ok(Call { meta: self.meta, input })
    }

    /// The tool called.
    pub fn tool(&self) -> &str {
        &self.meta.tool
    }
}

/// What a call did, for the agent: a short text, optionally JSON of the
/// same, and the item id it was shown under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    text: String,
    json: Option<String>,
    item: Option<String>,
    later: bool,
}

impl Reply {
    /// A reply in words ("Showing “auth-flow”."); at most 4 KiB for a
    /// `show` or `present` call, 256 KiB for `read` and `act`.
    pub fn text(text: impl Into<String>) -> Reply {
        Reply { text: text.into(), json: None, item: None, later: false }
    }

    /// A reply with JSON (at most 256 KiB) and its text.
    pub fn json<S: Serialize + ?Sized>(text: impl Into<String>, value: &S) -> Result<Reply, ToolError> {
        let json = serde_json::to_string(value).map_err(|e| ToolError::plain(format!("The answer couldn't be written: {e}")))?;
        Ok(Reply { text: text.into(), json: Some(json), item: None, later: false })
    }

    /// No answer yet: a `read` or `act` call the tile answers later with
    /// [`reply`] and the call's [`Call::origin`] (an `act` that waits on
    /// the network), within its time limit (10 s for `read`, 120 s for
    /// `act`; then [`Event::ToolCancelled`](crate::Event::ToolCancelled)).
    pub fn later() -> Reply {
        Reply { text: String::new(), json: None, item: None, later: true }
    }

    /// Whether it is [`Reply::later`].
    pub fn is_later(&self) -> bool {
        self.later
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
    let meta = Meta { id: call, tool: tool.to_string(), kind: Kind::Present, view: Some(view.to_string()), item: content.id().map(String::from), caller: content.caller().cloned(), append: content.is_append(), large: content.large() };
    let raw = RawCall { meta, json: content.json_text().map(String::from) };
    let result = match tools.and_then(|d| d(tile, tool, raw, cx)) {
        Some(r) => r,
        None => Err(ToolError::plain(format!("This tile has no tool “{tool}” (its #[tools] impl and its manifest disagree)."))),
    };
    let ok = result.is_ok();
    reply(call, result);
    ok
}

/// A `read` or `act` call from the guest's `tool` export: its method run,
/// its answer now (`None`: [`Reply::later`]).
pub(crate) fn run_tool<T: Tile>(tile: &mut T, cx: &mut Cx<T>, tools: Option<Dispatch<T>>, call: wit::ToolCall) -> Option<wit::ToolOutput> {
    let meta = Meta { id: call.id, tool: call.tool.clone(), kind: call.kind.into(), view: call.view, item: None, caller: call.caller.map(Into::into), append: false, large: None };
    let raw = RawCall { meta, json: Some(call.input) };
    let result = match tools.and_then(|d| d(tile, &call.tool, raw, cx)) {
        Some(r) => r,
        None => Err(ToolError::plain(format!("This tile has no tool “{}” (its #[tools] impl and its manifest disagree).", call.tool))),
    };
    match result {
        Ok(r) if r.later => None,
        r => Some(output(r)),
    }
}

/// The answer when the tile isn't set up (its `init` failed).
pub(crate) fn no_tile() -> wit::ToolOutput {
    output(Err(ToolError::plain("The tile isn't running.")))
}

/// What agents put in a tile, kept per agent and item with each call's
/// value from before, so the user can undo any agent call
/// ([`Event::UndoPresent`](crate::Event::UndoPresent)) and the tile can
/// tell agents' items from the user's edits.
///
/// ```ignore
/// // In a present tool:
/// self.drafts.present(&call, call.input().text.clone());
/// // On Undo:
/// fn on_event(&mut self, e: &Event, cx: &mut Cx<Self>) {
///     if let Event::UndoPresent(origin) = e {
///         if self.drafts.undo(*origin) { cx.notify(); }
///     }
/// }
/// // When the user edits an agent's item:
/// if let Some(origin) = self.drafts.edit_by_user(&key, text) { tools::user_edited(origin); }
/// ```
#[derive(Debug, Clone)]
pub struct Presented<T> {
    items: std::collections::BTreeMap<String, Item<T>>,
    /// Each call's (key, the value before it), newest last.
    history: Vec<(u64, String, Option<T>)>,
}

#[derive(Debug, Clone)]
struct Item<T> {
    value: T,
    origin: Option<u64>,
}

impl<T> Default for Presented<T> {
    fn default() -> Self {
        Presented { items: Default::default(), history: Vec::new() }
    }
}

/// How many calls back Undo reaches.
const UNDO_DEPTH: usize = 200;

impl<T: Clone> Presented<T> {
    pub fn new() -> Presented<T> {
        Presented::default()
    }

    /// Call `call` puts `value` under its key ([`Call::key`]): replaced,
    /// or added. Returns the key.
    pub fn present<I>(&mut self, call: &Call<I>, value: T) -> String {
        let key = call.key();
        self.set(call.origin(), &key, value);
        key
    }

    /// Call `origin` puts `value` under `key` (for a tile that keys items
    /// its own way).
    pub fn set(&mut self, origin: u64, key: &str, value: T) {
        let before = self.items.get(key).map(|i| i.value.clone());
        self.history.push((origin, key.to_string(), before));
        if self.history.len() > UNDO_DEPTH {
            self.history.remove(0);
        }
        self.items.insert(key.to_string(), Item { value, origin: Some(origin) });
    }

    /// Call `call` adds to its item ([`Call::is_append`]): `add` changes
    /// the value (or makes it from `None`). Undo takes the whole append
    /// back.
    pub fn append<I>(&mut self, call: &Call<I>, add: impl FnOnce(Option<&T>) -> T) -> String {
        let key = call.key();
        let next = add(self.items.get(&key).map(|i| &i.value));
        self.set(call.origin(), &key, next);
        key
    }

    /// The user changed item `key` to `value`: it is theirs now (no agent
    /// mark). The call that put it there, if an agent did: tell Pitwall
    /// with [`user_edited`] so that agent can't replace the user's edit.
    pub fn edit_by_user(&mut self, key: &str, value: T) -> Option<u64> {
        let origin = self.items.get(key).and_then(|i| i.origin);
        self.items.insert(key.to_string(), Item { value, origin: None });
        origin
    }

    /// Undo what call `origin` did: the value before it is back (or the
    /// item goes, if the call made it). Whether it did anything.
    pub fn undo(&mut self, origin: u64) -> bool {
        let Some(at) = self.history.iter().rposition(|(o, _, _)| *o == origin) else { return false };
        let (_, key, before) = self.history.remove(at);
        match before {
            Some(v) => {
                self.items.insert(key, Item { value: v, origin: None });
            }
            None => {
                self.items.remove(&key);
            }
        }
        true
    }

    /// The item under `key`.
    pub fn get(&self, key: &str) -> Option<&T> {
        self.items.get(key).map(|i| &i.value)
    }

    /// The agent call that last put item `key` there (`None`: the user's
    /// own, or no such item).
    pub fn origin(&self, key: &str) -> Option<u64> {
        self.items.get(key).and_then(|i| i.origin)
    }

    /// Every item, by key.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &T)> {
        self.items.iter().map(|(k, i)| (k.as_str(), &i.value))
    }

    /// Remove item `key` (the user closed it).
    pub fn remove(&mut self, key: &str) -> Option<T> {
        self.items.remove(key).map(|i| i.value)
    }

    /// How many items there are.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: u64, agent: &str, item: &str) -> Call<()> {
        let caller = Caller { agent_id: agent.into(), agent_name: agent.into(), project: None };
        RawCall::new(id, "show", "main", Some(item), caller, Some("null")).typed().unwrap()
    }

    #[test]
    fn undo_restores_the_value_before_the_call() {
        let mut p = Presented::new();
        let k = p.present(&call(1, "api", "flow"), "a".to_string());
        p.present(&call(2, "api", "flow"), "b".to_string());
        p.present(&call(3, "web", "flow"), "mine".to_string());
        assert_eq!((p.get(&k).map(String::as_str), p.len()), (Some("b"), 2));
        assert!(p.undo(2));
        assert_eq!(p.get(&k).map(String::as_str), Some("a"));
        assert!(p.undo(1), "the call that made it: the item goes");
        assert!(p.get(&k).is_none() && p.len() == 1);
        assert!(!p.undo(9));
        assert_eq!(p.edit_by_user("web/flow", "edited".into()), Some(3));
        assert_eq!((p.origin("web/flow"), p.get("web/flow").map(String::as_str)), (None, Some("edited")));
        let c = call(4, "api", "log");
        p.append(&c, |v| format!("{}x", v.map(String::as_str).unwrap_or("")));
        p.append(&call(5, "api", "log"), |v| format!("{}y", v.map(String::as_str).unwrap_or("")));
        assert_eq!(p.get("api/log").map(String::as_str), Some("xy"));
        assert!(p.undo(5));
        assert_eq!(p.get("api/log").map(String::as_str), Some("x"));
    }
}
