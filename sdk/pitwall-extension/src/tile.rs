//! The [`Tile`] trait, its context [`Cx`], views and their content, and
//! the runtime that turns Pitwall's calls into calls on the tile.

use std::any::Any;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::bindings::pitwall::tile::{tasks, types as wit};
use crate::element::{AnyElement, Axis, ButtonKind, Handler, IntoElement, Kind, Listen};
use crate::events::{Click, CodeSelection, Dropped, Event, Fired, Hover, InputChange, KeyPress, LineClick, ListenerEvent, Pointer, PointerKind, ScrollPosition, Selection, TextPos, Theme};
use crate::host::log;

/// A tile: its state, how it draws each view, and what it does when the
/// user acts. One value of it runs per extension; every place in the
/// layout showing one of the extension's tiles is a view of it, rendered
/// with its own id.
///
/// ```ignore
/// use pitwall_extension::prelude::*;
///
/// #[derive(Default)]
/// struct Counter { clicks: u32 }
///
/// impl Tile for Counter {
///     fn new(_cx: &mut Cx<Self>) -> Self {
///         Counter::default()
///     }
///
///     fn render(&mut self, _view: &str, cx: &mut Cx<Self>) -> impl IntoElement {
///         v_flex().gap_2().p_3()
///             .child(text(format!("Clicked {} times", self.clicks)))
///             .child(button("Click me").primary().on_click(cx.listener(|this, _, cx| {
///                 this.clicks += 1;
///                 cx.notify();
///             })))
///     }
/// }
///
/// pitwall_extension::export!(Counter);
/// ```
pub trait Tile: Sized + 'static {
    /// Start: make the tile's state. Called once, before any view opens.
    /// Read what [`Tile::save_state`] kept with [`Cx::restore`], start
    /// timers with [`Cx::every`]. Deadline: 2 s.
    fn new(cx: &mut Cx<Self>) -> Self;

    /// The whole tree of view `view` (`cx.view(view)` says which tile it
    /// shows and its content). Called after the view opens, after a
    /// listener or timer called [`Cx::notify`], and when the tile is shown
    /// again. Keep it quick (under 8 ms; the deadline is 250 ms): read
    /// state, don't compute it here.
    fn render(&mut self, view: &str, cx: &mut Cx<Self>) -> impl IntoElement;

    /// A view was opened (by the user, an agent, or
    /// [`ui::open_view`](crate::ui::open_view)); its first render follows.
    /// When it was opened with content, [`Tile::on_content`] follows too.
    fn on_view_opened(&mut self, view: &ViewInfo, cx: &mut Cx<Self>) {
        let _ = (view, cx);
    }

    /// A view was given content: when it opened with some, or later (an
    /// agent showing something new in the same place). `view.content` is
    /// set; parse it with [`Content::parse`]. The view renders again after.
    fn on_content(&mut self, view: &ViewInfo, cx: &mut Cx<Self>) {
        let _ = (view, cx);
    }

    /// A view was closed; it is no longer rendered.
    fn on_view_closed(&mut self, view: &str, cx: &mut Cx<Self>) {
        let _ = (view, cx);
    }

    /// Anything else that happened: shown or hidden, focus, a view's size,
    /// the theme, settings or grants changed. By default the tile renders
    /// again when the theme, its settings or its grants change.
    fn on_event(&mut self, event: &Event, cx: &mut Cx<Self>) {
        if matches!(event, Event::Theme(_) | Event::SettingsChanged | Event::GrantsChanged(_)) {
            cx.notify();
        }
    }

    /// State to keep while the tile is dropped (to save memory) or
    /// reloaded during development, given back through [`Cx::restore`].
    /// At most 1 MiB; deadline 50 ms. What the user must not lose belongs
    /// in [`storage`](crate::storage) instead, which is on disk at once.
    fn save_state(&self) -> Option<SavedState> {
        None
    }
}

/// What [`Tile::save_state`] returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedState(Vec<u8>);

impl SavedState {
    /// `value` as JSON, for [`Cx::restore`] to read back.
    pub fn json<S: Serialize + ?Sized>(value: &S) -> Option<SavedState> {
        match serde_json::to_vec(value) {
            Ok(b) => Some(SavedState(b)),
            Err(e) => {
                log::warn(format!("the state couldn't be saved: {e}"));
                None
            }
        }
    }

    /// Bytes of your own format (read back with [`Cx::saved_state`]).
    pub fn bytes(bytes: Vec<u8>) -> SavedState {
        SavedState(bytes)
    }
}

/// Content given to a view when it opened or later, by the user or an
/// agent. The manifest's tile entry names a JSON schema (`"input"`), and
/// Pitwall checks the content against it before it arrives here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Content {
    id: Option<String>,
    json: Option<String>,
    bytes: Option<Vec<u8>>,
    call: Option<u64>,
    tool: Option<String>,
    caller: Option<crate::tools::Caller>,
}

/// Content that isn't what the tile expected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentError(String);

impl std::fmt::Display for ContentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ContentError {}

impl Content {
    /// Content with this JSON text (for tests of your own code).
    pub fn from_json(json: impl Into<String>) -> Content {
        Content { id: None, json: Some(json.into()), bytes: None, call: None, tool: None, caller: None }
    }

    /// The same content as agent `caller`'s call `call` with item `item`
    /// brought it (for tests of your own code).
    pub fn from_agent(mut self, call: u64, item: Option<&str>, caller: crate::tools::Caller) -> Content {
        self.call = Some(call);
        self.id = item.map(String::from);
        self.caller = Some(caller);
        self
    }

    /// The caller's id for this content (an agent's `item`): a later call
    /// with the same id updates what it shows in place.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// The agent's call that brought it (also the origin of what it
    /// shows); `None`: the user opened the view. Answer it with
    /// [`tools::reply`](crate::tools::reply) to return problems in the
    /// input.
    pub fn call(&self) -> Option<u64> {
        self.call
    }

    /// The declared tool called; `None` for the open every tile has
    /// (`<extension-id>__open`), or when the user opened the view.
    pub fn tool(&self) -> Option<&str> {
        self.tool.as_deref()
    }

    /// The agent that called, as Pitwall identified it; `None`: the user
    /// opened the view.
    pub fn caller(&self) -> Option<&crate::tools::Caller> {
        self.caller.as_ref()
    }

    /// The payload as JSON text.
    pub fn json_text(&self) -> Option<&str> {
        self.json.as_deref()
    }

    /// A binary payload (an image, a file's bytes), when the schema says
    /// so; at most 8 MiB.
    pub fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }

    /// The JSON payload as your input type (derive `serde::Deserialize`
    /// for it, matching the tile's input schema).
    pub fn parse<I: DeserializeOwned>(&self) -> Result<I, ContentError> {
        let json = self.json.as_deref().ok_or_else(|| ContentError("the content has no JSON".into()))?;
        serde_json::from_str(json).map_err(|e| ContentError(format!("the content isn't what this tile takes: {e}")))
    }
}

impl From<wit::Content> for Content {
    fn from(c: wit::Content) -> Content {
        Content { id: c.id, json: c.json, bytes: c.bytes, call: c.call, tool: c.tool, caller: c.caller.map(Into::into) }
    }
}

/// One open view: a place in the layout showing one of the extension's
/// tiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewInfo {
    /// The view's id, chosen by Pitwall: the tile's id for a tile opened
    /// once, another for each further place.
    pub id: String,
    /// Which of the manifest's tiles it shows.
    pub tile: String,
    /// Its content, if it was given any.
    pub content: Option<Content>,
}

/// A running timer, from [`Cx::after`] or [`Cx::every`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Timer(u32);

type Callback<T> = dyn FnMut(&mut T, &Fired, &mut Cx<T>);
/// A listener's closure, shared by its handler slot and a pending call.
type Shared<T> = Rc<RefCell<Box<Callback<T>>>>;
type TimerFn<T> = dyn FnMut(&mut T, &mut Cx<T>);
type FetchFn<T> = dyn FnOnce(&mut T, Result<crate::net::Response, crate::net::Error>, &mut Cx<T>);

/// What a listener stands for, by handler number.
struct Entry<T> {
    callback: Option<Rc<RefCell<Box<Callback<T>>>>>,
    /// A text field's change listener: its view and key path, to keep
    /// its value.
    input: Option<(String, String)>,
}

/// What a handler number stands for in a view: a node's key path and the
/// event, or a key binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Slot {
    Node(String, Listen),
    Key(String),
    /// A canvas region's click (`Listen::Click`) or hover, by the canvas's
    /// key path and the region's key.
    Region(String, String, Listen),
}

/// What the SDK knows of a text field's value.
#[derive(Debug, Clone)]
struct InputState {
    value: String,
    revision: u64,
}

struct TimerEntry<T> {
    callback: Option<Box<TimerFn<T>>>,
    repeat: bool,
}

/// The tile's context: passed to every call of [`Tile`], like GPUI's
/// `Context`. Makes listeners and timers, asks for renders, and knows the
/// open views, the theme and the grants.
pub struct Cx<T> {
    extension: String,
    version: String,
    theme: Theme,
    granted: Vec<String>,
    state: Option<Vec<u8>>,
    visible: bool,
    notified: bool,
    views: BTreeMap<String, ViewInfo>,
    // Rendering.
    generation: u64,
    pending: Vec<Rc<RefCell<Box<Callback<T>>>>>,
    pending_keys: Vec<(String, u32, u64)>,
    pending_focus: Option<String>,
    // Handler numbers: stable per (view, slot), so an unchanged view
    // renders an unchanged frame.
    next_handler: u32,
    ids: HashMap<String, HashMap<Slot, u32>>,
    handlers: HashMap<u32, Entry<T>>,
    inputs: HashMap<(String, String), InputState>,
    timers: HashMap<u32, TimerEntry<T>>,
    firing: Option<(u32, bool)>,
    /// The focused node of each view, as `focus-changed` said.
    focused: HashMap<String, String>,
    /// The blur listener of a focused node a frame took away, kept until
    /// the focus moves (its blur comes after that frame).
    lost_blur: HashMap<String, (String, Shared<T>)>,
    /// Renders so far: a list's revision unless the tile names one.
    renders: u64,
    /// The virtual lists, tables and trees of each view, by (view, key
    /// path): how to make their rows, and their rows' handlers.
    lists: HashMap<(String, String), ListRows<T>>,
    /// What to call with each `net` request's answer.
    fetches: HashMap<u32, Box<FetchFn<T>>>,
}

/// How a row is made (`Cx::rows`).
type RowFn<T> = dyn Fn(&mut T, usize, &mut Cx<T>) -> crate::components::TreeRow;

/// A list's row closure and the handler numbers of the rows fetched.
struct ListRows<T> {
    make: Rc<Box<RowFn<T>>>,
    /// Handler numbers by slot, stable while a row stays fetched.
    slots: HashMap<Slot, u32>,
    /// The rows fetched, oldest first (their key paths): the oldest let go
    /// past [`KEEP_ROWS`].
    fetched: std::collections::VecDeque<String>,
}

/// Rows whose listeners are kept per list. Pitwall keeps fewer, so a row it
/// shows always has its listeners.
const KEEP_ROWS: usize = 4096;

impl<T: 'static> Cx<T> {
    fn new(init: wit::InitContext) -> Cx<T> {
        Cx {
            extension: init.extension,
            version: init.version,
            theme: init.theme.into(),
            granted: init.granted,
            state: init.state,
            visible: true,
            notified: false,
            views: BTreeMap::new(),
            generation: 0,
            pending: Vec::new(),
            pending_keys: Vec::new(),
            pending_focus: None,
            next_handler: 1,
            ids: HashMap::new(),
            handlers: HashMap::new(),
            inputs: HashMap::new(),
            timers: HashMap::new(),
            firing: None,
            focused: HashMap::new(),
            lost_blur: HashMap::new(),
            renders: 0,
            lists: HashMap::new(),
            fetches: HashMap::new(),
        }
    }

    /// How a virtual list's, table's or tree's rows are made, for
    /// [`virtual_list`](crate::virtual_list), [`table`](crate::table) and
    /// [`tree`](crate::tree): `f` gets the tile, the row's index and this
    /// context, and returns the row (any element; a
    /// [`tree_row`](crate::tree_row) for a tree). Pitwall calls it only for
    /// the rows coming into sight, a page at a time, with the tile's state
    /// as it is then; listeners made in it work as anywhere
    /// (`cx.listener`). Keep it quick: a page of 64 rows within a few
    /// milliseconds.
    pub fn rows<R: crate::components::IntoRow>(&mut self, f: impl Fn(&mut T, usize, &mut Cx<T>) -> R + 'static) -> crate::components::Rows {
        let make: Box<RowFn<T>> = Box::new(move |t, i, cx| f(t, i, cx).into_row());
        crate::components::Rows(Rc::new(make))
    }

    /// Send `request` through Pitwall ([`net`](crate::net)); `f` runs with
    /// the answer when it comes (call [`Cx::notify`] in it to show what
    /// changed). Errors at once when it can't be sent: not granted `net`,
    /// a host it may not reach, an address that isn't valid, 8 requests
    /// already waiting. A tile's own server
    /// ([`service::url`](crate::service::url)) needs no grant.
    pub fn fetch(&mut self, request: crate::net::Request, f: impl FnOnce(&mut T, Result<crate::net::Response, crate::net::Error>, &mut Cx<T>) + 'static) -> Result<crate::net::RequestId, crate::net::Error> {
        let id = crate::bindings::pitwall::tile::net::fetch(&request.wit())?;
        self.fetches.insert(id, Box::new(f));
        Ok(crate::net::RequestId(id))
    }

    /// Stop waiting for a request: its callback never runs.
    pub fn cancel_fetch(&mut self, id: crate::net::RequestId) {
        crate::bindings::pitwall::tile::net::cancel(id.0);
        self.fetches.remove(&id.0);
    }

    /// A listener for an element: `f` runs with the tile, the event and
    /// this context when the user acts. Make listeners while rendering
    /// and hand them to the element at once:
    /// `button("Save").on_click(cx.listener(|this, _, cx| this.save(cx)))`.
    /// Call [`Cx::notify`] in it when what the tile shows changed.
    pub fn listener<E: ListenerEvent + ?Sized + 'static>(&mut self, mut f: impl FnMut(&mut T, &E, &mut Cx<T>) + 'static) -> Handler<E> {
        let cb: Box<Callback<T>> = Box::new(move |t, fired, cx| {
            if let Some(e) = E::pick(fired) {
                f(t, e, cx)
            }
        });
        self.pending.push(Rc::new(RefCell::new(cb)));
        Handler::new(self.pending.len() as u32 - 1, self.generation)
    }

    /// Render the tile's open views again after this call (in a listener,
    /// a timer or `on_*`).
    pub fn notify(&mut self) {
        self.notified = true;
    }

    /// While rendering: when the view has focus, `keys` (`j`, `enter`,
    /// `cmd-r`, `shift-tab`, `/`; `cmd` is ⌘ on macOS and Ctrl elsewhere)
    /// call `handler`. Pitwall's own shortcuts always win.
    pub fn bind_key(&mut self, keys: impl Into<String>, handler: Handler<KeyPress>) {
        self.pending_keys.push((keys.into(), handler.slot, handler.generation));
    }

    /// While rendering: focus the node at `key_path` (keys from the root
    /// down, joined by `/`, without the root's: `"editor/text"` for an
    /// input keyed `text` in a box keyed `editor` under the root; the same
    /// path [`ui::scroll_to`](crate::ui::scroll_to) takes), honoured only
    /// while the tile already has focus.
    pub fn focus(&mut self, key_path: impl Into<String>) {
        self.pending_focus = Some(key_path.into());
    }

    /// Call `f` once, after `delay` (at least 100 ms). Timers pause while
    /// the tile is hidden. `None` (and a Log line) when it can't be made:
    /// at most 32 timers at once.
    pub fn after(&mut self, delay: Duration, f: impl FnMut(&mut T, &mut Cx<T>) + 'static) -> Option<Timer> {
        self.timer(delay, false, Box::new(f))
    }

    /// Call `f` every `period` (at least 100 ms) until cancelled. A period
    /// missed while hidden fires once when shown again.
    pub fn every(&mut self, period: Duration, f: impl FnMut(&mut T, &mut Cx<T>) + 'static) -> Option<Timer> {
        self.timer(period, true, Box::new(f))
    }

    fn timer(&mut self, d: Duration, repeat: bool, f: Box<TimerFn<T>>) -> Option<Timer> {
        let ms = d.as_millis().min(u32::MAX as u128) as u32;
        match tasks::timer(ms, repeat) {
            Ok(id) => {
                self.timers.insert(id, TimerEntry { callback: Some(f), repeat });
                Some(Timer(id))
            }
            Err(e) => {
                log::warn(format!("a timer couldn't be started: {}", crate::HostError::from(e)));
                None
            }
        }
    }

    /// Stop a timer (one already finished is ignored).
    pub fn cancel(&mut self, timer: Timer) {
        tasks::cancel(timer.0);
        self.timers.remove(&timer.0);
        if let Some((id, cancelled)) = &mut self.firing {
            if *id == timer.0 {
                *cancelled = true;
            }
        }
    }

    /// What [`Tile::save_state`] kept, read back as JSON. `None` on a fresh
    /// start, or (with a note in the Log) when it can't be read as `S`.
    pub fn restore<S: DeserializeOwned>(&self) -> Option<S> {
        let bytes = self.state.as_deref()?;
        match serde_json::from_slice(bytes) {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn(format!("the saved state couldn't be read ({e}); starting fresh"));
                None
            }
        }
    }

    /// What [`Tile::save_state`] kept, as bytes.
    pub fn saved_state(&self) -> Option<&[u8]> {
        self.state.as_deref()
    }

    /// The extension's id.
    pub fn extension(&self) -> &str {
        &self.extension
    }

    /// The extension's version, from its manifest.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The user's appearance settings.
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// Whether the user granted `permission`.
    pub fn is_granted(&self, permission: &str) -> bool {
        self.granted.iter().any(|g| g == permission)
    }

    /// The permissions the user granted, by name.
    pub fn granted(&self) -> &[String] {
        &self.granted
    }

    /// Whether the tile is shown.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// The key path of the node of view `view` that has keyboard focus
    /// (as [`Cx::focus`] names it), if one has.
    pub fn focused(&self, view: &str) -> Option<&str> {
        self.focused.get(view).map(String::as_str)
    }

    /// The open views.
    pub fn views(&self) -> impl Iterator<Item = &ViewInfo> {
        self.views.values()
    }

    /// Open view `id`.
    pub fn view(&self, id: &str) -> Option<&ViewInfo> {
        self.views.get(id)
    }

    /// View `id`'s content, if it has some.
    pub fn content(&self, id: &str) -> Option<&Content> {
        self.views.get(id)?.content.as_ref()
    }

    // ---- The runtime's side ---------------------------------------------

    fn intern(&mut self, view: &str, slot: Slot, fresh: &mut HashMap<Slot, u32>) -> u32 {
        if let Some(id) = fresh.get(&slot) {
            return *id;
        }
        let id = match self.ids.get(view).and_then(|m| m.get(&slot)) {
            Some(id) => *id,
            None => {
                let id = self.next_handler;
                self.next_handler = self.next_handler.wrapping_add(1).max(1);
                id
            }
        };
        fresh.insert(slot, id);
        id
    }

    fn callback(&self, slot: u32, generation: u64) -> Option<Rc<RefCell<Box<Callback<T>>>>> {
        if generation != self.generation {
            log::warn("a listener made in an earlier render was used again and is ignored: make listeners with cx.listener while rendering");
            return None;
        }
        self.pending.get(slot as usize).cloned()
    }

    /// The frame of `view` from its root element: keys made unique, key
    /// paths, listeners numbered by (key path, event), text fields' values
    /// sent only when they change.
    fn frame(&mut self, view: &str, root: AnyElement) -> wit::Frame {
        let mut b = Build { view: view.to_string(), nodes: Vec::new(), fresh: HashMap::new(), seen_inputs: HashSet::new(), lists: HashSet::new() };
        self.walk(&mut b, root, None, String::new(), "root".to_string());
        self.forget_lists(view, Some(&b.lists));
        let mut keymap = Vec::new();
        for (keys, slot, generation) in std::mem::take(&mut self.pending_keys) {
            let id = self.intern(view, Slot::Key(keys.clone()), &mut b.fresh);
            let callback = self.callback(slot, generation);
            self.handlers.insert(id, Entry { callback, input: None });
            keymap.push(wit::Binding { keys, handler: id });
        }
        // Handler numbers no longer used in this view are forgotten; a
        // focused node's blur is kept until the focus moves off it.
        let old = self.ids.insert(view.to_string(), b.fresh).unwrap_or_default();
        let now = &self.ids[view];
        if let Some(path) = self.focused.get(view) {
            let slot = Slot::Node(path.clone(), Listen::Blur);
            if let (Some(id), None) = (old.get(&slot), now.get(&slot)) {
                if let Some(cb) = self.handlers.get(id).and_then(|e| e.callback.clone()) {
                    self.lost_blur.insert(view.to_string(), (path.clone(), cb));
                }
            }
        }
        for (slot, id) in old {
            if now.get(&slot) != Some(&id) {
                self.handlers.remove(&id);
            }
        }
        self.inputs.retain(|(v, path), _| v != view || b.seen_inputs.contains(path));
        wit::Frame { nodes: b.nodes, focus: self.pending_focus.take(), keymap }
    }

    fn walk(&mut self, b: &mut Build, el: AnyElement, parent: Option<u32>, path: String, key: String) {
        let index = b.nodes.len() as u32;
        let AnyElement { key: _, kind, style, listeners, focusable, tooltip, label, children, dnd } = el;
        let mut on = Vec::new();
        let mut has_change = false;
        let mut style = style.wit();
        for (listen, slot, generation) in listeners {
            let id = self.intern(&b.view, Slot::Node(path.clone(), listen), &mut b.fresh);
            let callback = self.callback(slot, generation);
            let input = (listen == Listen::Change && matches!(kind, Kind::Input(_))).then(|| (b.view.clone(), path.clone()));
            has_change |= input.is_some();
            self.handlers.insert(id, Entry { callback, input });
            // Focus and blur are the SDK's, from `focus-changed`.
            if let Some(on_wit) = listen.wit() {
                on.push(wit::Listener { on: on_wit, handler: id });
            }
            if listen == Listen::Drop {
                let accepts = dnd.as_ref().map(|d| d.accepts.clone()).unwrap_or_default();
                style.push(wit::Style::DropTarget(wit::DropTarget { accepts, handler: id }));
            }
        }
        if let Some((kind, data, label)) = dnd.and_then(|d| d.drag) {
            style.push(wit::Style::Drag(wit::DragSource { kind, data, label }));
        }
        let element = match kind {
            Kind::Box => wit::Element::Box,
            Kind::Scroll(axis) => wit::Element::Scroll(wit::Scroll { axis: axis.wit() }),
            Kind::Text(t) => wit::Element::Text(t),
            Kind::Button { label, kind, small, icon, disabled } => wit::Element::Button(wit::Button {
                label,
                kind: kind.wit(),
                size: if small { wit::ButtonSize::Small } else { wit::ButtonSize::Regular },
                icon: icon.map(|i| wit::Icon::Named(i.name().to_string())),
                disabled,
            }),
            Kind::Icon(i) => wit::Element::Icon(wit::Icon::Named(i.name().to_string())),
            Kind::Input(spec) => {
                // Every field reports its edits, so the SDK knows what it
                // holds and sends `value` only when it differs.
                if !has_change {
                    let id = self.intern(&b.view, Slot::Node(path.clone(), Listen::Change), &mut b.fresh);
                    self.handlers.insert(id, Entry { callback: None, input: Some((b.view.clone(), path.clone())) });
                    on.push(wit::Listener { on: wit::Listen::Change, handler: id });
                }
                b.seen_inputs.insert(path.clone());
                let at = (b.view.clone(), path.clone());
                let (value, revision) = match (spec.value, self.inputs.get(&at)) {
                    (None, _) => (None, 0),
                    (Some(v), Some(s)) if s.value == v => (None, 0),
                    (Some(v), s) => {
                        let seen = s.map_or(0, |s| s.revision);
                        self.inputs.insert(at, InputState { value: v.clone(), revision: seen });
                        (Some(v), seen + 1)
                    }
                };
                let selection = match (&value, spec.selection) {
                    (Some(_), Some(r)) => Some(wit::TextRange { start: r.start as u32, end: r.end as u32 }),
                    _ => None,
                };
                wit::Element::Input(wit::Input {
                    value,
                    revision,
                    selection,
                    placeholder: spec.placeholder,
                    multiline: spec.multiline,
                    rows: spec.rows,
                    disabled: spec.disabled,
                })
            }
            Kind::Canvas(spec) => {
                let spec = *spec;
                let mut regions = Vec::with_capacity(spec.regions.len());
                for r in spec.regions {
                    let mut handler = |listen: Listen, h: Option<(u32, u64)>, cx: &mut Self| {
                        let (slot, generation) = h?;
                        let id = cx.intern(&b.view, Slot::Region(path.clone(), r.key.clone(), listen), &mut b.fresh);
                        let callback = cx.callback(slot, generation);
                        cx.handlers.insert(id, Entry { callback, input: None });
                        Some(id)
                    };
                    let click = handler(Listen::Click, r.click, self);
                    let hover = handler(Listen::Hover, r.hover, self);
                    regions.push(wit::HitRegion { key: clean_key(&r.key), rect: r.rect, label: r.label, tooltip: r.tooltip, click, hover });
                }
                wit::Element::Canvas(wit::Canvas { ops: spec.ops, transform: spec.transform, regions })
            }
            Kind::Code(spec) => wit::Element::CodeView(spec.0),
            Kind::Diff(spec) => wit::Element::DiffView(spec.0),
            Kind::Component(crate::components::Spec::List(l)) => {
                b.lists.insert(path.clone());
                self.list_node(&b.view, &path, l)
            }
            Kind::Component(spec) => spec.wit().unwrap_or(wit::Element::Box),
        };
        b.nodes.push(wit::Node { parent, key, element, style, on, focusable, tooltip, label });
        let mut used: HashSet<String> = HashSet::new();
        for (i, child) in children.into_iter().enumerate() {
            let mut key = clean_key(child.key.as_deref().unwrap_or(&i.to_string()));
            if !used.insert(key.clone()) {
                let base = key.clone();
                let mut n = 2;
                while !used.insert(format!("{base}~{n}")) {
                    n += 1;
                }
                key = format!("{base}~{n}");
                log::warn(format!("two children of “{path}” have the key “{base}”; keys must be unique among siblings"));
            }
            let child_path = if path.is_empty() { key.clone() } else { format!("{path}/{key}") };
            self.walk(b, child, Some(index), child_path, key);
        }
    }

    /// Call the listener with handler number `handler`.
    fn fire(&mut self, tile: &mut T, view_input: Option<(u64, &InputChange)>, handler: u32, fired: Fired) {
        let Some(entry) = self.handlers.get(&handler) else { return };
        let callback = entry.callback.clone();
        if let (Some(at), Some((revision, change))) = (entry.input.clone(), view_input) {
            self.inputs.insert(at, InputState { value: change.value.clone(), revision });
        }
        if let Some(cb) = callback {
            (cb.borrow_mut())(tile, &fired, self);
        }
    }

    fn fire_timer(&mut self, tile: &mut T, id: u32) {
        let Some(entry) = self.timers.get_mut(&id) else { return };
        let repeat = entry.repeat;
        let Some(mut f) = entry.callback.take() else { return };
        if !repeat {
            self.timers.remove(&id);
        }
        self.firing = Some((id, false));
        f(tile, self);
        let cancelled = matches!(self.firing.take(), Some((_, true)));
        if repeat && !cancelled {
            if let Some(e) = self.timers.get_mut(&id) {
                e.callback = Some(f);
            }
        }
    }

    /// The focus in `view` moved to `to`: the node it left hears its blur,
    /// the one it reached its focus.
    fn focus_moved(&mut self, tile: &mut T, view: &str, to: Option<String>) {
        let from = match &to {
            Some(p) => self.focused.insert(view.to_string(), p.clone()),
            None => self.focused.remove(view),
        };
        if from == to {
            return;
        }
        let lost = self.lost_blur.remove(view);
        if let Some(from) = from {
            let id = self.ids.get(view).and_then(|m| m.get(&Slot::Node(from.clone(), Listen::Blur))).copied();
            match (id, lost) {
                (Some(id), _) => self.fire(tile, None, id, Fired::Blur),
                (None, Some((path, cb))) if path == from => (cb.borrow_mut())(tile, &Fired::Blur, self),
                _ => {}
            }
        }
        if let Some(to) = to {
            let id = self.ids.get(view).and_then(|m| m.get(&Slot::Node(to, Listen::Focus))).copied();
            if let Some(id) = id {
                self.fire(tile, None, id, Fired::Focus);
            }
        }
    }

    fn forget_view(&mut self, view: &str) {
        if let Some(ids) = self.ids.remove(view) {
            for id in ids.values() {
                self.handlers.remove(id);
            }
        }
        self.inputs.retain(|(v, _), _| v != view);
        self.views.remove(view);
        self.focused.remove(view);
        self.lost_blur.remove(view);
        self.forget_lists(view, None);
    }
}

struct Build {
    view: String,
    nodes: Vec<wit::Node>,
    fresh: HashMap<Slot, u32>,
    seen_inputs: HashSet<String>,
    /// The key paths of the lists in it.
    lists: HashSet<String>,
}

impl<T: 'static> Cx<T> {
    /// A virtual list, table or tree node: its row closure kept by (view,
    /// key path) for `rows`, its revision this render's unless the tile
    /// named one.
    fn list_node(&mut self, view: &str, path: &str, l: crate::components::ListSpec) -> wit::Element {
        use crate::components::ListForm;
        match l.rows.0.downcast::<Box<RowFn<T>>>() {
            Ok(make) => {
                let key = (view.to_string(), path.to_string());
                match self.lists.get_mut(&key) {
                    Some(e) => e.make = make,
                    None => {
                        self.lists.insert(key, ListRows { make, slots: HashMap::new(), fetched: Default::default() });
                    }
                }
            }
            Err(_) => log::warn(format!("the rows of “{path}” were made for another tile's type")),
        }
        let rows = wit::VirtualList { count: l.count.min(u32::MAX as usize) as u32, row_height: l.row_height, revision: l.revision.unwrap_or(self.renders), selected: l.selected };
        match l.form {
            ListForm::Plain => wit::Element::VirtualList(rows),
            ListForm::Table { columns, sort } => wit::Element::Table(wit::Table {
                columns: columns.iter().map(|c| c.wit()).collect(),
                sort: sort.map(|(column, d)| wit::SortState { column, direction: d.wit() }),
                rows,
            }),
            ListForm::Tree => wit::Element::Tree(wit::Tree { rows }),
        }
    }

    /// Let go of view `view`'s lists not in `keep` (all, with `None`), and
    /// their rows' listeners.
    fn forget_lists(&mut self, view: &str, keep: Option<&HashSet<String>>) {
        let gone: Vec<(String, String)> = self.lists.keys().filter(|(v, p)| v == view && keep.is_none_or(|k| !k.contains(p))).cloned().collect();
        for k in gone {
            if let Some(l) = self.lists.remove(&k) {
                for id in l.slots.values() {
                    self.handlers.remove(id);
                }
            }
        }
    }

    /// Rows `start..start + count` of the list at `path` in `view`.
    fn rows_of(&mut self, tile: &mut T, view: &str, path: &str, start: u32, count: u32) -> Vec<wit::Row> {
        let key = (view.to_string(), path.to_string());
        let Some(make) = self.lists.get(&key).map(|l| l.make.clone()) else { return Vec::new() };
        let mut fresh = self.lists.get_mut(&key).map(|l| std::mem::take(&mut l.slots)).unwrap_or_default();
        self.generation += 1;
        self.pending.clear();
        let mut out = Vec::with_capacity(count as usize);
        let mut keys = HashSet::new();
        let mut fetched = Vec::new();
        for i in start..start.saturating_add(count) {
            let row = make(tile, i as usize, self);
            let mut k = clean_key(row.element.key.as_deref().unwrap_or(&i.to_string()));
            if !keys.insert(k.clone()) {
                k = format!("{k}~{i}");
                keys.insert(k.clone());
            }
            let row_path = format!("{path}/{k}");
            let mut b = Build { view: view.to_string(), nodes: Vec::new(), fresh, seen_inputs: HashSet::new(), lists: HashSet::new() };
            self.walk(&mut b, row.element, None, row_path.clone(), k.clone());
            fresh = b.fresh;
            fetched.push(row_path);
            out.push(wit::Row { key: k, nodes: b.nodes, depth: row.depth, expanded: row.expanded });
        }
        self.pending.clear();
        if let Some(l) = self.lists.get_mut(&key) {
            l.slots = fresh;
            l.fetched.extend(fetched);
            if l.fetched.len() > KEEP_ROWS {
                // The oldest quarter let go, with their listeners.
                let old: HashSet<String> = l.fetched.drain(..KEEP_ROWS / 4).collect();
                let handlers = &mut self.handlers;
                l.slots.retain(|slot, id| {
                    let Slot::Node(p, _) = slot else { return true };
                    let row = p.strip_prefix(path).and_then(|r| r.strip_prefix('/')).map(|r| r.split('/').next().unwrap_or(r));
                    let drop = row.is_some_and(|r| old.contains(&format!("{path}/{r}")));
                    if drop {
                        handlers.remove(id);
                    }
                    !drop
                });
            }
        }
        out
    }
}

/// A key as Pitwall takes it: no `/` (it joins key paths), at most 128
/// bytes.
fn clean_key(k: &str) -> String {
    let mut k = k.replace('/', "∕");
    if k.len() > 128 {
        let mut end = 128;
        while !k.is_char_boundary(end) {
            end -= 1;
        }
        k.truncate(end);
    }
    k
}

impl Listen {
    /// The WIT listener, for those the host sends events for.
    fn wit(self) -> Option<wit::Listen> {
        Some(match self {
            Listen::Click => wit::Listen::Click,
            Listen::Change => wit::Listen::Change,
            Listen::Select => wit::Listen::Select,
            Listen::Submit => wit::Listen::Submit,
            Listen::Scroll => wit::Listen::Scroll,
            Listen::Hover => wit::Listen::Hover,
            Listen::Pointer => wit::Listen::Pointer,
            Listen::Wheel => wit::Listen::Wheel,
            Listen::LineClick => wit::Listen::LineClick,
            Listen::Focus | Listen::Blur | Listen::Drop => return None,
            Listen::Toggle => wit::Listen::Toggle,
            Listen::Link => wit::Listen::Link,
            Listen::Sort => wit::Listen::Sort,
            Listen::ColumnResized => wit::Listen::ColumnResized,
        })
    }
}

impl Axis {
    fn wit(self) -> wit::Axis {
        match self {
            Axis::Vertical => wit::Axis::Vertical,
            Axis::Horizontal => wit::Axis::Horizontal,
            Axis::Both => wit::Axis::Both,
        }
    }
}

impl ButtonKind {
    fn wit(self) -> wit::ButtonKind {
        match self {
            ButtonKind::Primary => wit::ButtonKind::Primary,
            ButtonKind::Ghost => wit::ButtonKind::Ghost,
            ButtonKind::Danger => wit::ButtonKind::Danger,
            ButtonKind::Link => wit::ButtonKind::Link,
        }
    }
}

// ---- The runtime: what `export!` calls -------------------------------------

struct Runtime<T: Tile> {
    tile: T,
    cx: Cx<T>,
    tools: Option<crate::tools::Dispatch<T>>,
}

thread_local! {
    static RUNTIME: RefCell<Option<Box<dyn Any>>> = const { RefCell::new(None) };
}

fn with<T: Tile, R>(f: impl FnOnce(&mut Runtime<T>) -> R) -> Option<R> {
    RUNTIME.with_borrow_mut(|rt| rt.as_mut().and_then(|b| b.downcast_mut::<Runtime<T>>()).map(f))
}

#[doc(hidden)]
pub fn init<T: Tile>(init: wit::InitContext) -> Result<(), String> {
    init_with::<T>(init, None)
}

/// [`init`], with the tile's `#[tools]` (`export!` finds them).
#[doc(hidden)]
pub fn init_with<T: Tile>(init: wit::InitContext, tools: Option<crate::tools::Dispatch<T>>) -> Result<(), String> {
    let mut cx = Cx::<T>::new(init);
    let tile = T::new(&mut cx);
    cx.notified = false;
    RUNTIME.with_borrow_mut(|rt| *rt = Some(Box::new(Runtime { tile, cx, tools })));
    Ok(())
}

#[doc(hidden)]
pub fn render<T: Tile>(view: String) -> wit::Frame {
    with::<T, _>(|rt| {
        rt.cx.generation += 1;
        rt.cx.renders += 1;
        rt.cx.pending.clear();
        rt.cx.pending_keys.clear();
        rt.cx.pending_focus = None;
        let root = rt.tile.render(&view, &mut rt.cx).into_element();
        let frame = rt.cx.frame(&view, root);
        rt.cx.pending.clear();
        frame
    })
    .unwrap_or(wit::Frame { nodes: vec![wit::Node { parent: None, key: "root".into(), element: wit::Element::Box, style: vec![], on: vec![], focusable: false, tooltip: None, label: None }], focus: None, keymap: vec![] })
}

#[doc(hidden)]
pub fn handle<T: Tile>(event: wit::Event) -> bool {
    with::<T, _>(|rt| {
        let Runtime { tile, cx, tools } = rt;
        cx.notified = false;
        let mods = |m| crate::events::Modifiers::from(m);
        match event {
            wit::Event::Click(c) => {
                let fired = Fired::Click(Click { button: c.button.into(), modifiers: mods(c.modifiers), count: c.count, at: c.at.into() });
                cx.fire(tile, None, c.handler, fired);
            }
            wit::Event::InputChanged(c) => {
                let change = InputChange { value: c.value, selection: c.selection.start as usize..c.selection.end as usize };
                cx.fire(tile, Some((c.revision, &change)), c.handler, Fired::Change(change.clone()));
            }
            wit::Event::InputSelected(s) => cx.fire(tile, None, s.handler, Fired::Select(Selection { range: s.selection.start as usize..s.selection.end as usize })),
            wit::Event::Submit(h) => cx.fire(tile, None, h, Fired::Submit),
            wit::Event::Key(k) => cx.fire(tile, None, k.handler, Fired::Key(KeyPress { keys: k.keys })),
            wit::Event::Scroll(s) => {
                let pos = ScrollPosition { x: s.x, y: s.y, content_width: s.content_width, content_height: s.content_height };
                cx.fire(tile, None, s.handler, Fired::Scroll(pos));
            }
            wit::Event::Hover(h) => cx.fire(tile, None, h.handler, Fired::Hover(Hover { inside: h.inside, at: h.at.into() })),
            wit::Event::Pointer(p) => {
                let kind = match p.kind {
                    wit::PointerKind::Down(b) => PointerKind::Down(b.into()),
                    wit::PointerKind::Up(b) => PointerKind::Up(b.into()),
                    wit::PointerKind::Move => PointerKind::Move,
                    wit::PointerKind::Wheel(d) => PointerKind::Wheel { dx: d.x, dy: d.y },
                    wit::PointerKind::Pinch(f) => PointerKind::Pinch(f),
                };
                cx.fire(tile, None, p.handler, Fired::Pointer(Pointer { kind, at: p.at.into(), modifiers: mods(p.modifiers) }));
            }
            wit::Event::Timer(id) => cx.fire_timer(tile, id),
            wit::Event::Dropped(d) => cx.fire(tile, None, d.handler, Fired::Drop(Dropped { kind: d.kind, data: d.data, at: d.at.into() })),
            wit::Event::LineClicked(l) => cx.fire(tile, None, l.handler, Fired::LineClick(LineClick { side: l.side.into(), line: l.line as usize })),
            wit::Event::CodeSelected(s) => {
                let pos = |p: wit::TextPos| TextPos { line: p.line as usize, column: p.column as usize };
                let sel = CodeSelection { side: s.side.into(), start: pos(s.start), end: pos(s.end), text: s.text };
                cx.fire(tile, None, s.handler, Fired::CodeSelect(sel));
            }
            wit::Event::ViewOpened(v) => {
                let content = v.content.map(Content::from);
                // A declared tool's call is the tool's, not the view's
                // content.
                let declared = content.as_ref().filter(|c| c.tool().is_some()).cloned();
                let info = ViewInfo { id: v.view.clone(), tile: v.tile, content: content.filter(|c| c.tool().is_none()) };
                cx.views.insert(v.view.clone(), info.clone());
                tile.on_view_opened(&info, cx);
                if info.content.is_some() {
                    tile.on_content(&info, cx);
                }
                if let Some(c) = declared {
                    crate::tools::run(tile, cx, *tools, &v.view, &c);
                }
                cx.notified = true;
            }
            wit::Event::ViewContent(v) => {
                let content = Content::from(v.content);
                if content.tool().is_some() {
                    if !cx.views.contains_key(&v.view) {
                        return false;
                    }
                    if crate::tools::run(tile, cx, *tools, &v.view, &content) {
                        cx.notified = true;
                    }
                    return std::mem::take(&mut cx.notified);
                }
                let Some(info) = cx.views.get_mut(&v.view) else { return false };
                info.content = Some(content);
                let info = info.clone();
                tile.on_content(&info, cx);
                cx.notified = true;
            }
            wit::Event::ViewClosed(view) => {
                cx.forget_view(&view);
                tile.on_view_closed(&view, cx);
            }
            wit::Event::Visibility(v) => {
                cx.visible = v;
                tile.on_event(&Event::Visibility(v), cx);
            }
            wit::Event::Focus(f) => tile.on_event(&Event::Focus(f), cx),
            wit::Event::FocusChanged(f) => {
                cx.focus_moved(tile, &f.view, f.key_path.clone());
                tile.on_event(&Event::FocusChanged { view: f.view, key_path: f.key_path }, cx);
            }
            wit::Event::Resize(r) => tile.on_event(&Event::Resize { view: r.view, width: r.size.width, height: r.size.height }, cx),
            wit::Event::Theme(t) => {
                cx.theme = t.into();
                let theme = cx.theme;
                tile.on_event(&Event::Theme(theme), cx);
            }
            wit::Event::SettingsChanged => tile.on_event(&Event::SettingsChanged, cx),
            wit::Event::GrantsChanged(g) => {
                cx.granted = g.clone();
                tile.on_event(&Event::GrantsChanged(g), cx);
            }
            wit::Event::AgentsChanged => tile.on_event(&Event::AgentsChanged, cx),
            wit::Event::AskAnswered(a) => tile.on_event(&Event::AskAnswered(a.into()), cx),
            wit::Event::Select(s) => {
                let picked = crate::events::Picked { key: s.key, index: s.index as usize, modifiers: mods(s.modifiers), activate: s.activate };
                cx.fire(tile, None, s.handler, Fired::Pick(picked));
            }
            wit::Event::Toggle(t) => cx.fire(tile, None, t.handler, Fired::Toggle(crate::events::Toggled { key: t.key, index: t.index as usize, on: t.on })),
            wit::Event::Link(l) => cx.fire(tile, None, l.handler, Fired::Link(crate::events::LinkClick { url: l.url, modifiers: mods(l.modifiers) })),
            wit::Event::Sort(s) => cx.fire(tile, None, s.handler, Fired::Sort(crate::events::SortChange { column: s.column, direction: s.direction.into() })),
            wit::Event::ColumnResized(c) => cx.fire(tile, None, c.handler, Fired::ColumnResized(crate::events::ColumnWidth { column: c.column, width: c.width })),
            wit::Event::Fetched(f) => {
                if let Some(cb) = cx.fetches.remove(&f.id) {
                    cb(tile, f.outcome.map(crate::net::Response::from).map_err(crate::net::Error::from), cx);
                }
            }
            wit::Event::ToolbarClicked(project) => tile.on_event(&Event::ToolbarClicked { project }, cx),
            wit::Event::ServiceChanged(info) => tile.on_event(&Event::ServiceChanged(info.into()), cx),
        }
        std::mem::take(&mut cx.notified)
    })
    .unwrap_or(false)
}

#[doc(hidden)]
pub fn rows<T: Tile>(view: String, key_path: String, start: u32, count: u32) -> Vec<wit::Row> {
    with::<T, _>(|rt| {
        let Runtime { tile, cx, .. } = rt;
        cx.rows_of(tile, &view, &key_path, start, count)
    })
    .unwrap_or_default()
}

#[doc(hidden)]
pub fn save_state<T: Tile>() -> Vec<u8> {
    with::<T, _>(|rt| rt.tile.save_state().map(|s| s.0).unwrap_or_default()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;

    #[derive(Default)]
    struct T {
        clicks: u32,
        typed: String,
    }

    fn cx() -> Cx<T> {
        Cx::new(wit::InitContext {
            extension: "t".into(),
            version: "0.1.0".into(),
            theme: wit::ThemeInfo { mode: wit::ThemeMode::Dark, density: wit::Density::Compact, reduce_motion: false },
            granted: vec![],
            state: None,
        })
    }

    fn view(t: &T, cx: &mut Cx<T>) -> AnyElement {
        cx.generation += 1;
        cx.pending.clear();
        v_flex()
            .child(text(format!("Clicked {}", t.clicks)))
            .child(button("Click").id("go").on_click(cx.listener(|t: &mut T, _, cx| {
                t.clicks += 1;
                cx.notify();
            })))
            .child(input("field").value(t.typed.clone()).on_change(cx.listener(|t: &mut T, v: &str, _| t.typed = v.into())))
            .child(div().p_2().px_4().border_b_1().border_l_1())
            .child(text("dup").id("x"))
            .child(text("dup").id("x"))
            .into_element()
    }

    fn click(cx: &mut Cx<T>, t: &mut T, h: u32) {
        cx.fire(t, None, h, Fired::Click(Click { button: crate::events::MouseButton::Left, modifiers: Default::default(), count: 1, at: Default::default() }));
    }

    #[test]
    fn frames_keep_their_handler_numbers_and_inputs_their_text() {
        let (mut cx, mut t) = (cx(), T::default());
        let root = view(&t, &mut cx);
        let f1 = cx.frame("main", root);
        assert_eq!(f1.nodes[0].key, "root");
        let keys: Vec<&str> = f1.nodes.iter().map(|n| n.key.as_str()).collect();
        assert_eq!(keys, ["root", "0", "go", "field", "3", "x", "x~2"]);
        // Padding merged per side, one border style.
        let pad = f1.nodes[4].style.iter().filter(|s| matches!(s, wit::Style::Padding(_))).count();
        let border = f1.nodes[4].style.iter().filter(|s| matches!(s, wit::Style::Border(_))).count();
        assert_eq!((pad, border), (1, 1));
        // The first frame gives the field its text ("" here) at revision 1.
        let wit::Element::Input(i) = &f1.nodes[3].element else { panic!() };
        assert_eq!((i.value.as_deref(), i.revision), (Some(""), 1));
        let go = f1.nodes[2].on[0].handler;
        click(&mut cx, &mut t, go);
        assert_eq!(t.clicks, 1);
        assert!(cx.notified);
        // Same tree: same numbers, and the field's text isn't sent again.
        let root = view(&t, &mut cx);
        let f2 = cx.frame("main", root);
        assert_eq!(f2.nodes[2].on[0].handler, go);
        let wit::Element::Input(i) = &f2.nodes[3].element else { panic!() };
        assert_eq!(i.value, None);
        // The user types: the tile's copy follows, nothing is sent back.
        let change_id = f2.nodes[3].on[0].handler;
        let change = InputChange { value: "abc".into(), selection: 3..3 };
        cx.fire(&mut t, Some((7, &change)), change_id, Fired::Change(change.clone()));
        assert_eq!(t.typed, "abc");
        let root = view(&t, &mut cx);
        let f3 = cx.frame("main", root);
        let wit::Element::Input(i) = &f3.nodes[3].element else { panic!() };
        assert_eq!(i.value, None);
        // The tile replaces the text: sent above the user's revision.
        t.typed = "reset".into();
        let root = view(&t, &mut cx);
        let f4 = cx.frame("main", root);
        let wit::Element::Input(i) = &f4.nodes[3].element else { panic!() };
        assert_eq!((i.value.as_deref(), i.revision), (Some("reset"), 8));
        // An unknown number is ignored.
        click(&mut cx, &mut t, 9999);
        assert_eq!(t.clicks, 1);
    }

    /// `.small()` keeps the look; a field hears its blur when the focus
    /// moves off it, also after a frame took it away.
    #[test]
    fn size_and_look_combine_and_a_field_hears_its_blur() {
        let (mut cx, mut t) = (cx(), T::default());
        let draw = |t: &T, cx: &mut Cx<T>, with_field: bool| {
            cx.generation += 1;
            cx.pending.clear();
            let root = v_flex()
                .child(button("Delete").small().danger().id("a"))
                .child(button("Keep").primary().small().id("b"))
                .when(with_field, |d| d.child(input("f").value(t.typed.clone()).on_blur(cx.listener(|t: &mut T, _: &Blur, _| t.clicks += 10))))
                .into_element();
            cx.frame("main", root)
        };
        let f = draw(&t, &mut cx, true);
        let look = |n: &wit::Node| match &n.element {
            wit::Element::Button(b) => (b.kind, b.size),
            _ => panic!(),
        };
        assert_eq!(look(&f.nodes[1]), (wit::ButtonKind::Danger, wit::ButtonSize::Small));
        assert_eq!(look(&f.nodes[2]), (wit::ButtonKind::Primary, wit::ButtonSize::Small));
        // No WIT listener for blur: it comes from focus-changed.
        assert!(f.nodes[3].on.iter().all(|l| l.on == wit::Listen::Change));
        cx.focus_moved(&mut t, "main", Some("f".into()));
        assert_eq!(cx.focused("main"), Some("f"));
        cx.focus_moved(&mut t, "main", Some("a".into()));
        assert_eq!(t.clicks, 10, "blurred once");
        cx.focus_moved(&mut t, "main", Some("f".into()));
        // The field goes away while focused: its blur still comes.
        draw(&t, &mut cx, false);
        cx.focus_moved(&mut t, "main", None);
        assert_eq!(t.clicks, 20);
    }

    #[test]
    fn views_have_their_own_numbers() {
        let (mut cx, t) = (cx(), T::default());
        let root = view(&t, &mut cx);
        let a = cx.frame("a", root);
        let root = view(&t, &mut cx);
        let b = cx.frame("b", root);
        assert_ne!(a.nodes[2].on[0].handler, b.nodes[2].on[0].handler);
        cx.forget_view("a");
        assert!(!cx.handlers.contains_key(&a.nodes[2].on[0].handler));
        assert!(cx.handlers.contains_key(&b.nodes[2].on[0].handler));
    }

    #[test]
    fn content_parses_into_the_tiles_type() {
        #[derive(Debug, serde::Deserialize)]
        struct Input {
            message: String,
        }
        let c = Content::from_json(r#"{"message":"hi"}"#);
        assert_eq!(c.parse::<Input>().unwrap().message, "hi");
        assert!(Content::from_json("[1]").parse::<Input>().unwrap_err().to_string().contains("isn't what this tile takes"));
    }
}
