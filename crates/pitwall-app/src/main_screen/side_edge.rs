//! The docked sidebar's right edge: dragged, it sets the sidebar's width
//! (live, the terminals beside it reflowing as their tiles do when a pane
//! divider moves: only when their cell count changes); a double click puts
//! the default width back. The width is the window's own while it is
//! open, saved in `ui.json` for the next window and the next run, and
//! collapsing the sidebar keeps it.
//!
//! The edge is drawn by the screen over the sidebar's cached part (the
//! part renders again only when the width changes), straddling the
//! sidebar's border, or the glass slab's edge under Liquid Glass.

use gpui::{
    canvas, prelude::*, px, AnyElement, Context, CursorStyle, DispatchPhase, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Window,
};

use super::MainScreen;
use crate::kit::divider::DIVIDER;
use crate::theme::{self, Theme};

/// The sidebar's edge being dragged.
pub(super) struct SideDrag {
    /// Where the press was (window x).
    start: Pixels,
    /// The width then.
    from: f32,
}

impl MainScreen {
    /// The docked sidebar's width in this window now.
    pub(super) fn sidebar_width(&self, window: &Window) -> Pixels {
        theme::sidebar_width(self.sidebar_w, f32::from(window.viewport_size().width))
    }

    /// Where the sidebar's visible edge is, from the window's left.
    fn sidebar_edge(&self, window: &Window, cx: &gpui::App) -> Pixels {
        let w = self.sidebar_width(window);
        // Under Liquid Glass the slab floats inside its margin.
        let outer = crate::kit::chrome_width(w, cx);
        w + (outer - w) / 2.
    }

    /// The grab bar on the docked sidebar's edge (and, while it is dragged,
    /// the resize cursor wherever the pointer is).
    pub(super) fn render_side_edge(&self, t: &Theme, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let edge = self.sidebar_edge(window, cx);
        let dragging = self.side_drag.is_some();
        gpui::div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(edge - px(DIVIDER / 2.))
            .child(
                crate::kit::divider("sidebar-edge", true, dragging, t)
                    .debug_selector(|| "sidebar-edge".into())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, e: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.side_edge_press(e.position.x, e.click_count, window, cx);
                        }),
                    ),
            )
            .when(dragging, |d| {
                // The window's own mouse events, wherever the pointer is
                // (past the window's edge too: the release there ends it).
                let screen = cx.entity().downgrade();
                d.child(
                    canvas(
                        |_, _, _| {},
                        move |_, _, window, _| {
                            window.set_window_cursor_style(CursorStyle::ResizeLeftRight);
                            let s = screen.clone();
                            window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                                if phase == DispatchPhase::Bubble {
                                    let _ = s.update(cx, |s, cx| s.side_edge_move(e, window, cx));
                                }
                            });
                            let s = screen.clone();
                            window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                                if phase == DispatchPhase::Bubble && e.button == MouseButton::Left {
                                    let _ = s.update(cx, |s, cx| s.side_edge_release(cx));
                                }
                            });
                        },
                    )
                    .size_0(),
                )
            })
            .into_any_element()
    }

    /// A press on the edge at window x `x`: a drag starts, or a double
    /// click puts the default width back.
    pub(super) fn side_edge_press(&mut self, x: Pixels, clicks: usize, window: &Window, cx: &mut Context<Self>) {
        if clicks >= 2 {
            self.side_drag = None;
            self.sidebar_w = None;
            self.save_sidebar_width(cx);
            return;
        }
        self.side_drag = Some(SideDrag { start: x, from: f32::from(self.sidebar_width(window)) });
        cx.notify();
    }

    /// The pointer at window x `x` while the edge is dragged: the width
    /// follows it, within its range.
    pub(super) fn side_edge_drag_to(&mut self, x: Pixels, window: &Window, cx: &mut Context<Self>) {
        let Some(d) = &self.side_drag else { return };
        let want = d.from + f32::from(x - d.start);
        let w = f32::from(theme::sidebar_width(Some(want), f32::from(window.viewport_size().width)));
        if self.sidebar_w != Some(w) {
            self.sidebar_w = Some(w);
            cx.notify();
        }
    }

    /// The drag ends: the width is kept.
    pub(super) fn side_edge_release(&mut self, cx: &mut Context<Self>) {
        if self.side_drag.take().is_some() {
            self.save_sidebar_width(cx);
            cx.notify();
        }
    }

    fn side_edge_move(&mut self, e: &MouseMoveEvent, window: &Window, cx: &mut Context<Self>) {
        if self.side_drag.is_none() {
            return;
        }
        // Let go where the window didn't see it (another window, a menu).
        if e.pressed_button != Some(MouseButton::Left) {
            self.side_edge_release(cx);
            return;
        }
        self.side_edge_drag_to(e.position.x, window, cx);
    }

    /// For the next window and the next run (`None`: the default).
    fn save_sidebar_width(&mut self, cx: &mut Context<Self>) {
        let w = self.sidebar_w.filter(|w| *w != f32::from(theme::SIDEBAR_W));
        self.sidebar_w = w;
        self.update_ui(
            |mut s| {
                s.sidebar_width = w;
                s
            },
            cx,
        );
        cx.notify();
    }

    /// Debug builds' `sidebar <px>|reset`: the edge pressed, dragged to
    /// that width and let go, through the drag's own steps.
    pub fn debug_sidebar(&mut self, arg: &str, window: &mut Window, cx: &mut Context<Self>) {
        if arg == "reset" {
            let x = self.sidebar_edge(window, cx);
            self.side_edge_press(x, 2, window, cx);
            return;
        }
        let Ok(want) = arg.parse::<f32>() else { return };
        let x = self.sidebar_edge(window, cx);
        let now = f32::from(self.sidebar_width(window));
        self.side_edge_press(x, 1, window, cx);
        self.side_edge_drag_to(x + px(want - now), window, cx);
        self.side_edge_release(cx);
    }
}
