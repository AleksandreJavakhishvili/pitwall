//! The floating "needs you" control (Settings → General, `general.floatingPrompts`).
//!
//! While Pitwall isn't in front and an agent's screen asks the user
//! something, a small window floats above the other apps, on every desktop
//! and over full-screen apps (a GPUI pop-up: on macOS a panel that takes
//! clicks without bringing Pitwall forward).
//!
//! It starts as a pill: the agent that has waited longest (its logo, its
//! name, and its project when two share a name), one plain line of what it
//! wants ("Run: pnpm test", "Edit src/cart/total.ts", the question), and
//! "+N" when others wait. When the prompt is a plain yes/no Pitwall has the
//! keys for and the line names all of what it asks, the pill has
//! **Reject** and **Accept**; otherwise **Show** (it opens the card) or,
//! for anything more than a yes or no, **Open** (Pitwall comes forward on
//! the agent). The pill never takes the keyboard.
//!
//! A click opens one card at a time, oldest first, with "1 of 3" and
//! arrows to the others. A card shows what the prompt asks, as lines of
//! the prompt itself (never a paraphrase, and nothing when they can't be
//! read off it reliably), over the part of the agent's terminal that holds
//! the prompt, drawn live as the Wall draws a tile (its text, as the rules
//! cut it out of the screen, until the screen arrives and for another
//! machine's agent): a few rows under a summary, all of it with "Show all",
//! and
//!
//! - **Accept** / **Reject** for a plain yes/no (`pitwall-detect`'s
//!   `[[answers]]`, per agent kind). The engine reads the screen again and
//!   types only if the same prompt is still there (`engine::input::answer`);
//!   if it changed: "The prompt changed: nothing was sent." **Accept all**
//!   in the header, only when every prompt that waits is such a yes/no;
//! - the keyboard: ⏎ Accept and esc Reject on a yes/no (the same re-check),
//!   ←/→ between the cards, esc closes a card that is more than a yes or
//!   no, and other keys go to the agent as they would in its pane (the
//!   panel takes the keyboard; Pitwall still doesn't come forward);
//! - **Open in Pitwall** for everything else (a question with options, a
//!   trust dialog, another machine's agent).
//!
//! After an answer, "Sent to <agent>" for about a second, then the next
//! prompt that waits.
//!
//! The open questions in the inboxes of the extensions that are on (and
//! running) are cards too, after the agents' (docs/spec/extensions.md, "In
//! the floating control"): the extension and connection, who asks and in
//! which thread, the question, its suggestions (a click sends one, or
//! toggles it when it takes several), a field for an answer of the user's
//! own, Send, Decline and Open in inbox. They are the inbox's own data and
//! are answered as the inbox tile answers ([`crate::main_screen::inbox_tile::
//! answer_ask`]): the hub is the one place either reads, so an answer in
//! one shows in the other. The control asks an extension for the threads
//! with open questions (the first [`model::ASK_THREADS`] of each) only
//! while it could show them; without such an extension it is as before.
//!
//! It can be dragged anywhere (by the pill, or by any part of the open
//! panel that isn't a button) and stays there, also the next time Pitwall
//! starts (`app-float.json` in the data folder). The open panel is resized
//! by any of its edges and corners (a double click on one: the default
//! size); the header's button takes it back to the pill, and a click on
//! the pill opens it again at the size it had. The size is kept with the
//! place. It slides in once when it appears (not with Reduce motion). It
//! goes away when nothing asks, when Pitwall comes forward, and when the
//! setting is off. Nothing here polls while nothing asks: the check for
//! "is Pitwall in front" runs only while something does.

mod demo;
pub mod model;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;

use gpui::{
    canvas, div, point, prelude::*, px, size, Animation, AnimationExt, App, AsyncApp, Bounds, Context, CursorStyle,
    DispatchPhase, Div, ElementId, Entity, FocusHandle, FontWeight, Global, IntoElement, KeyDownEvent, MouseButton,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ScrollEvent, SharedString, Task, Window, WindowBackgroundAppearance,
    WindowBounds, WindowHandle, WindowKind, WindowOptions,
};

use crate::agents::AgentStore;
use crate::extensions::hub::{self, Run};
use crate::kit::{self, InputEvent, TextInput};
use crate::platform::decorations::no_drag;
use crate::platform::FloatCursor;
use crate::theme::{self, Theme};
use crate::wall::paint::{screen_view, term_colors, ScreenOpts};
use crate::wall::screen::Screen;
use crate::wall::source::{forward, FrameMsg, ScreenSource, Screens};
use model::{
    AskItem, Item, Preset, Shape, TermRows, ANSWER_H, BOX_PAD, BUTTONS_H, CARD_HEAD, CHIP_H, GAP, HEADER_H, HINT_H, LINE_H,
    NOTE_H, OPTION_H, PAD, PILL_H, PILL_W, QUESTION_LINE, RULE, TERM_FONT,
};

/// The setting that turns the control on and off.
pub const KEY: &str = "general.floatingPrompts";

/// How often "is Pitwall in front" is looked at while an agent is blocked.
const WATCH: Duration = Duration::from_millis(400);
/// The gap between the control's window (which has [`PAD`] of clear space
/// inside) and the screen's edges, where it first shows.
const MARGIN: f32 = 4.;
/// The pointer has to travel this far before a press becomes a drag.
const DRAG_THRESHOLD: f32 = 3.;
/// The corner that stays put when the control changes size: the bottom
/// right on macOS (it opens upward, clear of notification banners), the
/// top right elsewhere (a window there is resized from its top).
const GROWS_UP: bool = cfg!(target_os = "macos");
/// The width follows a resize drag as it goes. Not where a change of
/// width opens the window again (see [`sync`]): there it is given when
/// the button is released.
const LIVE_WIDTH: bool = GROWS_UP;
/// The strips along the open panel's edges that resize it: this wide,
/// half over the panel and half in the clear space around it; the corner
/// between them is a square this large.
const EDGE: f32 = 6.;
const CORNER: f32 = 14.;

thread_local! {
    /// Where a left press on a drag handle went down.
    static PRESS: Cell<Option<Point<Pixels>>> = const { Cell::new(None) };
    /// That press became a drag: its release is not a click.
    static DRAGGED: Cell<bool> = const { Cell::new(false) };
}

/// Types the answer into an agent: (agent id, accept, the prompt as seen).
pub type Answerer = Arc<dyn Fn(&str, bool, &str) -> Result<(), String>>;
/// Types keys into an agent's terminal: (agent id, the bytes).
pub type Typer = Arc<dyn Fn(&str, &[u8]) -> Result<(), String>>;
/// Brings Pitwall forward on an agent.
pub type Opener = Arc<dyn Fn(&str, &mut App)>;
/// Brings Pitwall forward on an extension's inbox at a thread: (extension,
/// thread).
pub type InboxOpener = Arc<dyn Fn(&str, &str, &mut App)>;

/// The control's state (one per app).
pub struct Float {
    store: Entity<AgentStore>,
    answer: Answerer,
    typer: Typer,
    open: Opener,
    open_inbox: InboxOpener,
    window: Option<WindowHandle<FloatView>>,
    /// The open panel has the keyboard: keys go to the card shown (not
    /// while a question's field has them).
    keys: bool,
    /// The panel is open (else the pill alone).
    expanded: bool,
    /// The card shown: an agent's id or a question's key (`None`, or one
    /// that is gone: the first).
    current: Option<String>,
    /// When each agent began to ask, as a number (lower is earlier), and
    /// the next number: the cards go oldest first.
    since: HashMap<String, u64>,
    next_since: u64,
    /// The cards show every row of the terminal under a summary.
    show_all: bool,
    /// Just answered, said on its card for a moment: (agent, its name).
    confirm: Option<(String, String)>,
    /// How many times it came up: its first appearance slides in.
    shown_count: u64,
    /// It is up (or would be, in a bench instance without its preview).
    up: bool,
    /// Answers that couldn't be given: agent → why.
    errors: HashMap<String, SharedString>,
    /// Answered, waiting for the agent to move on: agent → the prompt.
    answered: HashMap<String, String>,
    /// The size the window was last given.
    size: (f32, f32),
    /// Where the user dragged it: the corner that stays put (see
    /// [`GROWS_UP`]), in screen coordinates. `None`: the screen's corner.
    anchor: Option<Point<Pixels>>,
    /// The open panel's width and rows when the user resized it (else the
    /// defaults, [`model::WIDTH`] and [`model::LINES_ALONE`]).
    width: Option<f32>,
    rows: Option<usize>,
    /// A resize by the pointer under way.
    resize: Option<Resize>,
    /// Where `anchor` and the size are kept between runs, and what was
    /// last written.
    file: Option<PathBuf>,
    saved: Place,
    /// Where the prompt is on each card's terminal (cards that have their
    /// agent's screen; the view says).
    term_rows: HashMap<String, TermRows>,
    /// What the pointer is over that takes a click or resizes, and the
    /// pointer it shows there.
    hot: HashMap<SharedString, FloatCursor>,
    /// Looks at who is in front while an agent is blocked.
    watch: Option<Task<()>>,
    /// Opening a window failed (a platform without pop-ups): don't retry.
    unsupported: bool,
    /// The cards were just opened: the window takes the keyboard when it
    /// next draws.
    grab: bool,
    /// Questions' cards: the height each was drawn at, for the width it
    /// had (until then [`model::ask_height`]).
    ask_heights: HashMap<String, (f32, f32)>,
    /// The suggestions picked so far of a question that takes several.
    picks: HashMap<String, Vec<u32>>,
    /// Questions answered here: the request, until their thread says they
    /// are answered (or it failed: the thread's note says why).
    sent: HashMap<String, u64>,
    /// The threads it asked their extension for: (extension, thread).
    opened: HashSet<(String, String)>,
    /// How far a resize by the edges it is anchored on moved that corner
    /// (right and down positive), not yet given to the window.
    shift: (f32, f32),
    /// A bench instance shows it (`float preview`): its view drawn in the
    /// main window ([`bench_preview`]), never in a window of its own.
    bench_shown: bool,
    /// That view, made in the main window on its first draw.
    preview: Option<Entity<FloatView>>,
    /// A bench instance's made-up agents (`float demo`), shown instead of
    /// the store's, and their screens.
    demo: Option<Vec<pitwall_proto::AgentView>>,
    demo_screens: HashMap<String, pitwall_proto::ScreenFrame>,
}

impl Global for Float {}

/// Which edge or corner of the open panel is being dragged: `x` is -1 for
/// its left, 1 for its right, 0 for neither; `y` -1 for its top, 1 for its
/// bottom. The window stays put at its right and (see [`GROWS_UP`]) its
/// bottom on macOS, its top elsewhere: dragging the edges it is anchored
/// on moves that corner with them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Edge {
    x: i8,
    y: i8,
}

impl Edge {
    const LEFT: Edge = Edge { x: -1, y: 0 };
    const RIGHT: Edge = Edge { x: 1, y: 0 };
    const TOP: Edge = Edge { x: 0, y: -1 };
    const BOTTOM: Edge = Edge { x: 0, y: 1 };
    const ALL: [Edge; 8] = [
        Edge::LEFT,
        Edge::RIGHT,
        Edge::TOP,
        Edge::BOTTOM,
        Edge { x: -1, y: -1 },
        Edge { x: 1, y: -1 },
        Edge { x: -1, y: 1 },
        Edge { x: 1, y: 1 },
    ];
    /// The end it is anchored at: its bottom on macOS, its top elsewhere.
    const ANCHORED_Y: i8 = if GROWS_UP { 1 } else { -1 };
    /// The edge across from it: what made it taller before every edge did.
    const FREE_END: Edge = Edge { x: 0, y: -Edge::ANCHORED_Y };
    /// The corner across from where it is anchored.
    const FREE_CORNER: Edge = Edge { x: -1, y: -Edge::ANCHORED_Y };

    fn resizes_width(self) -> bool {
        self.x != 0
    }

    fn resizes_rows(self) -> bool {
        self.y != 0
    }

    /// Dragging it moves the corner the window is anchored at.
    fn moves_anchor(self) -> bool {
        self.x == 1 || self.y == Edge::ANCHORED_Y
    }

    fn cursor(self) -> FloatCursor {
        match (self.x, self.y) {
            (_, 0) => FloatCursor::ResizeLeftRight,
            (0, _) => FloatCursor::ResizeUpDown,
            // Top left to bottom right, or bottom left to top right.
            (x, y) if x == y => FloatCursor::ResizeDiagonal,
            _ => FloatCursor::ResizeDiagonalUp,
        }
    }

    /// GPUI's pointer for it (where GPUI's pointers apply: not macOS).
    fn style(self) -> CursorStyle {
        match (self.x, self.y) {
            (_, 0) => CursorStyle::ResizeLeftRight,
            (0, _) => CursorStyle::ResizeUpDown,
            (x, y) if x == y => CursorStyle::ResizeUpLeftDownRight,
            _ => CursorStyle::ResizeUpRightDownLeft,
        }
    }

    /// Its strip's name (tests and the debug hook find it by this).
    fn name(self) -> &'static str {
        match (self.x, self.y) {
            (-1, 0) => "float-resize-left",
            (1, 0) => "float-resize-right",
            (0, -1) => "float-resize-top",
            (0, 1) => "float-resize-bottom",
            (-1, -1) => "float-resize-top-left",
            (1, -1) => "float-resize-top-right",
            (-1, 1) => "float-resize-bottom-left",
            _ => "float-resize-bottom-right",
        }
    }
}

/// A resize by the pointer: what was dragged, and the size when it began.
#[derive(Debug, Clone, Copy)]
struct Resize {
    edge: Edge,
    width: f32,
    /// The rows a card shows in full.
    rows: usize,
    /// The corner the window is anchored at, and what is drawn's height.
    anchor: Point<Pixels>,
    height: f32,
    /// Where the press was on the screen, when the window's own mouse
    /// events are followed (not macOS: see [`crate::platform::float_resize`]).
    from: Option<Point<Pixels>>,
    /// What waits for the release (see [`LIVE_WIDTH`]).
    pending: Option<Pending>,
}

/// A size and place that wait for a resize's release.
#[derive(Debug, Clone, Copy, Default)]
struct Pending {
    width: Option<f32>,
    rows: Option<usize>,
    anchor: Option<Point<Pixels>>,
}

/// What `app-float.json` keeps: where the control was dragged to, and the
/// size the user gave it. A file from before it could be resized has the
/// place alone.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Place {
    anchor: Option<Point<Pixels>>,
    width: Option<f32>,
    rows: Option<usize>,
}

/// Set the control up for `store`'s agents and the extensions' inboxes.
/// `answer` types into an agent (the engine) if it still asks the same,
/// `typer` types keys into its terminal, `open` shows one in Pitwall,
/// `open_inbox` an inbox at a thread.
pub fn init(store: &Entity<AgentStore>, answer: Answerer, typer: Typer, open: Opener, open_inbox: InboxOpener, cx: &mut App) {
    let file = cx.try_global::<crate::AppState>().and_then(|s| s.root.as_deref().map(place_file));
    let place = file.as_deref().and_then(load_place).unwrap_or_default();
    // A spot on a display that is still there.
    let displays: Vec<_> = cx.displays().iter().map(|d| d.bounds()).collect();
    let anchor = place.anchor.filter(|a| displays.iter().any(|d| d.contains(a)));
    let width = place.width.map(|w| w.clamp(model::MIN_WIDTH, model::MAX_WIDTH));
    let rows = place.rows.map(|r| r.clamp(model::MIN_ROWS, model::MAX_ROWS));
    cx.set_global(Float {
        store: store.clone(),
        answer,
        typer,
        open,
        open_inbox,
        window: None,
        keys: false,
        expanded: false,
        current: None,
        since: HashMap::new(),
        next_since: 0,
        show_all: false,
        confirm: None,
        shown_count: 0,
        up: false,
        errors: HashMap::new(),
        answered: HashMap::new(),
        size: (0., 0.),
        anchor,
        width,
        rows,
        resize: None,
        file,
        saved: Place { anchor, width, rows },
        term_rows: HashMap::new(),
        hot: HashMap::new(),
        watch: None,
        unsupported: false,
        grab: false,
        ask_heights: HashMap::new(),
        picks: HashMap::new(),
        sent: HashMap::new(),
        opened: HashSet::new(),
        shift: (0., 0.),
        bench_shown: false,
        preview: None,
        demo: None,
        demo_screens: HashMap::new(),
    });
    cx.observe(store, |_, cx| sync(cx)).detach();
    let ui = crate::ui_state::store(cx);
    cx.observe(&ui, |_, cx| sync(cx)).detach();
    // The extensions' inboxes (made at start, before this).
    if let Some(h) = hub::try_hub(cx) {
        cx.observe(&h, |_, cx| sync(cx)).detach();
    }
    sync(cx);
}

fn place_file(root: &Path) -> PathBuf {
    root.join("app-float.json")
}

fn load_place(file: &Path) -> Option<Place> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    let num = |k: &str| v.get(k).and_then(|n| n.as_f64());
    Some(Place {
        anchor: num("x").zip(num("y")).map(|(x, y)| point(px(x as f32), px(y as f32))),
        width: num("width").map(|w| w as f32),
        rows: v.get("rows").and_then(|r| r.as_u64()).map(|r| r as usize),
    })
}

/// Keep where the control was dragged to and the size it was given (when
/// that changed).
fn save_place(cx: &mut App) {
    let f = cx.global_mut::<Float>();
    let now = Place { anchor: f.anchor, width: f.width, rows: f.rows };
    let Some(file) = f.file.clone() else { return };
    if f.saved == now {
        return;
    }
    f.saved = now;
    let mut v = serde_json::Map::new();
    if let Some(a) = now.anchor {
        v.insert("x".into(), f32::from(a.x).into());
        v.insert("y".into(), f32::from(a.y).into());
    }
    if let Some(w) = now.width {
        v.insert("width".into(), w.into());
    }
    if let Some(r) = now.rows {
        v.insert("rows".into(), r.into());
    }
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, serde_json::Value::Object(v).to_string());
}

/// The size the user gave the open panel (else the defaults).
fn shape(f: &Float) -> Shape {
    Shape { width: f.width.unwrap_or(model::WIDTH), rows: f.rows }
}

/// The usable area of the screen the control is on, and its corner that
/// stays put (see [`GROWS_UP`]) there.
fn screen_area(cx: &App) -> Option<(Point<Pixels>, Bounds<Pixels>)> {
    let anchor = cx.global::<Float>().anchor;
    let display = match anchor {
        // (Off every display: a bench instance's, which has no bounds.)
        Some(a) => cx.displays().into_iter().find(|d| d.bounds().contains(&a))?,
        None => cx.primary_display()?,
    };
    let area = display.visible_bounds();
    let corner = anchor.unwrap_or_else(|| {
        let y = if GROWS_UP { area.origin.y + area.size.height - px(MARGIN) } else { area.origin.y + px(MARGIN) };
        point(area.origin.x + area.size.width - px(MARGIN), y)
    });
    Some((corner, area))
}

/// A question's card's height: as drawn at `width`, else as
/// [`model::ask_height`] reckons.
fn ask_card_height(f: &Float, a: &AskItem, width: f32) -> f32 {
    match f.ask_heights.get(&a.key()) {
        Some((w, h)) if (w - width).abs() < 0.5 => *h,
        _ => model::ask_height(a, width, ask_note(f, a).is_some()),
    }
}

/// An agent's card is answered: sent, waiting for it to move on.
fn sent(f: &Float, id: &str) -> bool {
    f.answered.contains_key(id)
}

/// How `item`'s card looks in shape `s`.
fn look(item: &Item, s: Shape, f: &Float) -> model::Look {
    let note = model::has_note(item, f.errors.contains_key(&item.id), sent(f, &item.id));
    model::look(item, s, f.term_rows.get(&item.id).copied(), f.show_all, note)
}

/// The height of the card shown, in shape `s` (`None`: nothing waits).
fn card_height(list: &[Item], asks: &[AskItem], s: Shape, f: &Float) -> Option<f32> {
    let entries = model::entries(list, asks);
    let at = model::position(&entries, f.current.as_deref());
    Some(match entries.get(at)? {
        model::Entry::Agent(i) => model::card_height(look(i, s, f)),
        model::Entry::Ask(a) => ask_card_height(f, a, s.width),
    })
}

/// What is drawn's height with the panel open in shape `s`.
fn content_height(list: &[Item], asks: &[AskItem], s: Shape, f: &Float) -> f32 {
    model::content_size(Some(card_height(list, asks, s, f).unwrap_or(0.)), s).1
}

/// Where the window can grow to: the panel's widest and tallest that fit
/// between the screen's edges and the window's edges that stay put while
/// `edge` is dragged (`None`: not known).
fn room(edge: Edge, anchor: Point<Pixels>, width: f32, height: f32, cx: &App) -> Option<(f32, f32)> {
    let (_, area) = screen_area(cx)?;
    let (left, right) = (f32::from(area.origin.x), f32::from(area.origin.x + area.size.width));
    let (top, bottom) = (f32::from(area.origin.y), f32::from(area.origin.y + area.size.height));
    let (ax, ay) = (f32::from(anchor.x), f32::from(anchor.y));
    // The right edge moves: the window's left stays.
    let w = if edge.x == 1 { right - (ax - width - 2. * PAD) } else { ax - left };
    let h = match (edge.y == Edge::ANCHORED_Y, GROWS_UP) {
        // The bottom moves (macOS): its top stays.
        (true, true) => bottom - (ay - height - 2. * PAD),
        // The top moves (elsewhere): its bottom stays.
        (true, false) => (ay + height + 2. * PAD) - top,
        (false, true) => ay - top,
        (false, false) => bottom - ay,
    };
    Some((w - 2. * PAD, h - 2. * PAD))
}

/// Start resizing the open panel by `edge`.
fn begin_resize(edge: Edge, cx: &mut App) {
    let (list, asks) = (items(cx), asks(cx));
    let corner = screen_area(cx).map(|(c, _)| c);
    let f = cx.global::<Float>();
    let count = list.len() + asks.len();
    if !f.expanded || count == 0 {
        return;
    }
    // Taller shows more of the terminal: all of it, not a few rows under
    // the summary.
    if edge.resizes_rows() {
        cx.global_mut::<Float>().show_all = true;
    }
    let f = cx.global::<Float>();
    let s = shape(f);
    let height = content_height(&list, &asks, s, f);
    let anchor = f.anchor.or(corner).unwrap_or_default();
    cx.global_mut::<Float>().resize = Some(Resize {
        edge,
        width: s.width,
        rows: s.max_lines(),
        anchor,
        height,
        from: None,
        pending: None,
    });
    log(|| format!("resize by {edge:?} starts at {s:?}"));
    crate::platform::float_cursor(edge.cursor());
}

/// The pointer is `dx`, `dy` points (right and down are positive) from
/// where the resize began: the panel takes the size that puts its edge
/// under it. Its width moves with the pointer; its height in whole rows,
/// a row per [`LINE_H`] shared by the agents' cards shown (questions'
/// cards are as tall as what they hold). Kept within the bounds of
/// [`model`] and so that the window fits on the screen. Dragging the edges
/// the window is anchored on (its right; its bottom on macOS, its top
/// elsewhere) moves that corner by as much as the size changed.
fn resize_to(dx: f32, dy: f32, cx: &mut App) {
    let Some(r) = cx.global::<Float>().resize else { return };
    let room = room(r.edge, r.anchor, r.width, r.height, cx);
    let (list, asks) = (items(cx), asks(cx));
    let f = cx.global::<Float>();
    let (mut width, mut rows) = (f.width, f.rows);
    if r.edge.resizes_width() {
        // Its left edge wider to the left, its right edge to the right.
        let want = r.width + dx * r.edge.x as f32;
        // No wider than the screen has room for; without room, a drag
        // outward keeps the width it had, never narrower.
        let most = room.map_or(model::MAX_WIDTH, |(w, _)| w.min(model::MAX_WIDTH)).max(r.width.min(want));
        let w = want.min(most).max(model::MIN_WIDTH).round();
        if w != r.width || width.is_some() {
            width = Some(w);
        }
    }
    if r.edge.resizes_rows() {
        // Its top edge taller upward, its bottom edge downward.
        let grow = dy * r.edge.y as f32;
        let by = (grow / LINE_H).round() as isize;
        if by != 0 || rows.is_some() {
            let mut alone = (r.rows as isize + by).clamp(model::MIN_ROWS as isize, model::MAX_ROWS as isize) as usize;
            // No taller than the screen has room for; without room (the
            // panel already reaches past that edge), a drag outward keeps
            // the rows it had, never fewer.
            let had = r.rows.clamp(model::MIN_ROWS, model::MAX_ROWS);
            let least = had.min(alone).max(model::MIN_ROWS);
            let fits = |alone: usize| {
                let s = Shape { width: width.unwrap_or(model::WIDTH), rows: Some(alone) };
                room.is_none_or(|(_, room)| content_height(&list, &asks, s, f) <= room)
            };
            while alone > least && !fits(alone) {
                alone -= 1;
            }
            rows = Some(alone);
        }
    }
    // The anchored corner follows the edges dragged that it is on.
    let s = Shape { width: width.unwrap_or(model::WIDTH), rows };
    let mut anchor = r.anchor;
    if r.edge.x == 1 {
        anchor.x += px(s.width - r.width);
    }
    if r.edge.y == Edge::ANCHORED_Y {
        let grew = content_height(&list, &asks, s, f) - r.height;
        anchor.y += px(grew * Edge::ANCHORED_Y as f32);
    }
    let moved = r.edge.moves_anchor().then_some(anchor);
    let f = cx.global_mut::<Float>();
    // Where the window is opened again for a new width or place (not
    // macOS), all of it waits for the release; a height of the free end
    // follows the pointer.
    let live = LIVE_WIDTH || !r.edge.moves_anchor();
    if live {
        if LIVE_WIDTH || !r.edge.resizes_width() {
            f.width = width;
        } else if let Some(p) = f.resize.as_mut() {
            p.pending = Some(Pending { width, ..Default::default() });
        }
        f.rows = rows;
        if let Some(a) = moved {
            let was = f.anchor.unwrap_or(r.anchor);
            f.shift.0 += f32::from(a.x - was.x);
            f.shift.1 += f32::from(a.y - was.y);
            f.anchor = Some(a);
        }
    } else if let Some(p) = f.resize.as_mut() {
        p.pending = Some(Pending { width, rows, anchor: moved });
    }
    sync_later(cx);
}

/// The resize is over: the size is kept, and the pointer shows what it is
/// over again.
fn end_resize(cx: &mut App) {
    let f = cx.global_mut::<Float>();
    let Some(r) = f.resize.take() else { return };
    if let Some(p) = r.pending {
        f.width = p.width.or(f.width);
        f.rows = p.rows.or(f.rows);
        f.anchor = p.anchor.or(f.anchor);
    }
    log(|| format!("resize ends at {:?}", shape(f)));
    crate::platform::float_cursor(cursor_now(f));
    save_place(cx);
    sync_later(cx);
}

/// Back to the default size (a double click on an edge).
fn reset_size(cx: &mut App) {
    set_preset(Preset::Medium, cx);
}

/// One of the header's sizes: the panel takes it (no wider or taller than
/// the screen has room for), growing from the corner it is anchored at.
fn set_preset(p: Preset, cx: &mut App) {
    let (width, rows) = p.size();
    let (list, asks) = (items(cx), asks(cx));
    let anchor = screen_area(cx).map(|(c, _)| c);
    let f = cx.global::<Float>();
    let (mut w, mut r) = (width, rows);
    if let Some(room) = anchor.and_then(|a| room(Edge::FREE_CORNER, a, shape(f).width, 0., cx)) {
        w = w.map(|w| w.min(room.0.max(model::MIN_WIDTH)));
        while let Some(n) = r.filter(|n| *n > model::MIN_ROWS) {
            let s = Shape { width: w.unwrap_or(model::WIDTH), rows: Some(n) };
            if content_height(&list, &asks, s, f) <= room.1 {
                break;
            }
            r = Some(n - 1);
        }
    }
    let f = cx.global_mut::<Float>();
    f.resize = None;
    f.width = w;
    f.rows = r;
    log(|| format!("size {}", p.name()));
    save_place(cx);
    sync_later(cx);
}

/// A press on `edge`: the resize follows the pointer until the release
/// (by the platform where it can, else by the window's own mouse events,
/// [`FloatView::render`]). A double press resets the size.
fn press_edge(edge: Edge, ev: &gpui::MouseDownEvent, window: &mut Window, cx: &mut App) {
    cx.stop_propagation();
    if ev.click_count >= 2 {
        reset_size(cx);
        return;
    }
    begin_resize(edge, cx);
    if cx.global::<Float>().resize.is_none() {
        return;
    }
    let tracked = crate::platform::float_resize(window, cx, resize_to, end_resize);
    if !tracked {
        let at = window.bounds().origin + ev.position;
        if let Some(r) = cx.global_mut::<Float>().resize.as_mut() {
            r.from = Some(at);
        }
    }
}

/// `PITWALL_FLOAT_LOG=<file>`: what the control does and why, a line
/// each (for finding out why a click or a drag did nothing).
fn log(what: impl FnOnce() -> String) {
    use std::io::Write;
    static FILE: std::sync::OnceLock<Option<std::sync::Mutex<std::fs::File>>> = std::sync::OnceLock::new();
    let file = FILE.get_or_init(|| {
        let path = std::env::var_os("PITWALL_FLOAT_LOG")?;
        std::fs::OpenOptions::new().create(true).append(true).open(path).ok().map(std::sync::Mutex::new)
    });
    if let Some(Ok(mut f)) = file.as_ref().map(|f| f.lock()) {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let _ = writeln!(f, "{ms} {}", what());
    }
}

/// The setting (on unless turned off).
pub fn enabled(cx: &App) -> bool {
    pitwall_proto::settings::find(KEY)
        .and_then(|def| crate::settings::data::value(&crate::ui_state::get(cx), def))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn items(cx: &App) -> Vec<Item> {
    let Some(f) = cx.try_global::<Float>() else { return Vec::new() };
    let list = match &f.demo {
        Some(agents) => model::items(agents),
        None => model::items(&f.store.read(cx).agents),
    };
    model::oldest_first(list, &f.since)
}

/// The extensions that are on, have an inbox and whose adapter runs, with
/// their state.
fn inboxes(cx: &App) -> Vec<(pitwall_core::extensions::Manifest, &hub::ExtState)> {
    let Some(h) = hub::try_hub(cx) else { return Vec::new() };
    let h = h.read(cx);
    crate::extensions::inboxes(cx).into_iter().filter_map(|m| h.get(&m.id).filter(|st| st.run == Run::Running).map(|st| (m, st))).collect()
}

/// The open questions in those inboxes whose threads have arrived.
fn asks(cx: &App) -> Vec<AskItem> {
    let Some(f) = cx.try_global::<Float>() else { return Vec::new() };
    let agents = &f.store.read(cx).agents;
    let mut out = Vec::new();
    for (m, st) in inboxes(cx) {
        let conn = |id: &str| st.label(id).map(str::to_string);
        let from = |th: &pitwall_core::extensions::protocol::Thread| {
            let fr = th.from.as_ref()?;
            Some(crate::extensions::agent_of(&m, agents, &th.id, fr, cx).map_or_else(|| fr.agent.clone(), |a| a.name.clone()))
        };
        let icon = crate::extensions::icon_of(&m).to_string();
        out.extend(model::asks(&[model::Inbox {
            ext: &m.id,
            name: &m.name,
            icon: &icon,
            threads: &st.threads,
            details: &st.details,
            busy: &st.busy,
            notes: &st.notes,
            conn: &conn,
            from: &from,
        }]));
    }
    // Tiles' questions, after the inboxes' (platform-services.md §14.2).
    out.extend(crate::tiles::group::float_asks(agents, cx));
    // Blocking ones first across the inboxes too (stable).
    out.sort_by_key(|a| !a.ask.blocking);
    out
}

/// The threads whose questions the control would show: (extension,
/// thread).
fn ask_threads(cx: &App) -> HashSet<(String, String)> {
    inboxes(cx).into_iter().flat_map(|(m, st)| model::ask_threads(&st.threads).into_iter().map(move |t| (m.id.clone(), t.id))).collect()
}

/// It can show here: not in a bench instance, unless one asked for it.
fn shows_here(cx: &App) -> bool {
    !crate::bench::enabled() || cx.try_global::<Float>().is_some_and(|f| f.bench_shown)
}

/// Whether the control should be up: on, something asks, and the user
/// isn't in Pitwall.
fn wanted(items: &[Item], asks: &[AskItem], cx: &App) -> bool {
    !(items.is_empty() && asks.is_empty()) && enabled(cx) && shows_here(cx) && !in_front(cx)
}

/// What a question's card says under its buttons: that the answer is on
/// its way, or why it didn't go (an error).
fn ask_note(f: &Float, a: &AskItem) -> Option<(SharedString, bool)> {
    if f.sent.contains_key(&a.key()) || a.busy {
        return Some(("Sending…".into(), false));
    }
    a.note.as_ref().map(|n| (format!("Not sent: {n}").into(), true))
}

/// Tests say whether the user is in Pitwall.
#[cfg(test)]
pub(crate) struct InFront(pub bool);
#[cfg(test)]
impl Global for InFront {}

/// The control's window, when it is up (tests).
#[cfg(test)]
pub(crate) fn window_of(cx: &App) -> Option<gpui::AnyWindowHandle> {
    cx.try_global::<Float>().and_then(|f| f.window).map(Into::into)
}

/// The control drawn in a window of its own at the size it asks for, as
/// a bench instance's preview is (screenshots: a test window keeps the
/// size it was opened at).
#[cfg(test)]
pub(crate) struct Preview(Entity<FloatView>);

#[cfg(test)]
impl Preview {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Preview {
        let v = cx.new(|cx| {
            let mut v = FloatView::new(window, cx);
            v.preview = true;
            v
        });
        Preview(v)
    }
}

#[cfg(test)]
impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (w, h) = cx.global::<Float>().size;
        div().size_full().bg(cx.global::<Theme>().bg).child(div().w(px(w)).h(px(h)).child(self.0.clone()))
    }
}

fn in_front(cx: &App) -> bool {
    #[cfg(test)]
    if let Some(f) = cx.try_global::<InFront>() {
        return f.0;
    }
    crate::platform::app_in_front(cx)
}

/// The pointer left everything that takes a click (the window closed or
/// was laid out again under it).
/// (While a resize goes on, the pointer keeps showing it.)
fn cool(cx: &mut App) {
    let f = cx.global_mut::<Float>();
    if !std::mem::take(&mut f.hot).is_empty() || f.resize.is_some() {
        crate::platform::float_cursor(cursor_now(f));
    }
}

/// The pointer the control shows now: the resize's while one goes on,
/// else what the pointer is over asks for (a resize edge before a button).
fn cursor_now(f: &Float) -> FloatCursor {
    if let Some(r) = f.resize {
        return r.edge.cursor();
    }
    let resize = f.hot.values().find(|c| !matches!(c, FloatCursor::Pointing | FloatCursor::Arrow));
    resize.copied().unwrap_or(if f.hot.is_empty() { FloatCursor::Arrow } else { FloatCursor::Pointing })
}

/// Bring the window in line with the agents, the setting and who is in
/// front: open, resize, redraw or close it.
pub fn sync(cx: &mut App) {
    if !cx.has_global::<Float>() {
        return;
    }
    let list = items(cx);
    let questions = asks(cx);
    let want = wanted(&list, &questions, cx);
    // The questions' threads are asked for while they could be shown.
    let threads = if enabled(cx) && shows_here(cx) && !in_front(cx) { ask_threads(cx) } else { HashSet::new() };
    follow_threads(threads, cx);
    // How the answers sent from here went.
    let failed: HashSet<String> = {
        let f = cx.global::<Float>();
        let h = hub::try_hub(cx);
        let result = |key: &str, req: u64| {
            let a = questions.iter().find(|a| a.key() == key)?;
            if a.tile.is_some() {
                return crate::tiles::group::ask_result(&a.ext, req, cx);
            }
            h.as_ref()?.read(cx).get(&a.ext)?.result(req)
        };
        f.sent.iter().filter(|(k, req)| result(k, **req) == Some(false)).map(|(k, _)| k.clone()).collect()
    };
    let f = cx.global_mut::<Float>();
    // What was answered or failed belongs to a prompt that is gone.
    let asks = |id: &str| list.iter().find(|i| i.id == id).map(|i| i.ask.text.as_str());
    f.answered.retain(|id, seen| asks(id) == Some(seen.as_str()));
    f.errors.retain(|id, _| asks(id).is_some());
    f.term_rows.retain(|id, _| asks(id).is_some());
    // And to a question that is gone (answered, here or in its inbox).
    let open: HashSet<String> = questions.iter().map(AskItem::key).collect();
    f.sent.retain(|k, _| open.contains(k) && !failed.contains(k));
    f.picks.retain(|k, _| open.contains(k));
    f.ask_heights.retain(|k, _| open.contains(k));
    // When each agent began to ask: the cards go oldest first.
    f.since.retain(|id, _| asks(id).is_some());
    for i in &list {
        if !f.since.contains_key(&i.id) {
            f.since.insert(i.id.clone(), f.next_since);
            f.next_since += 1;
        }
    }
    // The card shown went: the first that waits.
    if f.current.as_deref().is_some_and(|c| !list.iter().any(|i| i.id == c) && !open.contains(c)) {
        f.current = None;
    }
    if f.confirm.as_ref().is_some_and(|(id, _)| asks(id).is_none()) {
        f.confirm = None;
    }
    if !f.expanded {
        f.keys = false;
    }
    if list.is_empty() && questions.is_empty() {
        f.expanded = false;
        f.show_all = false;
    }
    // It comes up (again): its first appearance slides in.
    if want && !f.up {
        f.shown_count += 1;
    }
    f.up = want;
    // Nothing open to resize (the cards were closed under the pointer).
    if !f.expanded && f.resize.is_some() {
        f.resize = None;
        crate::platform::float_cursor(FloatCursor::Arrow);
    }
    // Watch who is in front only while it can matter.
    let watching = (!list.is_empty() || !questions.is_empty() || inboxes(cx).iter().any(|(_, st)| st.threads.iter().any(|t| t.open_asks > 0))) && enabled(cx);
    if !watching {
        cx.global_mut::<Float>().watch = None;
    } else if cx.global::<Float>().watch.is_none() {
        let watch = cx.spawn(async move |cx: &mut AsyncApp| loop {
            cx.background_executor().timer(WATCH).await;
            cx.update(sync);
        });
        cx.global_mut::<Float>().watch = Some(watch);
    }
    save_place(cx);
    let f = cx.global_mut::<Float>();
    if !want {
        if let Some(w) = f.window.take() {
            let _ = w.update(cx, |_, window, _| {
                crate::platform::glass::slab(window, None);
                window.remove_window()
            });
            cool(cx);
            log(|| {
                let (on, front, raw) = (enabled(cx), in_front(cx), crate::platform::app_active_raw());
                format!("closed: {} asking, {} questions, enabled {on}, pitwall in front {front} (system says active {raw})", list.len(), questions.len())
            });
        }
        cx.global_mut::<Float>().shift = (0., 0.);
        return;
    }
    let card = if f.expanded { card_height(&list, &questions, shape(f), f) } else { None };
    let new = model::size(card, shape(f));
    let old = std::mem::replace(&mut f.size, new);
    let shift = std::mem::take(&mut f.shift);
    if let Some(v) = f.preview.clone() {
        v.update(cx, |_, cx| cx.notify());
        return;
    }
    match f.window {
        // A window that isn't resized from the corner it sits on is opened
        // again in its new place when its width or that corner changes
        // (the mark ↔ the cards); a change of height alone keeps its top
        // where it is.
        Some(w) if (old.0 != new.0 || shift != (0., 0.)) && !GROWS_UP && !cfg!(test) => {
            f.window = None;
            let _ = w.update(cx, |_, window, _| {
                crate::platform::glass::slab(window, None);
                window.remove_window()
            });
            cool(cx);
            cx.defer(open_window);
        }
        Some(w) => {
            let alive = w.update(cx, |_, window, cx| {
                if old != new || shift != (0., 0.) {
                    crate::platform::resize_float(window, size(px(new.0), px(new.1)), shift, cx);
                }
                cx.notify();
            });
            if old != new {
                cool(cx);
            }
            // Closed behind our back: open it again on the next pass. (An
            // error also means "being updated right now": still open.)
            if alive.is_err() && !cx.windows().iter().any(|o| o.window_id() == w.window_id()) {
                cx.global_mut::<Float>().window = None;
            }
        }
        // Deferred: this may run while a window is being updated.
        None if !f.unsupported => cx.defer(open_window),
        None => {}
    }
}

/// Ask the extensions for exactly `threads` (kept current while asked
/// for), letting go of the others it asked for. An inbox tile showing one
/// of them keeps it (the hub counts who asks).
fn follow_threads(threads: HashSet<(String, String)>, cx: &mut App) {
    let Some(h) = hub::try_hub(cx) else { return };
    let f = cx.global_mut::<Float>();
    if f.opened == threads {
        return;
    }
    let gone: Vec<(String, String)> = f.opened.difference(&threads).cloned().collect();
    let new: Vec<(String, String)> = threads.difference(&f.opened).cloned().collect();
    f.opened = threads;
    h.update(cx, |h, cx| {
        for (ext, thread) in &gone {
            h.close(ext, thread);
        }
        for (ext, thread) in &new {
            h.open(ext, thread, cx);
        }
        cx.notify();
    });
}

/// Where a window of `size` sits: its fixed corner on the user's spot,
/// else in the primary display's corner.
fn bounds((w, h): (f32, f32), cx: &App) -> Option<Bounds<Pixels>> {
    let anchor = match cx.global::<Float>().anchor {
        Some(a) => a,
        None => {
            let area = cx.primary_display()?.visible_bounds();
            let y = if GROWS_UP { area.origin.y + area.size.height - px(MARGIN) } else { area.origin.y + px(MARGIN) };
            point(area.origin.x + area.size.width - px(MARGIN), y)
        }
    };
    let y = if GROWS_UP { anchor.y - px(h) } else { anchor.y };
    Some(Bounds::new(point(anchor.x - px(w), y), size(px(w), px(h))))
}

fn open_window(cx: &mut App) {
    if cx.try_global::<Float>().is_none_or(|f| f.window.is_some()) || !wanted(&items(cx), &asks(cx), cx) {
        return;
    }
    // A bench instance opens no window for it: its view is drawn in the
    // main window ([`bench_preview`]).
    if crate::bench::enabled() && !crate::bench::visible() {
        return;
    }
    let Some(bounds) = bounds(cx.global::<Float>().size, cx) else { return };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        // Never takes the keyboard from the app the user is in.
        focus: false,
        show: true,
        kind: WindowKind::PopUp,
        // Moved by our own drag (`drag_handle`): a movable window's top
        // strip is a title bar to AppKit, which then keeps its presses.
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(crate::platform::WINDOW_APP_ID.into()),
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| {
        crate::platform::lean_renderer(window);
        crate::platform::float_setup(window);
        cx.new(|cx| FloatView::new(window, cx))
    }) {
        Ok(w) => {
            cx.global_mut::<Float>().window = Some(w);
            log(|| format!("opened at {bounds:?}"));
        }
        Err(e) => {
            eprintln!("pitwall: no floating control on this system: {e}");
            cx.global_mut::<Float>().unsupported = true;
        }
    }
}

/// After a click in the control: bring its window in line once the click
/// is done (the window can't be resized while it handles an event).
fn sync_later(cx: &mut App) {
    cx.defer(sync);
}

/// How long "Sent to …" stays on a card before the next one that waits
/// is shown.
const CONFIRM: Duration = Duration::from_millis(1100);

/// Open or close the panel. Opened, it has the keyboard: Enter and Esc
/// answer the card shown, or go to its terminal (see [`key`]).
fn toggle(cx: &mut App) {
    let f = cx.global_mut::<Float>();
    f.expanded = !f.expanded;
    let open = f.expanded;
    f.keys = open;
    f.grab = open;
    log(|| format!("toggle: panel {}", if open { "open" } else { "closed" }));
    sync_later(cx);
}

/// Open the panel on `key`'s card (an agent's id or a question's key).
fn expand_on(key: &str, cx: &mut App) {
    cx.global_mut::<Float>().current = Some(key.to_string());
    if !cx.global::<Float>().expanded {
        toggle(cx);
    }
}

/// The card `by` steps from the one shown (around at either end).
fn go(by: isize, cx: &mut App) {
    let (list, questions) = (items(cx), asks(cx));
    let entries = model::entries(&list, &questions);
    let f = cx.global_mut::<Float>();
    let at = model::step(entries.len(), model::position(&entries, f.current.as_deref()), by);
    f.current = entries.get(at).map(|e| e.key());
    f.confirm = None;
    sync_later(cx);
}

/// Why an answer wasn't typed, in the words the card uses.
fn not_sent(e: &str) -> SharedString {
    if e.contains("prompt changed") {
        "The prompt changed: nothing was sent.".into()
    } else {
        format!("Nothing was sent: {e}.").into()
    }
}

/// Type the answer into `id`'s terminal, if it still shows the prompt the
/// user is looking at (the engine reads its screen again first).
fn answer(id: &str, accept: bool, cx: &mut App) {
    let list = items(cx);
    let Some(item) = list.iter().find(|i| i.id == id) else { return };
    log(|| format!("{} pressed for {} (yes/no {}, no key {})", if accept { "accept" } else { "reject" }, item.name, item.can_accept(), item.can_reject()));
    if !(if accept { item.can_accept() } else { item.can_reject() }) {
        return;
    }
    let (seen, name) = (item.ask.text.clone(), item.name.clone());
    let f = cx.global_mut::<Float>();
    if f.answered.contains_key(id) {
        return;
    }
    match (f.answer)(id, accept, &seen) {
        Ok(()) => {
            f.errors.remove(id);
            f.answered.insert(id.to_string(), seen);
            f.confirm = Some((id.to_string(), name));
            let id = id.to_string();
            cx.spawn(async move |cx: &mut AsyncApp| {
                cx.background_executor().timer(CONFIRM).await;
                cx.update(|cx| confirmed(&id, cx));
            })
            .detach();
        }
        Err(e) => {
            log(|| format!("not sent: {e}"));
            f.errors.insert(id.to_string(), not_sent(&e));
        }
    }
    sync_later(cx);
}

/// "Sent to …" has been said: the next card that waits is shown (the
/// answered one goes when its agent moves on).
fn confirmed(id: &str, cx: &mut App) {
    if !cx.has_global::<Float>() || cx.global::<Float>().confirm.as_ref().is_none_or(|(c, _)| c != id) {
        return;
    }
    let (list, questions) = (items(cx), asks(cx));
    let entries = model::entries(&list, &questions);
    let f = cx.global_mut::<Float>();
    f.confirm = None;
    let at = model::position(&entries, f.current.as_deref());
    if entries.get(at).is_some_and(|e| e.key() == id) {
        let answered = |k: &str| f.answered.contains_key(k);
        let next = model::next_waiting(&entries, at, &answered);
        f.current = entries.get(next).map(|e| e.key());
    }
    sync(cx);
}

/// A key pressed while the open panel has the keyboard. ←/→ go between
/// the cards. On a yes/no Pitwall can answer, Enter is Accept and Esc is
/// Reject (the same re-check as the buttons); other keys go to the agent's
/// terminal as its own pane would send them (the terminal shows what they
/// do). On anything else Esc closes the panel and the rest goes to the
/// terminal. `true` when the key was used.
fn key(ev: &KeyDownEvent, cx: &mut App) -> bool {
    use pitwall_term_view::keys::{encode_key, KeyModes, KeyPress, Mods};
    let (list, questions) = (items(cx), asks(cx));
    let entries = model::entries(&list, &questions);
    let f = cx.global::<Float>();
    if !f.expanded || !f.keys {
        return false;
    }
    let ks = &ev.keystroke;
    let plain = !(ks.modifiers.shift || ks.modifiers.alt || ks.modifiers.control || ks.modifiers.platform);
    let current = entries.get(model::position(&entries, f.current.as_deref())).copied();
    match ks.key.as_str() {
        "left" if plain && entries.len() > 1 => {
            go(-1, cx);
            return true;
        }
        "right" if plain && entries.len() > 1 => {
            go(1, cx);
            return true;
        }
        _ => {}
    }
    let item = match current {
        Some(model::Entry::Agent(i)) if i.local => i,
        _ => {
            if ks.key == "escape" {
                toggle(cx);
                return true;
            }
            return false;
        }
    };
    match ks.key.as_str() {
        "enter" if plain && item.can_accept() => {
            answer(&item.id, true, cx);
            return true;
        }
        "escape" if plain && item.can_reject() => {
            answer(&item.id, false, cx);
            return true;
        }
        "escape" if plain && !item.can_accept() => {
            toggle(cx);
            return true;
        }
        _ => {}
    }
    let mods = Mods { shift: ks.modifiers.shift, alt: ks.modifiers.alt, ctrl: ks.modifiers.control, cmd: ks.modifiers.platform };
    let press = KeyPress { key: ks.key.as_str(), text: ks.key_char.as_deref(), mods };
    // Plain text isn't a terminal key: it is typed as it is.
    let bytes = encode_key(press, KeyModes::default())
        .or_else(|| ks.key_char.as_ref().filter(|_| !mods.cmd && !mods.ctrl).map(|t| t.as_bytes().to_vec()));
    let Some(bytes) = bytes else { return false };
    let id = item.id.clone();
    if let Err(e) = (f.typer)(&id, &bytes) {
        log(|| format!("key not sent: {e}"));
        cx.global_mut::<Float>().errors.insert(id, not_sent(&e));
        sync_later(cx);
    }
    true
}

/// Accept every card, when every one is a yes/no Pitwall can answer
/// ([`model::accept_all`]).
fn accept_all(cx: &mut App) {
    let (list, questions) = (items(cx), asks(cx));
    let entries = model::entries(&list, &questions);
    let f = cx.global::<Float>();
    if !model::accept_all(&entries, &|k| f.answered.contains_key(k)) {
        return;
    }
    for id in list.iter().map(|i| i.id.clone()) {
        answer(&id, true, cx);
    }
}

/// Bring Pitwall forward on `id` (the control then goes away).
fn open(id: &str, cx: &mut App) {
    let opener = cx.global::<Float>().open.clone();
    opener(id, cx);
}

/// Pick suggestion `i` of question `key`: sent at once, or toggled when
/// it takes several (as in its inbox).
fn pick_ask(key: &str, i: u32, cx: &mut App) {
    let Some(a) = asks(cx).into_iter().find(|a| a.key() == key) else { return };
    if i as usize >= a.ask.suggestions.len() || cx.global::<Float>().sent.contains_key(key) {
        return;
    }
    if a.ask.multiple {
        let p = cx.global_mut::<Float>().picks.entry(key.to_string()).or_default();
        match p.iter().position(|x| *x == i) {
            Some(at) => {
                p.remove(at);
            }
            None => {
                p.push(i);
                p.sort();
            }
        }
        sync_later(cx);
    } else {
        send_ask(&a, vec![i], None, false, cx);
    }
}

/// Answer question `a` (picks and/or the user's own words, or a
/// decline), as its inbox does: `true` when it went.
fn send_ask(a: &AskItem, picks: Vec<u32>, text: Option<String>, declined: bool, cx: &mut App) -> bool {
    let key = a.key();
    if cx.global::<Float>().sent.contains_key(&key) {
        return false;
    }
    log(|| format!("answer to {key}: picks {picks:?}, own words {}, declined {declined}", text.as_deref().is_some_and(|t| !t.trim().is_empty())));
    let req = if a.tile.is_some() {
        crate::tiles::group::answer(&a.ext, &a.thread, picks, text, declined, cx)
    } else {
        crate::main_screen::inbox_tile::answer_ask(&a.ext, &a.thread, &a.ask, picks, text, declined, cx)
    };
    if let Some(req) = req {
        let f = cx.global_mut::<Float>();
        f.sent.insert(key.clone(), req);
        f.picks.remove(&key);
    }
    sync_later(cx);
    req.is_some()
}

/// Bring Pitwall forward on question `a`'s thread in its inbox, or on the
/// tile that asks it.
fn open_ask(a: &AskItem, cx: &mut App) {
    if let Some((tile, _)) = &a.tile {
        crate::windows::show_extension_view(&a.ext, tile, cx);
        return;
    }
    let opener = cx.global::<Float>().open_inbox.clone();
    opener(&a.ext, &a.thread, cx);
}

/// Debug builds: `PITWALL_DEBUG_PROMPTS=expand[,accept|reject|open|all|
/// click-puck|click-collapse|click-accept|resize-wide|resize-narrow|
/// resize-tall|resize-short|resize-corner|resize-right|resize-anchored|
/// resize-reset|size-small|size-medium|size-large]` does those to the
/// control once it is up (the first agent listed; `all` is "Accept all";
/// the `click-` ones are real clicks, macOS only; the `resize-` ones go
/// through what a drag of an edge does, with a made-up travel), a step
/// every 1.5 s, and logs its state after each:
/// for screenshots and checks of the running app without synthetic input
/// (as `windows::debug_hook`).
pub fn debug_hook(cx: &mut App) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Ok(steps) = std::env::var("PITWALL_DEBUG_PROMPTS") else { return };
    cx.spawn(async move |cx: &mut AsyncApp| {
        let step = Duration::from_millis(1500);
        // Wait for the control (an agent asking, Pitwall not in front).
        loop {
            cx.background_executor().timer(step).await;
            if cx.update(|cx| cx.try_global::<Float>().is_some_and(|f| f.window.is_some())) {
                break;
            }
        }
        for verb in steps.split(',') {
            // A resize as a drag of an edge does it (the same steps, with
            // the pointer's travel made up), a little every frame.
            if let Some((edge, dx, dy)) = cx.update(|cx| debug_resize(verb, cx)) {
                cx.update(|cx| begin_resize(edge, cx));
                for i in 1..=20 {
                    cx.background_executor().timer(Duration::from_millis(16)).await;
                    let k = i as f32 / 20.;
                    cx.update(|cx| resize_to(dx * k, dy * k, cx));
                }
                cx.update(end_resize);
            }
            cx.update(|cx| {
                let first = items(cx).first().map(|i| i.id.clone()).unwrap_or_default();
                match verb {
                    "expand" => toggle(cx),
                    "accept" => answer(&first, true, cx),
                    "reject" => answer(&first, false, cx),
                    "all" => accept_all(cx),
                    "open" => open(&first, cx),
                    // Real clicks (macOS): the pill (its name), the
                    // header's collapse button, the pill's Accept.
                    "click-puck" => debug_click(PAD + 70., PAD + PILL_H / 2., cx),
                    "click-collapse" => {
                        let width = shape(cx.global::<Float>()).width;
                        debug_click(PAD + width - 20., PAD + HEADER_H / 2., cx)
                    }
                    "resize-reset" => reset_size(cx),
                    "size-small" => set_preset(Preset::Small, cx),
                    "size-medium" => set_preset(Preset::Medium, cx),
                    "size-large" => set_preset(Preset::Large, cx),
                    "click-accept" => debug_click(PAD + PILL_W - PILL_PAD - 38., PAD + PILL_H / 2., cx),
                    _ => {}
                }
            });
            cx.background_executor().timer(step).await;
            cx.update(|cx| {
                eprintln!("pitwall-debug: prompts after {verb}: {}", state(cx));
            });
        }
    })
    .detach();
}

/// Bench commands, `float …` (`PITWALL_BENCH`, for screenshots; nothing is
/// clicked or typed on the screen): `show` (the control up in a bench
/// instance, which otherwise never shows it, far off every display; capture
/// it by its window), `hide`, `expand`, `minimise` (back to the pill),
/// `next`, `prev` (the cards), `show-all` (every row under a summary),
/// `size small|medium|large`, `resize <left|right|top|bottom|top-left|…>
/// <dx> <dy>` (that edge dragged so far, as a drag does it), `pick <n>`
/// (suggestion n of the first question's card, as a click), `type <text>`
/// (into its own-answer field), `send`, `decline`, `open`, and `state` (on
/// stderr).
pub fn bench(cmd: &str, cx: &mut App) {
    if !cx.has_global::<Float>() {
        return eprintln!("pitwall-debug: float: no floating control here");
    }
    let mut words = cmd.split_whitespace();
    let what = words.next().unwrap_or_default();
    let first = |cx: &App| asks(cx).into_iter().next();
    match what {
        "preview" => {
            let f = cx.global_mut::<Float>();
            // (An anchor on no display: sizes are as asked, not fitted.)
            f.anchor = Some(point(px(crate::bench::OFF_SCREEN.0 + 3000.), px(crate::bench::OFF_SCREEN.1 + 2000.)));
            match words.next() {
                Some("off") => {
                    f.bench_shown = false;
                    f.preview = None;
                }
                Some("collapsed") => (f.bench_shown, f.expanded) = (true, false),
                None => f.bench_shown = true,
                Some(p) => {
                    let preset = Preset::ALL.into_iter().find(|s| s.name().eq_ignore_ascii_case(p));
                    let custom = p.split_once('x').and_then(|(w, r)| Some((w.parse::<f32>().ok()?, r.parse::<usize>().ok()?)));
                    let (width, rows) = match (preset, custom) {
                        (Some(s), _) => s.size(),
                        (None, Some((w, r))) => (Some(w.clamp(model::MIN_WIDTH, model::MAX_WIDTH)), Some(r.clamp(model::MIN_ROWS, model::MAX_ROWS))),
                        _ => return eprintln!("pitwall-debug: float preview [collapsed|small|medium|large|<w>x<rows>|off]"),
                    };
                    (f.bench_shown, f.width, f.rows, f.expanded) = (true, width, rows, true);
                }
            }
            sync(cx);
            cx.refresh_windows();
        }
        "expand" if !cx.global::<Float>().expanded => toggle(cx),
        "minimise" | "minimize" if cx.global::<Float>().expanded => toggle(cx),
        "size" => match words.next() {
            Some("small") => set_preset(Preset::Small, cx),
            Some("medium") => set_preset(Preset::Medium, cx),
            Some("large") => set_preset(Preset::Large, cx),
            _ => eprintln!("pitwall-debug: float size small|medium|large"),
        },
        "resize" => {
            let name = format!("float-resize-{}", words.next().unwrap_or_default());
            let mut by = words.filter_map(|w| w.parse::<f32>().ok());
            let (Some(edge), Some(dx), Some(dy)) = (Edge::ALL.into_iter().find(|e| e.name() == name), by.next(), by.next()) else {
                return eprintln!("pitwall-debug: float resize <edge> <dx> <dy>");
            };
            begin_resize(edge, cx);
            resize_to(dx, dy, cx);
            end_resize(cx);
        }
        "pick" => match (first(cx), words.next().and_then(|n| n.parse::<u32>().ok())) {
            (Some(a), Some(n)) => pick_ask(&a.key(), n.saturating_sub(1), cx),
            _ => eprintln!("pitwall-debug: float pick <n> (with a question shown)"),
        },
        "type" | "send" => {
            let Some(a) = first(cx) else { return eprintln!("pitwall-debug: float has no question") };
            let f = cx.global::<Float>();
            let view = f.window.and_then(|w| w.read(cx).ok()).or_else(|| f.preview.as_ref().map(|v| v.read(cx)));
            let input = view.and_then(|v| v.others.get(&a.key()).map(|(i, _)| i.clone()));
            let Some(input) = input else { return eprintln!("pitwall-debug: float's question has no field drawn (expand first)") };
            if what == "type" {
                let text = cmd.trim_start().strip_prefix("type").unwrap_or_default().trim().to_string();
                input.update(cx, |i, cx| i.replace_text(text, cx));
                sync(cx);
            } else {
                submit_own(&a.key(), &input, cx);
            }
        }
        "decline" => {
            if let Some(a) = first(cx) {
                send_ask(&a, vec![], None, true, cx);
            }
        }
        "open" => {
            if let Some(a) = first(cx) {
                open_ask(&a, cx);
            }
        }
        // Made-up agents instead of the store's (`demo`), and the first
        // one's answer as failed or sent.
        "demo" => {
            let name = words.next().unwrap_or_default();
            let f = cx.global_mut::<Float>();
            if name == "off" {
                (f.demo, f.demo_screens) = (None, HashMap::new());
            } else if let Some(list) = demo::scenario(name) {
                f.demo_screens = list.iter().map(|(a, s)| (a.id.clone(), s.clone())).collect();
                f.demo = Some(list.into_iter().map(|(a, _)| a).collect());
            } else {
                return eprintln!("pitwall-debug: float demo one|two|many|question|long|remote|off");
            }
            (f.errors, f.answered, f.confirm, f.current, f.show_all) = (HashMap::new(), HashMap::new(), None, None, false);
            sync(cx);
            cx.refresh_windows();
        }
        // The card shown's answer as failed (the prompt changed) or just
        // sent (`demo-sent`; `demo-sent-later`: after "Sent to …").
        "demo-error" | "demo-sent" | "demo-sent-later" => {
            let (list, questions) = (items(cx), asks(cx));
            let entries = model::entries(&list, &questions);
            let f = cx.global_mut::<Float>();
            let Some(model::Entry::Agent(item)) = entries.get(model::position(&entries, f.current.as_deref())).copied() else { return };
            if what == "demo-error" {
                f.errors.insert(item.id.clone(), not_sent("its prompt changed"));
            } else {
                f.answered.insert(item.id.clone(), item.ask.text.clone());
                f.confirm = (what == "demo-sent").then(|| (item.id.clone(), item.name.clone()));
            }
            sync(cx);
            cx.refresh_windows();
        }
        "next" => go(1, cx),
        "prev" => go(-1, cx),
        "show-all" => {
            let f = cx.global_mut::<Float>();
            f.show_all = !f.show_all;
            sync(cx);
        }
        "state" => eprintln!("pitwall-debug: float: {}", state(cx)),
        _ => eprintln!("pitwall-debug: float {cmd}?"),
    }
}

/// What the control shows and how, in a line (the debug hook, `float
/// state`).
fn state(cx: &App) -> String {
    let (list, questions) = (items(cx), asks(cx));
    let f = cx.global::<Float>();
    format!(
        "up {}, expanded {}, size {:?}, shape {:?} ({}), anchor {:?}, agents {:?}, questions {:?}, answered {:?}, errors {:?}, threads asked for {}",
        f.window.is_some(),
        f.expanded,
        f.size,
        shape(f),
        Preset::of(f.width, f.rows).map_or("dragged", Preset::name),
        f.anchor,
        list.iter().map(|i| (i.name.as_str(), i.can_accept(), i.can_reject())).collect::<Vec<_>>(),
        questions.iter().map(|a| (a.ext_name.as_str(), a.who(), a.ask.question.as_str(), ask_note(f, a).map(|n| n.0.to_string()))).collect::<Vec<_>>(),
        f.answered.keys().collect::<Vec<_>>(),
        f.errors,
        f.opened.len()
    )
}

/// The `resize-` verbs of [`debug_hook`]: the edge, and how far the
/// pointer goes (right and down are positive).
fn debug_resize(verb: &str, _: &App) -> Option<(Edge, f32, f32)> {
    let cards = 1.;
    // Up is taller on macOS (its top edge), down elsewhere.
    let taller = if GROWS_UP { -1. } else { 1. };
    Some(match verb {
        "resize-wide" => (Edge::LEFT, -200., 0.),
        "resize-narrow" => (Edge::LEFT, 150., 0.),
        "resize-tall" => (Edge::FREE_END, 0., taller * 6. * LINE_H * cards),
        "resize-short" => (Edge::FREE_END, 0., -taller * 8. * LINE_H * cards),
        "resize-corner" => (Edge::FREE_CORNER, -120., taller * 4. * LINE_H * cards),
        // The edges it is anchored on: its right, and its bottom on macOS
        // (its top elsewhere).
        "resize-right" => (Edge::RIGHT, 120., 0.),
        "resize-anchored" => (Edge { x: 0, y: Edge::ANCHORED_Y }, 0., -taller * 4. * LINE_H * cards),
        _ => return None,
    })
}

fn debug_click(x: f32, y: f32, cx: &mut App) {
    if let Some(w) = cx.global::<Float>().window {
        let _ = w.update(cx, |_, window, cx| crate::platform::debug_click(window, x, y, cx));
    }
}

/// The pointing hand while the pointer is over `e`. (GPUI's own cursor
/// styles apply to the active app's windows; this window shows while
/// another app is in front, so the platform is told directly.)
fn hot<E: StatefulInteractiveElement>(e: E, key: impl Into<SharedString>) -> E {
    hot_as(e, key, FloatCursor::Pointing)
}

/// `cursor` while the pointer is over `e` (as [`hot`]).
fn hot_as<E: StatefulInteractiveElement>(e: E, key: impl Into<SharedString>, cursor: FloatCursor) -> E {
    let key: SharedString = key.into();
    e.on_hover(move |over, _, cx| {
        let f = cx.global_mut::<Float>();
        if *over {
            f.hot.insert(key.clone(), cursor);
        } else {
            f.hot.remove(&key);
        }
        crate::platform::float_cursor(cursor_now(f));
    })
}

/// The strip along `edge` of the open panel (in a window `w` × `h`) that
/// resizes it: a press follows the pointer, a double press resets the
/// size. Over the panel's edge, so a press there never moves the window.
fn edge_zone(edge: Edge, w: f32, h: f32) -> impl IntoElement {
    // The panel's edges in the window, and the strips' thickness.
    let (left, top, right, bottom) = (PAD, PAD, w - PAD, h - PAD);
    let (half, corner) = (EDGE / 2., CORNER / 2.);
    let at_x = if edge.x < 0 { left } else { right };
    let at_y = if edge.y < 0 { top } else { bottom };
    let (x, y, zw, zh) = match (edge.x, edge.y) {
        // A side, between the corners.
        (_, 0) => (at_x - half, top + corner, EDGE, bottom - top - 2. * corner),
        // The top or the bottom, between the corners.
        (0, _) => (left + corner, at_y - half, right - left - 2. * corner, EDGE),
        _ => (at_x - corner, at_y - corner, CORNER, CORNER),
    };
    let name = edge.name();
    hot_as(
        div()
            .id(name)
            .debug_selector(move || name.into())
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(zw.max(0.)))
            .h(px(zh.max(0.)))
            .cursor(edge.style())
            .occlude()
            .on_mouse_down(MouseButton::Left, move |ev, window, cx| press_edge(edge, ev, window, cx)),
        name,
        edge.cursor(),
    )
}

/// Where the platform leaves following a resize to the window's own mouse
/// events (see [`crate::platform::float_resize`]): the pointer's travel
/// while the button is held, from the screen's point of view (the
/// window's origin plus where in it), and the release.
fn follow_resize(window: &mut Window) {
    window.on_mouse_event(|ev: &MouseMoveEvent, phase, window, cx| {
        let Some(from) = cx.global::<Float>().resize.and_then(|r| r.from) else { return };
        if phase != DispatchPhase::Capture {
            return;
        }
        if ev.pressed_button != Some(MouseButton::Left) {
            cx.defer(end_resize);
            return;
        }
        let by = window.bounds().origin + ev.position - from;
        let (dx, dy) = (f32::from(by.x), f32::from(by.y));
        // The window can't change size while it handles an event.
        cx.defer(move |cx| resize_to(dx, dy, cx));
    });
    window.on_mouse_event(|_: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Capture && cx.global::<Float>().resize.is_some_and(|r| r.from.is_some()) {
            cx.defer(end_resize);
        }
    });
}

/// Dragging `e` moves the window (it then follows the pointer until the
/// button is released); a press that doesn't travel stays a click.
fn drag_handle<E: InteractiveElement>(e: E) -> E {
    e.on_mouse_down(MouseButton::Left, |ev, _, _| {
        log(|| format!("press at {:?} (system says active {})", ev.position, crate::platform::app_active_raw()));
        PRESS.set(Some(ev.position));
        DRAGGED.set(false);
    })
    .on_mouse_move(|ev, window, cx| {
        let Some(at) = PRESS.get() else { return };
        if ev.pressed_button != Some(MouseButton::Left) {
            PRESS.set(None);
        } else if (ev.position - at).magnitude() > DRAG_THRESHOLD as f64 {
            PRESS.set(None);
            DRAGGED.set(true);
            log(|| "drag starts".to_string());
            crate::platform::float_drag(window, cx);
        }
    })
    .on_mouse_up(MouseButton::Left, |_, _, _| PRESS.set(None))
}

/// How much of the theme's own surface is laid over the glass: enough
/// for the mark and the text to read over a white page or a dark one,
/// little enough that what is behind still shows through.
const GLASS_FILL: f32 = 0.58;
/// The opened panel holds text: a little more of it.
const GLASS_FILL_OPEN: f32 = 0.72;

/// Corners of the pill and of the opened panel, and the pill's padding.
const PILL_RADIUS: f32 = 16.;
const PANEL_RADIUS: f32 = 14.;
const PILL_PAD: f32 = 12.;

/// The control's own shadow, small enough for the [`PAD`] around it, and
/// the hairline around it.
fn shadow(t: &Theme, liquid: bool) -> Vec<gpui::BoxShadow> {
    let line = if t.is_glass() { t.g.line_strong } else { t.line_strong };
    // On the system's glass only the hairline: a shadow of ours would show
    // through the glass as a dark smudge inside it.
    if liquid {
        return vec![t.ring(line)];
    }
    vec![
        gpui::BoxShadow { color: gpui::black().opacity(0.30), offset: point(px(0.), px(3.)), blur_radius: px(10.), spread_radius: px(0.), inset: false },
        t.ring(line),
    ]
}

/// An agent's screen as a card shows it.
struct Watched {
    /// The watch at the source (`None`: it couldn't be watched).
    id: Option<u64>,
    /// Frames of an earlier watch of the same agent are ignored.
    token: u64,
    screen: Screen,
}

/// The window's content. What is asked and answered is [`Float`]; this
/// draws it, and keeps the screens of the agents that have a card.
pub struct FloatView {
    source: Option<Arc<dyn ScreenSource>>,
    watched: HashMap<String, Watched>,
    /// How far each card's terminal was scrolled from its prompt, in
    /// points (down is positive), and the prompt that was for.
    scrolled: HashMap<String, (f32, String)>,
    frames: UnboundedSender<FrameMsg>,
    tokens: u64,
    /// Has the keyboard while a card's terminal is typed into.
    focus: FocusHandle,
    /// Each question's field for an answer of the user's own, by its key.
    others: HashMap<String, (Entity<TextInput>, gpui::Subscription)>,
    /// Drawn in a bench instance's main window ([`bench_preview`]), not
    /// as its own window: no glass, no keyboard taken, its size the model's.
    preview: bool,
    _pump: Task<()>,
    _subs: [gpui::Subscription; 2],
}

impl Drop for FloatView {
    fn drop(&mut self) {
        self.watch(&[], &HashMap::new());
    }
}

impl FloatView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Wherever the user drags it is where it opens and resizes from.
        let moved = cx.observe_window_bounds(window, |_, window, cx| {
            // (A bench instance's is off every display: it has no bounds.
            // A test window keeps its top left whatever it is told.)
            if crate::bench::enabled() || cfg!(test) {
                return;
            }
            let b = window.bounds();
            let y = if GROWS_UP { b.origin.y + b.size.height } else { b.origin.y };
            cx.global_mut::<Float>().anchor = Some(point(b.origin.x + b.size.width, y));
        });
        let (frames, mut rx) = unbounded::<FrameMsg>();
        let pump = cx.spawn(async move |this, cx| {
            while let Some(m) = rx.next().await {
                let applied = this.update(cx, |v, cx| {
                    if let Some(w) = v.watched.get_mut(&m.agent).filter(|w| w.token == m.token) {
                        w.screen.apply(&m.frame);
                        cx.notify();
                    }
                });
                if applied.is_err() {
                    break;
                }
            }
        });
        FloatView {
            source: Screens::get(cx),
            watched: HashMap::new(),
            scrolled: HashMap::new(),
            frames,
            tokens: 0,
            focus: cx.focus_handle(),
            others: HashMap::new(),
            preview: false,
            _pump: pump,
            _subs: [cx.observe_global::<Theme>(|_, cx| cx.notify()), moved],
        }
    }

    /// Watch the screens of exactly `ids` (a bench instance's made-up
    /// agents: their made-up screens, `demo`).
    fn watch(&mut self, ids: &[&str], demo: &HashMap<String, pitwall_proto::ScreenFrame>) {
        let source = self.source.clone();
        self.watched.retain(|id, w| {
            let keep = ids.contains(&id.as_str());
            if let (false, Some(wid), Some(source)) = (keep, w.id, &source) {
                source.unwatch(id, wid);
            }
            keep
        });
        for id in ids {
            if self.watched.contains_key(*id) {
                continue;
            }
            if let Some(frame) = demo.get(*id) {
                let mut screen = Screen::default();
                screen.apply(frame);
                self.watched.insert(id.to_string(), Watched { id: None, token: 0, screen });
                continue;
            }
            let Some(source) = &source else { continue };
            self.tokens += 1;
            let sink = forward(self.frames.clone(), id.to_string(), self.tokens);
            let watch = source.watch(id, sink).ok();
            self.watched.insert(id.to_string(), Watched { id: watch, token: self.tokens, screen: Screen::default() });
        }
    }

    /// The rows of `item`'s terminal that hold its prompt, as a screen of
    /// their own, once its screen is here, and where the prompt is on it.
    fn term(&self, item: &Item, max: usize, fill: bool) -> Option<(Screen, TermRows)> {
        let s = &self.watched.get(&item.id)?.screen;
        if !s.has_frame() {
            return None;
        }
        let texts: Vec<String> = s.lines.iter().map(|r| r.text()).collect();
        let at = TermRows::find(&texts, &item.ask.text)?;
        let (start, end) = at.span(max, fill);
        // Scrolled: the same number of rows, further up or down the screen.
        let rows = end - start;
        let last = texts.iter().rposition(|r| !r.trim().is_empty()).map_or(end, |i| i + 1);
        let by = self.scrolled.get(&item.id).filter(|(_, ask)| *ask == item.ask.text).map_or(0., |(by, _)| *by);
        let start = (start as isize + (by / LINE_H).round() as isize).clamp(0, last.saturating_sub(rows) as isize) as usize;
        Some((Screen { cols: s.cols, rows: rows as u16, cursor: None, lines: s.lines[start..start + rows].to_vec(), frames: 1 }, at))
    }

    /// The wheel over `item`'s terminal: `dy` points (down is positive).
    /// Kept within what the screen has above and below the prompt.
    fn scroll(&mut self, item: &Item, max: usize, fill: bool, dy: f32, cx: &mut Context<Self>) {
        let Some(s) = self.watched.get(&item.id).map(|w| &w.screen).filter(|s| s.has_frame()) else { return };
        let texts: Vec<String> = s.lines.iter().map(|r| r.text()).collect();
        let Some((start, end)) = model::term_span(&texts, &item.ask.text, max, fill) else { return };
        let last = texts.iter().rposition(|r| !r.trim().is_empty()).map_or(end, |i| i + 1);
        let (up, down) = (start as f32 * LINE_H, last.saturating_sub(end) as f32 * LINE_H);
        let was = self.scrolled.get(&item.id).filter(|(_, ask)| *ask == item.ask.text).map_or(0., |(by, _)| *by);
        self.scrolled.insert(item.id.clone(), ((was + dy).clamp(-up, down), item.ask.text.clone()));
        cx.notify();
    }

    /// Collapsed: a pill that says who waits (the oldest not answered
    /// yet), what it wants, and how many wait; Accept and Reject when the
    /// pill can answer it in one click, else a way to look first.
    fn pill(entries: &[model::Entry], f: &Float, fill: Option<gpui::Hsla>, t: &Theme) -> impl IntoElement {
        let answered = |k: &str| f.answered.contains_key(k);
        let at = model::next_waiting(entries, 0, &answered);
        let entry = entries[at];
        let key = entry.key();
        let more = entries.len().saturating_sub(1);
        let sent_to = f.answered.contains_key(&key).then(|| entry.name().to_string());
        // Who: the agent's logo (the extension's icon), its name, and its
        // project when another that waits has the same name.
        let (logo, name, project) = match entry {
            model::Entry::Agent(i) => (
                kit::agent_logo(&i.kind, &i.kind_name, 20., t),
                i.name.clone(),
                model::clashes(entries, &i.name).then(|| i.project.clone()),
            ),
            model::Entry::Ask(a) => (kit::icon(&a.icon, 18., t.text_2).into_any_element(), a.who().to_string(), Some(a.ext_name.clone())),
        };
        let error = f.errors.get(&key).cloned();
        let what: (SharedString, gpui::Hsla) = match (&error, &sent_to, entry) {
            (Some(e), _, _) => (e.clone(), t.red),
            (None, Some(n), _) => (format!("Sent to {n}").into(), t.green),
            (None, None, model::Entry::Agent(i)) => (model::gist(i).into(), t.text_2),
            (None, None, model::Entry::Ask(a)) => (a.ask.question.clone().into(), t.text_2),
        };
        let ident = div()
            .flex_none()
            .size(px(36.))
            .rounded(px(10.))
            .bg(t.surface_2)
            .border_1()
            .border_color(t.amber.opacity(0.55))
            .flex()
            .items_center()
            .justify_center()
            .child(logo);
        let count_chip = (more > 0).then(|| {
            let chip = div()
                .debug_selector(|| "float-pill-count".into())
                .flex_none()
                .h(px(18.))
                .px(px(6.))
                .rounded_full()
                .bg(t.amber_soft)
                .flex()
                .items_center()
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.amber)
                .child(SharedString::from(format!("+{more}")));
            // A change of count: a short fade in, nothing that blinks.
            if kit::motion::on() {
                chip.with_animation(ElementId::Name(format!("float-count-{more}").into()), Animation::new(Duration::from_millis(260)), |d, k| d.opacity(0.35 + 0.65 * k)).into_any_element()
            } else {
                chip.into_any_element()
            }
        });
        let words = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .min_w_0()
                    .child(div().flex_none().max_w(px(170.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).child(SharedString::from(name)))
                    .when_some(project, |d, p| d.child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.5)).text_color(t.text_3).child(SharedString::from(p))))
                    .children(count_chip),
            )
            .child(
                div()
                    .debug_selector(|| "float-pill-what".into())
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.5))
                    .text_color(what.1)
                    .child(what.0),
            );
        // The buttons: one click answers a plain yes/no whose summary says
        // all it does; anything else is looked at first.
        let k = key.clone();
        let review = move |label: &'static str, t: &Theme| {
            let k = k.clone();
            no_drag(hot(
                kit::text_button("float-pill-review", label, kit::BtnKind::Ghost, false, t).debug_selector(|| "float-pill-review".into()).on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    expand_on(&k, cx);
                }),
                "pill-review",
            ))
            .into_any_element()
        };
        let buttons: Vec<gpui::AnyElement> = match entry {
            _ if sent_to.is_some() => vec![],
            _ if error.is_some() => vec![review("Show", t)],
            model::Entry::Agent(i) if model::pill_answers(i) => {
                let (yes, no) = (i.id.clone(), i.id.clone());
                let mut b = Vec::new();
                if i.can_reject() {
                    b.push(
                        no_drag(hot(
                            kit::text_button("float-pill-reject", "Reject", kit::BtnKind::Ghost, false, t).debug_selector(|| "float-pill-reject".into()).on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                answer(&no, false, cx);
                            }),
                            "pill-reject",
                        ))
                        .into_any_element(),
                    );
                }
                b.push(
                    no_drag(hot(
                        kit::accent_button("float-pill-accept", t.amber, gpui::rgb(0x1a1200).into(), t).debug_selector(|| "float-pill-accept".into()).child("Accept").on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            answer(&yes, true, cx);
                        }),
                        "pill-accept",
                    ))
                    .into_any_element(),
                );
                b
            }
            model::Entry::Agent(i) if i.can_accept() => vec![review("Show", t)],
            model::Entry::Agent(i) => {
                let id = i.id.clone();
                vec![no_drag(hot(
                    kit::text_button("float-pill-open", "Open", kit::BtnKind::Ghost, false, t).debug_selector(|| "float-pill-open".into()).on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        open(&id, cx);
                    }),
                    "pill-open",
                ))
                .into_any_element()]
            }
            model::Entry::Ask(a) => {
                let a = a.clone();
                vec![no_drag(hot(
                    kit::text_button("float-pill-open", "Open", kit::BtnKind::Ghost, false, t).debug_selector(|| "float-pill-open".into()).on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        open_ask(&a, cx);
                    }),
                    "pill-open",
                ))
                .into_any_element()]
            }
        };
        let k = key.clone();
        let pill = div()
            .id("float-puck")
            .debug_selector(|| "float-puck".into())
            .absolute()
            .left(px(PAD))
            .top(px(PAD))
            .w(px(PILL_W))
            .h(px(PILL_H))
            .rounded(px(PILL_RADIUS))
            .bg(fill.unwrap_or(t.raised))
            .shadow(shadow(t, fill.is_some()))
            .px(px(PILL_PAD))
            .flex()
            .items_center()
            .gap(px(10.))
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                if !DRAGGED.replace(false) {
                    expand_on(&k, cx);
                }
            })
            .child(ident)
            .child(words)
            .child(div().flex_none().flex().items_center().gap(px(6.)).children(buttons));
        div().relative().size_full().child(hot(drag_handle(pill), "puck"))
    }

    /// Open: who asks (logo, name, project), where this card is among
    /// those that wait ("1 of 3" and the arrows), "Accept all" when every
    /// one is a plain yes/no, and the button back to the pill.
    fn header(entries: &[model::Entry], at: usize, f: &Float, t: &Theme) -> impl IntoElement {
        let entry = entries[at];
        let (logo, name, place) = match entry {
            model::Entry::Agent(i) => {
                let place = if i.local { i.project.clone() } else { format!("{} · another machine", i.project) };
                (kit::agent_logo(&i.kind, &i.kind_name, 18., t), i.name.clone(), place)
            }
            model::Entry::Ask(a) => (kit::icon(&a.icon, 16., t.text_2).into_any_element(), a.who().to_string(), a.ext_name.clone()),
        };
        let all = model::accept_all(entries, &|k| f.answered.contains_key(k));
        let nav = |id: &'static str, glyph: &'static str, by: isize| {
            no_drag(hot(
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .flex_none()
                    .size(px(28.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(16.))
                    .text_color(t.text_2)
                    .cursor_pointer()
                    .hover(|d| d.bg(t.surface_3).text_color(t.text))
                    .on_click(move |_, _, cx| go(by, cx))
                    .child(glyph),
                id,
            ))
        };
        let collapse = div()
            .id("float-collapse")
            .debug_selector(|| "float-collapse".into())
            .flex_none()
            .size(px(28.))
            .rounded(px(7.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|d| d.bg(t.surface_3))
            .on_click(|_, _, cx| toggle(cx))
            // Minimise: back to the pill.
            .child(div().w(px(11.)).h(px(1.5)).rounded_full().bg(t.text_2));
        div()
            .id("float-header")
            .debug_selector(|| "float-header".into())
            .h(px(HEADER_H))
            .flex_none()
            .pl(px(14.))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_none().flex().items_center().child(logo))
            .child(div().flex_none().max_w(px(200.)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(SharedString::from(name)))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.5)).text_color(t.text_3).child(SharedString::from(place)))
            .when(all, |d| {
                d.child(no_drag(hot(
                    div()
                        .id("float-accept-all")
                        .debug_selector(|| "float-accept-all".into())
                        .flex_none()
                        .h(px(28.))
                        .px(px(10.))
                        .rounded(kit::button::btn_radius(t))
                        .flex()
                        .items_center()
                        .border_1()
                        .border_color(t.amber.opacity(0.7))
                        .text_size(px(12.5))
                        .font_weight(kit::WEIGHT_550)
                        .text_color(t.text)
                        .cursor_pointer()
                        .hover(|d| d.bg(t.amber_soft))
                        .on_click(|_, _, cx| accept_all(cx))
                        .child(SharedString::from(format!("Accept all {}", entries.len()))),
                    "accept-all",
                )))
            })
            .when(entries.len() > 1, |d| {
                d.child(nav("float-prev", "‹", -1))
                    .child(
                        div()
                            .debug_selector(|| "float-position".into())
                            .flex_none()
                            .text_size(px(12.5))
                            .text_color(t.text_2)
                            .child(SharedString::from(format!("{} of {}", at + 1, entries.len()))),
                    )
                    .child(nav("float-next", "›", 1))
            })
            .child(no_drag(hot(collapse, "collapse")))
    }

    /// The keys the open panel takes, for the card shown.
    fn hints(entry: model::Entry, count: usize, t: &Theme) -> impl IntoElement {
        let hint = |keys: &str, what: &str| div().flex_none().flex().items_center().gap(px(5.)).child(kit::kbd(keys, t)).child(div().text_size(px(12.)).text_color(t.text_3).child(SharedString::from(what.to_string())));
        let mut list = Vec::new();
        match entry {
            model::Entry::Agent(i) if i.can_accept() => {
                list.push(hint("⏎", "Accept"));
                if i.can_reject() {
                    list.push(hint("esc", "Reject"));
                }
            }
            model::Entry::Agent(i) if i.local => {
                list.push(hint("↑ ↓ ⏎", "Answer in its terminal"));
                list.push(hint("esc", "Close"));
            }
            _ => list.push(hint("esc", "Close")),
        }
        if count > 1 {
            list.push(hint("← →", "Other prompts"));
        }
        div().debug_selector(|| "float-hints".into()).h(px(HINT_H)).flex_none().px(px(14.)).flex().items_center().gap(px(14.)).children(list)
    }

    /// One agent's card: the summary of what it asks (when its prompt has
    /// a shape Pitwall knows), the terminal's rows that ask (a few under a
    /// summary, "Show all" for the rest; `term`: as its terminal shows
    /// them, else the prompt's text), the buttons, and a line of what
    /// happened or how to answer.
    fn card(item: &Item, term: Option<Screen>, total: usize, on_scroll: impl Fn(&ScrollEvent, &mut Window, &mut App) + 'static, focus: FocusHandle, f: &Float, t: &Theme) -> impl IntoElement {
        let s = shape(f);
        let summary = model::summary(item);
        let few = summary.is_some() && !f.show_all;
        let lines = if term.is_some() { Vec::new() } else { item.lines(if few { model::RAW_FEW } else { s.max_lines() }, s.cols()) };
        let sel_term = format!("float-term-{}", item.id);
        let (local, keys) = (item.local, f.keys);
        let error = f.errors.get(&item.id).cloned();
        let sent = sent(f, &item.id);
        let confirm = f.confirm.as_ref().filter(|(id, _)| *id == item.id).map(|(_, n)| n.clone());
        let note: Option<(SharedString, gpui::Hsla)> = match (&error, sent, item.can_accept()) {
            (Some(e), _, _) => Some((e.clone(), t.red)),
            (None, true, _) => Some((format!("Sent to {}{}", item.name, if confirm.is_some() { "" } else { ". It goes when the agent moves on." }).into(), t.green)),
            (None, false, false) if !item.local => Some(("On another machine: answer it there, or open it to look.".into(), t.text_3)),
            (None, false, false) => Some(("More than a yes or no: answer it with the keyboard here, or open it.".into(), t.text_3)),
            (None, false, true) => None,
        };
        let key = |what: &str| format!("float-{what}-{}", item.id);
        let (accept_id, reject_id, open_id) = (item.id.clone(), item.id.clone(), item.id.clone());
        let (sel_accept, sel_reject, sel_open, sel_prompt, sel_note, sel_card, sel_summary) =
            (key("accept"), key("reject"), key("open"), key("prompt"), key("note"), key("card"), key("summary"));
        let summary_block = summary.as_ref().map(|sm| {
            let lines = model::summary_lines(sm, s.width);
            div()
                .debug_selector(move || sel_summary)
                .flex_none()
                .mb(px(GAP))
                .flex()
                .flex_col()
                .child(div().h(px(model::SUMMARY_LABEL_H)).flex_none().text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.amber).child(SharedString::from(sm.label.to_uppercase())))
                .children(lines.into_iter().map(|l| {
                    let line = div().h(px(model::SUMMARY_LINE)).flex_none().whitespace_nowrap().overflow_hidden().line_height(px(model::SUMMARY_LINE)).text_color(t.text);
                    let line = if sm.code { line.font_family(kit::MONO_FONT).text_size(px(13.)) } else { line.text_size(px(14.)).font_weight(FontWeight::MEDIUM) };
                    line.child(SharedString::from(l))
                }))
        });
        // Over the rows when a summary is above them: what they are, and
        // "Show all" when there are more than a few.
        let raw_head = summary.is_some().then(|| {
            div()
                .h(px(model::RAW_HEAD_H))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().text_size(px(12.)).text_color(t.text_3).child("As its terminal shows it"))
                .when(total > model::RAW_FEW || f.show_all, |d| {
                    d.child(no_drag(hot(
                        div()
                            .id("float-show-all")
                            .debug_selector(|| "float-show-all".into())
                            .flex_none()
                            .h(px(22.))
                            .px(px(6.))
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .text_size(px(12.))
                            .font_weight(kit::WEIGHT_550)
                            .text_color(t.text_2)
                            .cursor_pointer()
                            .hover(|d| d.bg(t.surface_3).text_color(t.text))
                            .on_click(|_, _, cx| {
                                let f = cx.global_mut::<Float>();
                                f.show_all = !f.show_all;
                                sync_later(cx);
                            })
                            .child(if f.show_all { "Show less" } else { "Show all" }),
                        "show-all",
                    )))
                })
        });
        let buttons = div()
            .h(px(BUTTONS_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .when(item.can_accept(), |d| {
                d.child(no_drag(hot(
                    kit::accent_button(SharedString::from(sel_accept.clone()), t.amber, gpui::rgb(0x1a1200).into(), t)
                        .debug_selector(move || sel_accept)
                        .when(sent, |b| b.opacity(0.45))
                        .child(if sent { "Sent" } else { "Accept" })
                        .on_click(move |_, _, cx| answer(&accept_id, true, cx)),
                    key("accept"),
                )))
            })
            .when(item.can_reject() && !sent, |d| {
                d.child(no_drag(hot(
                    kit::text_button(SharedString::from(sel_reject.clone()), "Reject", kit::BtnKind::Ghost, false, t)
                        .debug_selector(move || sel_reject)
                        .on_click(move |_, _, cx| answer(&reject_id, false, cx)),
                    key("reject"),
                )))
            })
            .child(div().flex_1())
            .child(no_drag(hot(
                kit::text_button(
                    SharedString::from(sel_open.clone()),
                    "Open in Pitwall",
                    if item.can_accept() { kit::BtnKind::Ghost } else { kit::BtnKind::Primary },
                    false,
                    t,
                )
                .debug_selector(move || sel_open)
                .on_click(move |_, _, cx| open(&open_id, cx)),
                key("open"),
            )));
        let term_id = item.id.clone();
        div()
            .debug_selector(move || sel_card)
            .flex_none()
            .px(px(14.))
            .py(px(GAP))
            .flex()
            .flex_col()
            .children(summary_block)
            .children(raw_head)
            .when_some(term, |d, screen| {
                // The terminal at its own size, cut off on the right: the
                // inner box is wider than any screen, so nothing is scaled.
                d.child(
                    div()
                        .id(SharedString::from(sel_term.clone()))
                        .debug_selector(move || sel_term)
                        .on_scroll(on_scroll)
                        // A click (not a drag) gives the panel the keyboard back.
                        .when(local, |d| {
                            d.on_click(move |_, window, cx| {
                                if DRAGGED.replace(false) {
                                    return;
                                }
                                window.focus(&focus, cx);
                                crate::platform::float_take_keys(window);
                                log(|| format!("keys go to {term_id}"));
                                cx.global_mut::<Float>().keys = true;
                                cx.refresh_windows();
                            })
                        })
                        .border_1()
                        .border_color(if keys && local { t.focus.opacity(0.6) } else { t.line })
                        .relative()
                        .h(px(f32::from(screen.rows) * LINE_H + BOX_PAD))
                        .mb(px(GAP))
                        .flex_none()
                        .rounded(px(7.))
                        .bg(t.term_bg)
                        .overflow_hidden()
                        .child(div().absolute().top(px(BOX_PAD / 2. - 1.)).left_0().h_full().w(px(4000.)).child(screen_view(
                            &screen,
                            ScreenOpts { font_size: TERM_FONT, colors: term_colors(t.mode), show_cursor: keys && local && !item.can_accept() },
                        ))),
                )
            })
            .when(!lines.is_empty(), |d| {
                d.child(
                    div()
                        .debug_selector(move || sel_prompt)
                        .h(px(lines.len() as f32 * LINE_H + BOX_PAD))
                        .mb(px(GAP))
                        .flex_none()
                        .px(px(10.))
                        .py(px(BOX_PAD / 2.))
                        .rounded(px(7.))
                        .border_1()
                        .border_color(t.line)
                        .bg(t.term_bg)
                        .overflow_hidden()
                        .font_family(kit::MONO_FONT)
                        .text_size(px(TERM_FONT))
                        .line_height(px(LINE_H))
                        .text_color(t.text_2)
                        .children(lines.into_iter().map(|l| div().h(px(LINE_H)).flex_none().whitespace_nowrap().overflow_hidden().child(SharedString::from(l)))),
                )
            })
            .child(buttons)
            .when_some(note, |d, (text, color)| {
                d.child(
                    div()
                        .debug_selector(move || sel_note)
                        .h(px(NOTE_H))
                        .flex_none()
                        .flex()
                        .items_end()
                        .gap(px(5.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(12.5))
                        .text_color(color)
                        .when(sent && error.is_none(), |d| d.child(kit::icon("check", 13., color)))
                        .child(text),
                )
            })
    }

    /// Question `a`'s field for an answer of the user's own (Enter sends
    /// it, with what is picked).
    fn other_input(&mut self, a: &AskItem, cx: &mut Context<Self>) -> Entity<TextInput> {
        let key = a.key();
        if let Some((i, _)) = self.others.get(&key) {
            return i.clone();
        }
        let input = cx.new(|cx| TextInput::new(cx, "", "Or write your own answer…").max_len(pitwall_core::extensions::protocol::MAX_REPLY));
        let k = key.clone();
        let sub = cx.subscribe(&input, move |_, input, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Submit) {
                submit_own(&k, &input, cx);
            }
        });
        self.others.insert(key, (input.clone(), sub));
        input
    }

    /// An open question in an extension's inbox: where it is from (the
    /// extension and its connection, who asks, the thread), the question,
    /// its suggestions (a click sends one, or toggles it when it takes
    /// several), a field for an answer of the user's own, Send, Decline,
    /// Open in inbox, and a line saying it is on its way or why it didn't
    /// go. It says how tall it was drawn (`Float::ask_heights`): the
    /// window is sized to it.
    fn ask_card(a: &AskItem, input: Entity<TextInput>, width: f32, picked: Vec<u32>, note: Option<(SharedString, bool)>, sent: bool, t: &Theme) -> impl IntoElement {
        let key = a.key();
        let sel = |what: &str| format!("float-{what}-{}-{}", a.thread, a.ask.number);
        let place = match &a.conn {
            Some(c) => format!("{} · {c}", a.ext_name),
            None => a.ext_name.clone(),
        };
        let whence = match &a.from {
            Some(who) => format!("{who} · {}", a.title),
            None => a.title.clone(),
        };
        let head = div()
            .h(px(CARD_HEAD))
            .flex_none()
            .flex()
            .items_start()
            .gap(px(7.))
            .child(div().flex_none().h(px(18.)).flex().items_center().child(kit::icon(&a.icon, 14., t.text_2)))
            .child(div().flex_none().max_w(px(190.)).overflow_hidden().whitespace_nowrap().text_ellipsis().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(place)))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(t.text_3).child(SharedString::from(whence)))
            .when(a.ask.blocking, |d| {
                d.child(
                    div()
                        .flex_none()
                        .h(px(18.))
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(div().size(px(7.)).rounded_full().bg(t.amber))
                        .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(t.amber).child("Waiting on you")),
                )
            });
        let described = a.described();
        let options = a.ask.suggestions.iter().enumerate().map(|(i, s)| {
            let on = picked.contains(&(i as u32));
            let k = key.clone();
            let id = sel(&format!("pick-{i}"));
            let num = (i < 9).then(|| kit::kbd(&(i + 1).to_string(), t));
            let base = div()
                .id(SharedString::from(id.clone()))
                .debug_selector(move || id)
                .flex_none()
                .flex()
                .items_center()
                .gap(px(7.))
                .border_1()
                .border_color(if on { t.text } else { t.line_strong })
                .bg(if on { t.surface_3 } else { t.surface })
                .when(!sent, |d| d.cursor_pointer().hover(|d| d.bg(t.surface_2).border_color(t.text_3)))
                .when(sent, |d| d.opacity(0.5))
                .on_click(move |_, _, cx| pick_ask(&k, i as u32, cx));
            let label = div().flex_none().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.text).child(s.label.clone());
            let base = if described {
                base.h(px(OPTION_H)).px(px(10.)).rounded(px(7.)).children(num).child(label).child(
                    div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(12.)).text_color(t.text_3).child(s.description.clone()),
                )
            } else {
                base.h(px(CHIP_H)).px(px(10.)).rounded_full().children(num).child(label)
            };
            no_drag(hot(base.when(on, |d| d.child(kit::icon("check", 13., t.text))), format!("pick-{key}-{i}")))
        });
        let options = if described {
            div().flex().flex_col().gap(px(4.)).children(options).into_any_element()
        } else {
            div().flex().flex_wrap().gap(px(6.)).children(options).into_any_element()
        };
        let send_label = if a.ask.multiple && !picked.is_empty() { format!("Send {} picked", picked.len()) } else { "Send".to_string() };
        let (k_send, input_send) = (key.clone(), input.clone());
        let (open_a, decline_a) = (a.clone(), a.clone());
        let named = |what: &str| {
            let s = sel(what);
            move || s
        };
        let answer = div()
            .h(px(ANSWER_H))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(no_drag(
                div()
                    .id(SharedString::from(sel("own")))
                    .debug_selector(named("own"))
                    .flex_1()
                    .min_w(px(120.))
                    // The panel takes the keyboard for typing in it
                    // (Pitwall still doesn't come forward); the keys go
                    // here, not to an agent's terminal.
                    .capture_any_mouse_down(|_, window, cx| {
                        crate::platform::float_take_keys(window);
                        cx.global_mut::<Float>().keys = false;
                    })
                    .child(input),
            ))
            .child(no_drag(hot(
                kit::text_button(SharedString::from(sel("send")), send_label, kit::BtnKind::Primary, sent, t)
                    .debug_selector(named("send"))
                    .on_click(move |_, _, cx| submit_own(&k_send, &input_send, cx)),
                format!("send-{key}"),
            )))
            .child(no_drag(hot(
                kit::text_button(SharedString::from(sel("decline")), "Decline", kit::BtnKind::Link, sent, t)
                    .debug_selector(named("decline"))
                    .on_click(move |_, _, cx| {
                        send_ask(&decline_a, vec![], None, true, cx);
                    }),
                format!("decline-{key}"),
            )))
            .child(no_drag(hot(
                kit::text_button(SharedString::from(sel("open")), a.tile.as_ref().map_or_else(|| "Open in inbox".to_string(), |(_, title)| format!("Open {title}")), kit::BtnKind::Ghost, false, t)
                    .debug_selector(named("open"))
                    .on_click(move |_, _, cx| open_ask(&open_a, cx)),
                format!("open-{key}"),
            )));
        // How tall it is drawn, for the window's size.
        let measured = key.clone();
        let measure = canvas(
            move |b, _, cx| {
                let h = f32::from(b.size.height);
                let known = cx.try_global::<Float>().and_then(|f| f.ask_heights.get(&measured).copied());
                if known.is_none_or(|(w, k)| (w - width).abs() >= 0.5 || (k - h).abs() >= 0.5) {
                    let key = measured.clone();
                    cx.defer(move |cx| {
                        if cx.has_global::<Float>() {
                            cx.global_mut::<Float>().ask_heights.insert(key, (width, h));
                            sync(cx);
                        }
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .debug_selector(named("ask"))
            .relative()
            .flex_none()
            .px(px(12.))
            .py(px(GAP))
            .flex()
            .flex_col()
            .child(measure)
            .child(head)
            .child(
                div()
                    .mb(px(GAP))
                    .text_size(px(13.5))
                    .line_height(px(QUESTION_LINE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.text)
                    .child(a.ask.question.clone()),
            )
            .when(!a.ask.suggestions.is_empty(), |d| d.child(div().mb(px(GAP)).child(options)))
            .child(answer)
            .when_some(note, |d, (text, error)| {
                d.child(
                    div()
                        .debug_selector(named("ask-note"))
                        .h(px(NOTE_H))
                        .flex_none()
                        .flex()
                        .items_end()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(11.5))
                        .text_color(if error { t.red } else { t.text_3 })
                        .child(text),
                )
            })
    }
}

/// Send question `key`'s answer: its field's words, with what is picked.
fn submit_own(key: &str, input: &Entity<TextInput>, cx: &mut App) {
    let Some(a) = asks(cx).into_iter().find(|a| a.key() == key) else { return };
    let text = input.read(cx).text().to_string();
    let picks = cx.global::<Float>().picks.get(key).cloned().unwrap_or_default();
    if send_ask(&a, picks, Some(text), false, cx) {
        input.update(cx, |i, cx| i.set_text("", cx));
    }
}

impl Render for FloatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = items(cx);
        let questions = asks(cx);
        let entries = model::entries(&list, &questions);
        let count = entries.len();
        let expanded = cx.try_global::<Float>().is_some_and(|f| f.expanded) && count > 0;
        // Liquid Glass: the control is a slab of the system's own glass
        // over whatever the user is in; nothing of ours is painted behind
        // its content (terminals stay opaque).
        let liquid = theme::glass_regions(cx) && !self.preview;
        let mut t = theme::theme(cx).clone();
        if liquid {
            // Dim text is too dim on glass over a bright page: each step
            // of it one brighter.
            t.text_3 = t.text_2;
            t.text_2 = t.text;
        }
        let radius = if expanded { PANEL_RADIUS } else { PILL_RADIUS };
        // The theme's own surface, thinly, over the glass.
        let fill = liquid.then(|| t.raised.opacity(if expanded { GLASS_FILL_OPEN } else { GLASS_FILL }));
        let slab = crate::platform::glass::Slab { inset: PAD, radius, mode: t.mode };
        if !self.preview {
            crate::platform::glass::slab(window, (liquid && count > 0).then_some(slab));
        }
        if !self.preview && cx.try_global::<Float>().is_some_and(|f| f.grab) {
            cx.global_mut::<Float>().grab = false;
            window.focus(&self.focus, cx);
            crate::platform::float_take_keys(window);
        }
        let focus = self.focus.clone();
        let root = div()
            .id("float")
            .track_focus(&self.focus)
            .on_key_down(move |ev, window, cx| {
                // (Not while a question's field has the keys: typed there.)
                if focus.is_focused(window) && key(ev, cx) {
                    cx.stop_propagation();
                }
            })
            .size_full()
            .font_family(kit::UI_FONT)
            .text_size(px(13.))
            .line_height(gpui::relative(kit::BODY_LINE_HEIGHT))
            .text_color(t.text);
        let shape = cx.try_global::<Float>().map(shape).unwrap_or_default();
        let at = cx.try_global::<Float>().map_or(0, |f| model::position(&entries, f.current.as_deref()));
        let current = entries.get(at).copied().filter(|_| expanded);
        let show_all = cx.try_global::<Float>().is_some_and(|f| f.show_all);
        // The screen of the card shown, when it is this machine's agent.
        let agent = match current {
            Some(model::Entry::Agent(i)) => Some(i.clone()),
            _ => None,
        };
        let demo = cx.try_global::<Float>().map(|f| f.demo_screens.clone()).unwrap_or_default();
        self.watch(&agent.iter().filter(|i| i.local).map(|i| i.id.as_str()).collect::<Vec<_>>(), &demo);
        let few = agent.as_ref().is_some_and(|i| model::summary(i).is_some()) && !show_all;
        let (max_lines, fill_rows) = if few { (model::RAW_FEW, false) } else { (shape.max_lines(), shape.fill()) };
        let term = agent.as_ref().and_then(|i| self.term(i, max_lines, fill_rows));
        // The window is sized for the rows the card shows.
        let rows: HashMap<String, TermRows> = agent.iter().zip(&term).map(|(i, t)| (i.id.clone(), t.1)).collect();
        if cx.try_global::<Float>().is_some_and(|f| f.term_rows != rows) {
            cx.defer(move |cx| {
                cx.global_mut::<Float>().term_rows = rows;
                sync(cx);
            });
        }
        // All of the prompt's rows, for "Show all".
        let total = agent.as_ref().map_or(0, |i| model::lines_shown(i, shape, term.as_ref().map(|t| t.1), false));
        let term = term.map(|(s, _)| s);
        // The question shown, and its field.
        let ask = match current {
            Some(model::Entry::Ask(a)) => Some(a.clone()),
            _ => None,
        };
        self.others.retain(|k, _| ask.as_ref().is_some_and(|a| a.key() == *k));
        let input = ask.as_ref().map(|a| self.other_input(a, cx));
        let Some(f) = cx.try_global::<Float>() else { return root };
        if count == 0 {
            return root;
        }
        // Its first appearance slides in, a little, once.
        let arrive = |d: Div| -> gpui::AnyElement {
            if !kit::motion::on() {
                return d.into_any_element();
            }
            d.with_animation(
                ElementId::Name(format!("float-in-{}", f.shown_count).into()),
                Animation::new(Duration::from_millis(240)).with_easing(gpui::ease_out_quint()),
                |d, k| d.opacity(k).top(px((1. - k) * 10.)),
            )
            .into_any_element()
        };
        if !expanded {
            return root.child(arrive(div().relative().size_full().child(Self::pill(&entries, f, fill, &t))));
        }
        let rule = || div().h(px(RULE)).flex_none().bg(t.line);
        let card = match (current, input) {
            (Some(model::Entry::Ask(a)), Some(input)) => {
                let picked = f.picks.get(&a.key()).cloned().unwrap_or_default();
                let sent = f.sent.contains_key(&a.key()) || a.busy;
                Self::ask_card(a, input, shape.width, picked, ask_note(f, a), sent, &t).into_any_element()
            }
            (Some(model::Entry::Agent(item)), _) => {
                let scrolled = item.clone();
                let on_scroll = cx.listener(move |this, ev: &ScrollEvent, _, cx| {
                    // The wheel moves the content: down shows what is above.
                    let dy = -f32::from(ev.delta.pixel_delta(px(LINE_H)).y);
                    this.scroll(&scrolled, max_lines, fill_rows, dy, cx);
                });
                Self::card(item, term, total, on_scroll, self.focus.clone(), f, &t).into_any_element()
            }
            _ => div().into_any_element(),
        };
        let panel = div()
            .id("float-panel")
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(PANEL_RADIUS))
            .bg(fill.unwrap_or(t.raised))
            .border_1()
            .border_color(t.line_strong)
            .child(Self::header(&entries, at, f, &t))
            .child(rule())
            .child(card)
            .child(div().flex_1())
            .child(rule())
            .child(Self::hints(entries[at], count, &t));
        // The edges that resize it, over the panel's (laid out for the
        // window's size now: the panel fills it).
        let view = window.viewport_size();
        let (w, h) = if self.preview { f.size } else { (f32::from(view.width), f32::from(view.height)) };
        // The panel's own shadow, in the clear space around it.
        root.relative()
            .p(px(PAD))
            .child(arrive(div().relative().size_full().rounded(px(PANEL_RADIUS)).shadow(shadow(&t, liquid)).child(drag_handle(panel))))
            .children(Edge::ALL.map(|e| edge_zone(e, w, h)))
            // Where the platform doesn't follow the pointer itself.
            .children((!self.preview).then(|| canvas(|_, _, _| {}, |_, _, window, _| follow_resize(window)).absolute().size_0()))
    }
}

/// Bench (`float preview`): the control's view, as it would be drawn in
/// its window, over the top left of the main window on a plain backdrop,
/// to capture with the main window. No window of its own is opened.
pub fn bench_preview(window: &mut Window, cx: &mut App) -> Option<gpui::AnyElement> {
    if !crate::bench::enabled() || !cx.try_global::<Float>().is_some_and(|f| f.bench_shown) {
        return None;
    }
    let view = match cx.global::<Float>().preview.clone() {
        Some(v) => v,
        None => {
            let v = cx.new(|cx| {
                let mut v = FloatView::new(window, cx);
                v.preview = true;
                v
            });
            cx.global_mut::<Float>().preview = Some(v.clone());
            v
        }
    };
    let (w, h) = cx.global::<Float>().size;
    let backdrop = gpui::hsla(0., 0., 0.5, 1.);
    Some(
        div()
            .id("float-preview")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .bg(backdrop)
            .child(div().absolute().top_0().left_0().w(px(w)).h(px(h)).child(view))
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests;
