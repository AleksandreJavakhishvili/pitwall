//! A side column that hides and shows and whose width is dragged: the file
//! viewer's tree, Review's list of changes (and, later, any other list
//! beside what it opens). One look and one behaviour everywhere:
//!
//! - [`toggle`]: the sidebar button in the screen's bar, pressed while the
//!   column shows (⌘⇧E where the screen binds it).
//! - [`column`]: slides in and out (140 ms, none with Reduce motion); its
//!   contents keep their width while it moves, so nothing reflows, and what
//!   it leaves takes the whole width.
//! - [`edge`]: the column's right edge, a [`super::divider`] (the grab bar
//!   the pane splits and the sidebar use), dragged for the width (the
//!   screen listens with `on_drag_move::<SideDrag>` and [`dragged_width`])
//!   and double-clicked for the default.
//! - [`Limits`]: the width kept between a minimum and a maximum, leaving
//!   what is beside the column its room; a screen too narrow for both
//!   starts with the column hidden.
//! - [`SideState`]: what `ui.json` keeps (shown, width).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    canvas, div, prelude::*, px, AnyElement, App, DragMoveEvent, Pixels, SharedString, Size,
    Stateful, Div, WeakEntity, Window,
};
use serde::{Deserialize, Serialize};

use super::motion;
use crate::theme::Theme;

/// How long the column takes to slide in or out.
pub const SLIDE: Duration = Duration::from_millis(140);

/// What `ui.json` keeps of a column.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SideState {
    /// Shown (`None`: never chosen; shown when there is room).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
    /// As last dragged (`None`: the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
}

impl SideState {
    pub fn is_empty(&self) -> bool {
        *self == SideState::default()
    }
}

/// A column's widths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// Never narrower (names stay readable)…
    pub min: f32,
    /// …nor wider.
    pub max: f32,
    /// What is beside the column keeps at least this much.
    pub keep: f32,
    /// Before any drag.
    pub default: f32,
}

impl Limits {
    /// `w` kept between the limits and (on a screen `total` wide) leaving
    /// what is beside it its room.
    pub fn clamp(&self, w: f32, total: Option<f32>) -> f32 {
        let max = total.map_or(self.max, |v| (v - self.keep).min(self.max)).max(self.min);
        if w.is_finite() {
            w.clamp(self.min, max)
        } else {
            self.min
        }
    }

    /// The width for what is kept, on a screen `total` wide.
    pub fn width(&self, s: SideState, total: Option<f32>) -> f32 {
        self.clamp(s.width.unwrap_or(self.default), total)
    }

    /// Too narrow for the column and what is beside it.
    pub fn narrow(&self, total: f32, w: f32) -> bool {
        total < w + self.keep
    }

    /// The column shows: as toggled on this screen (`session`), else as
    /// kept, except that a narrow screen starts without it.
    pub fn shown(&self, session: Option<bool>, s: SideState, total: Option<f32>) -> bool {
        session.unwrap_or_else(|| {
            let w = self.clamp(s.width.unwrap_or(self.default), None);
            s.open.unwrap_or(true) && !total.is_some_and(|t| self.narrow(t, w))
        })
    }
}

/// What a column's edge drags (the column's id).
#[derive(Debug, Clone, Copy)]
pub struct SideDrag(pub &'static str);

/// Nothing follows the pointer while an edge drags.
struct NoGhost;

impl Render for NoGhost {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The width an edge drag asks for, from the left of the element that
/// listens (the row the column starts).
pub fn dragged_width(e: &DragMoveEvent<SideDrag>) -> f32 {
    f32::from(e.event.position.x - e.bounds.left())
}

/// The bar's button that hides and shows a column (`what` names it in the
/// tooltip, `key` is its shortcut as macOS writes it).
pub fn toggle(id: &'static str, shown: bool, what: &str, key: &str, t: &Theme) -> Stateful<Div> {
    let tip = super::keys(&format!("{} {what} ({key})", if shown { "Hide" } else { "Show" }), false);
    super::icon_btn_state(id, "sidebar", tip, true, 14., Some(shown), t)
        .debug_selector(move || id.into())
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// The column: `body` (its own width `width`, laid out once) sliding in
/// and out with `shown`; instantly while its edge is dragged.
pub fn column(id: &'static str, shown: bool, width: f32, dragging: bool, body: AnyElement) -> impl IntoElement {
    motion::tween(
        SharedString::from(format!("{id}-slide")),
        if shown { 1. } else { 0. },
        if dragging { Duration::ZERO } else { SLIDE },
        motion::Ease::OutSoft,
        move |v| {
            if v <= 0.001 {
                return div().into_any_element();
            }
            div().flex_none().h_full().w(px(width * v)).overflow_hidden().child(body).into_any_element()
        },
    )
}

/// The column's right edge (put it in the column, which is `relative`),
/// straddling its border: drag for the width (`dragging`: lit), double-click
/// for the default (`on_reset`).
pub fn edge(
    id: &'static str,
    dragging: bool,
    t: &Theme,
    on_reset: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    super::divider(SharedString::from(format!("{id}-edge")), true, dragging, t)
        .debug_selector(move || format!("{id}-edge"))
        .absolute()
        .top_0()
        .bottom_0()
        .right(px(-super::divider::DIVIDER / 2.))
        .on_drag(SideDrag(id), |_, _, _, cx| cx.new(|_| NoGhost))
        .on_click(move |e, window, cx| {
            if e.click_count() == 2 {
                on_reset(window, cx);
            }
        })
        .tooltip(super::tooltip("Drag to resize · double-click for the default width"))
}

/// Writes the size it is laid out at into `cell` and redraws `owner` when
/// it changed (a screen's width decides whether a column fits). Put it in
/// a `relative` box.
pub fn measure<V: 'static>(cell: Rc<Cell<Option<Size<Pixels>>>>, owner: WeakEntity<V>) -> impl IntoElement {
    canvas(
        move |b, _, cx| {
            if cell.get() != Some(b.size) {
                cell.set(Some(b.size));
                let owner = owner.clone();
                cx.defer(move |cx| {
                    let _ = owner.update(cx, |_, cx| cx.notify());
                });
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Limits = Limits { min: 160., max: 520., keep: 380., default: 232. };

    #[test]
    fn the_width_stays_between_its_limits_and_leaves_room() {
        assert_eq!(L.clamp(40., None), 160.);
        assert_eq!(L.clamp(2000., None), 520.);
        assert_eq!(L.clamp(300., None), 300.);
        assert_eq!(L.clamp(500., Some(700.)), 320.);
        assert_eq!(L.clamp(300., Some(400.)), 160.);
        assert_eq!(L.clamp(f32::NAN, None), 160.);
        assert_eq!(L.width(SideState::default(), None), 232.);
        assert_eq!(L.width(SideState { open: None, width: Some(200.) }, Some(1000.)), 200.);
    }

    #[test]
    fn toggled_kept_or_too_narrow() {
        let open = SideState { open: Some(true), width: None };
        let closed = SideState { open: Some(false), width: None };
        assert!(!L.shown(Some(false), open, Some(1200.)));
        assert!(L.shown(Some(true), closed, Some(300.)));
        assert!(!L.shown(None, closed, Some(1200.)));
        assert!(L.shown(None, SideState::default(), Some(1200.)));
        assert!(L.shown(None, SideState::default(), None));
        // Narrow: hidden at first, even if kept open.
        assert!(!L.shown(None, open, Some(500.)));
        assert!(L.shown(None, open, Some(232. + 380. + 1.)));
        let wide = SideState { open: None, width: Some(400.) };
        assert!(!L.shown(None, wide, Some(700.)));
        assert!(L.shown(None, wide, Some(800.)));
        // In ui.json's words.
        assert_eq!(serde_json::to_string(&closed).unwrap(), r#"{"open":false}"#);
        assert!(serde_json::from_str::<SideState>("{}").unwrap().is_empty());
    }
}
