//! Motion: `src/styles/motion.css` and the components' CSS transitions,
//! with the same durations and easings.
//!
//! - [`enter`]: an element's opening animation ([`Fx`]: dialogs, the
//!   palette, menus, toasts, drawers, scrims, route cross-fades). GPUI 0.2.2
//!   has no transforms: the shift is a paint offset (layout never moves)
//!   and the scale is drawn on the element's own surface (background,
//!   border and shadow shrink about the centre; the content only fades).
//! - [`pulse_dot`]: the working dot's pulse (Flat) or ring (Glass), the one
//!   thing that loops. Loops redraw at [`LOOP_FPS`], not every vsync.
//! - [`tween`]: a CSS transition (chevrons, toggles, the status strip,
//!   bars): a value that eases to its new target.
//! - [`keyframes`]: a one-shot keyframe animation played when the element
//!   first shows (or its id changes): "needs you", "done", bounces.
//! - [`press_scale`] / [`PressSkin`]: the press pop on buttons (`m-press`).
//! - [`Sheen`]: the Glass primary button's sheen on hover.
//!
//! Nothing moves with Reduce motion (the setting or the OS:
//! [`crate::theme::motion_on`]). Frames come from the display link, which
//! GPUI stops while the window is hidden or occluded, so every animation
//! pauses then and costs no CPU.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui::{
    point, prelude::*, px, quad, size, transparent_black, AnyElement, App, Background,
    BorderStyle, Bounds, BoxShadow, Corners, Edges, Element, ElementId, EntityId,
    GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, Pixels, Rgba,
    Context, Render, Style, Styled, Window, WindowId,
};

use crate::theme::cubic_bezier;

// ───────────────────────────────────────────── switches

static MOTION: AtomicBool = AtomicBool::new(true);

/// Animations may run: Reduce motion is off (the setting and the OS). Kept
/// by `theme::apply`; this copy serves code that has no `App` at hand.
pub fn on() -> bool {
    MOTION.load(Ordering::Relaxed)
}

/// Set by `theme::apply` whenever the appearance is resolved.
pub fn set_on(on: bool) {
    MOTION.store(on, Ordering::Relaxed);
}

fn motion(cx: &App) -> bool {
    on() && crate::theme::motion_on(cx)
}

// ───────────────────────────────────────────── easing

/// CSS timing functions used by the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ease {
    Linear,
    /// `ease`
    Ease,
    /// `ease-out`
    EaseOut,
    /// `ease-in-out`
    EaseInOut,
    /// `--ease-spring`: cubic-bezier(0.2, 0.9, 0.3, 1.18) (overshoots).
    Spring,
    /// `--ease-out-soft`: cubic-bezier(0.16, 1, 0.3, 1).
    OutSoft,
}

impl Ease {
    /// The eased progress at `t` (0..=1); a spring goes a little past 1.
    pub fn at(self, t: f32) -> f32 {
        let t = t.clamp(0., 1.);
        match self {
            Ease::Linear => t,
            Ease::Ease => cubic_bezier(0.25, 0.1, 0.25, 1.0)(t),
            Ease::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0)(t),
            Ease::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0)(t),
            Ease::Spring => cubic_bezier(0.2, 0.9, 0.3, 1.18)(t),
            Ease::OutSoft => cubic_bezier(0.16, 1.0, 0.3, 1.0)(t),
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// `a` blended towards `b` by `t` (in sRGB, as CSS transitions do).
pub fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    if t <= 0. {
        return a;
    }
    if t >= 1. {
        return b;
    }
    let (x, y) = (Rgba::from(a), Rgba::from(b));
    Rgba {
        r: lerp(x.r, y.r, t),
        g: lerp(x.g, y.g, t),
        b: lerp(x.b, y.b, t),
        a: lerp(x.a, y.a, t),
    }
    .into()
}

// ───────────────────────────────────────────── opening animations

/// An opening animation (CSS `@keyframes` with a `from` only): fade from
/// `opacity`, shift from (`dx`, `dy`) px, scale from `scale`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fx {
    pub dur: Duration,
    pub ease: Ease,
    pub opacity: f32,
    pub dx: f32,
    pub dy: f32,
    pub scale: f32,
    /// `transform-origin`'s y (0: top, 0.5: centre).
    pub origin_y: f32,
}

const fn fx(ms: u64, ease: Ease, dx: f32, dy: f32, scale: f32) -> Fx {
    Fx {
        dur: Duration::from_millis(ms),
        ease,
        opacity: 0.,
        dx,
        dy,
        scale,
        origin_y: 0.5,
    }
}

impl Fx {
    /// `.modal`: `m-open` 0.2s spring (rise 8 px, scale 0.97).
    pub const MODAL: Fx = fx(200, Ease::Spring, 0., 8., 0.97);
    /// The palette's `.modal`: the same, scaling from its top edge.
    pub const PALETTE: Fx = Fx {
        origin_y: 0.,
        ..Fx::MODAL
    };
    /// `.menu`: `m-open-sm` 0.15s soft (drop 4 px, scale 0.98).
    pub const MENU: Fx = fx(150, Ease::OutSoft, 0., -4., 0.98);
    /// `.toast`: `m-toast` 0.24s spring (from 18 px right, 6 px down).
    pub const TOAST: Fx = fx(240, Ease::Spring, 18., 6., 0.98);
    /// `.sidebar-overlay`: `slide-left` 0.2s soft.
    pub const SLIDE_LEFT: Fx = fx(200, Ease::OutSoft, -16., 0., 1.);
    /// `.right-drawer`: `slide-right` 0.2s soft.
    pub const SLIDE_RIGHT: Fx = fx(200, Ease::OutSoft, 16., 0., 1.);
    /// Space / Wall / Review switching: `m-fade` 0.16s ease-out.
    pub const FADE: Fx = fx(160, Ease::EaseOut, 0., 0., 1.);
    /// `.backdrop`, `.scrim`: `fade-in` 0.12s ease-out.
    pub const SCRIM: Fx = fx(120, Ease::EaseOut, 0., 0., 1.);
    /// The code view's find widget sliding down (`.rv-find`: 0.2s linear
    /// from 10 px above its own height; no fade).
    pub const FIND_SLIDE: Fx = Fx {
        opacity: 1.,
        ..fx(200, Ease::Linear, 0., -43., 1.)
    };
    /// Onboarding's screens: `fade-in` 0.2s ease-out.
    pub const ONB_FADE: Fx = fx(200, Ease::EaseOut, 0., 0., 1.);
    /// Onboarding's steps: `rise` 0.22s ease-out.
    pub const ONB_RISE: Fx = fx(220, Ease::EaseOut, 0., 8., 1.);
    /// Onboarding's ticks: `pop` 0.22s ease-out.
    pub const ONB_POP: Fx = fx(220, Ease::EaseOut, 0., 6., 0.985);

    /// The state at `t` (0..=1 of the duration).
    pub fn frame(&self, t: f32) -> Frame {
        let p = self.ease.at(t);
        Frame {
            opacity: lerp(self.opacity, 1., p).clamp(0., 1.),
            dx: self.dx * (1. - p),
            dy: self.dy * (1. - p),
            scale: lerp(self.scale, 1., p),
            origin_y: self.origin_y,
        }
    }
}

/// One frame of an [`Fx`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub opacity: f32,
    pub dx: f32,
    pub dy: f32,
    pub scale: f32,
    pub origin_y: f32,
}

/// An element's surface, drawn scaled while it opens or is pressed.
#[derive(Clone, Default)]
pub struct Skin {
    pub bg: Option<Background>,
    pub border: Option<Hsla>,
    pub widths: Edges<Pixels>,
    pub radii: Corners<Pixels>,
    pub shadows: Vec<BoxShadow>,
}

impl Skin {
    /// A plain rounded surface.
    pub fn new(bg: Option<Hsla>, border: Option<Hsla>, radius: Pixels) -> Skin {
        Skin {
            bg: bg.map(Into::into),
            border,
            widths: Edges::all(if border.is_some() { px(1.) } else { px(0.) }),
            radii: Corners::all(radius),
            shadows: Vec::new(),
        }
    }

    /// Take the surface out of `style` (it is drawn by the animation
    /// meanwhile); the border keeps its width so nothing moves.
    fn take(style: &mut gpui::StyleRefinement, rem: Pixels) -> Skin {
        let len = |l: Option<gpui::AbsoluteLength>| l.map(|l| l.to_pixels(rem)).unwrap_or_default();
        let b = &style.border_widths;
        let r = &style.corner_radii;
        let skin = Skin {
            bg: style.background.take().map(|f| match f {
                gpui::Fill::Color(bg) => bg,
            }),
            border: style.border_color.take(),
            widths: Edges {
                top: len(b.top),
                right: len(b.right),
                bottom: len(b.bottom),
                left: len(b.left),
            },
            radii: Corners {
                top_left: len(r.top_left),
                top_right: len(r.top_right),
                bottom_right: len(r.bottom_right),
                bottom_left: len(r.bottom_left),
            },
            shadows: style.box_shadow.take().unwrap_or_default().into_iter().collect(),
        };
        style.border_color = Some(transparent_black());
        skin
    }

    /// Paint at `bounds` scaled by `scale` about its centre (or, with
    /// `origin_y` 0, its top edge), faded to `opacity`.
    fn paint(&self, bounds: Bounds<Pixels>, scale: f32, origin_y: f32, opacity: f32, window: &mut Window) {
        let (w, h) = (bounds.size.width, bounds.size.height);
        let oy = bounds.top() + h * origin_y;
        let sz = size(w * scale, h * scale);
        let b = Bounds::new(
            point(bounds.center().x - sz.width / 2., oy - sz.height * origin_y),
            sz,
        );
        let r = &self.radii;
        let radii = Corners {
            top_left: r.top_left * scale,
            top_right: r.top_right * scale,
            bottom_right: r.bottom_right * scale,
            bottom_left: r.bottom_left * scale,
        };
        if !self.shadows.is_empty() {
            let shadows: Vec<BoxShadow> = self
                .shadows
                .iter()
                .map(|s| BoxShadow {
                    color: s.color.opacity(opacity),
                    ..s.clone()
                })
                .collect();
            window.paint_shadows(b, radii, &shadows);
        }
        let bg = self
            .bg
            .map(|bg| bg.opacity(opacity))
            .unwrap_or_else(|| transparent_black().into());
        let border = self
            .border
            .map(|c| c.opacity(opacity))
            .unwrap_or_else(transparent_black);
        window.paint_quad(quad(
            b,
            radii,
            bg,
            self.widths,
            border,
            BorderStyle::Solid,
        ));
    }
}

/// Play `fx` when `el` first shows (or its `id` changes). See [`Enter`].
pub fn enter<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    fx: Fx,
    el: E,
) -> Enter<E> {
    Enter {
        id: id.into(),
        child: Some(el),
        fx,
    }
}

/// `el.enter(id, fx)`.
pub trait EnterExt: IntoElement + Styled + Sized + 'static {
    fn enter(self, id: impl Into<ElementId>, fx: Fx) -> Enter<Self> {
        enter(id, fx, self)
    }
}

impl<E: IntoElement + Styled + Sized + 'static> EnterExt for E {}

/// An element playing its opening animation.
pub struct Enter<E> {
    id: ElementId,
    child: Option<E>,
    fx: Fx,
}

impl<E: IntoElement + Styled + 'static> IntoElement for Enter<E> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<E: IntoElement + Styled + 'static> Element for Enter<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
    ) -> (LayoutId, AnyElement) {
        let start = window.with_element_state(id.expect("Enter has an id"), |s: Option<Instant>, _| {
            let s = s.unwrap_or_else(Instant::now);
            (s, s)
        });
        let el = self.child.take().expect("laid out once");
        let t = if motion(cx) {
            start.elapsed().as_secs_f32() / self.fx.dur.as_secs_f32()
        } else {
            1.
        };
        let mut child = if t < 1. {
            window.request_animation_frame();
            transformed(el, self.fx.frame(t)).into_any_element()
        } else {
            el.into_any_element()
        };
        let layout = child.request_layout(window, cx);
        (layout, child)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

/// `el` drawn at `frame` (CSS `opacity` and `transform: translate() scale()`):
/// faded, shifted at paint time (layout stays), and its surface scaled
/// about its centre.
pub fn transformed<E: IntoElement + Styled + 'static>(el: E, frame: Frame) -> Transformed<E> {
    Transformed {
        child: Some(el),
        frame,
    }
}

pub struct Transformed<E> {
    child: Option<E>,
    frame: Frame,
}

pub struct TransformedLayout {
    child: AnyElement,
    skin: Option<(Skin, f32)>,
}

impl<E: IntoElement + Styled + 'static> IntoElement for Transformed<E> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<E: IntoElement + Styled + 'static> Element for Transformed<E> {
    type RequestLayoutState = TransformedLayout;
    type PrepaintState = Point;

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
    ) -> (LayoutId, TransformedLayout) {
        let mut el = self.child.take().expect("laid out once");
        let f = self.frame;
        let rem = window.rem_size();
        let style = el.style();
        let base = style.opacity.unwrap_or(1.);
        if f.opacity < 1. {
            style.opacity = Some(base * f.opacity);
        }
        let skin = (f.scale != 1.).then(|| (Skin::take(style, rem), base * f.opacity));
        let mut child = el.into_any_element();
        let layout = child.request_layout(window, cx);
        (layout, TransformedLayout { child, skin })
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        state: &mut TransformedLayout,
        window: &mut Window,
        cx: &mut App,
    ) -> Point {
        let off = point(px(self.frame.dx), px(self.frame.dy));
        window.with_element_offset(off, |w| state.child.prepaint(w, cx));
        off
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut TransformedLayout,
        off: &mut Point,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some((skin, opacity)) = &state.skin {
            skin.paint(bounds + *off, self.frame.scale, self.frame.origin_y, *opacity, window);
        }
        state.child.paint(window, cx);
    }
}

impl Frame {
    /// At rest.
    pub const REST: Frame = Frame {
        opacity: 1.,
        dx: 0.,
        dy: 0.,
        scale: 1.,
        origin_y: 0.5,
    };
}

type Point = gpui::Point<Pixels>;

// ───────────────────────────────────────────── loops

/// How often looping animations redraw (frames per second). The working
/// dot's ring is small and slow: 30 fps reads as smooth and costs a
/// quarter of a 120 Hz display's frames.
pub const LOOP_FPS: f32 = pitwall_term_view::frames::OUTPUT_FPS;

thread_local! {
    static WAITING: RefCell<HashSet<(WindowId, EntityId)>> = RefCell::new(HashSet::new());
}

/// Ask for the next frame of a loop drawn by the current view, at most
/// [`LOOP_FPS`] times a second. Frames ride on the display link: while the
/// window is hidden or occluded none come, and the loop waits.
pub fn loop_frame(window: &mut Window) {
    let key = (window.window_handle().window_id(), window.current_view());
    if !WAITING.with(|w| w.borrow_mut().insert(key)) {
        return;
    }
    // On the terminals' output grid (pitwall_term_view::frames), so a ring
    // frame and the panes' repaints are one frame, not two.
    let due = pitwall_term_view::frames::next_tick(Instant::now() + Duration::from_millis(4));
    wait_for(window, key, due);
}

fn wait_for(window: &Window, key: (WindowId, EntityId), due: Instant) {
    window.on_next_frame(move |window, cx| {
        // A vsync early counts as on time (60 Hz: every other frame).
        if pitwall_term_view::frames::is_due(Instant::now(), due) {
            WAITING.with(|w| w.borrow_mut().remove(&key));
            cx.notify(key.1);
        } else {
            wait_for(window, key, due);
        }
    });
}

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// Where a loop of `period` is now (0..1).
fn phase(period: Duration) -> f32 {
    let p = period.as_secs_f32();
    (epoch().elapsed().as_secs_f32() % p) / p
}

/// The working dot (`.pulse-dot`, `dot` px): Flat pulses a disc out of it
/// (`pulse`, 1.8 s ease-out); Glass sends a 1.5 px ring out (`m-ring`, 2 s
/// soft). `still`: on the Wall, where nothing moves (Glass keeps the ring
/// standing, as the CSS does with its animation off).
pub fn pulse_dot(dot: f32, color: Hsla, glass: bool, still: bool) -> PulseDot {
    PulseDot {
        dot,
        color,
        glass,
        still,
    }
}

pub struct PulseDot {
    dot: f32,
    color: Hsla,
    glass: bool,
    still: bool,
}

impl IntoElement for PulseDot {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// A circle of `d` px centred on `c`, a ring `w` px wide when given.
fn circle(c: Point, d: f32, fill: Hsla, ring: Option<(f32, Hsla)>, window: &mut Window) {
    let b = Bounds::new(point(c.x - px(d / 2.), c.y - px(d / 2.)), size(px(d), px(d)));
    let (w, border) = ring.unwrap_or((0., transparent_black()));
    window.paint_quad(quad(
        b,
        Corners::all(px(d / 2.)),
        fill,
        Edges::all(px(w)),
        border,
        BorderStyle::Solid,
    ));
}

impl Element for PulseDot {
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
        let mut style = Style::default();
        style.size.width = px(self.dot).into();
        style.size.height = px(self.dot).into();
        style.flex_shrink = 0.;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let c = bounds.center();
        let (dot, color) = (self.dot, self.color);
        let moving = !self.still && motion(cx);
        // Inside a pulse scope (the main screen) the moving ring is drawn
        // by the window's pulse layer, so its frames don't redraw the
        // screen: here only the still dot.
        if moving && rings::record(window, Ring { center: c, dot, color, glass: self.glass }) {
            circle(c, dot, color, None, window);
            return;
        }
        paint_pulse(c, dot, color, self.glass, moving, window);
        if moving {
            loop_frame(window);
        }
    }
}

/// The working dot at `c`: Glass a ring going out, Flat a disc; still
/// (Glass keeps its ring standing).
fn paint_pulse(c: Point, dot: f32, color: Hsla, glass: bool, moving: bool, window: &mut Window) {
    if glass {
        // A 1.5 px ring just outside the dot, scaled with it.
        let (s, a) = if moving {
            let p = phase(Duration::from_millis(2000));
            if p < 0.7 {
                let v = Ease::OutSoft.at(p / 0.7);
                (1. + 1.5 * v, 0.9 * (1. - v))
            } else {
                (2.5, 0.)
            }
        } else {
            (1., 1.)
        };
        if a > 0.001 {
            let d = (dot + 3.) * s;
            circle(c, d, transparent_black(), Some((1.5 * s, color.opacity(a))), window);
        }
    } else if moving {
        let v = Ease::EaseOut.at(phase(Duration::from_millis(1800)));
        let a = 0.55 * (1. - v);
        if a > 0.001 {
            circle(c, dot * (1. + 1.6 * v), color.opacity(a), None, window);
        }
    }
    circle(c, dot, color, None, window);
}

/// A working dot's moving ring, drawn by the window's [`PulseLayer`].
#[derive(Clone, Copy)]
struct Ring {
    center: Point,
    dot: f32,
    color: Hsla,
    glass: bool,
}

/// The one shared clock for the working dots. The main screen paints
/// inside a [`pulse_scope`]: its dots only record where they are (with
/// the clip they are painted in), and a small layer over the screen draws
/// every ring at [`LOOP_FPS`]. A ring frame then redraws that layer and
/// replays the screen's last paint (the screen is a cached view) instead
/// of laying the whole window out again.
///
/// Rings are kept per scope owner (the view painting the scope): a cached
/// view inside the screen with a scope of its own keeps its rings while
/// the screen around it paints again, and the other way round.
mod rings {
    use super::*;
    use std::collections::HashMap;

    pub(super) struct Recorded {
        pub ring: Ring,
        pub clip: Bounds<Pixels>,
    }

    type Owner = (WindowId, EntityId);

    thread_local! {
        /// The scopes painting now, innermost last.
        static SCOPES: RefCell<Vec<Owner>> = const { RefCell::new(Vec::new()) };
        pub(super) static RINGS: RefCell<HashMap<Owner, Vec<Recorded>>> = RefCell::new(HashMap::new());
    }

    /// Note a ring while a scope paints; false outside one (the dot then
    /// animates itself).
    pub(super) fn record(window: &mut Window, ring: Ring) -> bool {
        let id = window.window_handle().window_id();
        let Some(owner) = SCOPES.with(|s| s.borrow().last().copied()).filter(|o| o.0 == id) else {
            return false;
        };
        let clip = window.content_mask().bounds;
        RINGS.with(|r| r.borrow_mut().entry(owner).or_default().push(Recorded { ring, clip }));
        true
    }

    pub(super) fn begin(owner: Owner) {
        SCOPES.with(|s| s.borrow_mut().push(owner));
        RINGS.with(|r| r.borrow_mut().insert(owner, Vec::new()));
    }

    pub(super) fn end() {
        SCOPES.with(|s| s.borrow_mut().pop());
    }

    pub(super) fn forget(owner: Owner) {
        RINGS.with(|r| r.borrow_mut().remove(&owner));
    }

    /// Every ring recorded in `window`.
    pub(super) fn of(window: WindowId) -> Vec<(Ring, Bounds<Pixels>)> {
        RINGS.with(|r| {
            r.borrow()
                .iter()
                .filter(|(o, _)| o.0 == window)
                .flat_map(|(_, v)| v.iter().map(|x| (x.ring, x.clip)))
                .collect()
        })
    }
}

/// Paint `child` as a pulse scope (see the `rings` module). `off`: a modal over
/// the screen; its dots animate themselves under it.
pub fn pulse_scope(child: impl IntoElement, off: bool) -> PulseScope {
    PulseScope { child: Some(child.into_any_element()), off }
}

/// Drop the rings of `view`'s pulse scope (a cached view that is not drawn
/// any more).
pub fn pulse_forget(window: &Window, view: EntityId) {
    rings::forget((window.window_handle().window_id(), view));
}

pub struct PulseScope {
    child: Option<AnyElement>,
    off: bool,
}

impl IntoElement for PulseScope {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for PulseScope {
    type RequestLayoutState = AnyElement;
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
    ) -> (LayoutId, AnyElement) {
        let mut child = self.child.take().expect("laid out once");
        (child.request_layout(window, cx), child)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let owner = (window.window_handle().window_id(), window.current_view());
        if self.off {
            rings::forget(owner);
            child.paint(window, cx);
            return;
        }
        rings::begin(owner);
        child.paint(window, cx);
        rings::end();
    }
}

/// The layer that draws the main screen's working-dot rings (put it right
/// over the screen, under the window's overlays).
pub struct PulseLayer;

impl Render for PulseLayer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::canvas(
            |_, _, _| {},
            |_, _, window, cx| {
                let rings = rings::of(window.window_handle().window_id());
                if rings.is_empty() || !motion(cx) {
                    return;
                }
                for (r, clip) in rings {
                    window.with_content_mask(Some(gpui::ContentMask { bounds: clip }), |window| {
                        paint_pulse(r.center, r.dot, r.color, r.glass, true, window)
                    });
                }
                loop_frame(window);
            },
        )
        .absolute()
        .inset_0()
    }
}

// ───────────────────────────────────────────── transitions

/// A CSS transition: `build` gets the value easing towards `target` over
/// `dur` (from wherever it was when the target changed).
pub fn tween<F>(id: impl Into<ElementId>, target: f32, dur: Duration, ease: Ease, build: F) -> Tween<F>
where
    F: FnOnce(f32) -> AnyElement + 'static,
{
    Tween {
        id: id.into(),
        target,
        dur,
        ease,
        build: Some(build),
    }
}

pub struct Tween<F> {
    id: ElementId,
    target: f32,
    dur: Duration,
    ease: Ease,
    build: Option<F>,
}

#[derive(Clone, Copy)]
struct TweenState {
    from: f32,
    to: f32,
    start: Instant,
}

impl TweenState {
    fn value(&self, dur: Duration, ease: Ease) -> (f32, bool) {
        let t = self.start.elapsed().as_secs_f32() / dur.as_secs_f32().max(0.001);
        if t >= 1. || self.from == self.to {
            (self.to, false)
        } else {
            (lerp(self.from, self.to, ease.at(t)), true)
        }
    }
}

impl<F: FnOnce(f32) -> AnyElement + 'static> IntoElement for Tween<F> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<F: FnOnce(f32) -> AnyElement + 'static> Element for Tween<F> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
    ) -> (LayoutId, AnyElement) {
        let (target, dur, ease) = (self.target, self.dur, self.ease);
        let moving = motion(cx);
        let (v, running) = window.with_element_state(id.expect("Tween has an id"), |s: Option<TweenState>, _| {
            let mut s = s.unwrap_or(TweenState {
                from: target,
                to: target,
                start: Instant::now(),
            });
            if s.to != target {
                let (now, _) = s.value(dur, ease);
                s = TweenState {
                    from: if moving { now } else { target },
                    to: target,
                    start: Instant::now(),
                };
            }
            (s.value(dur, ease), s)
        });
        if running {
            window.request_animation_frame();
        }
        let mut child = (self.build.take().expect("laid out once"))(v);
        let layout = child.request_layout(window, cx);
        (layout, child)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

/// A value a transition can blend.
pub trait Blend: Clone + PartialEq + 'static {
    fn blend_to(&self, to: &Self, t: f32) -> Self;
}

impl Blend for Hsla {
    fn blend_to(&self, to: &Self, t: f32) -> Self {
        mix(*self, *to, t)
    }
}

impl Blend for f32 {
    fn blend_to(&self, to: &Self, t: f32) -> Self {
        lerp(*self, *to, t)
    }
}

/// A CSS transition of colours (`background-color`, `border-color`,
/// `color`): `build` gets `target` blended from whatever showed when it
/// changed.
pub fn tween_colors<F>(
    id: impl Into<ElementId>,
    target: Vec<Hsla>,
    dur: Duration,
    ease: Ease,
    build: F,
) -> TweenMany<Hsla, F>
where
    F: FnOnce(&[Hsla]) -> AnyElement + 'static,
{
    tween_many(id, target, dur, ease, build)
}

/// A CSS transition of several values at once (colours, a rectangle's
/// edges, …).
pub fn tween_many<T: Blend, F>(
    id: impl Into<ElementId>,
    target: Vec<T>,
    dur: Duration,
    ease: Ease,
    build: F,
) -> TweenMany<T, F>
where
    F: FnOnce(&[T]) -> AnyElement + 'static,
{
    TweenMany {
        id: id.into(),
        target,
        dur,
        ease,
        build: Some(build),
    }
}

pub struct TweenMany<T, F> {
    id: ElementId,
    target: Vec<T>,
    dur: Duration,
    ease: Ease,
    build: Option<F>,
}

#[derive(Clone)]
struct ManyState<T> {
    from: Vec<T>,
    to: Vec<T>,
    start: Instant,
}

impl<T: Blend> ManyState<T> {
    fn value(&self, dur: Duration, ease: Ease) -> (Vec<T>, bool) {
        let t = self.start.elapsed().as_secs_f32() / dur.as_secs_f32().max(0.001);
        if t >= 1. || self.from.len() != self.to.len() {
            return (self.to.clone(), false);
        }
        let p = ease.at(t);
        let v = self.from.iter().zip(&self.to).map(|(a, b)| a.blend_to(b, p)).collect();
        (v, true)
    }
}

impl<T: Blend, F: FnOnce(&[T]) -> AnyElement + 'static> IntoElement for TweenMany<T, F> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<T: Blend, F: FnOnce(&[T]) -> AnyElement + 'static> Element for TweenMany<T, F> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
    ) -> (LayoutId, AnyElement) {
        let (target, dur, ease) = (std::mem::take(&mut self.target), self.dur, self.ease);
        let moving = motion(cx);
        let (v, running) = window.with_element_state(id.expect("tween has an id"), |s: Option<ManyState<T>>, _| {
            let mut s = s.unwrap_or_else(|| ManyState {
                from: target.clone(),
                to: target.clone(),
                start: Instant::now(),
            });
            if s.to != target {
                let (now, _) = s.value(dur, ease);
                s = ManyState {
                    from: if moving { now } else { target.clone() },
                    to: target,
                    start: Instant::now(),
                };
            }
            (s.value(dur, ease), s)
        });
        if running {
            window.request_animation_frame();
        }
        let mut child = (self.build.take().expect("laid out once"))(&v);
        let layout = child.request_layout(window, cx);
        (layout, child)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

// ───────────────────────────────────────────── one-shot keyframes

/// Play a keyframe animation once (or `times` times) when the element first
/// shows, or when `id` changes (put the status in it: CSS restarts an
/// animation when the attribute changes). `f(t)` builds the element at
/// linear time `t` (0..=1 of one run; 1 when finished or with Reduce
/// motion).
pub fn keyframes(
    id: impl Into<ElementId>,
    dur: Duration,
    times: u32,
    f: impl FnOnce(f32) -> AnyElement + 'static,
) -> Keyframes {
    Keyframes {
        id: id.into(),
        dur,
        times: times.max(1),
        f: Some(Box::new(f)),
    }
}

pub struct Keyframes {
    id: ElementId,
    dur: Duration,
    times: u32,
    f: Option<Box<dyn FnOnce(f32) -> AnyElement>>,
}

impl IntoElement for Keyframes {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Keyframes {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
    ) -> (LayoutId, AnyElement) {
        let start = window.with_element_state(id.expect("Keyframes has an id"), |s: Option<Instant>, _| {
            let s = s.unwrap_or_else(Instant::now);
            (s, s)
        });
        let runs = start.elapsed().as_secs_f32() / self.dur.as_secs_f32();
        let t = if !motion(cx) || runs >= self.times as f32 {
            1.
        } else {
            window.request_animation_frame();
            runs.fract()
        };
        let mut child = (self.f.take().expect("laid out once"))(t);
        let layout = child.request_layout(window, cx);
        (layout, child)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
    }
}

/// Piecewise keyframes: `stops` are (offset, value) pairs from 0 to 1, each
/// segment eased with `ease` (CSS applies the timing function per segment).
pub fn at_stops(t: f32, stops: &[(f32, f32)], ease: Ease) -> f32 {
    let t = t.clamp(0., 1.);
    for w in stops.windows(2) {
        let ((t0, v0), (t1, v1)) = (w[0], w[1]);
        if t <= t1 {
            let local = if t1 > t0 { (t - t0) / (t1 - t0) } else { 1. };
            return lerp(v0, v1, ease.at(local));
        }
    }
    stops.last().map(|s| s.1).unwrap_or(0.)
}

/// `m-flag`: from opacity 0, scale 0.3 (and −12°) to the rest, spring.
/// Returns (opacity, scale) at linear time `t`.
pub fn flag_pop(t: f32) -> (f32, f32) {
    let p = Ease::Spring.at(t);
    (p.clamp(0., 1.), lerp(0.3, 1., p))
}

/// `m-bounce` (blocked glyph, the loud count, a tab's ▲): returns (dy px,
/// scale) at linear time `t` of one run.
pub fn bounce(t: f32) -> (f32, f32) {
    let dy = at_stops(t, &[(0., 0.), (0.3, -3.), (0.6, 0.), (1., 0.)], Ease::Spring);
    let s = at_stops(t, &[(0., 1.), (0.3, 1.08), (0.6, 0.98), (1., 1.)], Ease::Spring);
    (dy, s)
}

/// `m-halo` ("needs you" rows): the halo's opacity at linear time `t`.
pub fn halo(t: f32) -> f32 {
    at_stops(t, &[(0., 0.), (0.5, 1.), (1., 0.)], Ease::EaseInOut)
}

/// Timings of the one-shot animations.
pub const HALO: Duration = Duration::from_millis(1300);
pub const BOUNCE: Duration = Duration::from_millis(550);
pub const FLAG: Duration = Duration::from_millis(450);
pub const FLAG_ROW: Duration = Duration::from_millis(500);
/// `.chev`: transform 0.12s; `.toggle-track`: 0.15s; status strip: 0.16s.
pub const CHEVRON: Duration = Duration::from_millis(120);
pub const TOGGLE: Duration = Duration::from_millis(150);
pub const STRIP: Duration = Duration::from_millis(160);
pub const PRESS: Duration = Duration::from_millis(200);
pub const SHEEN: Duration = Duration::from_millis(700);

// ───────────────────────────────────────────── press

thread_local! {
    static PRESSED: Cell<Option<(u64, Instant)>> = const { Cell::new(None) };
}

/// The key a pressable element is known by (its id, hashed).
pub fn press_key(id: &ElementId) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}

/// `m-press`: the scale of the element `key` now, while its press pop runs
/// (0.2 s: down to 0.95 at 40 %, back by the end).
pub fn press_scale(key: u64) -> Option<f32> {
    if !on() {
        return None;
    }
    let (k, at) = PRESSED.with(|p| p.get())?;
    let t = at.elapsed().as_secs_f32() / PRESS.as_secs_f32();
    (k == key && t < 1.).then(|| at_stops(t, &[(0., 1.), (0.4, 0.95), (1., 1.)], Ease::OutSoft))
}

/// Whether `id`'s press pop runs now (keep its hover background off then:
/// the pop draws the surface itself).
pub fn pressing(id: &ElementId) -> bool {
    press_scale(press_key(id)).is_some()
}

/// Make `el` pop when pressed (call before adding its children): `skin` is its surface (drawn scaled while
/// the pop runs; the element's own background goes clear meanwhile).
pub fn pressable<E>(el: E, id: &ElementId, disabled: bool, skin: Skin) -> E
where
    E: InteractiveElement + ParentElement + Styled,
{
    if disabled {
        return el;
    }
    let key = press_key(id);
    let el = el.on_mouse_down(MouseButton::Left, move |_, window, _| {
        if on() {
            PRESSED.with(|p| p.set(Some((key, Instant::now()))));
            window.refresh();
        }
    });
    match press_scale(key) {
        Some(_) => {
            let mut el = el;
            let style = el.style();
            style.background = None;
            style.box_shadow = None;
            style.border_color = Some(transparent_black());
            el.child(PressSkin { key, skin })
        }
        None => el,
    }
}

/// The scaled surface under a pressed element's content.
pub struct PressSkin {
    key: u64,
    skin: Skin,
}

impl IntoElement for PressSkin {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for PressSkin {
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
        (window.request_layout(cover(), [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) {
        window.request_animation_frame();
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) {
        let s = press_scale(self.key).unwrap_or(1.);
        // Painted after the content's background layer but before its
        // children paint (it is the first child): under the label.
        self.skin.paint(bounds, s, 0.5, 1., window);
    }
}

// ───────────────────────────────────────────── sheen

/// Glass primary buttons: a light band sweeps across on hover (0.7 s soft;
/// it jumps back when the pointer leaves, as the CSS transition is on
/// `:hover` only). Put it first among the button's children.
pub fn sheen(color: Hsla) -> Sheen {
    Sheen { color }
}

pub struct Sheen {
    color: Hsla,
}

impl IntoElement for Sheen {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Sheen {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::Name("sheen".into()))
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
        (window.request_layout(cover(), [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if !motion(cx) {
            return;
        }
        let hovered = bounds.contains(&window.mouse_position());
        let start = window.with_element_state(id.expect("Sheen has an id"), |s: Option<Option<Instant>>, _| {
            let s = match (s.flatten(), hovered) {
                (_, false) => None,
                (Some(at), true) => Some(at),
                (None, true) => Some(Instant::now()),
            };
            (s, s)
        });
        let Some(start) = start else {
            return;
        };
        let t = start.elapsed().as_secs_f32() / SHEEN.as_secs_f32();
        if t >= 1. {
            return;
        }
        window.request_animation_frame();
        // translateX(-130% → 130%); the band is 30 %–70 % of the width
        // (`linear-gradient(105deg, transparent 30%, sheen 50%, transparent 70%)`).
        let w = f32::from(bounds.size.width);
        let shift = w * (-1.3 + 2.6 * Ease::OutSoft.at(t));
        let mid = f32::from(bounds.left()) + w * 0.5 + shift;
        let half = w * 0.2;
        let clear = self.color.opacity(0.);
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for (x0, from, to) in [(mid - half, clear, self.color), (mid, self.color, clear)] {
                let b = Bounds::new(
                    point(px(x0), bounds.top()),
                    size(px(half), bounds.size.height),
                );
                window.paint_quad(quad(
                    b,
                    Corners::default(),
                    gpui::linear_gradient(
                        90.,
                        gpui::linear_color_stop(from, 0.),
                        gpui::linear_color_stop(to, 1.),
                    ),
                    Edges::default(),
                    transparent_black(),
                    BorderStyle::Solid,
                ));
            }
        });
    }
}

/// `m-bounce` twice on `el` (the loud count, a blocked tab's ▲), keyed by
/// `id` (it plays when the element first shows).
pub fn bounce_in<E: IntoElement + Styled + 'static>(id: impl Into<ElementId>, el: E) -> Keyframes {
    keyframes(id, BOUNCE, 2, move |t| {
        let (dy, scale) = bounce(t);
        transformed(el, Frame { dy, scale, ..Frame::REST }).into_any_element()
    })
}

/// `m-flag` once on `el` (`dur`: [`FLAG`] or [`FLAG_ROW`]).
pub fn flag_in<E: IntoElement + Styled + 'static>(id: impl Into<ElementId>, dur: Duration, el: E) -> Keyframes {
    keyframes(id, dur, 1, move |t| {
        let (opacity, scale) = flag_pop(t);
        transformed(el, Frame { opacity, scale, ..Frame::REST }).into_any_element()
    })
}

/// A layout style covering the parent (absolute, inset 0).
fn cover() -> Style {
    let mut style = Style {
        position: gpui::Position::Absolute,
        ..Default::default()
    };
    style.inset.top = px(0.).into();
    style.inset.left = px(0.).into();
    style.size.width = gpui::relative(1.).into();
    style.size.height = gpui::relative(1.).into();
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easings_start_at_zero_and_end_at_one() {
        for e in [
            Ease::Linear,
            Ease::Ease,
            Ease::EaseOut,
            Ease::EaseInOut,
            Ease::Spring,
            Ease::OutSoft,
        ] {
            assert!(e.at(0.).abs() < 1e-3, "{e:?}");
            assert!((e.at(1.) - 1.).abs() < 1e-3, "{e:?}");
        }
        // The spring overshoots, as --ease-spring does.
        assert!((0..100).any(|i| Ease::Spring.at(i as f32 / 100.) > 1.0));
    }

    #[test]
    fn openings_end_at_rest() {
        for f in [Fx::MODAL, Fx::MENU, Fx::TOAST, Fx::SLIDE_LEFT, Fx::FADE] {
            let end = f.frame(1.);
            assert_eq!((end.opacity, end.dx, end.dy, end.scale), (1., 0., 0., 1.));
            let start = f.frame(0.);
            assert_eq!(start.opacity, 0.);
            assert_eq!((start.dx, start.dy, start.scale), (f.dx, f.dy, f.scale));
        }
    }

    #[test]
    fn keyframe_stops_hit_their_values() {
        let (dy, s) = bounce(0.3);
        assert!((dy + 3.).abs() < 1e-3 && (s - 1.08).abs() < 1e-3);
        assert_eq!(bounce(1.), (0., 1.));
        assert!((halo(0.5) - 1.).abs() < 1e-3);
        assert_eq!(halo(0.), 0.);
        let (o, s) = flag_pop(0.);
        assert_eq!((o, s), (0., 0.3));
    }

    #[test]
    fn colours_mix_in_srgb() {
        let (a, b) = (gpui::black(), gpui::white());
        assert_eq!(mix(a, b, 0.), a);
        assert_eq!(mix(a, b, 1.), b);
        let m = Rgba::from(mix(a, b, 0.5));
        assert!((m.r - 0.5).abs() < 0.01);
    }
}
