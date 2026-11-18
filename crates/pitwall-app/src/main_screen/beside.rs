//! A file viewer an agent opened beside its terminal (the `pitwall` MCP
//! server's tools, `pitwall open`: `crate::explorer::show`): the pane is
//! split, the terminal on the left and the viewer on the right, with a
//! divider to drag. The viewer is the window's explorer's
//! ([`register_beside`] hands it over); the screen only places it. Its
//! share of the pane is one for every pane and kept in `ui.json`
//! (`filesBeside`), so it opens again at the width last dragged.
//!
//! The terminal keeps the keyboard: the viewer takes it only when clicked
//! into, and gives it back on Esc or its close button.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    canvas, div, prelude::*, px, AnyElement, AnyView, App, Bounds, Context, Global, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, SharedString, Window,
};

use pitwall_proto::AgentView;

use super::{MainScreen, Route};
use crate::theme::Theme;

/// The viewer's share of the pane at first.
pub const DEFAULT_SHARE: f32 = 0.45;
/// Neither side gets less than this share.
const MIN_SHARE: f32 = 0.2;
const DIVIDER: f32 = 6.;

type BesideBuilder = Arc<dyn Fn(&str, &mut Window, &mut App) -> Option<AnyView>>;

/// Hook: the viewer beside agent `id`'s terminal in this window, if one is
/// open (the explorer module's).
#[derive(Clone)]
pub struct BesideHook(pub BesideBuilder);

impl Global for BesideHook {}

pub fn register_beside(cx: &mut App, build: impl Fn(&str, &mut Window, &mut App) -> Option<AnyView> + 'static) {
    cx.set_global(BesideHook(Arc::new(build)));
}

/// The divider being dragged.
struct Drag {
    start: Pixels,
    total: f32,
    from: f32,
}

/// The window's panes with a viewer beside them.
#[derive(Default)]
pub(super) struct Besides {
    drag: Option<Drag>,
    /// The share while dragging (written to `ui.json` on release).
    live: Option<f32>,
    /// Each split's own box (the divider's drag).
    split: HashMap<String, Rc<Cell<Option<Bounds<Pixels>>>>>,
}

/// `share` kept where both sides stay usable.
pub fn clamp_share(share: f32) -> f32 {
    share.clamp(MIN_SHARE, 1. - MIN_SHARE)
}

/// The share after dragging the divider by `moved` of the pane's width
/// (right: the viewer gets narrower).
pub fn dragged(from: f32, moved: f32) -> f32 {
    clamp_share(from - moved)
}

impl MainScreen {
    /// The viewer's share of a pane (as last dragged, or the default).
    pub(super) fn beside_share(&self) -> f32 {
        clamp_share(self.besides.live.unwrap_or_else(|| self.ui.files_beside.map_or(DEFAULT_SHARE, |s| s as f32)))
    }

    /// `term` (a pane's terminal, with its Preview) split with the viewer
    /// the agent opened beside it, if any.
    pub(super) fn with_beside(
        &mut self,
        a: &AgentView,
        term: AnyElement,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(hook) = cx.try_global::<BesideHook>().cloned() else {
            return term;
        };
        let Some(view) = (hook.0)(&a.id, window, cx) else {
            self.besides.split.remove(&a.id);
            return term;
        };
        let share = self.beside_share();
        let split = self.besides.split.entry(a.id.clone()).or_default().clone();
        let measure = {
            let split = split.clone();
            canvas(move |b, _, _| split.set(Some(b)), |_, _, _, _| {})
                .absolute()
                .size_full()
        };
        let cell = |grow: f32| {
            let mut c = div().relative().flex().min_w_0().min_h_0().flex_basis(px(0.));
            c.style().flex_grow = Some(grow);
            c.style().flex_shrink = Some(1.);
            c
        };
        let divider = div()
            .id(SharedString::from(format!("beside-divider-{}", a.id)))
            .debug_selector({
                let id = a.id.clone();
                move || format!("beside-divider-{id}")
            })
            .flex_none()
            .w(px(DIVIDER))
            .h_full()
            .cursor_col_resize()
            .border_l_1()
            .border_color(t.line)
            .bg(t.surface)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let total = split.get().map_or(1., |b| f32::from(b.size.width).max(1.));
                    let from = this.beside_share();
                    this.besides.drag = Some(Drag { start: e.position.x, total, from });
                    cx.notify();
                }),
            );
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .child(measure)
            .child(cell(1. - share).child(term))
            .child(divider)
            .child(
                cell(share)
                    .debug_selector({
                        let id = a.id.clone();
                        move || format!("beside-pane-{id}")
                    })
                    .child(view),
            )
            .into_any_element()
    }

    pub(super) fn beside_drag_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = &self.besides.drag else { return };
        let moved = f32::from(e.position.x - d.start) / d.total;
        self.besides.live = Some(dragged(d.from, moved));
        cx.notify();
    }

    pub(super) fn beside_drag_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.besides.drag.take().is_none() {
            return;
        }
        if let Some(share) = self.besides.live.take() {
            self.update_ui(
                |mut s| {
                    s.files_beside = Some(f64::from(share));
                    s
                },
                cx,
            );
        }
        cx.notify();
    }

    /// Show the pane of agent `id` in this window when it isn't on screen:
    /// a space here that shows it, and the space view (not the Wall or
    /// Review). Nothing else moves: the keyboard stays where it is.
    pub fn reveal_pane_of(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let shown_now = self.route == Route::Space
            && self.ui.space(&self.active).is_some_and(|sp| sp.layout.find_by_agent(id).is_some());
        if shown_now {
            return true;
        }
        let label = self.label.clone();
        let space = self
            .ui
            .spaces
            .iter()
            .find(|sp| self.ui.window_of_space(&sp.id) == label && sp.layout.find_by_agent(id).is_some())
            .map(|sp| sp.id.clone());
        match space {
            Some(sp) => {
                self.show_space(&sp, cx);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_divider_keeps_both_sides_usable() {
        assert_eq!(clamp_share(0.05), MIN_SHARE);
        assert_eq!(clamp_share(0.97), 1. - MIN_SHARE);
        // Dragged right by a tenth of the pane: the viewer a tenth narrower.
        assert!((dragged(0.45, 0.1) - 0.35).abs() < 1e-6);
        assert!((dragged(0.45, -0.9) - (1. - MIN_SHARE)).abs() < 1e-6);
    }
}
