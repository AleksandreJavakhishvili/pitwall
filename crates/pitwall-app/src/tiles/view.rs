//! A tile of an extension, drawn (docs/spec/tile-sdk/ui.md §3): the
//! frames its component sends, as Pitwall's own tiles draw — the kit's
//! buttons, text fields, icons and scroll bars, the theme's colours — and
//! what the user does in it sent back as events.
//!
//! - Nodes keep their state by key path across frames: text typed in a
//!   field, a scroll position, focus.
//! - Text fields belong to Pitwall (ui.md §3.5): typing never waits for
//!   the tile. It hears `input-changed` (at most one per field per 50 ms,
//!   and always the last one before a blur or a submit) with the field's
//!   revision; a frame replaces the text only when it names a revision at
//!   least as new as the field's.
//! - Keys reach the tile only while it has the keyboard; Pitwall's own
//!   keys come first. A multi-line field submits on ⌘⏎ as Pitwall's do.
//! - Tab and Shift-Tab move focus among its focusable nodes (buttons,
//!   enabled fields, nodes marked `focusable`) in tree order, wrapping;
//!   Enter or Space presses a focused button (ui.md §3.5).
//! - `ui.scroll-to` scrolls each scroll node the keyed node is in so it
//!   is in sight, once a frame with the node is drawn.
//! - While it starts it says so ("Preparing…" when it is compiled first);
//!   when it stopped it says why in plain words, with Restart.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{canvas, div, prelude::*, px, AnyElement, App, Bounds, Context, Entity, FocusHandle, Focusable, FontStyle, HighlightStyle, KeyDownEvent, MouseButton, Pixels, Point, ScrollHandle, SharedString, Size, StyledText, Subscription, UnderlineStyle, Window};
use pitwall_tile_host::ui::{self, Element, Event, Listen, Node};

use super::render::{self, Inherit};
use super::{State, Tiles};
use crate::kit::{self, InputEvent, TextInput};
use crate::theme::{theme, Theme};

mod comp;
pub use comp::Comps;

/// How often a field's typing reaches the tile at most.
const CHANGE_EVERY: Duration = Duration::from_millis(50);

/// A text field Pitwall keeps for an `input` node.
struct Field {
    input: Entity<TextInput>,
    multiline: bool,
    /// Bumped by each edit of the user's.
    rev: u64,
    /// The newest revision the tile heard.
    sent: u64,
    last_sent: Option<Instant>,
    /// A change waits for the 50 ms to pass.
    waiting: bool,
    /// The selection last seen (a move of it alone is `input-selected`).
    sel: std::ops::Range<usize>,
    /// What the last frame named (value, revision, selection): applied
    /// once, when it changes.
    named: (Option<String>, u64, Option<(u32, u32)>),
    _subs: Vec<Subscription>,
}

/// What a node keeps across frames.
#[derive(Default)]
struct NodeState {
    field: Option<Field>,
    scroll: Option<ScrollHandle>,
    /// Where it was last drawn (positions in events are relative to it).
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The scroll position last told (x, y).
    told: Rc<Cell<Option<(f32, f32)>>>,
    focus: Option<FocusHandle>,
    /// A code or diff view's (native-blocks.md §16.2).
    block: Option<super::blocks::Block>,
    /// A component's own (a list's scroll, a select open…).
    comp: Option<comp::Comp>,
}

pub struct TileView {
    ext: String,
    /// The tile id (the view's id too: one view per tile).
    tile: String,
    tiles: Entity<Tiles>,
    focus: FocusHandle,
    /// The frame drawn, and the counter it came with.
    frame: Option<Arc<ui::Frame>>,
    applied: u64,
    /// Each node's key path, children by index.
    paths: Vec<String>,
    kids: Vec<Vec<usize>>,
    nodes: HashMap<String, NodeState>,
    size: Rc<Cell<Option<Size<Pixels>>>>,
    had_focus: bool,
    /// The node with keyboard focus the tile was last told of
    /// (`focus-changed`).
    node_focus: Option<String>,
    /// A node the tile asked to focus, done while the tile has focus.
    want_focus: Option<String>,
    /// The newest `ui.scroll-to` taken, and the node it waits to scroll
    /// to (with the counter it came with).
    scrolled: u64,
    scroll_want: Option<(String, u64)>,
    shown: bool,
    /// Its menus: which node opens which, the one open.
    ui: comp::ViewUi,
    _subs: Vec<Subscription>,
}

impl Focusable for TileView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The handler of node `n` for `on`.
fn handler(n: &Node, on: Listen) -> Option<u32> {
    n.on.iter().find(|l| l.on == on).map(|l| l.handler)
}

fn point(p: Point<Pixels>, origin: Option<Bounds<Pixels>>) -> ui::Point {
    let o = origin.map(|b| b.origin).unwrap_or_default();
    ui::Point { x: f32::from(p.x - o.x), y: f32::from(p.y - o.y) }
}

fn modifiers(m: &gpui::Modifiers) -> ui::Modifiers {
    let mut out = ui::Modifiers::empty();
    if m.shift {
        out |= ui::Modifiers::SHIFT;
    }
    if m.control {
        out |= ui::Modifiers::CTRL;
    }
    if m.alt {
        out |= ui::Modifiers::ALT;
    }
    if m.platform {
        out |= ui::Modifiers::PLATFORM;
    }
    out
}

fn button(b: MouseButton) -> ui::MouseButton {
    match b {
        MouseButton::Right => ui::MouseButton::Right,
        MouseButton::Middle => ui::MouseButton::Middle,
        _ => ui::MouseButton::Left,
    }
}

fn range(r: &std::ops::Range<usize>) -> ui::TextRange {
    ui::TextRange { start: r.start as u32, end: r.end as u32 }
}

impl TileView {
    /// Tile `tile` of extension `ext`: its component starts (or the view
    /// opens in the running one); closed when this is let go.
    pub fn new(ext: &str, tile: &str, cx: &mut Context<Self>) -> TileView {
        let tiles = super::tiles(cx);
        let (e, v) = (ext.to_string(), tile.to_string());
        tiles.update(cx, |t, cx| t.open(&e, &v, cx));
        let mut subs = vec![cx.observe(&tiles, |this: &mut TileView, _, cx| this.sync(cx))];
        subs.push(cx.on_release(|this: &mut TileView, cx: &mut App| {
            let (e, v) = (this.ext.clone(), this.tile.clone());
            this.tiles.update(cx, |t, cx| t.close(&e, &v, cx));
        }));
        let mut me = TileView {
            ext: ext.to_string(),
            tile: tile.to_string(),
            tiles,
            focus: cx.focus_handle(),
            frame: None,
            applied: 0,
            paths: vec![],
            kids: vec![],
            nodes: HashMap::new(),
            size: Rc::new(Cell::new(None)),
            had_focus: false,
            node_focus: None,
            want_focus: None,
            scrolled: 0,
            scroll_want: None,
            shown: false,
            ui: Default::default(),
            _subs: subs,
        };
        me.sync(cx);
        me
    }

    pub fn ext(&self) -> &str {
        &self.ext
    }

    pub fn tile(&self) -> &str {
        &self.tile
    }

    /// Shown (drawn in a layout or a window) or hidden: its component is
    /// suspended while none of its views shows.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.shown == shown {
            return;
        }
        self.shown = shown;
        let (e, v) = (self.ext.clone(), self.tile.clone());
        self.tiles.update(cx, |t, cx| t.set_visible(&e, &v, shown, cx));
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    /// "from <agent>", in the tile's bottom right corner: the agent whose
    /// call it shows last; pressed, that agent's terminal is focused.
    fn origin_chip(&self, id: &str, agent: &str, t: &crate::theme::Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let id = id.to_string();
        div()
            .absolute()
            .bottom(px(6.))
            .right(px(8.))
            .child(
                div()
                    .id(SharedString::from(format!("tile-origin-{}-{}", self.ext, self.tile)))
                    .debug_selector(|| "tile-origin".into())
                    .cursor_pointer()
                    .tooltip(crate::kit::tooltip(format!("Shown by {agent}: press to go to its terminal")))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        crate::windows::focus_in_front(&id, cx);
                    }))
                    .child(crate::kit::chip_tone(format!("from {agent}"), crate::kit::Tone::Subtle, t)),
            )
    }

    fn send(&self, event: Event, cx: &App) -> bool {
        self.tiles.read(cx).send(&self.ext, event)
    }

    /// The newest frame, applied when it is new; a newer `ui.scroll-to`
    /// taken.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let ask = self.tiles.read(cx).get(&self.ext).and_then(|r| r.scroll_to.get(&self.tile)).cloned();
        if let Some((path, seq)) = ask.filter(|(_, seq)| *seq > self.scrolled) {
            self.scrolled = seq;
            self.scroll_want = Some((path, seq));
        }
        self.sync_frame(cx);
        self.scroll_wanted(cx);
    }

    fn sync_frame(&mut self, cx: &mut Context<Self>) {
        let newest = self.tiles.read(cx).get(&self.ext).and_then(|r| r.frames.get(&self.tile)).map(|(u, seq)| (u.frame.clone(), *seq));
        match newest {
            Some((frame, seq)) if seq != self.applied => {
                self.applied = seq;
                self.apply(frame, cx);
            }
            // Restarted: no frame yet.
            None if self.frame.is_some() && self.tiles.read(cx).get(&self.ext).is_none_or(|r| r.state != State::Running) => {
                self.frame = None;
                self.applied = 0;
            }
            _ => {}
        }
        cx.notify();
    }

    /// Draw `frame`: node state moved by key path, text fields made or
    /// updated.
    fn apply(&mut self, frame: Arc<ui::Frame>, cx: &mut Context<Self>) {
        let n = frame.nodes.len();
        let mut paths: Vec<String> = Vec::with_capacity(n);
        let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, node) in frame.nodes.iter().enumerate() {
            let path = match node.parent {
                None => String::new(),
                Some(p) => {
                    let p = p as usize;
                    if p < i {
                        kids[p].push(i);
                    }
                    match paths.get(p).map(String::as_str) {
                        Some("") | None => node.key.clone(),
                        Some(pp) => format!("{pp}/{}", node.key),
                    }
                }
            };
            paths.push(path);
        }
        let mut old = std::mem::take(&mut self.nodes);
        let mut nodes = HashMap::with_capacity(n);
        for (i, node) in frame.nodes.iter().enumerate() {
            let path = &paths[i];
            let mut st = old.remove(path).unwrap_or_default();
            match &node.element {
                Element::Input(inp) => {
                    self.field(&mut st, path, inp, cx);
                    // A multi-line field given a height (or room to grow)
                    // fills it; else it is its rows tall.
                    let fill = inp.multiline && node.style.iter().any(|s| matches!(s, ui::Style::Height(_) | ui::Style::Grow(_)));
                    if let Some(f) = &st.field {
                        f.input.update(cx, |i, cx| i.set_fill(fill, cx));
                    }
                }
                _ => st.field = None,
            }
            match &node.element {
                Element::Scroll(_) => {
                    st.scroll.get_or_insert_with(ScrollHandle::new);
                }
                _ => st.scroll = None,
            }
            super::blocks::Block::sync(&mut st.block, path, &node.element, cx);
            self.sync_comp(&mut st, node, path, cx);
            // Buttons are focusable whatever the node says; fields have
            // their own.
            if (node.focusable || matches!(node.element, Element::Button(_))) && st.focus.is_none() && !matches!(node.element, Element::Input(_)) {
                st.focus = Some(cx.focus_handle().tab_stop(true));
            }
            nodes.insert(path.clone(), st);
        }
        if frame.focus != self.frame.as_ref().and_then(|f| f.focus.clone()) {
            self.want_focus = frame.focus.clone();
        }
        self.paths = paths;
        self.kids = kids;
        self.nodes = nodes;
        self.frame = Some(frame);
        self.comps_applied();
    }

    /// Make or update the field of input node `path`.
    fn field(&mut self, st: &mut NodeState, path: &str, inp: &ui::Input, cx: &mut Context<Self>) {
        let rows = inp.rows.unwrap_or(3).max(1) as usize;
        if st.field.as_ref().is_some_and(|f| f.multiline != inp.multiline) {
            st.field = None;
        }
        let f = st.field.get_or_insert_with(|| {
            let (value, placeholder, multiline) = (inp.value.clone().unwrap_or_default(), inp.placeholder.clone().unwrap_or_default(), inp.multiline);
            let input = cx.new(|cx| {
                let t = TextInput::new(cx, &value, &placeholder);
                if multiline {
                    t.multiline(rows)
                } else {
                    t
                }
            });
            let p = path.to_string();
            let subs = vec![
                cx.subscribe(&input, move |this: &mut TileView, _, e: &InputEvent, cx| this.on_input(&p, e, cx)),
                {
                    let p = path.to_string();
                    cx.observe(&input, move |this: &mut TileView, _, cx| this.on_select(&p, cx))
                },
            ];
            let sel = input.read(cx).selection();
            Field { input, multiline, rev: inp.revision, sent: inp.revision, last_sent: None, waiting: false, sel, named: (inp.value.clone(), inp.revision, None), _subs: subs }
        });
        let input = f.input.clone();
        let placeholder = inp.placeholder.clone().unwrap_or_default();
        let disabled = inp.disabled;
        input.update(cx, |i, cx| {
            i.set_placeholder(placeholder);
            i.set_disabled(disabled, cx);
        });
        let named = (inp.value.clone(), inp.revision, inp.selection.as_ref().map(|s| (s.start, s.end)));
        if named != f.named {
            // The tile's text replaces the user's only when the tile has
            // seen the user's newest edit.
            if inp.revision >= f.rev {
                if let Some(v) = &inp.value {
                    if input.read(cx).text() != v {
                        input.update(cx, |i, cx| i.set_text(v.clone(), cx));
                    }
                }
                if let Some(s) = &inp.selection {
                    let r = s.start as usize..s.end as usize;
                    input.update(cx, |i, cx| i.set_selection(r, cx));
                }
                f.rev = inp.revision;
                f.sent = f.sent.max(inp.revision);
                f.sel = input.read(cx).selection();
            }
            f.named = named;
        }
    }

    /// The nodes Tab stops at, in tree order, with their focus handles:
    /// enabled buttons and fields, and nodes marked `focusable`.
    fn focus_stops(&self, cx: &App) -> Vec<(String, FocusHandle)> {
        let Some(frame) = &self.frame else { return vec![] };
        tree_order(&self.kids)
            .into_iter()
            .filter_map(|i| {
                let st = self.nodes.get(&self.paths[i])?;
                let h = match &frame.nodes[i].element {
                    Element::Input(inp) if !inp.disabled => st.field.as_ref().map(|f| f.input.read(cx).focus_handle(cx)),
                    Element::Input(_) => None,
                    Element::Button(b) if b.disabled => None,
                    _ => st.focus.clone(),
                }?;
                Some((self.paths[i].clone(), h))
            })
            .collect()
    }

    /// Tab (Shift-Tab: `back`): focus moves to the next focusable node in
    /// tree order, wrapping (the first, or the last, when none has it).
    /// The node focused. (Also what tests call: no key is pressed.)
    pub fn move_focus(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) -> Option<String> {
        let stops = self.focus_stops(cx);
        let at = stops.iter().position(|(_, h)| h.contains_focused(window, cx));
        let (path, h) = stops.get(step(stops.len(), at, back)?)?.clone();
        window.focus(&h, cx);
        cx.notify();
        Some(path)
    }

    /// The node that has focus in the tile (tests).
    pub fn focused(&self, window: &Window, cx: &App) -> Option<String> {
        self.focus_stops(cx).into_iter().find(|(_, h)| h.contains_focused(window, cx)).map(|(p, _)| p)
    }

    /// Scroll to the node `ui.scroll-to` named, once it is drawn: each
    /// scroll node it is in scrolls the child it is under into sight. A
    /// node not in the first frame drawn after the ask is let go.
    fn scroll_wanted(&mut self, cx: &mut Context<Self>) {
        let Some((path, seq)) = self.scroll_want.clone() else { return };
        let Some(frame) = self.frame.clone() else { return };
        let Some(i) = self.paths.iter().position(|p| *p == path) else {
            if self.applied > seq {
                self.scroll_want = None;
            }
            return;
        };
        for (s, child) in scroll_steps(&frame.nodes, &self.kids, i) {
            if let Some(h) = self.nodes.get(&self.paths[s]).and_then(|st| st.scroll.as_ref()) {
                // The scroll's content: its probe first, then its children.
                h.scroll_to_item(child + 1);
            }
        }
        self.scroll_want = None;
        cx.notify();
    }

    /// The node at key path `path`, by index.
    fn node(&self, path: &str) -> Option<&Node> {
        let i = self.paths.iter().position(|p| p == path)?;
        self.frame.as_ref()?.nodes.get(i)
    }

    /// The handler of node `path`'s `on` listener.
    pub(super) fn listener_of(&self, path: &str, on: Listen) -> Option<u32> {
        self.node(path).and_then(|n| handler(n, on))
    }

    /// Send `event` to the tile.
    pub(super) fn send_event(&self, event: Event, cx: &App) -> bool {
        self.send(event, cx)
    }

    /// The code or diff view of node `path`.
    pub(super) fn block_mut(&mut self, path: &str) -> Option<&mut super::blocks::Block> {
        self.nodes.get_mut(path)?.block.as_mut()
    }

    /// The code or diff view of node `path` (tests and bench commands).
    pub fn code_view_of(&self, path: &str) -> Option<Entity<crate::code_view::CodeView>> {
        self.nodes.get(path)?.block.as_ref().map(|b| b.view.clone())
    }

    /// A click on canvas region `key` of node `path`, at its centre (tests
    /// and bench commands: nothing is clicked on the screen).
    pub fn click_region(&self, path: &str, key: &str, cx: &App) -> bool {
        let Some(Element::Canvas(c)) = self.node(path).map(|n| &n.element) else { return false };
        let Some(r) = c.regions.iter().find(|r| r.key == key) else { return false };
        let Some(handler) = r.click else { return false };
        let b = super::canvas::Tf::new(&c.transform, Point::default()).rect(&r.rect);
        let at = ui::Point { x: f32::from(b.center().x), y: f32::from(b.center().y) };
        self.send(Event::Click(ui::Click { handler, button: ui::MouseButton::Left, modifiers: ui::Modifiers::empty(), count: 1, at }), cx)
    }

    /// Send pointer event `kind` at `at` to node `path`'s `pointer` (or,
    /// for a wheel or pinch, `wheel`) listener (tests and bench commands).
    pub fn pointer(&self, path: &str, kind: ui::PointerKind, at: ui::Point, cx: &App) -> bool {
        let on = if matches!(kind, ui::PointerKind::Wheel(_) | ui::PointerKind::Pinch(_)) { Listen::Wheel } else { Listen::Pointer };
        let Some(handler) = self.listener_of(path, on) else { return false };
        self.send(Event::Pointer(ui::PointerEvent { handler, kind, at, modifiers: ui::Modifiers::empty() }), cx)
    }

    fn on_input(&mut self, path: &str, e: &InputEvent, cx: &mut Context<Self>) {
        match e {
            InputEvent::Changed => {
                let Some(f) = self.nodes.get_mut(path).and_then(|s| s.field.as_mut()) else { return };
                f.rev += 1;
                let due = f.last_sent.is_none_or(|t| t.elapsed() >= CHANGE_EVERY);
                if due {
                    self.tell_change(path, cx);
                } else if !f.waiting {
                    f.waiting = true;
                    let p = path.to_string();
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(CHANGE_EVERY).await;
                        let _ = this.update(cx, |this, cx| this.flush(&p, cx));
                    })
                    .detach();
                }
            }
            InputEvent::Submit | InputEvent::SubmitShift => {
                self.flush(path, cx);
                if let Some(h) = self.node(path).and_then(|n| handler(n, Listen::Submit)) {
                    self.send(Event::Submit(h), cx);
                }
            }
            InputEvent::Blur => self.flush(path, cx),
            InputEvent::Cancel => {}
        }
    }

    /// A change the tile hasn't heard, told now.
    fn flush(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(f) = self.nodes.get_mut(path).and_then(|s| s.field.as_mut()) else { return };
        f.waiting = false;
        if f.rev > f.sent {
            self.tell_change(path, cx);
        }
    }

    fn tell_change(&mut self, path: &str, cx: &mut Context<Self>) {
        let h = self.node(path).and_then(|n| handler(n, Listen::Change));
        let Some(f) = self.nodes.get_mut(path).and_then(|s| s.field.as_mut()) else { return };
        let (value, sel) = {
            let i = f.input.read(cx);
            (i.text().to_string(), i.selection())
        };
        f.sent = f.rev;
        f.last_sent = Some(Instant::now());
        f.waiting = false;
        f.sel = sel.clone();
        let rev = f.rev;
        if let Some(handler) = h {
            self.send(Event::InputChanged(ui::InputChange { handler, value, revision: rev, selection: range(&sel) }), cx);
        }
    }

    /// The selection moved without an edit: `input-selected`.
    fn on_select(&mut self, path: &str, cx: &mut Context<Self>) {
        let h = self.node(path).and_then(|n| handler(n, Listen::Select));
        let Some(f) = self.nodes.get_mut(path).and_then(|s| s.field.as_mut()) else { return };
        let sel = f.input.read(cx).selection();
        if sel == f.sel || f.rev != f.sent {
            return;
        }
        f.sel = sel.clone();
        let rev = f.rev;
        if let Some(handler) = h {
            self.send(Event::InputSelected(ui::InputSelection { handler, selection: range(&sel), revision: rev }), cx);
        }
    }

    /// A click on node `path` (also what tests and bench commands call:
    /// nothing is clicked on the screen).
    pub fn click(&self, path: &str, button: ui::MouseButton, mods: ui::Modifiers, count: u32, at: ui::Point, cx: &App) -> bool {
        let Some(handler) = self.node(path).and_then(|n| handler(n, Listen::Click)) else { return false };
        self.send(Event::Click(ui::Click { handler, button, modifiers: mods, count, at }), cx)
    }

    /// The field of node `path` (tests and bench commands type with it).
    pub fn field_of(&self, path: &str) -> Option<Entity<TextInput>> {
        self.nodes.get(path)?.field.as_ref().map(|f| f.input.clone())
    }

    /// The texts drawn, in tree order (tests).
    pub fn texts(&self) -> Vec<String> {
        let Some(f) = &self.frame else { return vec![] };
        f.nodes
            .iter()
            .filter_map(|n| match &n.element {
                Element::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.contains_focused(window, cx) {
            return;
        }
        let Some(frame) = self.frame.clone() else { return };
        let ks = &e.keystroke;
        let plain = !(ks.modifiers.platform || ks.modifiers.control || ks.modifiers.alt);
        // Esc closes an open menu.
        if ks.key == "escape" && self.close_menu(cx) {
            cx.stop_propagation();
            return;
        }
        // Tab moves focus, in a field too.
        if plain && ks.key == "tab" {
            if self.move_focus(ks.modifiers.shift, window, cx).is_some() {
                cx.stop_propagation();
            }
            return;
        }
        // A field gets plain keys (typing "j" is typing).
        let typing = self.nodes.values().filter_map(|s| s.field.as_ref()).any(|f| f.input.read(cx).is_focused(window));
        if typing && plain {
            return;
        }
        // A focused button: Enter or Space presses it.
        if plain && !ks.modifiers.shift && matches!(ks.key.as_str(), "enter" | "space") {
            let button = self.focused(window, cx).filter(|p| self.node(p).is_some_and(|n| matches!(n.element, Element::Button(_))));
            if let Some(path) = button {
                self.click(&path, ui::MouseButton::Left, ui::Modifiers::empty(), 1, ui::Point { x: 0., y: 0. }, cx);
                cx.stop_propagation();
                return;
            }
        }
        let mac = cfg!(target_os = "macos");
        if let Some(b) = frame.keymap.iter().find(|b| !render::reserved(&b.keys) && render::matches(&b.keys, ks, mac)) {
            self.send(Event::Key(ui::KeyEvent { handler: b.handler, keys: b.keys.clone() }), cx);
            cx.stop_propagation();
        }
    }

    /// Node `i` and what is under it, as elements.
    fn build(&self, i: usize, inherit: Inherit, window: &mut Window, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Some(frame) = self.frame.clone() else { return div().into_any_element() };
        let node = &frame.nodes[i];
        let path = self.paths[i].clone();
        let st = self.nodes.get(&path);
        let inherit = inherit.with(&node.style, t);
        let id = SharedString::from(format!("tile-{}-{}-{path}", self.ext, self.tile));
        let sel = format!("tile-node-{path}");
        match &node.element {
            Element::Text(text) => {
                // Text children are runs of it, each with its own styles.
                let mut all = text.clone();
                let mut runs = Vec::new();
                for &k in &self.kids[i] {
                    if let Element::Text(run) = &frame.nodes[k].element {
                        let start = all.len();
                        all.push_str(run);
                        let mut h = HighlightStyle::default();
                        for s in &frame.nodes[k].style {
                            match s {
                                ui::Style::Color(c) => h.color = Some(render::color(*c, t)),
                                ui::Style::Weight(w) => h.font_weight = Some(render::weight(*w)),
                                ui::Style::Italic => h.font_style = Some(FontStyle::Italic),
                                ui::Style::Underline => h.underline = Some(UnderlineStyle { thickness: px(1.), ..Default::default() }),
                                ui::Style::Background(c) => h.background_color = Some(render::color(*c, t)),
                                _ => {}
                            }
                        }
                        runs.push((start..all.len(), h));
                    }
                }
                let (d, f) = render::apply(div(), &node.style, t);
                let d = self.interact(d.id(id), node, &path, f, st, t, cx);
                d.debug_selector(move || sel.clone()).child(StyledText::new(all).with_highlights(runs)).into_any_element()
            }
            Element::Button(b) => {
                let (kind, shrink) = render::button_kind(b.kind, b.size);
                let fg = match kind {
                    kit::BtnKind::Primary => kit::on_primary(t),
                    kit::BtnKind::Danger => gpui::white(),
                    kit::BtnKind::Small => t.text,
                    _ => t.text_2,
                };
                let mut el = kit::button(id, kind, b.disabled, t).debug_selector(move || sel.clone());
                // A small primary or danger button: the small button's size.
                if shrink {
                    el = el.h(px(26.)).px(px(9.)).text_size(px(12.));
                }
                if let Some(ui::Icon::Named(name)) = &b.icon {
                    if kit::has_icon(name) {
                        el = el.child(kit::icon(name, 13., fg));
                    }
                }
                // An icon button is square.
                if b.label.is_empty() && b.icon.is_some() && kind != kit::BtnKind::Link {
                    let side = if shrink || kind == kit::BtnKind::Small { 26. } else { 30. };
                    el = el.w(px(side)).px(px(0.)).justify_center();
                }
                el = el.child(b.label.clone());
                // Its styles: size and place only (the kit draws it).
                for s in &node.style {
                    el = match s {
                        ui::Style::Width(l) => el.w(render::length(l)),
                        ui::Style::Grow(g) => el.flex_grow(g.max(0.)),
                        ui::Style::SelfAlign(ui::Align::Stretch) => el.self_stretch(),
                        ui::Style::SelfAlign(ui::Align::Center) => el.self_center(),
                        ui::Style::SelfAlign(ui::Align::End) => el.self_end(),
                        ui::Style::SelfAlign(ui::Align::Start) => el.self_start(),
                        ui::Style::Margin(e) => el.mt(render::length(&e.top)).mr(render::length(&e.right)).mb(render::length(&e.bottom)).ml(render::length(&e.left)),
                        ui::Style::VisibleOnGroupHover(g) => el.invisible().group_hover(render::group(g), |s| s.visible()),
                        _ => el,
                    };
                }
                let el = el.when_some(node.tooltip.clone(), |d, tip| d.tooltip(kit::tooltip(tip)));
                // Focus (Tab), with the ring boxes have.
                let el = match st.and_then(|s| s.focus.as_ref()) {
                    Some(h) => {
                        let focused = h.is_focused(window);
                        el.track_focus(h).when(focused, |d| d.shadow(vec![focus_ring(t)]))
                    }
                    None => el,
                };
                match (b.disabled, handler(node, Listen::Click)) {
                    (false, Some(h)) => {
                        let bounds = st.map(|s| s.bounds.clone()).unwrap_or_default();
                        el.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |v, e: &gpui::ClickEvent, _, cx| {
                                cx.stop_propagation();
                                let b = if e.is_right_click() { ui::MouseButton::Right } else if e.is_middle_click() { ui::MouseButton::Middle } else { ui::MouseButton::Left };
                                v.send(Event::Click(ui::Click { handler: h, button: b, modifiers: modifiers(&e.modifiers()), count: e.click_count() as u32, at: point(e.position(), bounds.get()) }), cx);
                            }))
                            .relative()
                            .child(bounds_probe(st))
                            .into_any_element()
                    }
                    _ => el.into_any_element(),
                }
            }
            Element::Icon(ui::Icon::Named(name)) => {
                let name = if kit::has_icon(name) { name.as_str() } else { "dot" };
                let size = inherit.size + 1.;
                let (d, f) = render::apply(div(), &node.style, t);
                let d = self.interact(d.id(id).flex_none().items_center().justify_center(), node, &path, f, st, t, cx);
                d.debug_selector(move || sel.clone()).child(kit::icon(name, size, inherit.color)).into_any_element()
            }
            Element::Input(_) => {
                let Some(field) = st.and_then(|s| s.field.as_ref()) else { return div().into_any_element() };
                let (d, f) = render::apply(div(), &node.style, t);
                let d = self.interact(d.id(id), node, &path, f, st, t, cx);
                d.debug_selector(move || sel.clone()).child(field.input.clone()).into_any_element()
            }
            Element::Scroll(s) => {
                let handle = st.and_then(|s| s.scroll.clone()).unwrap_or_default();
                let children: Vec<AnyElement> = self.kids[i].clone().into_iter().map(|k| self.build(k, inherit, window, t, cx)).collect();
                // Size and place on the outer box; the rest on the content.
                let (outer_styles, inner_styles): (Vec<ui::Style>, Vec<ui::Style>) = node.style.iter().cloned().partition(|s| {
                    matches!(s, ui::Style::Width(_) | ui::Style::Height(_) | ui::Style::MinWidth(_) | ui::Style::MaxWidth(_) | ui::Style::MinHeight(_) | ui::Style::MaxHeight(_) | ui::Style::Grow(_) | ui::Style::Shrink(_) | ui::Style::Margin(_) | ui::Style::SelfAlign(_))
                });
                let (outer, _) = render::apply(div(), &outer_styles, t);
                let (inner, _) = render::apply(div(), &inner_styles, t);
                let inner = inner.id(id.clone()).track_scroll(&handle).child(scroll_probe(&handle, st, handler(node, Listen::Scroll), &self.tiles, &self.ext)).children(children);
                let outer = outer.min_h_0().min_w_0().debug_selector(move || sel.clone());
                match s.axis {
                    ui::Axis::Vertical => outer.child(kit::vscroll(SharedString::from(format!("{id}-bar")), &handle, t, inner.overflow_y_scroll().flex_1().min_h_0()).flex_1().min_h_0()).into_any_element(),
                    ui::Axis::Horizontal => outer.child(inner.overflow_x_scroll().flex_1().min_w_0()).into_any_element(),
                    ui::Axis::Both => outer.child(inner.overflow_scroll().flex_1().min_h_0().min_w_0()).into_any_element(),
                }
            }
            Element::Canvas(c) => {
                let (d, f) = render::apply(div(), &node.style, t);
                let d = self.interact(d.id(id.clone()), node, &path, f, st, t, cx);
                let (paint_frame, theme) = (frame.clone(), t.clone());
                let paint = canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        if let Some(Element::Canvas(c)) = paint_frame.nodes.get(i).map(|n| &n.element) {
                            super::canvas::draw(c, bounds, &theme, window, cx);
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full();
                let regions: Vec<AnyElement> = c.regions.iter().map(|r| self.region(&id, &path, c, r, st, cx)).collect();
                d.debug_selector(move || sel.clone()).relative().overflow_hidden().child(paint).children(regions).into_any_element()
            }
            Element::CodeView(_) | Element::DiffView(_) => {
                let Some(view) = st.and_then(|s| s.block.as_ref()).map(|b| b.view.clone()) else { return div().into_any_element() };
                let (d, f) = render::apply(div(), &node.style, t);
                let d = self.interact(d.id(id), node, &path, f, st, t, cx);
                d.debug_selector(move || sel.clone()).min_h_0().min_w_0().overflow_hidden().child(view).into_any_element()
            }
            Element::Box => {
                let children: Vec<AnyElement> = self.kids[i].clone().into_iter().map(|k| self.build(k, inherit, window, t, cx)).collect();
                // The root fills the tile (its styles may say otherwise).
                let base = if i == 0 { div().flex_1().min_h_0().min_w_0() } else { div() };
                let (d, f) = render::apply(base, &node.style, t);
                let focused = st.and_then(|s| s.focus.as_ref()).is_some_and(|h| h.is_focused(window));
                let d = self.interact(d.id(id), node, &path, f, st, t, cx);
                d.debug_selector(move || sel.clone())
                    .when(focused, |d| d.shadow(vec![focus_ring(t)]))
                    .children(children)
                    .into_any_element()
            }
            // Lists, tables, trees, tabs, selects, menus, Markdown, images.
            _ => self.build_comp(i, node, &path, inherit, window, t, cx),
        }
    }

    /// A node's listeners, tooltip, hover and cursor.
    #[allow(clippy::too_many_arguments)]
    fn interact(&self, d: gpui::Stateful<gpui::Div>, node: &Node, path: &str, f: render::Flags, st: Option<&NodeState>, t: &Theme, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let bounds = st.map(|s| s.bounds.clone()).unwrap_or_default();
        let click = handler(node, Listen::Click);
        let hover = handler(node, Listen::Hover);
        let pointer = handler(node, Listen::Pointer);
        let mut d = d
            .when_some(f.group.as_deref(), |d, g| d.group(render::group(g)))
            .when_some(f.group_hover.as_deref(), |d, g| d.invisible().group_hover(render::group(g), |s| s.visible()))
            .when_some(node.tooltip.clone(), |d, tip| d.tooltip(kit::tooltip(tip)))
            .when_some(f.hover_bg, |d, c| {
                let c = render::color(c, t);
                d.hover(move |s| s.bg(c))
            })
            .map(|d| match f.cursor {
                Some(ui::Cursor::Pointer) => d.cursor_pointer(),
                Some(ui::Cursor::Text) => d.cursor_text(),
                Some(ui::Cursor::Default) => d.cursor_default(),
                None if click.is_some() => d.cursor_pointer(),
                None => d,
            });
        if let Some(h) = st.and_then(|s| s.focus.clone()) {
            d = d.track_focus(&h);
        }
        let mut d = self.context_menu_on(d, path, cx);
        let wheel = handler(node, Listen::Wheel);
        let drag = node.style.iter().rev().find_map(|s| match s {
            ui::Style::Drag(d) => Some(d.clone()),
            _ => None,
        });
        let drop = node.style.iter().rev().find_map(|s| match s {
            ui::Style::DropTarget(d) => Some(d.clone()),
            _ => None,
        });
        if !self.ui.probed(path) && click.is_none() && hover.is_none() && pointer.is_none() && wheel.is_none() && drag.is_none() && drop.is_none() {
            return d;
        }
        d = d.relative().child(bounds_probe(st));
        if let Some(h) = wheel {
            let b = bounds.clone();
            d = d.on_scroll(cx.listener(move |v, e: &gpui::ScrollEvent, window, cx| {
                cx.stop_propagation();
                let delta = e.delta.pixel_delta(window.line_height());
                // GPUI's delta moves the content; the tile's is the scroll.
                let kind = ui::PointerKind::Wheel(ui::Delta { x: -f32::from(delta.x), y: -f32::from(delta.y) });
                v.send(Event::Pointer(ui::PointerEvent { handler: h, kind, at: point(e.position, b.get()), modifiers: modifiers(&e.modifiers) }), cx);
            }));
            let b = bounds.clone();
            d = d.on_pinch(cx.listener(move |v, e: &gpui::PinchEvent, _, cx| {
                cx.stop_propagation();
                v.send(Event::Pointer(ui::PointerEvent { handler: h, kind: ui::PointerKind::Pinch(e.delta), at: point(e.position, b.get()), modifiers: modifiers(&e.modifiers) }), cx);
            }));
        }
        if let Some(src) = drag {
            let value = NodeDrag { ext: self.ext.clone(), tile: self.tile.clone(), kind: src.kind, data: src.data, label: src.label.into() };
            d = d.on_drag(value, |d, grab, _, cx| cx.new(|_| crate::main_screen::topbar::AgentGhost::named(d.label.clone(), grab)));
        }
        if let Some(target) = drop {
            let (ext, tile) = (self.ext.clone(), self.tile.clone());
            let ok = Rc::new(move |d: &NodeDrag| d.ext == ext && d.tile == tile && target.accepts.contains(&d.kind));
            let over = t.focus.opacity(0.12);
            let ring = t.focus;
            let (o1, o2, o3) = (ok.clone(), ok.clone(), ok);
            let b = bounds.clone();
            let h = target.handler;
            d = d
                .drag_over::<NodeDrag>(move |s, drag, _, _| if o1(drag) { s.bg(over).border_color(ring) } else { s })
                .can_drop(move |any, _, _| any.downcast_ref::<NodeDrag>().is_some_and(|d| o2(d)))
                .on_drop(cx.listener(move |v, drag: &NodeDrag, window, cx| {
                    if o3(drag) {
                        let at = point(window.mouse_position(), b.get());
                        v.send(Event::Dropped(ui::DropEvent { handler: h, kind: drag.kind.clone(), data: drag.data.clone(), at }), cx);
                    }
                }));
        }
        if let Some(h) = click {
            let b = bounds.clone();
            let focus = st.and_then(|s| s.focus.clone());
            d = d.on_click(cx.listener(move |v, e: &gpui::ClickEvent, window, cx| {
                cx.stop_propagation();
                if let Some(f) = &focus {
                    window.focus(f, cx);
                }
                let button = if e.is_right_click() { ui::MouseButton::Right } else if e.is_middle_click() { ui::MouseButton::Middle } else { ui::MouseButton::Left };
                v.send(Event::Click(ui::Click { handler: h, button, modifiers: modifiers(&e.modifiers()), count: e.click_count() as u32, at: point(e.position(), b.get()) }), cx);
            }));
        }
        if let Some(h) = hover {
            let b = bounds.clone();
            d = d.on_hover(cx.listener(move |v, inside: &bool, window, cx| {
                v.send(Event::Hover(ui::Hover { handler: h, inside: *inside, at: point(window.mouse_position(), b.get()) }), cx);
            }));
        }
        if let Some(h) = pointer {
            for mb in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
                let b = bounds.clone();
                d = d.on_mouse_down(mb, cx.listener(move |v, e: &gpui::MouseDownEvent, _, cx| {
                    v.send(Event::Pointer(ui::PointerEvent { handler: h, kind: ui::PointerKind::Down(button(e.button)), at: point(e.position, b.get()), modifiers: modifiers(&e.modifiers) }), cx);
                }));
                let b = bounds.clone();
                d = d.on_mouse_up(mb, cx.listener(move |v, e: &gpui::MouseUpEvent, _, cx| {
                    v.send(Event::Pointer(ui::PointerEvent { handler: h, kind: ui::PointerKind::Up(button(e.button)), at: point(e.position, b.get()), modifiers: modifiers(&e.modifiers) }), cx);
                }));
            }
            let b = bounds.clone();
            d = d.on_mouse_move(cx.listener(move |v, e: &gpui::MouseMoveEvent, _, cx| {
                if b.get().is_some_and(|b| b.contains(&e.position)) {
                    v.send(Event::Pointer(ui::PointerEvent { handler: h, kind: ui::PointerKind::Move, at: point(e.position, b.get()), modifiers: modifiers(&e.modifiers) }), cx);
                }
            }));
        }
        d
    }

    /// Canvas region `r` of node `path` (canvas `c`): a box over the
    /// drawing with its tooltip, the hand cursor when it can be clicked,
    /// and its click and hover sent with positions in the canvas node.
    fn region(&self, id: &SharedString, path: &str, c: &ui::Canvas, r: &ui::HitRegion, st: Option<&NodeState>, cx: &mut Context<Self>) -> AnyElement {
        let b = super::canvas::Tf::new(&c.transform, Point::default()).rect(&r.rect);
        let canvas_at = st.map(|s| s.bounds.clone()).unwrap_or_default();
        let sel = format!("tile-region-{path}/{}", r.key);
        let tip = r.tooltip.clone().unwrap_or_else(|| r.label.clone());
        let mut d = div()
            .id(SharedString::from(format!("{id}-region-{}", r.key)))
            .debug_selector(move || sel.clone())
            .absolute()
            .left(b.origin.x)
            .top(b.origin.y)
            .w(b.size.width)
            .h(b.size.height)
            .when(!tip.is_empty(), |d| d.tooltip(kit::tooltip(tip)));
        if let Some(h) = r.click {
            let at = canvas_at.clone();
            d = d.cursor_pointer().on_click(cx.listener(move |v, e: &gpui::ClickEvent, _, cx| {
                // A press that dragged (to pan) isn't a click.
                if let gpui::ClickEvent::Mouse(m) = e {
                    let moved = m.up.position - m.down.position;
                    if f32::from(moved.x.abs() + moved.y.abs()) > super::canvas::CLICK_SLACK {
                        return;
                    }
                }
                cx.stop_propagation();
                let button = if e.is_right_click() { ui::MouseButton::Right } else if e.is_middle_click() { ui::MouseButton::Middle } else { ui::MouseButton::Left };
                v.send(Event::Click(ui::Click { handler: h, button, modifiers: modifiers(&e.modifiers()), count: e.click_count() as u32, at: point(e.position(), at.get()) }), cx);
            }));
        }
        if let Some(h) = r.hover {
            d = d.on_hover(cx.listener(move |v, inside: &bool, window, cx| {
                v.send(Event::Hover(ui::Hover { handler: h, inside: *inside, at: point(window.mouse_position(), canvas_at.get()) }), cx);
            }));
        }
        d.into_any_element()
    }

    /// The key path of the node that has the keyboard: a text field or a
    /// focusable node.
    fn focused_node(&self, window: &Window, cx: &App) -> Option<String> {
        self.nodes.iter().find_map(|(path, st)| {
            let field = st.field.as_ref().is_some_and(|f| f.input.read(cx).is_focused(window));
            let own = st.focus.as_ref().is_some_and(|h| h.is_focused(window));
            (field || own).then(|| path.clone())
        })
    }

    /// What the tile shows while it can't draw: starting, stopped, off.
    fn status(&self, name: &str, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        use crate::main_screen::calm;
        if !crate::ui_state::get(cx).extension_on(&self.ext) {
            let el = calm("extension", &format!("{name} is off"), "Turn it on in Settings → Extensions.".into(), t).child(
                kit::text_button("tile-settings", "Open Settings", kit::BtnKind::Primary, false, t).debug_selector(|| "tile-settings".into()).on_click(|_, window, cx| {
                    crate::settings::data::set_page(cx, pitwall_proto::settings::Page::Extensions);
                    window.dispatch_action(Box::new(crate::menu::OpenSettings), cx)
                }),
            );
            return Some(el.debug_selector(|| "tile-state".into()).into_any_element());
        }
        let state = self.tiles.read(cx).get(&self.ext).map(|r| r.state.clone());
        let el = match state {
            Some(State::Stopped(why)) => {
                let ext = self.ext.clone();
                calm("alert", &format!("{name} stopped"), format!("{} Restart it, or see its Log in Settings → Extensions.", super::stop_words(&why)), t).child(
                    kit::text_button("tile-restart", "Restart", kit::BtnKind::Primary, false, t).debug_selector(|| "tile-restart".into()).on_click(cx.listener(move |v, _, _, cx| {
                        let ext = ext.clone();
                        v.tiles.update(cx, |t, cx| t.restart(&ext, cx));
                    })),
                )
            }
            Some(State::Starting { preparing: true }) => calm("refresh", "Starting…", format!("Preparing {name} for this computer. This happens once for each version."), t),
            None | Some(State::Starting { .. } | State::Dropped) => calm("refresh", "Starting…", format!("{name} is starting."), t),
            Some(State::Running) if self.frame.is_none() => calm("refresh", "Starting…", format!("{name} is starting."), t),
            Some(State::Running) => return None,
        };
        Some(el.debug_selector(|| "tile-state".into()).into_any_element())
    }
}

/// A node of a tile dragged inside its view (ui.md §3.6): dropped only on
/// a target of the same view that accepts its kind.
#[derive(Clone, Debug)]
pub struct NodeDrag {
    ext: String,
    tile: String,
    kind: String,
    data: String,
    label: SharedString,
}

/// The ring around a focused node.
fn focus_ring(t: &Theme) -> gpui::BoxShadow {
    gpui::BoxShadow { color: t.focus.opacity(0.55), offset: gpui::point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(2.), inset: false }
}

/// The nodes from the root down, each before its children, siblings in
/// order (`kids`: each node's children by index).
pub(super) fn tree_order(kids: &[Vec<usize>]) -> Vec<usize> {
    let mut out = Vec::with_capacity(kids.len());
    let mut stack = if kids.is_empty() { vec![] } else { vec![0] };
    while let Some(i) = stack.pop() {
        out.push(i);
        stack.extend(kids[i].iter().rev().copied());
    }
    out
}

/// Of `n` stops, the one after `at` (before it, `back`), wrapping; the
/// first (the last, `back`) when none is at; none when there are none.
pub(super) fn step(n: usize, at: Option<usize>, back: bool) -> Option<usize> {
    if n == 0 {
        return None;
    }
    Some(match (at, back) {
        (None, false) => 0,
        (None, true) => n - 1,
        (Some(i), false) => (i + 1) % n,
        (Some(i), true) => (i + n - 1) % n,
    })
}

/// What scrolling to node `i` scrolls: each scroll node it is in,
/// innermost first, with which of that scroll's children it is under.
pub(super) fn scroll_steps(nodes: &[Node], kids: &[Vec<usize>], i: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut child = i;
    while let Some(p) = nodes.get(child).and_then(|n| n.parent).map(|p| p as usize).filter(|p| *p < child) {
        if matches!(nodes[p].element, Element::Scroll(_)) {
            if let Some(ix) = kids[p].iter().position(|k| *k == child) {
                out.push((p, ix));
            }
        }
        child = p;
    }
    out
}

/// An empty box over its parent that notes where the parent was drawn.
fn bounds_probe(st: Option<&NodeState>) -> AnyElement {
    let cell = st.map(|s| s.bounds.clone()).unwrap_or_default();
    canvas(move |b, _, _| cell.set(Some(b)), |_, _, _, _| {}).absolute().top_0().left_0().size_full().into_any_element()
}

/// Tells the tile where a scroll node is scrolled when it moved.
fn scroll_probe(handle: &ScrollHandle, st: Option<&NodeState>, h: Option<u32>, tiles: &Entity<Tiles>, ext: &str) -> AnyElement {
    let told = st.map(|s| s.told.clone()).unwrap_or_default();
    let (handle, tiles, ext) = (handle.clone(), tiles.clone(), ext.to_string());
    canvas(
        move |_, _, cx| {
            let Some(h) = h else { return };
            let off = handle.offset();
            let now = (-f32::from(off.x), -f32::from(off.y));
            if told.get() == Some(now) {
                return;
            }
            told.set(Some(now));
            let size = handle.bounds().size;
            let max = handle.max_offset();
            let (w, hgt) = (f32::from(size.width + max.x), f32::from(size.height + max.y));
            tiles.read(cx).send(&ext, Event::Scroll(ui::ScrollPosition { handler: h, x: now.0, y: now.1, content_width: w, content_height: hgt }));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .size_0()
    .into_any_element()
}

impl Render for TileView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        if !self.shown {
            self.set_shown(true, cx);
        }
        // Turned on again, or restarted elsewhere: its view opens.
        let on = crate::ui_state::get(cx).extension_on(&self.ext);
        if on && !self.tiles.read(cx).views(&self.ext).contains(&self.tile) {
            let (e, v) = (self.ext.clone(), self.tile.clone());
            let shown = self.shown;
            let tiles = self.tiles.clone();
            cx.defer(move |cx| {
                tiles.update(cx, |t, cx| {
                    t.open(&e, &v, cx);
                    t.set_visible(&e, &v, shown, cx);
                });
            });
        }
        // The tile's focus and its focused node (ui.md §3.4): gaining it,
        // `focus(true)` first; losing it, the node's `focus-changed` first.
        let focused = self.focus.contains_focused(window, cx);
        if focused && !self.had_focus {
            self.had_focus = true;
            self.send(Event::Focus(true), cx);
        }
        let node = if focused { self.focused_node(window, cx) } else { None };
        if node != self.node_focus {
            self.node_focus = node.clone();
            self.send(Event::FocusChanged(ui::NodeFocus { view: self.tile.clone(), key_path: node }), cx);
        }
        if !focused && self.had_focus {
            self.had_focus = false;
            self.send(Event::Focus(false), cx);
        }
        if focused {
            if let Some(path) = self.want_focus.take() {
                let h = self.nodes.get(&path).and_then(|s| s.field.as_ref().map(|f| f.input.read(cx).focus_handle(cx)).or_else(|| s.focus.clone()));
                if let Some(h) = h {
                    window.defer(cx, move |window, cx| window.focus(&h, cx));
                }
            }
        }
        let name = crate::extensions::manifest_of(&self.ext, cx).map(|m| m.name).unwrap_or_else(|| self.ext.clone());
        let size = self.size.clone();
        let (tiles, ext, view) = (self.tiles.clone(), self.ext.clone(), self.tile.clone());
        let probe = canvas(
            move |b, _, cx| {
                if size.get() != Some(b.size) {
                    size.set(Some(b.size));
                    tiles.read(cx).send(&ext, Event::Resize(ui::ViewSize { view: view.clone(), size: ui::Size { width: f32::from(b.size.width), height: f32::from(b.size.height) } }));
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let root = div()
            .id(SharedString::from(format!("tile-view-{}-{}", self.ext, self.tile)))
            .debug_selector(|| "tile-view".into())
            .key_context({
                // `Tile` with `tile=<id>` (ui.md §3.5's `tile:<id>`).
                let mut k = gpui::KeyContext::default();
                k.add("Tile");
                k.set("tile", self.ext.clone());
                k
            })
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| {
                if !this.focus.contains_focused(window, cx) {
                    window.focus(&this.focus, cx);
                }
            }))
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .font_family(kit::UI_FONT)
            .text_size(px(render::text_px(ui::TextSize::Md)))
            .text_color(t.text)
            .child(probe);
        if let Some(s) = self.status(&name, &t, cx) {
            return root.child(s).into_any_element();
        }
        let body = self.build(0, Inherit::root(&t), window, &t, cx);
        let menu = self.menu_overlay(window, &t, cx);
        let root = root.child(div().flex_1().min_h_0().min_w_0().flex().flex_col().child(body));
        // What an agent showed in it says so (agent-tools.md §11.6): its
        // name, a press away from its terminal.
        let origin = self.tiles.read(cx).origin(&self.ext, &self.tile).cloned();
        let root = match origin {
            Some((id, agent)) => root.child(self.origin_chip(&id, &agent, &t, cx)),
            None => root,
        };
        root.children(menu).into_any_element()
    }
}
