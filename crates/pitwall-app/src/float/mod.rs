//! The floating "needs you" control (Settings → General, `general.floatingPrompts`).
//!
//! While Pitwall isn't in front and an agent's screen asks the user
//! something, a small window floats above the other apps, on every desktop
//! and over full-screen apps (a GPUI pop-up: on macOS a panel that takes
//! clicks without bringing Pitwall forward). It starts as the Pitwall mark
//! with the count on it; a click opens a card per agent, all at once, each
//! with the part of its terminal that holds the prompt, drawn live as
//! the Wall draws a tile (its text, as the rules cut it out of the screen,
//! until the screen arrives and for another machine's agent), and
//!
//! - **Accept** / **Reject** when the prompt is a plain yes/no Pitwall has
//!   the keys for (`pitwall-detect`'s `[[answers]]`, per agent kind). The
//!   engine reads the screen again and types only if the same prompt is
//!   still there (`engine::input::answer`). **Accept all** in the header
//!   does it for every card that has an Accept;
//! - the keyboard, after a click on the card's terminal: arrows, Enter,
//!   Esc and text go to that agent as they would in its pane (the panel
//!   takes the keyboard for that; Pitwall still doesn't come forward);
//! - **Open** for everything else (a question with options, a trust
//!   dialog, another machine's agent): Pitwall comes forward on the agent.
//!
//! It can be dragged anywhere (by the mark, or by any part of the open
//! panel that isn't a button) and stays there, also the next time Pitwall
//! starts (`app-float.json` in the data folder). It goes away when nothing asks,
//! when Pitwall comes forward, and when the setting is off. Nothing here
//! polls while no agent is blocked: the check for "is Pitwall in front"
//! runs only while one is.

pub mod model;

use std::cell::Cell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc::{unbounded, UnboundedSender};
use futures::StreamExt;

use gpui::{
    canvas, div, point, prelude::*, px, size, App, AsyncApp, Bounds, Context, CursorStyle, DispatchPhase, Div, Entity,
    FocusHandle, FontWeight, Global, IntoElement, KeyDownEvent, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    Render, ScrollEvent, SharedString, Stateful, Task, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle,
    WindowKind, WindowOptions,
};

use crate::agents::AgentStore;
use crate::kit;
use crate::platform::decorations::no_drag;
use crate::platform::FloatCursor;
use crate::theme::{self, Theme};
use crate::wall::paint::{screen_view, term_colors, ScreenOpts};
use crate::wall::screen::Screen;
use crate::wall::source::{forward, FrameMsg, ScreenSource, Screens};
use model::{
    Item, Shape, TermRows, BOX_PAD, BUTTONS_H, CARD_HEAD, GAP, HEADER_H, LINE_H, MAX_CARDS, MORE_H, NOTE_H, PAD, PUCK, RULE,
    TERM_FONT,
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

/// The control's state (one per app).
pub struct Float {
    store: Entity<AgentStore>,
    answer: Answerer,
    typer: Typer,
    open: Opener,
    window: Option<WindowHandle<FloatView>>,
    /// The agent whose terminal the keyboard goes to (the user clicked it).
    typing: Option<String>,
    /// The cards are open (else the mark alone).
    expanded: bool,
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
}

impl Global for Float {}

/// Which edge of the open panel is being dragged: the one across from
/// the side it is anchored on (its left), the one across from the end it
/// is anchored at (its top on macOS, its bottom elsewhere), or the corner
/// between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Side,
    End,
    Corner,
}

impl Edge {
    fn resizes_width(self) -> bool {
        self != Edge::End
    }

    fn resizes_rows(self) -> bool {
        self != Edge::Side
    }

    fn cursor(self) -> FloatCursor {
        match self {
            Edge::Side => FloatCursor::ResizeLeftRight,
            Edge::End => FloatCursor::ResizeUpDown,
            Edge::Corner => FloatCursor::ResizeDiagonal,
        }
    }

    /// GPUI's pointer for it (where GPUI's pointers apply: not macOS).
    fn style(self) -> CursorStyle {
        match self {
            Edge::Side => CursorStyle::ResizeLeftRight,
            Edge::End => CursorStyle::ResizeUpDown,
            // The corner's diagonal: top left to bottom right on macOS,
            // bottom left to top right elsewhere.
            Edge::Corner if GROWS_UP => CursorStyle::ResizeUpLeftDownRight,
            Edge::Corner => CursorStyle::ResizeUpRightDownLeft,
        }
    }
}

/// A resize by the pointer: what was dragged, and the size when it began.
#[derive(Debug, Clone, Copy)]
struct Resize {
    edge: Edge,
    width: f32,
    /// Each card's rows, the agents asking and the cards shown.
    per_card: usize,
    count: usize,
    cards: usize,
    /// Where the press was on the screen, when the window's own mouse
    /// events are followed (not macOS: see [`crate::platform::float_resize`]).
    from: Option<Point<Pixels>>,
    /// A width that waits for the release (see [`LIVE_WIDTH`]).
    pending: Option<f32>,
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

/// Set the control up for `store`'s agents. `answer` types into an agent
/// (the engine) if it still asks the same, `typer` types keys into its
/// terminal, `open` shows one in Pitwall.
pub fn init(store: &Entity<AgentStore>, answer: Answerer, typer: Typer, open: Opener, cx: &mut App) {
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
        window: None,
        typing: None,
        expanded: false,
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
    });
    cx.observe(store, |_, cx| sync(cx)).detach();
    let ui = crate::ui_state::store(cx);
    cx.observe(&ui, |_, cx| sync(cx)).detach();
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
    let display = anchor
        .and_then(|a| cx.displays().into_iter().find(|d| d.bounds().contains(&a)))
        .or_else(|| cx.primary_display())?;
    let area = display.visible_bounds();
    let corner = anchor.unwrap_or_else(|| {
        let y = if GROWS_UP { area.origin.y + area.size.height - px(MARGIN) } else { area.origin.y + px(MARGIN) };
        point(area.origin.x + area.size.width - px(MARGIN), y)
    });
    Some((corner, area))
}

/// Start resizing the open panel by `edge`.
fn begin_resize(edge: Edge, cx: &mut App) {
    let list = items(cx);
    let f = cx.global_mut::<Float>();
    if !f.expanded || list.is_empty() {
        return;
    }
    let s = shape(f);
    f.resize = Some(Resize {
        edge,
        width: s.width,
        per_card: s.max_lines(list.len()),
        count: list.len(),
        cards: model::shown(&list).len().max(1),
        from: None,
        pending: None,
    });
    log(|| format!("resize by {edge:?} starts at {s:?}"));
    crate::platform::float_cursor(edge.cursor());
}

/// The pointer is `dx`, `dy` points (right and down are positive) from
/// where the resize began: the panel takes the size that puts its edge
/// under it. Its width moves with the pointer; its height in whole rows,
/// a row per [`LINE_H`] shared by the cards shown. Kept within the
/// bounds of [`model`] and so that the window fits between its anchored
/// corner and the screen's far edges.
fn resize_to(dx: f32, dy: f32, cx: &mut App) {
    let Some(r) = cx.global::<Float>().resize else { return };
    let room = screen_area(cx).map(|(corner, area)| {
        let w = f32::from(corner.x - area.origin.x);
        let h = if GROWS_UP { f32::from(corner.y - area.origin.y) } else { f32::from(area.origin.y + area.size.height - corner.y) };
        (w - 2. * PAD, h - 2. * PAD)
    });
    let list = items(cx);
    let f = cx.global_mut::<Float>();
    if r.edge.resizes_width() {
        let most = room.map_or(model::MAX_WIDTH, |(w, _)| w.min(model::MAX_WIDTH));
        // The left edge: moving it left makes the panel wider.
        let w = (r.width - dx).min(most).max(model::MIN_WIDTH).round();
        if w != r.width || f.width.is_some() {
            if LIVE_WIDTH {
                f.width = Some(w);
            } else if let Some(r) = f.resize.as_mut() {
                r.pending = Some(w);
            }
        }
    }
    if r.edge.resizes_rows() {
        // The top edge on macOS (up makes it taller), the bottom elsewhere.
        let grow = if GROWS_UP { -dy } else { dy };
        let by = (grow / LINE_H / r.cards as f32).round() as isize;
        if by != 0 || f.rows.is_some() {
            let per = (r.per_card as isize + by).clamp(model::MIN_ROWS as isize, model::MAX_ROWS as isize) as usize;
            let mut alone = model::alone_for(r.count, per).clamp(model::MIN_ROWS, model::MAX_ROWS);
            // No taller than the screen has room for.
            let fits = |alone: usize, f: &Float| {
                let s = Shape { rows: Some(alone), ..shape(f) };
                let h = model::content_size(&list, true, s, &|id| f.errors.contains_key(id), &|id| f.term_rows.get(id).copied()).1;
                room.is_none_or(|(_, room)| h <= room)
            };
            while alone > model::MIN_ROWS && !fits(alone, f) {
                alone -= 1;
            }
            f.rows = Some(alone);
        }
    }
    sync_later(cx);
}

/// The resize is over: the size is kept, and the pointer shows what it is
/// over again.
fn end_resize(cx: &mut App) {
    let f = cx.global_mut::<Float>();
    let Some(r) = f.resize.take() else { return };
    if let Some(w) = r.pending {
        f.width = Some(w);
    }
    log(|| format!("resize ends at {:?}", shape(f)));
    crate::platform::float_cursor(cursor_now(f));
    save_place(cx);
    sync_later(cx);
}

/// Back to the default size (a double click on an edge).
fn reset_size(cx: &mut App) {
    let f = cx.global_mut::<Float>();
    f.resize = None;
    f.width = None;
    f.rows = None;
    log(|| "size reset".to_string());
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
    cx.try_global::<Float>().map(|f| model::items(&f.store.read(cx).agents)).unwrap_or_default()
}

/// Whether the control should be up: on, something asks, and the user
/// isn't in Pitwall.
fn wanted(items: &[Item], cx: &App) -> bool {
    !items.is_empty() && enabled(cx) && !crate::bench::enabled() && !in_front(cx)
}

/// Tests say whether the user is in Pitwall.
#[cfg(test)]
pub(crate) struct InFront(pub bool);
#[cfg(test)]
impl Global for InFront {}

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
    let want = wanted(&list, cx);
    let f = cx.global_mut::<Float>();
    // What was answered or failed belongs to a prompt that is gone.
    let asks = |id: &str| list.iter().find(|i| i.id == id).map(|i| i.ask.text.as_str());
    f.answered.retain(|id, seen| asks(id) == Some(seen.as_str()));
    f.errors.retain(|id, _| asks(id).is_some());
    f.term_rows.retain(|id, _| asks(id).is_some());
    if f.typing.as_deref().is_some_and(|id| asks(id).is_none()) || !f.expanded {
        f.typing = None;
    }
    if list.is_empty() {
        f.expanded = false;
    }
    // Nothing open to resize (the cards were closed under the pointer).
    if !f.expanded && f.resize.is_some() {
        f.resize = None;
        crate::platform::float_cursor(FloatCursor::Arrow);
    }
    // Watch who is in front only while it can matter.
    let watching = !list.is_empty() && enabled(cx);
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
                format!("closed: {} asking, enabled {on}, pitwall in front {front} (system says active {raw})", list.len())
            });
        }
        return;
    }
    let new = model::size(&list, f.expanded, shape(f), &|id| f.errors.contains_key(id), &|id| f.term_rows.get(id).copied());
    let old = std::mem::replace(&mut f.size, new);
    match f.window {
        // A window that isn't resized from the corner it sits on is opened
        // again in its new place when its width changes (the mark ↔ the
        // cards); a change of height alone keeps its top where it is.
        Some(w) if old.0 != new.0 && !GROWS_UP && !cfg!(test) => {
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
                if old != new {
                    crate::platform::resize_float(window, size(px(new.0), px(new.1)), cx);
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
    if cx.try_global::<Float>().is_none_or(|f| f.window.is_some()) || !wanted(&items(cx), cx) {
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

/// Open or close the cards.
fn toggle(cx: &mut App) {
    // Opened: the keyboard goes to the first card's terminal right away,
    // so arrows and Enter answer it with no click in between.
    let first = model::shown(&items(cx)).iter().find(|i| i.local).map(|i| i.id.clone());
    let f = cx.global_mut::<Float>();
    f.expanded = !f.expanded;
    let open = f.expanded;
    f.typing = first.filter(|_| open);
    f.grab = open && f.typing.is_some();
    log(|| format!("toggle: cards {}", if open { "open" } else { "closed" }));
    sync_later(cx);
}

/// Type the answer into `id`'s terminal, if it still shows the prompt the
/// user is looking at.
fn answer(id: &str, accept: bool, cx: &mut App) {
    let list = items(cx);
    let Some(item) = list.iter().find(|i| i.id == id) else { return };
    log(|| format!("{} pressed for {} (yes/no {}, no key {})", if accept { "accept" } else { "reject" }, item.name, item.can_accept(), item.can_reject()));
    if !(if accept { item.can_accept() } else { item.can_reject() }) {
        return;
    }
    let seen = item.ask.text.clone();
    let f = cx.global_mut::<Float>();
    if f.answered.contains_key(id) {
        return;
    }
    match (f.answer)(id, accept, &seen) {
        Ok(()) => {
            f.errors.remove(id);
            f.answered.insert(id.to_string(), seen);
        }
        Err(e) => {
            log(|| format!("not sent: {e}"));
            f.errors.insert(id.to_string(), format!("Not sent: {e}.").into());
        }
    }
    sync_later(cx);
}

/// The keyboard goes to `id`'s terminal from now on.
fn type_into(id: &str, cx: &mut App) {
    let Some(item) = items(cx).into_iter().find(|i| i.id == id && i.local) else { return };
    log(|| format!("keys go to {}", item.name));
    cx.global_mut::<Float>().typing = Some(item.id);
}

/// A key pressed while a card's terminal has the keyboard: sent to its
/// agent as its own pane would send it. `true` when it was.
fn key(ev: &KeyDownEvent, cx: &mut App) -> bool {
    use pitwall_term_view::keys::{encode_key, KeyModes, KeyPress, Mods};
    let f = cx.global::<Float>();
    let Some(id) = f.typing.clone() else { return false };
    let ks = &ev.keystroke;
    let mods = Mods { shift: ks.modifiers.shift, alt: ks.modifiers.alt, ctrl: ks.modifiers.control, cmd: ks.modifiers.platform };
    let press = KeyPress { key: ks.key.as_str(), text: ks.key_char.as_deref(), mods };
    // Plain text isn't a terminal key: it is typed as it is.
    let bytes = encode_key(press, KeyModes::default())
        .or_else(|| ks.key_char.as_ref().filter(|_| !mods.cmd && !mods.ctrl).map(|t| t.as_bytes().to_vec()));
    let Some(bytes) = bytes else { return false };
    if let Err(e) = (f.typer)(&id, &bytes) {
        log(|| format!("key not sent: {e}"));
        cx.global_mut::<Float>().errors.insert(id, format!("Not sent: {e}.").into());
        sync_later(cx);
    }
    true
}

/// Accept every card that has an Accept.
fn accept_all(cx: &mut App) {
    let list = items(cx);
    for id in model::acceptable(&list).into_iter().map(|i| i.id.clone()) {
        answer(&id, true, cx);
    }
}

/// Bring Pitwall forward on `id` (the control then goes away).
fn open(id: &str, cx: &mut App) {
    let opener = cx.global::<Float>().open.clone();
    opener(id, cx);
}

/// Debug builds: `PITWALL_DEBUG_PROMPTS=expand[,accept|reject|open|all|
/// click-puck|click-collapse|click-accept|resize-wide|resize-narrow|
/// resize-tall|resize-short|resize-corner|resize-reset]` does those to the
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
                    // Real clicks (macOS): the mark, the header's
                    // collapse button, the first card's Accept.
                    "click-puck" => debug_click(PAD + PUCK / 2., PAD + PUCK / 2., cx),
                    "click-collapse" => {
                        let width = shape(cx.global::<Float>()).width;
                        debug_click(PAD + width - 20., PAD + HEADER_H / 2., cx)
                    }
                    "resize-reset" => reset_size(cx),
                    "click-accept" => {
                        let list = items(cx);
                        let f = cx.global::<Float>();
                        let rows = list.first().map_or(0, |i| model::lines_shown(i, list.len(), shape(f), f.term_rows.get(&i.id).copied()));
                        let y = HEADER_H + RULE + GAP + CARD_HEAD + rows as f32 * LINE_H + BOX_PAD + GAP + BUTTONS_H / 2.;
                        debug_click(PAD + 40., PAD + y, cx);
                    }
                    _ => {}
                }
            });
            cx.background_executor().timer(step).await;
            cx.update(|cx| {
                let list = items(cx);
                let f = cx.global::<Float>();
                eprintln!(
                    "pitwall-debug: prompts after {verb}: up {}, expanded {}, size {:?}, shape {:?}, anchor {:?}, agents {:?}, answered {:?}, errors {:?}",
                    f.window.is_some(),
                    f.expanded,
                    f.size,
                    shape(f),
                    f.anchor,
                    list.iter().map(|i| (i.name.as_str(), i.can_accept(), i.can_reject())).collect::<Vec<_>>(),
                    f.answered.keys().collect::<Vec<_>>(),
                    f.errors
                );
            });
        }
    })
    .detach();
}

/// The `resize-` verbs of [`debug_hook`]: the edge, and how far the
/// pointer goes (right and down are positive).
fn debug_resize(verb: &str, cx: &App) -> Option<(Edge, f32, f32)> {
    let cards = model::shown(&items(cx)).len().max(1) as f32;
    // Up is taller on macOS (its top edge), down elsewhere.
    let taller = if GROWS_UP { -1. } else { 1. };
    Some(match verb {
        "resize-wide" => (Edge::Side, -200., 0.),
        "resize-narrow" => (Edge::Side, 150., 0.),
        "resize-tall" => (Edge::End, 0., taller * 6. * LINE_H * cards),
        "resize-short" => (Edge::End, 0., -taller * 8. * LINE_H * cards),
        "resize-corner" => (Edge::Corner, -120., taller * 4. * LINE_H * cards),
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
    // The edge across from the anchored end: the top on macOS.
    let end = if GROWS_UP { top } else { bottom };
    let (x, y, zw, zh) = match edge {
        Edge::Side if GROWS_UP => (left - half, top + corner, EDGE, bottom - top - corner),
        Edge::Side => (left - half, top, EDGE, bottom - top - corner),
        Edge::End => (left + corner, end - half, right - left - corner, EDGE),
        Edge::Corner => (left - corner, end - corner, CORNER, CORNER),
    };
    let name = match edge {
        Edge::Side => "float-resize-side",
        Edge::End => "float-resize-end",
        Edge::Corner => "float-resize-corner",
    };
    hot_as(
        div()
            .id(name)
            .debug_selector(move || name.into())
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(zw))
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

/// Corners of the tile and of the opened panel.
const PUCK_RADIUS: f32 = 13.;
const PANEL_RADIUS: f32 = 12.;

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
    _pump: Task<()>,
    _subs: [gpui::Subscription; 2],
}

impl Drop for FloatView {
    fn drop(&mut self) {
        self.watch(&[]);
    }
}

impl FloatView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Wherever the user drags it is where it opens and resizes from.
        let moved = cx.observe_window_bounds(window, |_, window, cx| {
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
            _pump: pump,
            _subs: [cx.observe_global::<Theme>(|_, cx| cx.notify()), moved],
        }
    }

    /// Watch the screens of exactly `ids`.
    fn watch(&mut self, ids: &[&str]) {
        let Some(source) = self.source.clone() else { return };
        self.watched.retain(|id, w| {
            let keep = ids.contains(&id.as_str());
            if let (false, Some(wid)) = (keep, w.id) {
                source.unwatch(id, wid);
            }
            keep
        });
        for id in ids {
            if !self.watched.contains_key(*id) {
                self.tokens += 1;
                let sink = forward(self.frames.clone(), id.to_string(), self.tokens);
                let watch = source.watch(id, sink).ok();
                self.watched.insert(id.to_string(), Watched { id: watch, token: self.tokens, screen: Screen::default() });
            }
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

    /// Collapsed: a tile with the Pitwall mark, and how many agents ask on
    /// its corner.
    fn puck(count: usize, fill: Option<gpui::Hsla>, t: &Theme) -> impl IntoElement {
        let tile = div()
            .id("float-puck")
            .debug_selector(|| "float-puck".into())
            .absolute()
            .left(px(PAD))
            .top(px(PAD))
            .size(px(PUCK))
            .rounded(px(PUCK_RADIUS))
            .bg(fill.unwrap_or(t.raised))
            .shadow(shadow(t, fill.is_some()))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|d| d.bg(t.surface_3))
            .on_click(|_, _, cx| {
                if !DRAGGED.replace(false) {
                    toggle(cx);
                }
            })
            .child(kit::brand_mark(t));
        // The count sits on the tile's corner, half outside it.
        let badge = div()
            .absolute()
            .left(px(PAD + PUCK - 12.))
            .top(px(PAD - 7.))
            .h(px(19.))
            .min_w(px(19.))
            .px(px(5.))
            .rounded_full()
            .bg(t.amber)
            // A dark rim, so it reads as a badge on the tile over anything.
            .shadow(vec![gpui::BoxShadow { color: gpui::black().opacity(0.55), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.5), inset: false }])
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(11.))
            .font_weight(FontWeight::BOLD)
            .text_color(gpui::rgb(0x1a1200))
            .child(SharedString::from(count.to_string()));
        div().relative().size_full().child(hot(drag_handle(tile), "puck")).child(badge)
    }

    /// Open: the mark, the count, "Accept all" when several can be, and
    /// the button that closes the cards.
    fn header(list: &[Item], f: &Float, t: &Theme) -> impl IntoElement {
        let several = model::acceptable(list).len() >= 2;
        let pending = model::acceptable(list).iter().any(|i| !f.answered.contains_key(&i.id));
        let collapse = div()
            .id("float-collapse")
            .debug_selector(|| "float-collapse".into())
            .flex_none()
            .size(px(24.))
            .rounded_full()
            .bg(t.surface_2)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|d| d.bg(t.surface_3))
            .on_click(|_, _, cx| toggle(cx))
            // Minimize: back to the tile.
            .child(div().w(px(10.)).h(px(1.5)).rounded_full().bg(t.text_2));
        div()
            .id("float-header")
            .debug_selector(|| "float-header".into())
            .h(px(HEADER_H))
            .flex_none()
            .pl(px(12.))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(9.))
            .child(kit::brand_mark(t))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(SharedString::from(model::title(list))),
            )
            .when(several, |d| {
                d.child(no_drag(hot(
                    div()
                        .id("float-accept-all")
                        .debug_selector(|| "float-accept-all".into())
                        .flex_none()
                        .h(px(24.))
                        .px(px(9.))
                        .rounded(kit::button::btn_radius(t))
                        .flex()
                        .items_center()
                        .border_1()
                        .border_color(t.amber.opacity(0.7))
                        .text_size(px(12.))
                        .font_weight(kit::WEIGHT_550)
                        .text_color(t.text)
                        .when(!pending, |d| d.opacity(0.45))
                        .when(pending, |d| d.cursor_pointer().hover(|d| d.bg(t.amber_soft)))
                        .on_click(|_, _, cx| accept_all(cx))
                        .child("Accept all"),
                    "accept-all",
                )))
            })
            .child(no_drag(hot(collapse, "collapse")))
    }

    /// One agent: who it is and what it wants, its prompt (`term`: as its
    /// terminal shows it; else its text), the buttons, and a line of help
    /// or of an error.
    fn card(
        item: &Item,
        max_lines: usize,
        term: Option<Screen>,
        on_scroll: impl Fn(&ScrollEvent, &mut Window, &mut App) + 'static,
        focus: FocusHandle,
        f: &Float,
        t: &Theme,
    ) -> impl IntoElement {
        let lines = if term.is_some() { Vec::new() } else { item.lines(max_lines, shape(f).cols()) };
        let sel_term = format!("float-term-{}", item.id);
        // The keyboard goes to this card's terminal.
        let typing = f.typing.as_deref() == Some(item.id.as_str());
        let (local, term_id) = (item.local, item.id.clone());
        let error = f.errors.get(&item.id).cloned();
        let sent = f.answered.contains_key(&item.id);
        let note: Option<(SharedString, gpui::Hsla)> = match (&error, item.can_accept()) {
            (Some(e), _) => Some((e.clone(), t.red)),
            (None, false) if !item.local => Some(("On another machine: answer it there, or open it to look.".into(), t.text_3)),
            (None, false) => Some(("More than a yes or no: open the agent to answer.".into(), t.text_3)),
            (None, true) => None,
        };
        let key = |what: &str| format!("float-{what}-{}", item.id);
        let (accept_id, reject_id, open_id) = (item.id.clone(), item.id.clone(), item.id.clone());
        let (sel_accept, sel_reject, sel_open, sel_prompt, sel_note, sel_card) =
            (key("accept"), key("reject"), key("open"), key("prompt"), key("note"), key("card"));
        let head = div()
            .h(px(CARD_HEAD))
            .flex_none()
            .flex()
            .items_start()
            .gap(px(7.))
            .child(div().flex_none().h(px(18.)).flex().items_center().child(kit::agent_logo(&item.kind, &item.kind_name, 14., t)))
            .child(div().flex_none().max_w(px(190.)).overflow_hidden().whitespace_nowrap().text_ellipsis().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(item.name.clone())))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(t.text_3)
                    .child(SharedString::from(item.detail.clone().unwrap_or_default())),
            );
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
            .when(item.can_reject(), |d| {
                d.child(no_drag(hot(
                    kit::text_button(SharedString::from(sel_reject.clone()), "Reject", kit::BtnKind::Ghost, sent, t)
                        .debug_selector(move || sel_reject)
                        .on_click(move |_, _, cx| answer(&reject_id, false, cx)),
                    key("reject"),
                )))
            })
            .child(div().flex_1())
            .child(no_drag(hot(
                kit::text_button(
                    SharedString::from(sel_open.clone()),
                    "Open",
                    if item.can_accept() { kit::BtnKind::Ghost } else { kit::BtnKind::Primary },
                    false,
                    t,
                )
                .debug_selector(move || sel_open)
                .on_click(move |_, _, cx| open(&open_id, cx)),
                key("open"),
            )));
        div()
            .debug_selector(move || sel_card)
            .flex_none()
            .px(px(12.))
            .py(px(GAP))
            .flex()
            .flex_col()
            .child(head)
            .when_some(term, |d, screen| {
                // The terminal at its own size, cut off on the right: the
                // inner box is wider than any screen, so nothing is scaled.
                d.child(
                    div()
                        .id(SharedString::from(sel_term.clone()))
                        .debug_selector(move || sel_term)
                        .on_scroll(on_scroll)
                        // A click (not a drag) gives it the keyboard.
                        .when(local, |d| {
                            d.on_click(move |_, window, cx| {
                                if DRAGGED.replace(false) {
                                    return;
                                }
                                window.focus(&focus, cx);
                                crate::platform::float_take_keys(window);
                                type_into(&term_id, cx);
                                cx.refresh_windows();
                            })
                        })
                        .border_1()
                        .border_color(if typing { t.focus } else { gpui::transparent_black() })
                        .relative()
                        .h(px(f32::from(screen.rows) * LINE_H + BOX_PAD))
                        .mb(px(GAP))
                        .flex_none()
                        .rounded(px(6.))
                        .bg(t.term_bg)
                        .overflow_hidden()
                        .child(div().absolute().top_0().left_0().h_full().w(px(4000.)).child(screen_view(
                            &screen,
                            ScreenOpts { font_size: TERM_FONT, colors: term_colors(t.mode), show_cursor: typing },
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
                        .rounded(px(6.))
                        .bg(t.term_bg)
                        .overflow_hidden()
                        .font_family(kit::MONO_FONT)
                        .text_size(px(TERM_FONT))
                        .line_height(px(LINE_H))
                        .text_color(t.text_2)
                        .children(lines.into_iter().map(|l| {
                            div().h(px(LINE_H)).flex_none().whitespace_nowrap().overflow_hidden().child(SharedString::from(l))
                        })),
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
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(11.5))
                        .text_color(color)
                        .child(text),
                )
            })
    }

    /// "+2 more in Pitwall": opens the first agent that has no card.
    fn more(list: &[Item], t: &Theme) -> Stateful<Div> {
        let first = list[MAX_CARDS].id.clone();
        no_drag(hot(
            div()
                .id("float-more")
                .debug_selector(|| "float-more".into())
                .h(px(MORE_H))
                .flex_none()
                .px(px(12.))
                .flex()
                .items_center()
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(t.text_3)
                .hover(|d| d.bg(t.surface_3))
                .on_click(move |_, _, cx| open(&first, cx))
                .child(SharedString::from(format!("+{} more in Pitwall", list.len() - MAX_CARDS))),
            "more",
        ))
    }
}

impl Render for FloatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = items(cx);
        let expanded = cx.try_global::<Float>().is_some_and(|f| f.expanded) && !list.is_empty();
        // Liquid Glass: the control is a slab of the system's own glass
        // over whatever the user is in; nothing of ours is painted behind
        // its content (terminals stay opaque).
        let liquid = theme::glass_regions(cx);
        let mut t = theme::theme(cx).clone();
        if liquid {
            // Dim text is too dim on glass over a bright page: each step
            // of it one brighter.
            t.text_3 = t.text_2;
            t.text_2 = t.text;
        }
        let radius = if expanded { PANEL_RADIUS } else { PUCK_RADIUS };
        // The theme's own surface, thinly, over the glass.
        let fill = liquid.then(|| t.raised.opacity(if expanded { GLASS_FILL_OPEN } else { GLASS_FILL }));
        let slab = crate::platform::glass::Slab { inset: PAD, radius, mode: t.mode };
        crate::platform::glass::slab(window, (liquid && !list.is_empty()).then_some(slab));
        if cx.try_global::<Float>().is_some_and(|f| f.grab) {
            cx.global_mut::<Float>().grab = false;
            window.focus(&self.focus, cx);
            crate::platform::float_take_keys(window);
        }
        let root = div()
            .id("float")
            .track_focus(&self.focus)
            .on_key_down(|ev, _, cx| {
                if key(ev, cx) {
                    cx.stop_propagation();
                }
            })
            .size_full()
            .font_family(kit::UI_FONT)
            .text_size(px(13.))
            .line_height(gpui::relative(kit::BODY_LINE_HEIGHT))
            .text_color(t.text);
        let shape = cx.try_global::<Float>().map(shape).unwrap_or_default();
        let (max_lines, fill_rows) = (shape.max_lines(list.len()), shape.fill());
        // The screens of this machine's agents that have a card.
        let cards = if expanded { model::shown(&list) } else { &[] };
        self.watch(&cards.iter().filter(|i| i.local).map(|i| i.id.as_str()).collect::<Vec<_>>());
        let terms: Vec<Option<(Screen, TermRows)>> = cards.iter().map(|i| self.term(i, max_lines, fill_rows)).collect();
        // The window is sized for the rows the cards show.
        let rows: HashMap<String, TermRows> = cards.iter().zip(&terms).filter_map(|(i, t)| Some((i.id.clone(), t.as_ref()?.1))).collect();
        let terms = terms.into_iter().map(|t| t.map(|(s, _)| s));
        if cx.try_global::<Float>().is_some_and(|f| f.term_rows != rows) {
            cx.defer(move |cx| {
                cx.global_mut::<Float>().term_rows = rows;
                sync(cx);
            });
        }
        let Some(f) = cx.try_global::<Float>() else { return root };
        if list.is_empty() {
            return root;
        }
        if !expanded {
            return root.child(Self::puck(list.len(), fill, &t));
        }
        let rule = || div().h(px(RULE)).flex_none().bg(t.line);
        let mut panel = div()
            .id("float-panel")
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(PANEL_RADIUS))
            .bg(fill.unwrap_or(t.raised))
            .border_1()
            .border_color(t.line_strong)
            .child(Self::header(&list, f, &t));
        for (item, term) in cards.iter().zip(terms) {
            let scrolled = item.clone();
            let on_scroll = cx.listener(move |this, ev: &ScrollEvent, _, cx| {
                // The wheel moves the content: down shows what is above.
                let dy = -f32::from(ev.delta.pixel_delta(px(LINE_H)).y);
                this.scroll(&scrolled, max_lines, fill_rows, dy, cx);
            });
            panel = panel.child(rule()).child(Self::card(item, max_lines, term, on_scroll, self.focus.clone(), f, &t));
        }
        if list.len() > MAX_CARDS {
            panel = panel.child(rule()).child(Self::more(&list, &t));
        }
        // The edges that resize it, over the panel's (laid out for the
        // window's size now: the panel fills it).
        let view = window.viewport_size();
        let (w, h) = (f32::from(view.width), f32::from(view.height));
        // The panel's own shadow, in the clear space around it.
        root.relative()
            .p(px(PAD))
            .child(div().size_full().rounded(px(PANEL_RADIUS)).shadow(shadow(&t, liquid)).child(drag_handle(panel)))
            .child(edge_zone(Edge::Side, w, h))
            .child(edge_zone(Edge::End, w, h))
            .child(edge_zone(Edge::Corner, w, h))
            // Where the platform doesn't follow the pointer itself.
            .child(canvas(|_, _, _| {}, |_, _, window, _| follow_resize(window)).absolute().size_0())
    }
}

#[cfg(test)]
mod tests;
