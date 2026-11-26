//! "2 queued" on an agent's tile header, and the Next up popover it opens
//! (`crate::next_up`): anchored under the chip, in the window as menus
//! are, or as a glass pop-up under Liquid Glass (`kit::pop`). Esc or a
//! press outside closes it. With nothing queued there is no chip; the
//! palette's "Show Next up for <agent>" still opens the popover there.

use std::rc::Rc;

use gpui::{
    anchored, deferred, div, prelude::*, px, AnyElement, App, Context, IntoElement, MouseButton, MouseDownEvent,
    SharedString, Window,
};

use pitwall_proto::AgentView;

use crate::kit::{pop, tooltip, HoverText as _};
use crate::next_up::{self, Shown};
use crate::theme::{Theme, RADIUS};

use super::MainScreen;

/// The open popover: whose, and which opening (a pop-up that closes late
/// closes only its own).
pub(super) struct NextUpOpen {
    pub agent: String,
    serial: usize,
}

impl MainScreen {
    /// Open the agent's Next up popover (closing any other).
    pub(super) fn open_next_up(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.next_up_serial += 1;
        self.next_up = Some(NextUpOpen { agent: id.to_string(), serial: self.next_up_serial });
        self.menu = None;
        // Esc reaches the screen's Dismiss (a terminal would eat it).
        if !pop::hosted(cx) {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    pub(super) fn close_next_up(&mut self, cx: &mut Context<Self>) {
        if self.next_up.take().is_some() {
            cx.notify();
        }
    }

    /// Closed by a press outside it (or, as a pop-up, by losing the
    /// keyboard): a press on its own chip then closes it rather than
    /// opening it again.
    fn dismiss_next_up(&mut self, cx: &mut Context<Self>) {
        if let Some(o) = self.next_up.take() {
            self.next_up_dismissed = Some((o.agent, std::time::Instant::now()));
            cx.notify();
        }
    }

    /// The chip was pressed: open, or close when it was open.
    fn press_next_up(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let just_closed = self
            .next_up_dismissed
            .take()
            .is_some_and(|(a, at)| a == id && at.elapsed() < std::time::Duration::from_millis(500));
        if self.next_up.as_ref().is_some_and(|o| o.agent == id) {
            self.close_next_up(cx);
        } else if !just_closed {
            self.open_next_up(id, window, cx);
        }
    }

    /// The agent's Next up popover is open (tests).
    #[cfg(test)]
    pub(super) fn next_up_open(&self) -> Option<&str> {
        self.next_up.as_ref().map(|o| o.agent.as_str())
    }

    /// The chip and, when open, the popover under it: `None` when neither
    /// is shown.
    pub(super) fn next_up_chip(&self, a: &AgentView, pane_id: &str, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.next_up.as_ref().filter(|o| o.agent == a.id);
        let label = next_up::chip_label(a);
        if open.is_none() && label.is_none() {
            return None;
        }
        // Clicks stay on the chip (no drag, no double-click maximise).
        let keep = |_: &MouseDownEvent, _: &mut Window, cx: &mut App| cx.stop_propagation();
        let chip = label.map(|label| {
            let id = a.id.clone();
            let on = open.is_some();
            let waits = a.queue.iter().any(|q| q.origin.is_some());
            let sel = format!("queued-{}", a.id);
            // (The selector on a box of its own: a hover style takes the
            // chip's for the hover probe in tests.)
            let chip = div()
                .id(SharedString::from(format!("queued-{pane_id}")))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(5.))
                .h(px(22.))
                .px(px(7.))
                .rounded(px(4.))
                .border_1()
                .border_color(if waits { t.focus.opacity(0.45) } else { t.line })
                .bg(if on { t.surface_3 } else { t.surface_2 })
                .text_size(px(11.5))
                .text_color(if on { t.text } else { t.text_2 })
                .cursor_pointer()
                .hover_text(t.text, |s| s)
                .tooltip(tooltip("Next up: what is sent to this agent next"))
                .on_mouse_down(MouseButton::Left, keep)
                .on_click(cx.listener(move |this, _, window, cx| this.press_next_up(&id, window, cx)))
                .child(crate::kit::icon("send", 11., t.text_3))
                .child(label);
            div().flex().debug_selector(move || sel).child(chip)
        });
        let pop = open.map(|o| self.next_up_pop(a, o.serial, t, cx));
        Some(div().relative().flex_none().children(chip).children(pop).into_any_element())
    }

    fn next_up_pop(&self, a: &AgentView, serial: usize, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let shown = Shown::of(a, cx);
        let screen = cx.weak_entity();
        // Under the chip, its right edge on the chip's.
        let place = div().absolute().right_0().top(px(26.));
        if pop::hosted(cx) {
            let build: pop::Build = Rc::new(move |_: &mut Window, cx: &mut App| {
                next_up::body(&shown, &pop::theme(cx), &screen).into_any_element()
            });
            let closer = cx.weak_entity();
            let on_dismiss: pop::OnDismiss = Rc::new(move |_: &mut Window, cx: &mut App| {
                let _ = closer.update(cx, |this, cx| {
                    if this.next_up.as_ref().is_some_and(|o| o.serial == serial) {
                        this.dismiss_next_up(cx);
                    }
                });
            });
            let spec = pop::Pop {
                at: None,
                radius: pop::MENU_RADIUS,
                focus: None,
                build,
                on_dismiss,
            };
            // The pop-up's top left is the anchor's: the anchor sits a
            // popover's width left of the chip's right edge.
            return place
                .child(div().absolute().right_0().w(px(next_up::WIDTH)).child(pop::anchor(("next-up-pop", serial), spec)))
                .into_any_element();
        }
        let panel = next_up::body(&shown, t, &screen)
            .occlude()
            .rounded(RADIUS)
            .border_1()
            .border_color(t.line_strong)
            .bg(t.raised)
            .shadow(t.float_shadow())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.dismiss_next_up(cx)));
        place
            .child(
                div().absolute().right_0().w(px(next_up::WIDTH)).child(
                    deferred(anchored().snap_to_window_with_margin(px(4.)).child(crate::kit::motion::enter(
                        "next-up-in",
                        crate::kit::motion::Fx::MENU,
                        panel,
                    )))
                    .with_priority(2),
                ),
            )
            .into_any_element()
    }
}
