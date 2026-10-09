//! The always-on styled scrollbar of the React app (`base.css`:
//! `::-webkit-scrollbar` 10 px, a `--line-strong` pill thumb inset by a
//! 3 px transparent border, transparent track). Like WebKit's classic
//! scrollbar it takes room: a 10 px gutter beside the content, shown only
//! while the content overflows. The thumb drags, a click on the track pages
//! towards it, and the wheel over the gutter scrolls the content.
//!
//! Use [`vscroll`] (or [`hscroll`]) around a scroll container that tracks the
//! same [`ScrollHandle`]: `vscroll(&handle, &t, div().id(..).track_scroll(&handle)
//! .overflow_y_scroll()...)`. The returned wrapper takes the sizing the scroll
//! container used to have (`flex_1`, `min_h_0`, `max_h_full`…). For a
//! `uniform_list`, pass `handle.0.borrow().base_handle.clone()`.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    div, point, prelude::*, px, quad, size, App, Axis, Bounds, BorderStyle, Corners, CursorStyle,
    DispatchPhase, Element, ElementId, GlobalElementId, Hitbox, HitboxBehavior, Hsla,
    InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, ScrollHandle, ScrollWheelEvent, Style, Window,
};

use crate::theme::Theme;

/// `::-webkit-scrollbar { width: 10px; height: 10px }`.
pub const GUTTER: f32 = 10.;
/// `border: 3px solid transparent` around the thumb.
const INSET: f32 = 3.;
/// The shortest thumb (WebKit keeps it grabbable).
const MIN_THUMB: f32 = 18.;

/// Lay out `content` and its vertical `bar`. `fill`: the area's size comes
/// from its parent (a sidebar, a panel), so the content is placed in it
/// absolutely and laid out once, not measured for its natural height first
/// (that measuring doubled the main screen's layout time). Otherwise the
/// area grows with the content up to its `max_h`.
fn frame(fill: bool, content: gpui::AnyElement, mut bar: Scrollbar) -> gpui::Div {
    if !fill {
        return div()
            .flex()
            .flex_row()
            .min_h_0()
            .min_w_0()
            .child(div().flex_1().min_w_0().min_h_0().flex().flex_col().child(content))
            .child(bar);
    }
    let gutter = if bar.overflows() { px(GUTTER) } else { px(0.) };
    bar.fill = true;
    div()
        .relative()
        .min_h_0()
        .min_w_0()
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .bottom_0()
                .right(gutter)
                .flex()
                .flex_col()
                .child(content),
        )
        .child(div().absolute().top_0().bottom_0().right_0().child(bar))
}

/// [`vscroll`] whose size comes from its parent (see [`frame`]).
pub fn vscroll_fill(
    id: impl Into<ElementId>,
    handle: &ScrollHandle,
    t: &Theme,
    content: impl IntoElement,
) -> gpui::Div {
    frame(
        true,
        content.into_any_element(),
        Scrollbar::new(id, handle.clone(), Axis::Vertical, t.line_strong),
    )
}

/// [`vscroll_list`] whose size comes from its parent.
pub fn vscroll_list_fill(
    id: impl Into<ElementId>,
    handle: &gpui::UniformListScrollHandle,
    t: &Theme,
    content: impl IntoElement,
) -> gpui::Div {
    let base = handle.0.borrow().base_handle.clone();
    let mut bar = Scrollbar::new(id, base, Axis::Vertical, t.line_strong);
    bar.list = true;
    frame(true, content.into_any_element(), bar)
}

/// A vertical scroll area: `content` (the scroll container tracking
/// `handle`) with the 10 px scrollbar gutter on its right.
pub fn vscroll(
    id: impl Into<ElementId>,
    handle: &ScrollHandle,
    t: &Theme,
    content: impl IntoElement,
) -> gpui::Div {
    div()
        .flex()
        .flex_row()
        .min_h_0()
        .min_w_0()
        .child(div().flex_1().min_w_0().min_h_0().flex().flex_col().child(content))
        .child(Scrollbar::new(id, handle.clone(), Axis::Vertical, t.line_strong))
}

/// [`vscroll`] for a `uniform_list` tracking `handle`.
pub fn vscroll_list(
    id: impl Into<ElementId>,
    handle: &gpui::UniformListScrollHandle,
    t: &Theme,
    content: impl IntoElement,
) -> gpui::Div {
    let base = handle.0.borrow().base_handle.clone();
    let mut bar = Scrollbar::new(id, base, Axis::Vertical, t.line_strong);
    bar.list = true;
    div()
        .flex()
        .flex_row()
        .min_h_0()
        .min_w_0()
        .child(div().flex_1().min_w_0().min_h_0().flex().flex_col().child(content))
        .child(bar)
}

/// A horizontal scroll area: the gutter sits under the content.
pub fn hscroll(
    id: impl Into<ElementId>,
    handle: &ScrollHandle,
    t: &Theme,
    content: impl IntoElement,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .child(div().flex_1().min_w_0().min_h_0().flex().child(content))
        .child(Scrollbar::new(id, handle.clone(), Axis::Horizontal, t.line_strong))
}

/// A vertical scroll area that keeps its own [`ScrollHandle`] (for
/// widgets drawn by plain functions): `build` gets the handle to track and
/// returns the scroll container. Style the area like the container it
/// replaces (`max_h`, borders, background…).
pub fn scroll_area(
    id: impl Into<ElementId>,
    t: &Theme,
    build: impl FnOnce(&ScrollHandle) -> gpui::AnyElement + 'static,
) -> ScrollArea {
    ScrollArea {
        id: id.into(),
        color: t.line_strong,
        style: gpui::StyleRefinement::default(),
        build: Some(Box::new(build)),
        fill: false,
    }
}

type Build = Box<dyn FnOnce(&ScrollHandle) -> gpui::AnyElement>;

pub struct ScrollArea {
    id: ElementId,
    color: Hsla,
    style: gpui::StyleRefinement,
    build: Option<Build>,
    fill: bool,
}

impl ScrollArea {
    /// Its size comes from its parent (see [`frame`]).
    pub fn fill(mut self) -> Self {
        self.fill = true;
        self
    }
}

impl Styled for ScrollArea {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        &mut self.style
    }
}

impl IntoElement for ScrollArea {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ScrollArea {
    type RequestLayoutState = gpui::AnyElement;
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
    ) -> (LayoutId, gpui::AnyElement) {
        let handle = window.with_element_state(id.expect("ScrollArea has an id"), |s: Option<ScrollHandle>, _| {
            let s = s.unwrap_or_default();
            (s.clone(), s)
        });
        let content = (self.build.take().expect("laid out once"))(&handle);
        let mut area = frame(
            self.fill,
            content,
            Scrollbar::new("bar", handle, Axis::Vertical, self.color),
        );
        area.style().refine(&self.style);
        let mut el = area.into_any_element();
        (el.request_layout(window, cx), el)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        el: &mut gpui::AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        el.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        el: &mut gpui::AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        el.paint(window, cx);
    }
}

/// Where a thumb of `view` px over `content` px sits in a `track` px lane
/// at scroll `offset` (0..=`max`): (start, length).
pub fn thumb(track: f32, view: f32, content: f32, offset: f32) -> (f32, f32) {
    if content <= view || track <= 0. {
        return (0., track.max(0.));
    }
    let len = (track * view / content).clamp(MIN_THUMB.min(track), track);
    let max = content - view;
    let start = (track - len) * (offset / max).clamp(0., 1.);
    (start, len)
}

/// The scroll offset for a thumb whose start is dragged to `start`.
pub fn offset_for(track: f32, len: f32, max: f32, start: f32) -> f32 {
    if track <= len {
        return 0.;
    }
    (start / (track - len)).clamp(0., 1.) * max
}

pub struct Scrollbar {
    id: ElementId,
    handle: ScrollHandle,
    axis: Axis,
    color: Hsla,
    /// The handle is a `uniform_list`'s (it records no children).
    list: bool,
    /// In a [`frame`] that fills its parent: as tall as it.
    fill: bool,
}

impl Scrollbar {
    pub fn new(id: impl Into<ElementId>, handle: ScrollHandle, axis: Axis, color: Hsla) -> Self {
        Scrollbar { id: id.into(), handle, axis, color, list: false, fill: false }
    }

    fn overflows(&self) -> bool {
        let m = self.handle.max_offset();
        // gpui 0.2.2 measures a scroll container with no children as its
        // own size, then adds the padding: it "overflows" by its padding.
        if !self.list && self.handle.children_count() == 0 {
            return false;
        }
        match self.axis {
            Axis::Vertical => m.height > px(0.),
            Axis::Horizontal => m.width > px(0.),
        }
    }
}

impl IntoElement for Scrollbar {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

/// Per-scrollbar state across frames: the gutter shown at layout, and the
/// grab point while the thumb is dragged.
#[derive(Default)]
pub struct BarState {
    shown: Cell<bool>,
    grab: Cell<Option<f32>>,
}

pub struct Prepaint {
    hitbox: Hitbox,
    state: Rc<BarState>,
}

impl Element for Scrollbar {
    type RequestLayoutState = Rc<BarState>;
    type PrepaintState = Prepaint;

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
    ) -> (LayoutId, Rc<BarState>) {
        let state = window.with_element_state(id.expect("Scrollbar has an id"), |s: Option<Rc<BarState>>, _| {
            let s = s.unwrap_or_default();
            (s.clone(), s)
        });
        // Last frame's overflow decides the gutter (the content is laid out
        // after this; a change re-lays out at once, see prepaint).
        let shown = self.overflows();
        state.shown.set(shown);
        let mut style = Style {
            flex_shrink: 0.,
            ..Default::default()
        };
        let w = if shown { px(GUTTER) } else { px(0.) };
        match self.axis {
            Axis::Vertical => style.size.width = w.into(),
            Axis::Horizontal => style.size.height = w.into(),
        }
        if self.fill {
            style.size.height = gpui::relative(1.).into();
        }
        (window.request_layout(style, [], cx), state)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Rc<BarState>,
        window: &mut Window,
        _: &mut App,
    ) -> Prepaint {
        // The content (an earlier sibling) has just measured itself.
        if self.overflows() != state.shown.get() {
            window.refresh();
        }
        Prepaint {
            hitbox: window.insert_hitbox(bounds, HitboxBehavior::Normal),
            state: state.clone(),
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Rc<BarState>,
        pre: &mut Prepaint,
        window: &mut Window,
        _: &mut App,
    ) {
        if !pre.state.shown.get() || !self.overflows() {
            pre.state.grab.set(None);
            return;
        }
        let vertical = self.axis == Axis::Vertical;
        let along = move |p: gpui::Point<Pixels>| -> f32 { f32::from(if vertical { p.y } else { p.x }) };
        let handle = self.handle.clone();
        let geom = move |bounds: Bounds<Pixels>| {
            let view_b = handle.bounds();
            let max = handle.max_offset();
            let (view, max) = if vertical {
                (f32::from(view_b.size.height), f32::from(max.height))
            } else {
                (f32::from(view_b.size.width), f32::from(max.width))
            };
            let off = handle.offset();
            let off = -f32::from(if vertical { off.y } else { off.x });
            let (start, track) = if vertical {
                (f32::from(bounds.top()) + INSET, f32::from(bounds.size.height) - 2. * INSET)
            } else {
                (f32::from(bounds.left()) + INSET, f32::from(bounds.size.width) - 2. * INSET)
            };
            let (s, len) = thumb(track, view, view + max, off);
            (start, track, s, len, max, view)
        };
        let (start, _, s, len, _, _) = geom(bounds);
        let rect = if vertical {
            Bounds::new(
                point(bounds.left() + px(INSET), px(start + s)),
                size(bounds.size.width - px(2. * INSET), px(len)),
            )
        } else {
            Bounds::new(
                point(px(start + s), bounds.top() + px(INSET)),
                size(px(len), bounds.size.height - px(2. * INSET)),
            )
        };
        let r = px(GUTTER / 2.);
        window.paint_quad(quad(
            rect,
            Corners::all(r),
            self.color,
            gpui::Edges::default(),
            gpui::transparent_black(),
            BorderStyle::default(),
        ));
        window.set_cursor_style(CursorStyle::Arrow, &pre.hitbox);

        let set = {
            let handle = self.handle.clone();
            move |v: f32| {
                let o = handle.offset();
                let v = px(-v);
                handle.set_offset(if vertical { point(o.x, v) } else { point(v, o.y) });
            }
        };
        let hitbox = pre.hitbox.clone();
        let st = pre.state.clone();
        {
            let (set, geom) = (set.clone(), geom.clone());
            window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || e.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                let (start, track, s, len, max, view) = geom(hitbox.bounds);
                let at = along(e.position) - start;
                if at >= s && at <= s + len {
                    st.grab.set(Some(at - s));
                } else {
                    // A click on the track pages towards it.
                    let cur = offset_for(track, len, max, s);
                    let page = (view - 40.).max(view * 0.875);
                    set(if at < s { cur - page } else { cur + page }.clamp(0., max));
                }
                cx.stop_propagation();
                window.refresh();
            });
        }
        let hitbox = pre.hitbox.clone();
        let st = pre.state.clone();
        {
            let (set, geom) = (set.clone(), geom.clone());
            window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, _| {
                let Some(grab) = st.grab.get() else { return };
                if phase != DispatchPhase::Bubble {
                    return;
                }
                if e.pressed_button != Some(MouseButton::Left) {
                    st.grab.set(None);
                    return;
                }
                let (start, track, _, len, max, _) = geom(hitbox.bounds);
                set(offset_for(track, len, max, along(e.position) - start - grab));
                window.refresh();
            });
        }
        let st = pre.state.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, _| {
            if phase == DispatchPhase::Bubble {
                st.grab.set(None);
            }
        });
        let hitbox = pre.hitbox.clone();
        window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, _| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            let (_, track, s, len, max, _) = geom(hitbox.bounds);
            let d = e.delta.pixel_delta(px(20.));
            let d = f32::from(if vertical { d.y } else { d.x });
            set((offset_for(track, len, max, s) - d).clamp(0., max));
            window.refresh();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thumb_is_the_viewport_share_of_the_track() {
        // 100 px of 400 px in a 200 px track: a 50 px thumb.
        assert_eq!(thumb(200., 100., 400., 0.), (0., 50.));
        // Scrolled to the end: the thumb ends where the track does.
        assert_eq!(thumb(200., 100., 400., 300.), (150., 50.));
        // Halfway.
        assert_eq!(thumb(200., 100., 400., 150.), (75., 50.));
    }

    #[test]
    fn a_long_list_keeps_a_grabbable_thumb() {
        let (_, len) = thumb(200., 100., 100_000., 0.);
        assert_eq!(len, MIN_THUMB);
    }

    #[test]
    fn dragging_the_thumb_maps_back_to_an_offset() {
        let (track, len, max) = (200., 50., 300.);
        assert_eq!(offset_for(track, len, max, 0.), 0.);
        assert_eq!(offset_for(track, len, max, 75.), 150.);
        assert_eq!(offset_for(track, len, max, 500.), 300.);
        assert_eq!(offset_for(track, len, max, -20.), 0.);
    }
}
