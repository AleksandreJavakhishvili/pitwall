//! Floating surfaces that only show (toasts, tooltips, hovers) as glass
//! pop-up windows: [`super::pop`]'s passive kind.
//!
//! Where [`super::pop::hosted`] says so (Liquid Glass on macOS 26), such a
//! surface puts an [`anchor`] where it would have drawn itself, and while
//! the anchor is drawn its pop-up is open above the window. Unlike a
//! menu's pop-up it is not something the user is in:
//!
//! - it never has the keyboard, and the window under it stays the one
//!   the user is in (`pop::over` and `pop::active` don't count it);
//! - nothing dismisses it: not Esc, not a press outside it, not the
//!   keyboard going elsewhere. It closes when its anchor isn't drawn any
//!   more (its owner took it away, or went, or the window closed);
//! - clicks go through it to what is under ([`Passive::clicks`] off:
//!   tooltips, hovers), or its panels take them without taking the
//!   keyboard (toasts: their buttons work, the terminal keeps the
//!   typing) while the clear space around and between them still lets
//!   them through;
//! - it is part of its window: it moves with it, sits at its level (not
//!   above other apps), and stays when the app isn't the active one.
//!
//! Its place is a point of the parent window and the point of its own
//! content that sits there ([`Passive::pin`]): toasts hang from their
//! bottom right corner, a tooltip from its top left. It is kept on the
//! screen that shows that point, sized to its content.
//!
//! The window is clear: what is built draws its glass itself, with one
//! [`panel`] per piece (each toast of a stack is one; the gaps between
//! them stay clear).
//!
//! The rule of [`super::pop::Build`] holds here too: what is built must
//! not read or update an entity the parent window draws. It is built from
//! values, and a handler in it reaches the owner through [`in_parent`].

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    div, point, prelude::*, px, size, AnyElement, AnyWindowHandle, App, AvailableSpace, Bounds, Context,
    DispatchPhase, Element, ElementId, Entity, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId,
    MouseExitEvent, MouseMoveEvent, Pixels, Point, Render, Size,
    Style, Subscription, WeakEntity, Window, WindowBackgroundAppearance, WindowBounds, WindowId, WindowKind,
    WindowOptions,
};

use super::motion::{self, Frame, Fx};
use super::pop::{self, PAD};
use crate::platform::glass::Piece;
use crate::platform::{self, PopParent};
use crate::theme::{self, Theme};

/// The gap kept to the screen's edges.
const MARGIN: f32 = 4.;
/// How much of the theme's own surface a [`panel`] lays over the glass
/// (as a menu's pop-up).
const FILL: f32 = 0.72;

/// Builds the surface's content, in the pop-up's window (see the module).
pub type Build = pop::Build;

/// A surface that only shows, in a window of its own.
pub struct Passive {
    /// The point of the parent window it is placed by (`None`: where the
    /// anchor is).
    pub at: Option<Point<Pixels>>,
    /// The point of its content that sits there, as fractions of its
    /// size: (0, 0) its top left, (1, 1) its bottom right, (0.5, 1) the
    /// middle of its bottom edge.
    pub pin: (f32, f32),
    /// It takes clicks (without the keyboard). Off: they go through it.
    pub clicks: bool,
    /// Changes when what `build` builds does: the pop-up is drawn again
    /// then (and when its place changes), not every time its anchor is.
    pub stamp: u64,
    pub build: Build,
}

/// A [`Passive::stamp`] for what is built from `what` alone.
pub fn stamp_of(what: &impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    what.hash(&mut h);
    h.finish()
}

/// The theme inside a passive pop-up (a menu's pop-up's).
pub fn theme(cx: &App) -> Theme {
    pop::theme(cx)
}

/// The top left of content of `size` whose `pin` point sits at `at`: on
/// whole pixels where the pin divides the size (a centred panel's text
/// would blur between two).
pub fn spot(at: Point<Pixels>, size: Size<Pixels>, pin: (f32, f32)) -> Point<Pixels> {
    let axis = |at: Pixels, len: Pixels, pin: f32| if pin == 0. { at } else { (at - len * pin).round() };
    point(axis(at.x, size.width, pin.0), axis(at.y, size.height, pin.1))
}

struct Shared {
    spec: Passive,
    /// The anchor's place in the parent window.
    at: Point<Pixels>,
    /// The parent window on its screen, when the anchor was last drawn (a
    /// window resized from its top or left keeps its content's places
    /// and moves them on the screen).
    parent_frame: Bounds<Pixels>,
    view: Option<WeakEntity<PassiveView>>,
    /// The pop-up's window, once it is open.
    window: Option<AnyWindowHandle>,
    /// Where it takes clicks (its panels), in the parent window.
    zones: Vec<Bounds<Pixels>>,
}

/// The mouse is for the pop-up: over one of its panels. Elsewhere its
/// window lets the mouse through (a window takes clicks as a whole, its
/// clear parts too, or not at all: so it is switched as the pointer
/// comes and goes).
pub fn takes_mouse(zones: &[Bounds<Pixels>], pointer: Point<Pixels>) -> bool {
    zones.iter().any(|z| z.contains(&pointer))
}

/// Where a pop-up was last put: its window in the parent's coordinates,
/// for which frame of the parent.
type Placed = Rc<Cell<Option<(Bounds<Pixels>, Bounds<Pixels>)>>>;

struct Entry {
    parent: AnyWindowHandle,
    placed: Placed,
    /// Its panels take clicks.
    clicks: bool,
}

thread_local! {
    /// The passive pop-ups that are open, by their window.
    static OPEN: RefCell<HashMap<WindowId, Entry>> = RefCell::new(HashMap::new());
    /// The panels of the pop-up being drawn (one window draws at a time).
    static PANELS: RefCell<Vec<Piece>> = const { RefCell::new(Vec::new()) };
}

fn id_of(window: &Window) -> WindowId {
    window.window_handle().window_id()
}

/// The window `window` is a passive pop-up of.
pub fn parent_of(window: &Window) -> Option<AnyWindowHandle> {
    OPEN.with_borrow(|all| all.get(&id_of(window)).map(|e| e.parent))
}

/// The open passive pop-ups: (pop-up, parent, where it was put). Tests.
#[cfg(test)]
pub(crate) fn open_passives() -> Vec<(WindowId, AnyWindowHandle, Option<Bounds<Pixels>>)> {
    OPEN.with_borrow(|all| all.iter().map(|(id, e)| (*id, e.parent, e.placed.get().map(|p| p.0))).collect())
}

/// Tests: Liquid Glass, with floating surfaces as pop-ups.
#[cfg(test)]
pub(crate) fn test_glass(cx: &mut App) {
    use crate::theme::{Appearance, GlassOffer, Inputs, Look, OsPrefs};
    let inputs = Inputs { look: Look::Glass, ..Inputs::default() };
    let ap = Appearance::resolve(inputs, gpui::WindowAppearance::Dark, GlassOffer::Liquid, OsPrefs::default());
    cx.set_global(ap.theme());
    cx.set_global(ap);
    cx.set_global(pop::TestHosted);
}

/// Tests: the passive pop-ups open over `window`.
#[cfg(test)]
pub(crate) fn test_over(window: &Window, cx: &App) -> Vec<AnyWindowHandle> {
    let (parent, all) = (id_of(window), cx.windows());
    let open = open_passives().into_iter().filter(|(_, p, _)| p.window_id() == parent);
    open.filter_map(|(id, _, _)| all.iter().find(|w| w.window_id() == id).copied()).collect()
}

/// Tests: draw the window and its passive pop-ups as they are now (a
/// test window draws when told to).
#[cfg(test)]
pub(crate) fn test_draw(cx: &mut gpui::VisualTestContext) {
    for _ in 0..2 {
        cx.update(|window, cx| {
            window.refresh();
            for pop in test_over(window, cx) {
                let _ = pop.update(cx, |_, window, _| window.refresh());
            }
        });
        cx.run_until_parked();
    }
}

/// Run `f` in the window the surface belongs to: now when it is drawn in
/// that window, and from a pop-up in its parent window, once this event
/// is done. For the handlers of what a passive surface shows (a toast's
/// buttons), which are the same wherever it is drawn.
pub fn in_parent(window: &mut Window, cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
    match parent_of(window) {
        None => f(window, cx),
        Some(parent) => cx.defer(move |cx| {
            let _ = parent.update(cx, |_, window, cx| f(window, cx));
        }),
    }
}

/// Close the passive pop-up `window` (nothing when it isn't one).
fn shut(window: &mut Window, cx: &mut App) {
    if OPEN.with_borrow_mut(|all| all.remove(&id_of(window))).is_none() {
        return;
    }
    platform::glass::pieces(window, &[], 0., theme::theme(cx).mode);
    platform::pop_close(window, cx);
    window.remove_window();
}

/// `PITWALL_POP_LOG=1`: what passive pop-ups do, on stderr.
fn log(what: impl FnOnce() -> String) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("PITWALL_POP_LOG").is_some()) {
        eprintln!("pitwall-passive: {}", what());
    }
}

/// Debug builds, from the bench hook (`passive <what>` in `bench-cmd`):
/// look at the open passive pop-ups of this process without synthetic
/// input. `state [<x> <y>]` logs where each is and what the system says
/// of it (with a point of it: which window a press there would go to),
/// `click <x> <y>` is a click as AppKit delivers one in those that take
/// clicks (macOS), `move <x> <y>` the pointer moving there as far as
/// they know.
pub fn debug(cmd: &str, cx: &mut App) {
    if !cfg!(debug_assertions) {
        return;
    }
    let (what, rest) = cmd.split_once(' ').unwrap_or((cmd, ""));
    let mut nums = rest.split_whitespace().filter_map(|v| v.parse::<f32>().ok());
    let at = nums.next().zip(nums.next());
    let open: Vec<_> = OPEN.with_borrow(|all| all.iter().map(|(id, e)| (*id, e.placed.get(), e.clicks)).collect());
    if open.is_empty() {
        eprintln!("pitwall-passive: debug {cmd}: none is open");
    }
    for (id, placed, clicks) in open {
        let Some(handle) = cx.windows().into_iter().find(|w| w.window_id() == id) else { continue };
        let _ = handle.update(cx, |_, window, cx| match (what, at) {
            ("click", Some((x, y))) if clicks => platform::debug_click(window, x, y, cx),
            ("click", _) => {}
            ("move", Some((x, y))) => {
                let event = MouseMoveEvent { position: point(px(x), px(y)), pressed_button: None, modifiers: Default::default() };
                window.dispatch_event(gpui::PlatformInput::MouseMove(event), cx);
            }
            _ => eprintln!(
                "pitwall-passive: state: placed {:?}, clicks {clicks}, gpui active {}, {}",
                placed.map(|p| p.0),
                window.is_window_active(),
                platform::passive_debug(window, at)
            ),
        });
    }
}

/// Lives as long as the anchor is drawn; the pop-up closes when it goes.
struct Token;

/// What the anchor keeps between frames.
struct Held {
    shared: Rc<RefCell<Shared>>,
    _token: Entity<Token>,
}

/// An anchor's pop-up that takes clicks, between its prepaint and paint.
pub struct Clicks(Rc<RefCell<Shared>>);

/// Where a passive surface would have drawn itself: see the module.
pub struct Anchor {
    id: ElementId,
    spec: Option<Passive>,
}

/// The passive pop-up `spec` is open while this is drawn. Give it a place
/// of no size where the surface's pin should be (or set [`Passive::at`]).
pub fn anchor(id: impl Into<ElementId>, spec: Passive) -> Anchor {
    Anchor { id: id.into(), spec: Some(spec) }
}

impl Element for Anchor {
    type RequestLayoutState = ();
    /// Its pop-up, when that takes clicks.
    type PrepaintState = Option<Clicks>;

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
    ) -> Self::PrepaintState {
        let (id, spec) = (id?, self.spec.take()?);
        let clicks = spec.clicks;
        let at = spec.at.unwrap_or(bounds.origin);
        let parent = window.window_handle();
        let parent_frame = window.bounds();
        window.with_element_state::<Held, _>(id, |held, _| {
            let held = match held {
                Some(held) => {
                    let view = {
                        let mut s = held.shared.borrow_mut();
                        let same = s.spec.stamp == spec.stamp
                            && s.spec.pin == spec.pin
                            && s.spec.clicks == spec.clicks
                            && s.at == at
                            && s.parent_frame == parent_frame;
                        s.spec = spec;
                        s.at = at;
                        s.parent_frame = parent_frame;
                        s.view.as_ref().filter(|_| !same).and_then(|v| v.upgrade())
                    };
                    // It shows something else, or elsewhere: drawn again.
                    if let Some(view) = view {
                        cx.notify(view.entity_id());
                    }
                    held
                }
                None => {
                    let shared =
                        Rc::new(RefCell::new(Shared { spec, at, parent_frame, view: None, window: None, zones: Vec::new() }));
                    let token = cx.new(|_| Token);
                    let (s, weak, asked) = (shared.clone(), token.downgrade(), Instant::now());
                    // Once this frame is drawn: a window can't be opened
                    // while another draws.
                    cx.defer(move |cx| open(s, weak, parent, asked, cx));
                    Held { shared, _token: token }
                }
            };
            (clicks.then(|| Clicks(held.shared.clone())), held)
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        shared: &mut Self::PrepaintState,
        window: &mut Window,
        _: &mut App,
    ) {
        // The pointer comes over one of its panels: from here on the
        // mouse is the pop-up's (which gives it back: `Measured`). A
        // drag that passes over them stays the window's.
        let Some(Clicks(shared)) = shared.take() else { return };
        window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture || e.pressed_button.is_some() {
                return;
            }
            let pop = {
                let s = shared.borrow();
                s.window.filter(|_| takes_mouse(&s.zones, e.position))
            };
            if let Some(pop) = pop {
                let _ = pop.update(cx, |_, window, _| platform::passive_through(window, false));
            }
        });
    }
}

/// Open the pop-up's window (not shown until it knows its size).
fn open(shared: Rc<RefCell<Shared>>, token: WeakEntity<Token>, parent: AnyWindowHandle, asked: Instant, cx: &mut App) {
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
    let placed: Placed = Rc::new(Cell::new(None));
    let opened = cx.open_window(options, |window, cx| {
        platform::lean_renderer(window);
        platform::pop_setup(window);
        platform::passive_through(window, true);
        let clicks = {
            let mut s = shared.borrow_mut();
            s.window = Some(window.window_handle());
            s.spec.clicks
        };
        OPEN.with_borrow_mut(|all| all.insert(id_of(window), Entry { parent, placed: placed.clone(), clicks }));
        let view = cx.new(|cx| PassiveView::new(shared.clone(), &token, parent, native, placed.clone(), window, cx));
        shared.borrow_mut().view = Some(view.downgrade());
        view
    });
    match opened {
        Ok(_) => log(|| format!("opened in {:.1} ms (its window made and drawn once)", asked.elapsed().as_secs_f64() * 1e3)),
        // Nothing to show it in: it isn't shown.
        Err(e) => eprintln!("pitwall: no pop-up window on this system: {e}"),
    }
}

/// The pop-up window's content: what the owner builds, on a clear window.
pub struct PassiveView {
    shared: Rc<RefCell<Shared>>,
    parent: AnyWindowHandle,
    native: Rc<PopParent>,
    placed: Placed,
    _subs: [Subscription; 2],
}

impl PassiveView {
    fn new(
        shared: Rc<RefCell<Shared>>,
        token: &Entity<Token>,
        parent: AnyWindowHandle,
        native: PopParent,
        placed: Placed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> PassiveView {
        let subs = [
            // The anchor isn't drawn any more.
            cx.observe_release_in(token, window, |_, _, window, cx| shut(window, cx)),
            cx.observe_global::<Theme>(|_, cx| cx.notify()),
        ];
        PassiveView { shared, parent, native: Rc::new(native), placed, _subs: subs }
    }

    /// A press in it is a press in its window: that one comes forward
    /// (the pop-up itself never activates anything). Not in a bench run,
    /// which never becomes the active app.
    fn pressed(&mut self, cx: &mut Context<Self>) {
        if crate::bench::enabled() {
            return;
        }
        let parent = self.parent;
        cx.defer(move |cx| {
            let _ = parent.update(cx, |_, window, _| {
                if !window.is_window_active() {
                    window.activate_window();
                }
            });
        });
    }
}

impl Render for PassiveView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().id("passive").size_full();
        if parent_of(window).is_none() {
            return root;
        }
        let t = theme(cx);
        let build = self.shared.borrow().spec.build.clone();
        let content = build(window, cx);
        let measured = Measured {
            child: Some(content),
            shared: self.shared.clone(),
            native: self.native.clone(),
            placed: self.placed.clone(),
            frame: Bounds::default(),
        };
        root.capture_any_mouse_down(cx.listener(|this, _, _, cx| this.pressed(cx)))
            .p(px(PAD))
            .font_family(super::fonts::UI_FONT)
            .text_size(px(13.))
            .line_height(gpui::relative(super::fonts::BODY_LINE_HEIGHT))
            .text_color(t.text)
            .child(measured)
    }
}

/// The content at its own size; the window is put where that size goes,
/// and gets the glass its panels ask for.
struct Measured {
    child: Option<AnyElement>,
    shared: Rc<RefCell<Shared>>,
    native: Rc<PopParent>,
    placed: Placed,
    /// The window the content was laid out for, in the parent's.
    frame: Bounds<Pixels>,
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
        let (at, pin, parent_frame) = {
            let s = self.shared.borrow();
            (s.at, s.spec.pin, s.parent_frame)
        };
        let area = platform::pop_area(&self.native, at);
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
        // The window: the content (kept on the screen) and the clear
        // space around it (which may leave the screen).
        let frame = pop::place(spot(at, content, pin), content, area, px(MARGIN)).dilate(px(PAD));
        let put = (frame, parent_frame);
        if self.placed.get() != Some(put) {
            let first = self.placed.replace(Some(put)).is_none();
            log(|| format!("{} at {frame:?}", if first { "shown" } else { "moved" }));
            platform::passive_place(window, &self.native, frame, cx);
        }
        self.frame = frame;
        PANELS.with_borrow_mut(|all| all.clear());
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
        let panels = PANELS.with_borrow_mut(std::mem::take);
        platform::glass::pieces(window, &panels, self.frame.size.height.into(), theme(cx).mode);
        if !self.shared.borrow().spec.clicks {
            return;
        }
        // Its panels take the mouse, the clear space lets it through:
        // as the pointer is now (a toast that shows under a pointer at
        // rest), and when it leaves them. (It comes over them in the
        // parent window: `Anchor`.)
        let zones: Rc<[Bounds<Pixels>]> = panels.iter().map(|p| p.bounds).collect();
        let origin = self.frame.origin;
        self.shared.borrow_mut().zones = zones.iter().map(|z| Bounds::new(z.origin + origin, z.size)).collect();
        let over = platform::passive_pointer(window).is_some_and(|at| takes_mouse(&zones, at));
        platform::passive_through(window, !over);
        window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, _| {
            if phase == DispatchPhase::Capture && e.pressed_button.is_none() && !takes_mouse(&zones, e.position) {
                platform::passive_through(window, true);
            }
        });
        window.on_mouse_event(|_: &MouseExitEvent, phase, window, _| {
            if phase == DispatchPhase::Capture {
                platform::passive_through(window, true);
            }
        });
    }
}

/// A piece of glass in a passive pop-up with `child` on it: the theme's
/// own surface, thinly, over the system's glass, with a hairline and no
/// shadow of ours (it would show through the glass; the glass has its
/// own). Only for what a [`Passive`] builds.
pub fn panel(radius: f32, child: impl IntoElement) -> Panel {
    Panel { radius, ring: None, enter: None, child: Some(child.into_any_element()) }
}

/// See [`panel`].
pub struct Panel {
    radius: f32,
    ring: Option<Hsla>,
    enter: Option<(ElementId, Fx)>,
    child: Option<AnyElement>,
}

impl Panel {
    /// Its hairline, when not the theme's.
    pub fn ring(mut self, color: Hsla) -> Self {
        self.ring = Some(color);
        self
    }

    /// Play `fx` when it first shows (as [`motion::enter`]): the glass
    /// fades and moves with what is drawn on it. The glass doesn't
    /// scale, so neither does the panel.
    pub fn enter(mut self, id: impl Into<ElementId>, fx: Fx) -> Self {
        self.enter = Some((id.into(), Fx { scale: 1., ..fx }));
        self
    }
}

/// Where an opening that started at `start` is now (`None`: at rest).
/// Reduce motion keeps the fade and drops the movement, as
/// [`motion::enter`] does.
fn entering(fx: Fx, start: Instant, cx: &App) -> Option<Frame> {
    let full = motion::on() && theme::motion_on(cx);
    let fade = !full && motion::on() && fx.opacity < 1.;
    if !full && !fade {
        return None;
    }
    let t = start.elapsed().as_secs_f32() / fx.dur.as_secs_f32();
    (t < 1.).then(|| if full { fx.frame(t) } else { fx.frame(t).fade_only() })
}

impl Element for Panel {
    /// What is drawn, and where its opening is.
    type RequestLayoutState = (AnyElement, Option<Frame>);
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        self.enter.as_ref().map(|(id, _)| id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let t = theme(cx);
        let surface = div()
            .rounded(px(self.radius))
            .bg(t.raised.opacity(FILL))
            .shadow(vec![t.ring(self.ring.unwrap_or(t.g.line_strong))])
            .overflow_hidden()
            .children(self.child.take());
        let frame = self.enter.as_ref().zip(id).and_then(|((_, fx), id)| {
            let start = window.with_element_state(id, |s: Option<Instant>, _| {
                let s = s.unwrap_or_else(Instant::now);
                (s, s)
            });
            entering(*fx, start, cx)
        });
        let mut el = match frame {
            Some(frame) => {
                window.request_animation_frame();
                motion::transformed(surface, frame).into_any_element()
            }
            None => surface.into_any_element(),
        };
        (el.request_layout(window, cx), (el, frame))
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        state.0.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let (dx, dy, alpha) = state.1.map_or((0., 0., 1.), |f| (f.dx, f.dy, f.opacity));
        let bounds = Bounds::new(bounds.origin + point(px(dx), px(dy)), bounds.size);
        PANELS.with_borrow_mut(|all| all.push(Piece { bounds, radius: self.radius, alpha }));
        state.0.paint(window, cx);
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
    fn content_hangs_from_its_pin() {
        let (at, sz) = (point(px(300.), px(200.)), size(px(101.), px(50.)));
        assert_eq!(spot(at, sz, (0., 0.)), at, "a tooltip: its top left");
        assert_eq!(spot(at, sz, (1., 1.)), point(px(199.), px(150.)), "toasts: their bottom right");
        // A notice: the middle of its bottom edge, on whole pixels.
        assert_eq!(spot(at, sz, (0.5, 1.)), point(px(250.), px(150.)));
        // A top left pin keeps the half pixel a pointer may be at.
        assert_eq!(spot(point(px(10.5), px(20.5)), sz, (0., 0.)), point(px(10.5), px(20.5)));
    }

    #[test]
    fn the_mouse_is_the_pop_ups_only_over_its_panels() {
        let zones = [b(24., 24., 320., 60.), b(24., 92., 320., 40.)];
        assert!(takes_mouse(&zones, point(px(100.), px(50.))), "a toast");
        assert!(takes_mouse(&zones, point(px(100.), px(100.))), "the next one");
        assert!(!takes_mouse(&zones, point(px(100.), px(88.))), "the gap between them");
        assert!(!takes_mouse(&zones, point(px(10.), px(10.))), "the clear space around");
        assert!(!takes_mouse(&[], point(px(100.), px(50.))), "nothing drawn yet");
    }

    #[test]
    fn a_stamp_follows_what_it_is_made_of() {
        assert_eq!(stamp_of(&"Saved"), stamp_of(&"Saved"));
        assert_ne!(stamp_of(&"Saved"), stamp_of(&"Copied"));
    }

    /// A surface's owner: an anchor while `open`, with what it shows.
    struct Owner {
        open: bool,
        stamp: u64,
        clicks: bool,
        built: Rc<Cell<usize>>,
    }

    impl Render for Owner {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let built = self.built.clone();
            let spec = Passive {
                at: None,
                pin: (1., 1.),
                clicks: self.clicks,
                stamp: self.stamp,
                build: Rc::new(move |_, _| {
                    built.set(built.get() + 1);
                    panel(6., div().w(px(100.)).h(px(50.))).into_any_element()
                }),
            };
            div().size_full().when(self.open, |d| d.child(div().absolute().left(px(400.)).top(px(300.)).child(anchor("surface", spec))))
        }
    }

    fn owner(cx: &mut TestAppContext, clicks: bool) -> (Entity<Owner>, &mut VisualTestContext) {
        cx.update(test_glass);
        let built = Rc::new(Cell::new(0));
        let (view, cx) = cx.add_window_view(|_, _| Owner { open: false, stamp: 1, clicks, built });
        cx.run_until_parked();
        (view, cx)
    }

    /// The passive pop-ups of the test's window.
    fn passives(cx: &mut VisualTestContext) -> Vec<(AnyWindowHandle, Option<Bounds<Pixels>>)> {
        let parent = cx.update(|window, _| id_of(window));
        let all = cx.windows();
        open_passives()
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
        let open = passives(cx);
        assert_eq!(open.len(), 1, "one pop-up for the anchor");
        open[0].0
    }

    #[gpui::test]
    fn an_anchor_opens_its_pop_up_hung_from_its_pin(cx: &mut TestAppContext) {
        let (view, cx) = owner(cx, false);
        assert!(passives(cx).is_empty());
        let pop = open(&view, cx);
        // The content (100 × 50) with its bottom right corner on the
        // anchor, and the clear space around it.
        assert_eq!(passives(cx)[0].1, Some(b(300. - PAD, 250. - PAD, 100. + 2. * PAD, 50. + 2. * PAD)));
        let parent = cx.update(|window, _| id_of(window));
        cx.update_window(pop, |_, window, _| {
            assert_eq!(parent_of(window).map(|p| p.window_id()), Some(parent));
            assert!(pop::parent_of(window).is_none(), "not a pop-up that takes the keyboard");
        })
        .unwrap();
        // The window isn't one with a menu open over it.
        cx.update(|window, _| assert!(!pop::over(window)));
        assert!(pop::open_pops().is_empty());
    }

    #[gpui::test]
    fn the_keyboard_coming_and_going_leaves_it_open(cx: &mut TestAppContext) {
        let (view, cx) = owner(cx, true);
        let pop = open(&view, cx);
        // Its window is made the active one, then the parent again (as a
        // click in each would do with a pop-up that has the keyboard).
        cx.update_window(pop, |_, window, _| window.activate_window()).unwrap();
        cx.run_until_parked();
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert_eq!(passives(cx).len(), 1);
        // Esc and a press on its clear space neither.
        cx.update_window(pop, |_, window, cx| {
            window.dispatch_action(Box::new(crate::menu::Dismiss), cx);
            let position = point(px(4.), px(4.));
            let (button, modifiers) = (gpui::MouseButton::Left, gpui::Modifiers::default());
            window.dispatch_event(gpui::PlatformInput::MouseDown(gpui::MouseDownEvent { position, button, modifiers, click_count: 1, first_mouse: false }), cx);
            window.dispatch_event(gpui::PlatformInput::MouseUp(gpui::MouseUpEvent { position, button, modifiers, click_count: 1 }), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(passives(cx).len(), 1);
    }

    #[gpui::test]
    fn it_closes_when_its_anchor_is_gone(cx: &mut TestAppContext) {
        let (view, cx) = owner(cx, false);
        open(&view, cx);
        view.update(cx, |o, cx| {
            o.open = false;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(passives(cx).is_empty());
        // And opens again with it.
        open(&view, cx);
    }

    #[gpui::test]
    fn it_closes_with_its_window(cx: &mut TestAppContext) {
        let (view, cx) = owner(cx, false);
        let pop = open(&view, cx);
        cx.update(|window, _| window.remove_window());
        cx.run_until_parked();
        assert!(open_passives().iter().all(|(id, _, _)| *id != pop.window_id()));
    }

    #[gpui::test]
    fn it_is_drawn_again_when_its_stamp_or_place_changes(cx: &mut TestAppContext) {
        let (view, cx) = owner(cx, false);
        open(&view, cx);
        let built = view.read_with(cx, |o, _| o.built.clone());
        let n = built.get();
        // The owner draws again with the same thing to show.
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(built.get(), n, "nothing changed: the pop-up is left alone");
        view.update(cx, |o, cx| {
            o.stamp = 2;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(built.get() > n, "something else to show");
        assert_eq!(passives(cx).len(), 1, "in the same pop-up");
    }

    #[gpui::test]
    fn a_handler_in_it_runs_in_the_parent_window(cx: &mut TestAppContext) {
        let (view, cx) = owner(cx, true);
        let pop = open(&view, cx);
        let parent = cx.update(|window, _| id_of(window));
        let ran = Rc::new(Cell::new(None));
        let r = ran.clone();
        cx.update_window(pop, |_, window, cx| in_parent(window, cx, move |window, _| r.set(Some(id_of(window))))).unwrap();
        cx.run_until_parked();
        assert_eq!(ran.get(), Some(parent));
        assert_eq!(passives(cx).len(), 1, "and it stays open");
        // Drawn in the window itself (not hosted): there, now.
        ran.set(None);
        let r = ran.clone();
        cx.update(|window, cx| in_parent(window, cx, move |window, _| r.set(Some(id_of(window)))));
        assert_eq!(ran.get(), Some(parent));
    }
}
