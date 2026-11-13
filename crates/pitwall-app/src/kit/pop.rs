//! Floating surfaces (menus, the palette, quick open) as windows of their
//! own, so that they can be the system's glass.
//!
//! The native glass sits under GPUI's view and only sees what is behind
//! the window (`platform::glass`), never what the app draws: a menu drawn
//! in the main window over a terminal can't be glass. A small transparent
//! window above the main one can: its background is one slab of
//! `NSGlassEffectView`, which bends and blurs the main window under it.
//!
//! [`hosted`] is the one place that decides: Liquid Glass on macOS 26
//! (`theme::glass_regions`, which follows the look and Reduce
//! transparency). Everywhere else a surface is drawn in its window as
//! before, and nothing here runs.
//!
//! A surface that is hosted puts an [`anchor`] where it would have drawn
//! itself. While the anchor is drawn, its pop-up is open:
//!
//! - at the anchor's place in the parent window (or [`Pop::at`]), kept on
//!   the screen that shows that place ([`place`]), sized to its content;
//! - the parent's child window: it moves with it and stays above it; the
//!   parent stays the main window, so it keeps its active look;
//! - with the keyboard (it is the key window), on [`Pop::focus`];
//! - closed, telling the owner ([`Pop::on_dismiss`], in the parent
//!   window), by Esc, a press outside it, and its losing the keyboard (a
//!   click in the parent, another app, the parent hidden);
//! - closed without a word when the anchor isn't drawn any more (the
//!   owner closed it, or went away, or the parent window closed).
//!
//! What is drawn in it is built in the pop-up's window, so a handler in it
//! gets that window: [`pick`] closes the pop-up and runs the rest in the
//! parent window.
//!
//! A pop-up is a window like any other to what is drawn in it: an anchor
//! there opens a pop-up of the pop-up (a dialog's select, a menu in it).
//! The outer one stays open while one in it has the keyboard, and closes
//! with it when the keyboard then goes to neither; closing a pop-up closes
//! those in it; Esc and a press outside reach the innermost only.
//!
//! A dialog ([`Anchor::modal`]) is centred in its parent window and stays
//! until its owner closes it, Esc, or a press on its backdrop: the
//! keyboard going to another app doesn't close it, and it takes it back
//! when its parent gets it.
//!
//! One rule for what is built ([`Build`]): it must not read or update,
//! while it is built and drawn, an entity that the parent window draws.
//! GPUI gives every entity one window, the one that last touched it while
//! drawing, and runs its `_in` callbacks (`subscribe_in`, `observe_in`,
//! `spawn_in`, `defer_in`) there: a screen read by its menu's pop-up would
//! get the pop-up's window for them, and none once it closed. So a menu
//! is built from values taken when its owner drew, with handlers that
//! reach the owner only when they run; a surface that is an entity of its
//! own (the palette) is touched only by its pop-up.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    div, point, prelude::*, px, size, AnyElement, AnyWindowHandle, App, AvailableSpace, Bounds, Context, Element,
    ElementId, Entity, FocusHandle, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels, Point, Render,
    Size, Style, Subscription, WeakEntity, Window, WindowBackgroundAppearance, WindowBounds, WindowId, WindowKind,
    WindowOptions,
};

use crate::platform::{self, PopParent};
use crate::theme::{self, Theme};

/// Clear space around the panel inside its window: room for the glass's
/// own soft shadow to fade out (cut short, it shows as a box around the
/// panel on a light window). A press there is a press outside.
pub const PAD: f32 = 24.;
/// A menu's corners on glass (its rows' 4 px inside its 4 px of padding;
/// the window's own menus are a little rounder still).
pub const MENU_RADIUS: f32 = 8.;
/// The gap kept to the screen's edges.
const MARGIN: f32 = 4.;
/// The gap a dialog keeps to its parent window's edges (the padding of
/// the backdrop it is drawn in elsewhere).
pub const MODAL_GUTTER: f32 = 24.;
/// How much of the theme's own surface is laid over the glass (see
/// `float::GLASS_FILL_OPEN`: text straight on glass can't be read over a
/// white page).
const FILL: f32 = 0.72;

/// Builds the surface's content, in the pop-up's window, without touching
/// an entity the parent window draws (see the module).
pub type Build = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
/// Tells the owner its surface was closed, in the parent window.
pub type OnDismiss = Rc<dyn Fn(&mut Window, &mut App)>;

/// A floating surface in a window of its own.
pub struct Pop {
    /// Where its top left goes, in the parent window (`None`: where the
    /// anchor is).
    pub at: Option<Point<Pixels>>,
    /// The panel's corners.
    pub radius: f32,
    /// What has the keyboard in it (`None`: the pop-up itself, for Esc).
    pub focus: Option<FocusHandle>,
    pub build: Build,
    pub on_dismiss: OnDismiss,
}

/// Tests: floating surfaces are hosted only where a test says so (with
/// Liquid Glass on), so that every other test draws them in its window.
#[cfg(test)]
pub(crate) struct TestHosted;
#[cfg(test)]
impl gpui::Global for TestHosted {}

/// Glass pop-up, or in the window as ever: Liquid Glass regions are on
/// (the look is Glass, on macOS 26, with transparency allowed) and the
/// system has pop-ups of this kind.
pub fn chooses(glass_regions: bool, supported: bool) -> bool {
    glass_regions && supported
}

/// Floating surfaces are glass pop-ups now (else: drawn in their window).
pub fn hosted(cx: &App) -> bool {
    #[cfg(test)]
    let supported = cx.has_global::<TestHosted>();
    #[cfg(not(test))]
    let supported = platform::pop_supported() && platform::glass::liquid_available();
    chooses(theme::glass_regions(cx), supported)
}

/// The theme inside a pop-up: the floating one, with dim text a step
/// brighter (too dim on glass over a bright page).
pub fn theme(cx: &App) -> Theme {
    let mut t = theme::theme(cx).float();
    t.text_3 = t.text_2;
    t.text_2 = t.text;
    t
}

/// `window` is a glass pop-up: what is drawn in it is on glass.
pub fn is_pop(window: &Window) -> bool {
    parent_of(window).is_some()
}

/// The viewport of the window the dialog `window` is over (`None`: it
/// isn't a dialog's pop-up). For what sizes itself to the window.
pub fn modal_viewport(window: &Window) -> Option<Size<Pixels>> {
    OPEN.with_borrow(|all| all.get(&id_of(window)).and_then(|e| e.shared.borrow().modal))
}

/// The theme a floating surface's content draws with in `window`: a
/// pop-up's ([`theme`]), or the window's floating one.
pub fn theme_in(window: &Window, cx: &App) -> Theme {
    if is_pop(window) { theme(cx) } else { theme::theme(cx).float() }
}

/// Where a panel of `size` goes that should have its top left at `at`:
/// there, moved (and, when it is larger than the screen, cut) to stay
/// `margin` inside `area`. All in the parent window's coordinates.
pub fn place(at: Point<Pixels>, size: Size<Pixels>, area: Option<Bounds<Pixels>>, margin: Pixels) -> Bounds<Pixels> {
    let Some(a) = area else { return Bounds::new(at, size) };
    let fit = |at: Pixels, len: Pixels, from: Pixels, room: Pixels| {
        let len = len.min((room - margin * 2.).max(px(0.)));
        // The far edge first: a panel larger than the room starts at its
        // near edge.
        let at = at.min(from + room - margin - len).max(from + margin);
        (at, len)
    };
    let (x, w) = fit(at.x, size.width, a.origin.x, a.size.width);
    let (y, h) = fit(at.y, size.height, a.origin.y, a.size.height);
    Bounds::new(point(x, y), gpui::size(w, h))
}

/// Where a dialog of `size` goes in a parent `viewport`: its centre.
pub fn centred(viewport: Size<Pixels>, size: Size<Pixels>) -> Point<Pixels> {
    // On whole points: half of one blurs every hairline in it.
    point(((viewport.width - size.width) / 2.).floor(), ((viewport.height - size.height) / 2.).floor())
}

/// Why a pop-up closes by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Close {
    /// Esc.
    Escape,
    /// A press in its window outside the panel.
    PressOutside,
    /// It lost the keyboard: a click in the parent or another app.
    LostKeys,
}

/// Whether the keyboard going to or from a pop-up closes it: only its
/// leaving after it was there (a pop-up opens without it and takes it a
/// moment later).
pub fn closes_on_keys(had_keys: bool, has_keys: bool) -> bool {
    had_keys && !has_keys
}

struct Shared {
    pop: Pop,
    /// The anchor's place in the parent window.
    at: Point<Pixels>,
    view: Option<WeakEntity<PopView>>,
    /// Closed (dismissed, or a row was picked) while its anchor may still
    /// be drawn.
    closed: bool,
    /// The owner has heard of that. An anchor it still draws after it
    /// did wants its surface open: it gets a new pop-up.
    told: bool,
    /// A dialog (see [`Anchor::modal`]): the parent's viewport.
    modal: Option<Size<Pixels>>,
}

struct Entry {
    parent: AnyWindowHandle,
    shared: Rc<RefCell<Shared>>,
    /// Where it was last put (its window, in the parent's coordinates).
    placed: Rc<Cell<Option<Bounds<Pixels>>>>,
}

thread_local! {
    /// The pop-ups that are open, by their window.
    static OPEN: RefCell<HashMap<WindowId, Entry>> = RefCell::new(HashMap::new());
}

fn id_of(window: &Window) -> WindowId {
    window.window_handle().window_id()
}

/// The window `window` is a pop-up of.
pub fn parent_of(window: &Window) -> Option<AnyWindowHandle> {
    OPEN.with_borrow(|all| all.get(&id_of(window)).map(|e| e.parent))
}

/// A pop-up of `window` is open.
pub fn over(window: &Window) -> bool {
    let id = id_of(window);
    OPEN.with_borrow(|all| all.values().any(|e| e.parent.window_id() == id))
}

/// The pop-ups of the window `id` (not those in them).
fn children_of(id: WindowId) -> Vec<WindowId> {
    OPEN.with_borrow(|all| all.iter().filter(|(_, e)| e.parent.window_id() == id).map(|(c, _)| *c).collect())
}

/// The window with this id, while it is open.
fn handle_of(id: WindowId, cx: &App) -> Option<AnyWindowHandle> {
    cx.windows().into_iter().find(|w| w.window_id() == id)
}

/// The innermost open pop-up (one with none in it), and the outermost
/// (one of a window that isn't a pop-up): what [`debug`] drives.
fn innermost() -> Option<WindowId> {
    OPEN.with_borrow(|all| all.keys().find(|id| !all.values().any(|e| e.parent.window_id() == **id)).copied())
}

fn outermost() -> Option<WindowId> {
    OPEN.with_borrow(|all| all.iter().find(|(_, e)| !all.contains_key(&e.parent.window_id())).map(|(id, _)| *id))
}

/// Whether a pop-up that lost the keyboard stays: a dialog does, and so
/// does one with a pop-up of its own open (which took it).
pub fn keeps_without_keys(modal: bool, has_child: bool) -> bool {
    modal || has_child
}

/// The pop-up `id` had a pop-up that closed because the keyboard left it.
/// If it didn't come here, it left this one too: it closes the same way
/// (a dialog stays).
fn lost_through_child(id: WindowId, cx: &mut App) {
    let view = OPEN.with_borrow(|all| all.get(&id).and_then(|e| e.shared.borrow().view.clone()));
    let (Some(view), Some(handle)) = (view.and_then(|v| v.upgrade()), handle_of(id, cx)) else { return };
    let _ = handle.update(cx, |_, window, cx| {
        let modal = view.read(cx).shared.borrow().modal.is_some();
        if !platform::pop_has_keys(window, cx) && !keeps_without_keys(modal, over(window)) {
            view.update(cx, |v, cx| v.dismiss(Close::LostKeys, window, cx));
        }
    });
}

/// `window` is the one the user is in: it is active, or one of its own
/// pop-ups has the keyboard for it. For what a window does when the user
/// leaves it (it didn't, when it opened a menu).
pub fn active(window: &Window) -> bool {
    window.is_window_active() || over(window)
}

/// The open pop-ups: (pop-up, parent, where it was put). Tests.
#[cfg(test)]
pub(crate) fn open_pops() -> Vec<(WindowId, AnyWindowHandle, Option<Bounds<Pixels>>)> {
    OPEN.with_borrow(|all| all.iter().map(|(id, e)| (*id, e.parent, e.placed.get())).collect())
}

/// Something in a floating surface was chosen. In a pop-up: it closes and
/// `f` runs in the parent window (once this event is done). In the window
/// itself (the surface is drawn there): `f` runs now.
pub fn pick(window: &mut Window, cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
    match parent_of(window) {
        None => f(window, cx),
        Some(parent) => {
            let shared = shut(window, cx);
            cx.defer(move |cx| {
                let _ = parent.update(cx, |_, window, cx| f(window, cx));
                if let Some(s) = shared {
                    s.borrow_mut().told = true;
                }
            });
        }
    }
}

/// Close the pop-up `window` (nothing when it isn't one, or is closed).
fn shut(window: &mut Window, cx: &mut App) -> Option<Rc<RefCell<Shared>>> {
    // Those in it first (their owners are in this one: nobody to tell).
    for child in children_of(id_of(window)) {
        if let Some(child) = handle_of(child, cx) {
            let _ = child.update(cx, |_, window, cx| {
                shut(window, cx);
            });
        }
    }
    let entry = OPEN.with_borrow_mut(|all| all.remove(&id_of(window)))?;
    entry.shared.borrow_mut().closed = true;
    platform::glass::slab(window, None);
    platform::pop_close(window, cx);
    window.remove_window();
    Some(entry.shared)
}

/// `PITWALL_POP_LOG=1`: how long a pop-up takes to open, on stderr.
fn log(what: impl FnOnce() -> String) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("PITWALL_POP_LOG").is_some()) {
        eprintln!("pitwall-pop: {}", what());
    }
}

/// Debug builds, from the bench hook (`pop <what>` in `bench-cmd`): drive
/// the open pop-up in this process, for checks and screenshots of the
/// running app without synthetic input. `key <keystroke>…` and `type
/// <text>` go through GPUI's key dispatch in the pop-up's window, `click
/// <x> <y>` is a click as AppKit delivers one (macOS), `state` logs where
/// it is and who has the keyboard.
pub fn debug(cmd: &str, cx: &mut App) {
    if !cfg!(debug_assertions) {
        return;
    }
    // `under <x> <y>`: a click in the first window that is not a pop-up.
    if let Some(rest) = cmd.strip_prefix("under ") {
        let mut at = rest.split_whitespace().filter_map(|v| v.parse::<f32>().ok());
        let under = cx.windows().into_iter().find(|w| !OPEN.with_borrow(|all| all.contains_key(&w.window_id())));
        if let (Some(x), Some(y), Some(under)) = (at.next(), at.next(), under) {
            let _ = under.update(cx, |_, window, cx| platform::debug_click(window, x, y, cx));
        }
        return;
    }
    // `outer <what>`: the outermost pop-up (a dialog under its select);
    // else the innermost.
    let (cmd, id) = match cmd.strip_prefix("outer ") {
        Some(rest) => (rest, outermost()),
        None => (cmd, innermost()),
    };
    let Some(id) = id else {
        eprintln!("pitwall-pop: debug {cmd}: no pop-up is open");
        return;
    };
    let Some(handle) = cx.windows().into_iter().find(|w| w.window_id() == id) else { return };
    let (what, rest) = cmd.split_once(' ').unwrap_or((cmd, ""));
    let _ = handle.update(cx, |_, window, cx| match what {
        "key" => {
            for k in rest.split_whitespace().filter_map(|k| gpui::Keystroke::parse_with_os(k, gpui::OperatingSystem::Mac).ok()) {
                window.dispatch_keystroke(k, cx);
            }
        }
        "type" => {
            for c in rest.chars() {
                let k = gpui::Keystroke { modifiers: Default::default(), key: c.to_string(), key_char: Some(c.to_string()) };
                window.dispatch_keystroke(k, cx);
            }
        }
        "click" => {
            let mut at = rest.split_whitespace().filter_map(|v| v.parse::<f32>().ok());
            if let (Some(x), Some(y)) = (at.next(), at.next()) {
                platform::debug_click(window, x, y, cx);
            }
        }
        _ => {
            let placed = OPEN.with_borrow(|all| all.get(&id).and_then(|e| e.placed.get()));
            let open = OPEN.with_borrow(|all| all.len());
            eprintln!("pitwall-pop: state: {open} open, placed {placed:?}, gpui active {}, {}", window.is_window_active(), platform::pop_debug(window));
        }
    });
}

/// Lives as long as the anchor is drawn; the pop-up closes when it goes.
struct Token;

/// What the anchor keeps between frames.
struct Held {
    shared: Rc<RefCell<Shared>>,
    _token: Entity<Token>,
}

/// Where a hosted surface would have drawn itself: see the module.
pub struct Anchor {
    id: ElementId,
    pop: Option<Pop>,
    modal: Option<Size<Pixels>>,
}

impl Anchor {
    /// A dialog's body, over its backdrop in a parent window whose
    /// content is `viewport`: centred in it (24 px in from its edges,
    /// and laid out in less when it doesn't fit), wherever the anchor is.
    /// It stays when the keyboard goes elsewhere (another app; the parent
    /// closes it on a press on its backdrop) and takes the keyboard back
    /// when the parent gets it.
    pub fn modal(mut self, viewport: Size<Pixels>) -> Anchor {
        self.modal = Some(viewport);
        self
    }
}

/// The pop-up `pop` is open while this is drawn. Give it a place of no
/// size where the surface's top left should be (or set [`Pop::at`]).
pub fn anchor(id: impl Into<ElementId>, pop: Pop) -> Anchor {
    Anchor { id: id.into(), pop: Some(pop), modal: None }
}

impl Element for Anchor {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (window.request_layout(Style::default(), [], cx), ())
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let (Some(id), Some(pop)) = (id, self.pop.take()) else { return };
        let at = pop.at.unwrap_or(bounds.origin);
        let parent = window.window_handle();
        let modal = self.modal;
        window.with_element_state::<Held, _>(id, |held, _| {
            // Closed, the owner knows, and here it is drawn again.
            let again = |h: &Held| {
                let s = h.shared.borrow();
                s.closed && s.told
            };
            let held = match held.filter(|h| !again(h)) {
                Some(held) => {
                    let view = {
                        let mut s = held.shared.borrow_mut();
                        s.pop = pop;
                        s.at = at;
                        s.modal = modal;
                        s.view.as_ref().and_then(|v| v.upgrade())
                    };
                    // Drawn again: so is the pop-up, with what it holds now.
                    if let Some(view) = view {
                        cx.notify(view.entity_id());
                    }
                    held
                }
                None => {
                    let shared = Rc::new(RefCell::new(Shared { pop, at, view: None, closed: false, told: false, modal }));
                    let token = cx.new(|_| Token);
                    let (s, weak, asked) = (shared.clone(), token.downgrade(), std::time::Instant::now());
                    // Once this frame is drawn: a window can't be opened
                    // while another draws.
                    cx.defer(move |cx| open(s, weak, parent, asked, cx));
                    Held { shared, _token: token }
                }
            };
            ((), held)
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }
}

/// Open the pop-up's window (not shown until it knows its size).
fn open(
    shared: Rc<RefCell<Shared>>,
    token: WeakEntity<Token>,
    parent: AnyWindowHandle,
    asked: std::time::Instant,
    cx: &mut App,
) {
    // The anchor was drawn once and is gone.
    let Some(token) = token.upgrade() else { return };
    let Some(native) = parent.update(cx, |_, window, _| platform::pop_parent(window)).ok().flatten() else {
        return;
    };
    let options = WindowOptions {
        // Put in its place, at its size, once its content is measured
        // (`Measured`); until then it isn't on screen.
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size(px(240.), px(120.))))),
        titlebar: None,
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(platform::WINDOW_APP_ID.into()),
        ..Default::default()
    };
    let placed = Rc::new(Cell::new(None));
    let opened = cx.open_window(options, |window, cx| {
        platform::lean_renderer(window);
        platform::pop_setup(window);
        let entry = Entry { parent, shared: shared.clone(), placed: placed.clone() };
        OPEN.with_borrow_mut(|all| all.insert(id_of(window), entry));
        let view = cx.new(|cx| PopView::new(shared.clone(), &token, parent, native, placed.clone(), window, cx));
        shared.borrow_mut().view = Some(view.downgrade());
        view
    });
    match opened {
        Ok(_) => {
            log(|| format!("opened in {:.1} ms (its window made and drawn once)", asked.elapsed().as_secs_f64() * 1e3));
            watch_parent(&shared, parent, cx);
        }
        Err(e) => {
            // Nothing to show it in: the owner hears it closed.
            eprintln!("pitwall: no pop-up window on this system: {e}");
            let on = {
                let mut s = shared.borrow_mut();
                s.closed = true;
                s.pop.on_dismiss.clone()
            };
            let _ = parent.update(cx, |_, window, cx| on(window, cx));
            shared.borrow_mut().told = true;
        }
    }
}

/// A new pop-up hears when its parent window gets the keyboard (a dialog
/// takes it back: see [`PopView::parent_active`]).
fn watch_parent(shared: &Rc<RefCell<Shared>>, parent: AnyWindowHandle, cx: &mut App) {
    let Some(view) = shared.borrow().view.as_ref().and_then(|v| v.upgrade()) else { return };
    let _ = parent.update(cx, |_, parent, cx| {
        view.update(cx, |v, cx| v._parent = Some(cx.observe_window_activation(parent, PopView::parent_active)));
    });
}

/// The pop-up window's content: the panel on the glass, and in it what
/// the owner builds.
pub struct PopView {
    shared: Rc<RefCell<Shared>>,
    parent: AnyWindowHandle,
    native: Rc<PopParent>,
    placed: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Has the keyboard when the surface names nothing of its own.
    focus: FocusHandle,
    /// What was last given the keyboard.
    focused: Option<FocusHandle>,
    /// It has had the keyboard (see [`closes_on_keys`]).
    had_keys: bool,
    _subs: [Subscription; 3],
    /// The parent window's activation (see [`watch_parent`]).
    _parent: Option<Subscription>,
}

impl PopView {
    fn new(
        shared: Rc<RefCell<Shared>>,
        token: &Entity<Token>,
        parent: AnyWindowHandle,
        native: PopParent,
        placed: Rc<Cell<Option<Bounds<Pixels>>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PopView {
        let subs = [
            // The anchor isn't drawn any more.
            cx.observe_release_in(token, window, |_, _, window, cx| {
                shut(window, cx);
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                let has = window.is_window_active();
                log(|| format!("has the keyboard: {has}"));
                if closes_on_keys(this.had_keys, has) && !this.keeps(window) {
                    this.dismiss(Close::LostKeys, window, cx);
                }
                this.had_keys |= has;
            }),
            cx.observe_global::<Theme>(|_, cx| cx.notify()),
        ];
        PopView {
            shared,
            parent,
            native: Rc::new(native),
            placed,
            focus: cx.focus_handle(),
            focused: None,
            had_keys: false,
            _subs: subs,
            _parent: None,
        }
    }

    /// It stays though the keyboard left it (see [`keeps_without_keys`]).
    fn keeps(&self, window: &Window) -> bool {
        keeps_without_keys(self.shared.borrow().modal.is_some(), over(window))
    }

    /// The parent window got or lost the keyboard. A dialog over it takes
    /// it: the user came back to the window (from another app, or by a
    /// press on its title bar), and what they type is for the dialog.
    fn parent_active(&mut self, parent: &mut Window, cx: &mut Context<Self>) {
        if !parent.is_window_active() || self.shared.borrow().modal.is_none() {
            return;
        }
        let shared = self.shared.clone();
        // Once the parent has handled what brought it the keyboard (a
        // press on the backdrop closes the dialog).
        cx.defer(move |cx| {
            let me = OPEN.with_borrow(|all| all.iter().find(|(_, e)| Rc::ptr_eq(&e.shared, &shared)).map(|(id, _)| *id));
            if let Some(me) = me.and_then(|id| handle_of(id, cx)) {
                let _ = me.update(cx, |_, window, cx| platform::pop_keys(window, cx));
            }
        });
    }

    /// Close, and tell the owner (in the parent window, once this event
    /// is done).
    fn dismiss(&mut self, why: Close, window: &mut Window, cx: &mut Context<Self>) {
        if parent_of(window).is_none() {
            return;
        }
        log(|| format!("closed: {why:?}"));
        let on = self.shared.borrow().pop.on_dismiss.clone();
        let (parent, shared) = (self.parent, self.shared.clone());
        shut(window, cx);
        cx.defer(move |cx| {
            let _ = parent.update(cx, |_, window, cx| on(window, cx));
            shared.borrow_mut().told = true;
            // The pop-up it was in: the keyboard may have left that too.
            if why == Close::LostKeys {
                lost_through_child(parent.window_id(), cx);
            }
        });
    }
}

impl Render for PopView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().id("pop").size_full();
        if parent_of(window).is_none() {
            return root;
        }
        let t = theme(cx);
        let (build, radius, focus) = {
            let s = self.shared.borrow();
            (s.pop.build.clone(), s.pop.radius, s.pop.focus.clone())
        };
        platform::glass::slab(window, Some(platform::glass::Slab { inset: PAD, radius, mode: t.mode }));
        if self.shared.borrow().modal.is_some() {
            platform::pop_modal(window);
        }
        let want = focus.unwrap_or_else(|| self.focus.clone());
        if self.focused.as_ref() != Some(&want) {
            window.focus(&want, cx);
            self.focused = Some(want);
        }
        let content = build(window, cx);
        let measured = Measured {
            child: Some(content),
            shared: self.shared.clone(),
            native: self.native.clone(),
            placed: self.placed.clone(),
        };
        root.track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &crate::menu::Dismiss, window, cx| this.dismiss(Close::Escape, window, cx)))
            .p(px(PAD))
            .font_family(super::fonts::UI_FONT)
            .text_size(px(13.))
            .line_height(gpui::relative(super::fonts::BODY_LINE_HEIGHT))
            .text_color(t.text)
            .child(
                div()
                    .id("pop-panel")
                    .size_full()
                    .rounded(px(radius))
                    // The theme's own surface, thinly, over the glass; no
                    // shadow of ours (it would show through the glass),
                    // only the hairline.
                    .bg(t.raised.opacity(FILL))
                    .shadow(vec![t.ring(t.g.line_strong)])
                    .overflow_hidden()
                    .on_mouse_down_out(cx.listener(|this, _, window, cx| this.dismiss(Close::PressOutside, window, cx)))
                    .child(measured),
            )
    }
}

/// The content at its own size; the window is put where that size goes.
struct Measured {
    child: Option<AnyElement>,
    shared: Rc<RefCell<Shared>>,
    native: Rc<PopParent>,
    placed: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl Element for Measured {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style { size: size(gpui::relative(1.).into(), gpui::relative(1.).into()), ..Style::default() };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(child) = self.child.as_mut() else { return };
        let at = self.shared.borrow().at;
        let area = platform::pop_area(&self.native, at);
        // A dialog: in the parent's viewport, not the screen.
        let modal = self.shared.borrow().modal;
        let area = modal.map(|v| Bounds::new(point(px(0.), px(0.)), v).inset(px(MODAL_GUTTER - MARGIN))).or(area);
        let free = size(AvailableSpace::MaxContent, AvailableSpace::MaxContent);
        let mut content = child.layout_as_root(free, window, cx);
        // Larger than the screen: laid out again in what there is.
        let room = area.map(|a| size(a.size.width - px(2. * MARGIN), a.size.height - px(2. * MARGIN)));
        if let Some(room) = room.filter(|r| content.width > r.width || content.height > r.height) {
            let cut = |len: Pixels, room: Pixels| {
                if len > room { AvailableSpace::Definite(room.max(px(0.))) } else { AvailableSpace::MaxContent }
            };
            let space = size(cut(content.width, room.width), cut(content.height, room.height));
            content = child.layout_as_root(space, window, cx);
            content = size(content.width.min(room.width.max(px(0.))), content.height.min(room.height.max(px(0.))));
        }
        let at = modal.map_or(at, |v| centred(v, content));
        // The window: the panel (kept on the screen) and the clear space
        // around it (which may leave the screen).
        let frame = place(at, content, area, px(MARGIN)).dilate(px(PAD));
        if self.placed.get() != Some(frame) {
            let first = self.placed.replace(Some(frame)).is_none();
            log(|| format!("{} at {frame:?}", if first { "shown" } else { "moved" }));
            platform::pop_place(window, &self.native, frame, cx);
        }
        child.prepaint_at(bounds.origin, window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(child) = self.child.as_mut() {
            child.paint(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    fn b(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[test]
    fn glass_pop_ups_only_with_liquid_glass_where_there_are_pop_ups() {
        assert!(chooses(true, true));
        // Flat, older macOS, Reduce transparency: no glass regions.
        assert!(!chooses(false, true));
        // Windows, Linux (even with the look forced).
        assert!(!chooses(true, false));
        assert!(!chooses(false, false));
    }

    #[test]
    fn a_panel_goes_where_asked_and_stays_on_its_screen() {
        let (m, at, sz) = (px(4.), point(px(100.), px(80.)), size(px(200.), px(120.)));
        // The screen, seen from a window at (300, 200) on it.
        let area = Some(b(-300., -200., 1728., 1080.));
        assert_eq!(place(at, sz, area, m), b(100., 80., 200., 120.), "room: where asked");
        assert_eq!(place(at, sz, None, m), b(100., 80., 200., 120.), "no screen known: where asked");
        // Past the right and the bottom edges: moved back in.
        assert_eq!(place(point(px(1300.), px(850.)), sz, area, m), b(1224., 756., 200., 120.));
        // Past the left and the top (a window partly off screen).
        assert_eq!(place(point(px(-400.), px(-300.)), sz, area, m), b(-296., -196., 200., 120.));
        // Larger than the screen: from its near edge, cut to fit.
        let tall = size(px(200.), px(2000.));
        assert_eq!(place(at, tall, area, m), b(100., -196., 200., 1072.));
        // Another display left of the window's: its own area.
        let left = Some(b(-2300., -200., 1920., 1080.));
        assert_eq!(place(point(px(-500.), px(80.)), sz, left, m), b(-584., 80., 200., 120.));
    }

    #[test]
    fn losing_the_keyboard_closes_only_after_having_had_it() {
        assert!(!closes_on_keys(false, false), "opens without it");
        assert!(!closes_on_keys(false, true), "takes it");
        assert!(!closes_on_keys(true, true));
        assert!(closes_on_keys(true, false), "a click in the parent or another app");
    }

    /// A surface's owner: an anchor while `open`.
    struct Owner {
        open: bool,
        dismissed: usize,
        focus: FocusHandle,
    }

    impl Render for Owner {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let this = cx.weak_entity();
            let focus = self.focus.clone();
            let pop = Pop {
                at: None,
                radius: 6.,
                focus: Some(self.focus.clone()),
                build: Rc::new(move |_, _| div().track_focus(&focus).w(px(100.)).h(px(50.)).into_any_element()),
                on_dismiss: Rc::new(move |_, cx| {
                    let _ = this.update(cx, |o, cx| {
                        o.open = false;
                        o.dismissed += 1;
                        cx.notify();
                    });
                }),
            };
            div().size_full().when(self.open, |d| d.child(div().absolute().left(px(40.)).top(px(30.)).child(anchor("surface", pop))))
        }
    }

    fn liquid(cx: &mut TestAppContext, look: crate::theme::Look) {
        use crate::theme::{Appearance, GlassOffer, Inputs, OsPrefs};
        cx.update(|cx| {
            let ap = Appearance::resolve(Inputs { look, ..Inputs::default() }, gpui::WindowAppearance::Dark, GlassOffer::Liquid, OsPrefs::default());
            cx.set_global(ap.theme());
            cx.set_global(ap);
        });
    }

    fn owner(cx: &mut TestAppContext) -> (Entity<Owner>, &mut VisualTestContext) {
        liquid(cx, crate::theme::Look::Glass);
        cx.update(|cx| cx.set_global(TestHosted));
        let (view, cx) = cx.add_window_view(|_, cx| Owner { open: false, dismissed: 0, focus: cx.focus_handle() });
        cx.run_until_parked();
        (view, cx)
    }

    /// The pop-ups of the test's window.
    fn pops(cx: &mut VisualTestContext) -> Vec<(AnyWindowHandle, Option<Bounds<Pixels>>)> {
        let parent = cx.update(|window, _| id_of(window));
        let all = cx.windows();
        open_pops()
            .into_iter()
            .filter(|(_, p, _)| p.window_id() == parent)
            .filter_map(|(id, _, placed)| Some((*all.iter().find(|w| w.window_id() == id)?, placed)))
            .collect()
    }

    fn open(view: &Entity<Owner>, cx: &mut VisualTestContext) -> AnyWindowHandle {
        view.update(cx, |o, cx| {
            o.open = true;
            cx.notify();
        });
        cx.run_until_parked();
        let pops = pops(cx);
        assert_eq!(pops.len(), 1, "one pop-up for the anchor");
        pops[0].0
    }

    #[gpui::test]
    fn hosted_only_where_a_test_says_so_and_with_glass(cx: &mut TestAppContext) {
        liquid(cx, crate::theme::Look::Glass);
        cx.update(|cx| {
            assert!(!hosted(cx), "tests draw surfaces in their window");
            cx.set_global(TestHosted);
            assert!(hosted(cx));
        });
        liquid(cx, crate::theme::Look::Flat);
        cx.update(|cx| assert!(!hosted(cx), "Flat: in the window"));
    }

    #[gpui::test]
    fn an_anchor_opens_its_pop_up_sized_to_the_content_at_its_place(cx: &mut TestAppContext) {
        let (view, cx) = owner(cx);
        assert!(pops(cx).is_empty());
        let pop = open(&view, cx);
        // The content (100 × 50) with the clear space around it, its
        // panel's corner on the anchor.
        assert_eq!(pops(cx)[0].1, Some(b(40. - PAD, 30. - PAD, 100. + 2. * PAD, 50. + 2. * PAD)));
        cx.update(|window, _| {
            assert!(over(window) && active(window), "the window counts as the one the user is in");
            assert!(parent_of(window).is_none());
        });
        let parent = cx.update(|window, _| id_of(window));
        cx.update_window(pop, |_, window, cx| {
            assert_eq!(parent_of(window).map(|p| p.window_id()), Some(parent));
            assert!(view.read(cx).focus.is_focused(window), "the surface's own focus, in the pop-up");
        })
        .unwrap();
        // Drawn again (the owner changed): still the one pop-up.
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(pops(cx).len(), 1);
    }

    #[gpui::test]
    fn it_closes_when_its_anchor_is_gone_without_telling_the_owner(cx: &mut TestAppContext) {
        let (view, cx) = owner(cx);
        open(&view, cx);
        view.update(cx, |o, cx| {
            o.open = false;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(pops(cx).is_empty());
        assert_eq!(view.read_with(cx, |o, _| o.dismissed), 0);
        cx.update(|window, _| assert!(!over(window)));
    }

    #[gpui::test]
    fn escape_closes_it_and_tells_the_owner_in_its_own_window(cx: &mut TestAppContext) {
        let (view, cx) = owner(cx);
        let pop = open(&view, cx);
        cx.update_window(pop, |_, window, cx| window.dispatch_action(Box::new(crate::menu::Dismiss), cx)).unwrap();
        cx.run_until_parked();
        assert!(pops(cx).is_empty());
        assert_eq!(view.read_with(cx, |o, _| (o.open, o.dismissed)), (false, 1));
        // And again: a new pop-up.
        open(&view, cx);
    }

    #[gpui::test]
    fn losing_the_keyboard_closes_it(cx: &mut TestAppContext) {
        let (view, cx) = owner(cx);
        let pop = open(&view, cx);
        cx.update_window(pop, |_, window, _| window.activate_window()).unwrap();
        cx.run_until_parked();
        assert_eq!(pops(cx).len(), 1, "having the keyboard keeps it");
        // A click in the parent: it has the keyboard again.
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert!(pops(cx).is_empty());
        assert_eq!(view.read_with(cx, |o, _| (o.open, o.dismissed)), (false, 1));
    }

    /// The kit's select, open, with what was picked and whether it closed.
    struct Select {
        picked: Rc<Cell<Option<(usize, WindowId)>>>,
        closed: Rc<Cell<bool>>,
    }

    impl Render for Select {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let (picked, closed) = (self.picked.clone(), self.closed.clone());
            let items = vec![("One".into(), true), ("Two".into(), false)];
            div().size_full().child(div().relative().child(crate::kit::select_menu(
                "select",
                items,
                &theme::theme(cx).clone(),
                Rc::new(move |i, window, _| picked.set(Some((i, id_of(window))))),
                Rc::new(move |_, _| closed.set(true)),
            )))
        }
    }

    #[gpui::test]
    fn the_kits_select_opens_in_a_pop_up_and_picks_in_its_window(cx: &mut TestAppContext) {
        liquid(cx, crate::theme::Look::Glass);
        cx.update(|cx| cx.set_global(TestHosted));
        let (picked, closed) = (Rc::new(Cell::new(None)), Rc::new(Cell::new(false)));
        let (view, cx) = cx.add_window_view(|_, _| Select { picked: picked.clone(), closed: closed.clone() });
        cx.run_until_parked();
        let open = pops(cx);
        assert_eq!(open.len(), 1, "the options are a pop-up");
        // Under the trigger (28 px), at the select's left edge.
        let at = open[0].1.unwrap();
        assert_eq!(at.origin, point(px(-PAD), px(28. - PAD)));
        // A click on the second option (rows are 4 px in, some 28 px tall).
        let parent = cx.update(|window, _| id_of(window));
        cx.update_window(open[0].0, |_, window, cx| {
            let position = point(px(PAD + 30.), px(PAD + 4. + 40.));
            let (button, modifiers) = (gpui::MouseButton::Left, gpui::Modifiers::default());
            window.dispatch_event(gpui::PlatformInput::MouseDown(gpui::MouseDownEvent { position, button, modifiers, click_count: 1, first_mouse: false }), cx);
            window.dispatch_event(gpui::PlatformInput::MouseUp(gpui::MouseUpEvent { position, button, modifiers, click_count: 1 }), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(picked.get(), Some((1, parent)), "the pick runs in the select's window");
        assert!(!closed.get(), "a pick is not a dismissal");
        assert!(pops(cx).is_empty());
        // Esc in a pop-up (this owner draws its select again: a new one).
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        let again = pops(cx)[0].0;
        cx.update_window(again, |_, window, cx| window.dispatch_action(Box::new(crate::menu::Dismiss), cx)).unwrap();
        cx.run_until_parked();
        assert!(closed.get(), "Esc closes a select in a pop-up");
    }

    #[gpui::test]
    fn a_pick_closes_it_and_runs_in_the_parent_window(cx: &mut TestAppContext) {
        let (view, cx) = owner(cx);
        let pop = open(&view, cx);
        let parent = cx.update(|window, _| id_of(window));
        let ran = Rc::new(Cell::new(None));
        let r = ran.clone();
        cx.update_window(pop, |_, window, cx| pick(window, cx, move |window, _| r.set(Some(id_of(window))))).unwrap();
        cx.run_until_parked();
        assert_eq!(ran.get(), Some(parent));
        assert!(pops(cx).is_empty(), "closed, though its anchor is still drawn");
        assert_eq!(view.read_with(cx, |o, _| o.dismissed), 0, "a pick is not a dismissal");
        // An owner that keeps its surface open after a pick (it draws the
        // anchor again) gets a new pop-up.
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        let again = pops(cx);
        assert_eq!(again.len(), 1);
        assert_ne!(again[0].0.window_id(), pop.window_id());
        // Drawn in the window itself (not hosted): the pick runs there, now.
        let r = ran.clone();
        r.set(None);
        cx.update(|window, cx| pick(window, cx, move |window, _| r.set(Some(id_of(window)))));
        assert_eq!(ran.get(), Some(parent));
    }

    #[test]
    fn a_dialog_or_a_pop_up_with_one_of_its_own_stays_without_the_keyboard() {
        assert!(!keeps_without_keys(false, false), "a menu closes");
        assert!(keeps_without_keys(false, true), "its own pop-up took the keyboard");
        assert!(keeps_without_keys(true, false), "a dialog: another app took it");
        assert!(keeps_without_keys(true, true));
    }

    #[test]
    fn a_dialog_is_centred_on_whole_points() {
        let view = size(px(1000.), px(701.));
        assert_eq!(centred(view, size(px(440.), px(200.))), point(px(280.), px(250.)));
    }

    /// A surface with a surface of its own in it (`inner`), as a dialog
    /// with a select: `modal` makes the outer one a dialog.
    struct Nest {
        open: bool,
        inner: Rc<Cell<bool>>,
        modal: bool,
        dismissed: usize,
        inner_dismissed: Rc<Cell<usize>>,
    }

    impl Render for Nest {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let this = cx.weak_entity();
            let (inner, inner_dismissed) = (self.inner.clone(), self.inner_dismissed.clone());
            // Built from values: the pop-up must not read its owner.
            let build: Build = Rc::new(move |_, _| {
                let (open, told) = (inner.clone(), inner_dismissed.clone());
                let pop = Pop {
                    at: None,
                    radius: 6.,
                    focus: None,
                    build: Rc::new(|_, _| div().w(px(60.)).h(px(20.)).into_any_element()),
                    on_dismiss: Rc::new(move |_, _| {
                        open.set(false);
                        told.set(told.get() + 1);
                    }),
                };
                div()
                    .w(px(200.))
                    .h(px(100.))
                    .when(inner.get(), |d| d.child(div().absolute().left(px(10.)).top(px(40.)).child(anchor("inner", pop))))
                    .into_any_element()
            });
            let pop = Pop {
                at: None,
                radius: 6.,
                focus: None,
                build,
                on_dismiss: Rc::new(move |_, cx| {
                    let _ = this.update(cx, |o, cx| {
                        o.open = false;
                        o.dismissed += 1;
                        cx.notify();
                    });
                }),
            };
            let a = anchor("outer", pop);
            let a = if self.modal { a.modal(window.viewport_size()) } else { a };
            div().size_full().when(self.open, |d| d.child(div().absolute().left(px(40.)).top(px(30.)).child(a)))
        }
    }

    /// The pop-ups of the window `of`.
    fn pops_of(of: WindowId, cx: &mut VisualTestContext) -> Vec<AnyWindowHandle> {
        let all = cx.windows();
        open_pops().into_iter().filter(|(_, p, _)| p.window_id() == of).filter_map(|(id, _, _)| all.iter().find(|w| w.window_id() == id).copied()).collect()
    }

    /// An open surface with its inner one open: (owner, outer, inner).
    fn nest(modal: bool, cx: &mut TestAppContext) -> (Entity<Nest>, AnyWindowHandle, AnyWindowHandle, &mut VisualTestContext) {
        liquid(cx, crate::theme::Look::Glass);
        cx.update(|cx| cx.set_global(TestHosted));
        let (view, cx) = cx.add_window_view(|_, _| Nest {
            open: true,
            inner: Rc::new(Cell::new(false)),
            modal,
            dismissed: 0,
            inner_dismissed: Rc::new(Cell::new(0)),
        });
        cx.run_until_parked();
        let outer = pops(cx)[0].0;
        cx.update_window(outer, |_, window, _| window.activate_window()).unwrap();
        cx.run_until_parked();
        // Its own surface opens and takes the keyboard.
        view.read_with(cx, |n, _| n.inner.set(true));
        cx.update_window(outer, |_, window, _| window.refresh()).unwrap();
        cx.run_until_parked();
        let inner = pops_of(outer.window_id(), cx);
        assert_eq!(inner.len(), 1, "a pop-up of the pop-up");
        cx.update_window(inner[0], |_, window, _| window.activate_window()).unwrap();
        cx.run_until_parked();
        (view, outer, inner[0], cx)
    }

    #[gpui::test]
    fn a_pop_up_stays_while_one_in_it_has_the_keyboard(cx: &mut TestAppContext) {
        let (view, outer, inner, cx) = nest(false, cx);
        assert_eq!(pops(cx).len(), 1, "the outer one lost the keyboard to its own and stays");
        assert_eq!(open_pops().len(), 2);
        cx.update(|window, _| assert!(active(window), "the window is still the one the user is in"));
        // Where the inner one is: in the outer one's coordinates.
        let at = open_pops().into_iter().find(|(id, ..)| *id == inner.window_id()).unwrap().2.unwrap();
        assert_eq!(at, b(10. + PAD - PAD, 40. + PAD - PAD, 60. + 2. * PAD, 20. + 2. * PAD));
        // Esc: the innermost only, told in the outer one's window.
        cx.update_window(inner, |_, window, cx| window.dispatch_action(Box::new(crate::menu::Dismiss), cx)).unwrap();
        cx.run_until_parked();
        assert!(pops_of(outer.window_id(), cx).is_empty());
        assert_eq!(pops(cx).len(), 1);
        assert_eq!(view.read_with(cx, |n, _| (n.inner_dismissed.get(), n.dismissed)), (1, 0));
    }

    #[gpui::test]
    fn the_keyboard_back_in_the_outer_pop_up_closes_the_inner_one_only(cx: &mut TestAppContext) {
        let (view, outer, _, cx) = nest(false, cx);
        // A press in the outer one.
        cx.update_window(outer, |_, window, _| window.activate_window()).unwrap();
        cx.run_until_parked();
        assert!(pops_of(outer.window_id(), cx).is_empty());
        assert_eq!(pops(cx).len(), 1);
        assert_eq!(view.read_with(cx, |n, _| (n.inner_dismissed.get(), n.dismissed)), (1, 0));
    }

    #[gpui::test]
    fn the_keyboard_leaving_both_closes_both(cx: &mut TestAppContext) {
        let (view, _, _, cx) = nest(false, cx);
        // A press in the window under them.
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert!(open_pops().is_empty());
        assert_eq!(view.read_with(cx, |n, _| (n.inner_dismissed.get(), n.dismissed, n.open)), (1, 1, false));
    }

    #[gpui::test]
    fn closing_a_pop_up_closes_those_in_it(cx: &mut TestAppContext) {
        let (view, outer, _, cx) = nest(false, cx);
        // Esc in the outer one (as when its owner's window sends it).
        cx.update_window(outer, |_, window, cx| window.dispatch_action(Box::new(crate::menu::Dismiss), cx)).unwrap();
        cx.run_until_parked();
        assert!(open_pops().is_empty());
        assert_eq!(view.read_with(cx, |n, _| (n.inner_dismissed.get(), n.dismissed)), (0, 1), "the inner one's owner went with it");
        // And when the owner stops drawing the outer one.
        let (view, _, _, cx) = nest(false, cx);
        view.update(cx, |n, cx| {
            n.open = false;
            cx.notify();
        });
        cx.run_until_parked();
        // (The test platform draws at the end of an update: what the
        // frame let go of is released in the next.)
        cx.update(|_, _| {});
        cx.run_until_parked();
        assert!(open_pops().is_empty());
        assert_eq!(view.read_with(cx, |n, _| n.dismissed), 0);
    }

    #[gpui::test]
    fn a_dialog_is_centred_and_keeps_through_the_keyboard_leaving(cx: &mut TestAppContext) {
        let (view, outer, inner, cx) = nest(true, cx);
        // Centred in the window (not at its anchor), with its clear space.
        let v = cx.update(|window, _| window.viewport_size());
        let at = pops(cx)[0].1.unwrap();
        let want = centred(v, size(px(200.), px(100.)));
        assert_eq!(at, Bounds::new(want, size(px(200.), px(100.))).dilate(px(PAD)));
        // The keyboard goes to the window under it (the user comes back
        // from another app): its select closes, the dialog stays and has
        // the keyboard again.
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert!(pops_of(outer.window_id(), cx).is_empty(), "{inner:?} closed");
        assert_eq!(pops(cx).len(), 1, "the dialog stays");
        assert_eq!(view.read_with(cx, |n, _| (n.inner_dismissed.get(), n.dismissed)), (1, 0));
        cx.update_window(outer, |_, window, _| assert!(window.is_window_active(), "and takes the keyboard back")).unwrap();
        // Esc closes it.
        cx.update_window(outer, |_, window, cx| window.dispatch_action(Box::new(crate::menu::Dismiss), cx)).unwrap();
        cx.run_until_parked();
        assert!(open_pops().is_empty());
        assert_eq!(view.read_with(cx, |n, _| (n.dismissed, n.open)), (1, false));
    }

    #[gpui::test]
    fn a_dialog_larger_than_its_window_is_laid_out_in_what_there_is(cx: &mut TestAppContext) {
        struct Tall;
        impl Render for Tall {
            fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let pop = Pop {
                    at: None,
                    radius: 6.,
                    focus: None,
                    build: Rc::new(|_, _| div().w(px(300.)).h(px(5000.)).max_h_full().into_any_element()),
                    on_dismiss: Rc::new(|_, _| {}),
                };
                div().size_full().child(anchor("tall", pop).modal(window.viewport_size()))
            }
        }
        liquid(cx, crate::theme::Look::Glass);
        cx.update(|cx| cx.set_global(TestHosted));
        let (_, cx) = cx.add_window_view(|_, _| Tall);
        cx.run_until_parked();
        let v = cx.update(|window, _| window.viewport_size());
        let at = pops(cx)[0].1.unwrap();
        let g = px(MODAL_GUTTER);
        assert_eq!(at.size.height, v.height - g * 2. + px(2. * PAD), "24 px from the window's edges");
        assert_eq!(at.origin.y, g - px(PAD));
    }
}
