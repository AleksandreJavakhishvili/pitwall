//! The modal frame (`Modal.tsx`, `overlays.css` `.modal`): a backdrop over
//! the window, a raised box with the app's shadow that rises in (still with
//! Reduce motion), an optional title row with ✕. A click on the backdrop
//! closes; Tab and Shift-Tab move between the fields inside.

use gpui::{
    actions, div, prelude::*, px, AnimationExt, AnyElement, App, IntoElement, KeyBinding,
    MouseButton, SharedString, Window,
};

use crate::theme::{Motion, Theme};

use super::button::icon_btn;
use super::text::label;

actions!(pw_dialog, [NextField, PrevField]);

/// The key context inside a modal.
pub const CONTEXT: &str = "PwDialog";

pub fn bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("tab", NextField, Some(CONTEXT)),
        KeyBinding::new("shift-tab", PrevField, Some(CONTEXT)),
    ]
}

/// A modal: `id` names it (and its animation), `width` in px, `t` the theme
/// (it draws with [`Theme::float`]), `motion` whether it rises in.
pub struct Modal {
    id: &'static str,
    width: f32,
    title: Option<SharedString>,
    on_close: super::OnClose,
    motion: bool,
}

impl Modal {
    pub fn new(
        id: &'static str,
        width: f32,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Modal {
        Modal {
            id,
            width,
            title: None,
            on_close: std::rc::Rc::new(on_close),
            motion: false,
        }
    }

    /// The title row ("NEW AGENT" with ✕).
    pub fn title(mut self, title: impl Into<SharedString>) -> Modal {
        self.title = Some(title.into());
        self
    }

    /// Rise in (pass `crate::theme::motion_on(cx)`).
    pub fn motion(mut self, on: bool) -> Modal {
        self.motion = on;
        self
    }

    pub fn render(self, t: &Theme, body: impl IntoElement) -> AnyElement {
        let t = t.float();
        let close_bg = self.on_close.clone();
        let panel = div()
            .id(self.id)
            .key_context(CONTEXT)
            .on_action(|_: &NextField, window, _| window.focus_next())
            .on_action(|_: &PrevField, window, _| window.focus_prev())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .occlude()
            .w(px(self.width))
            .max_w_full()
            .max_h_full()
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(t.raised)
            .border_1()
            .border_color(t.line_strong)
            .shadow(t.drop_shadow())
            .overflow_hidden()
            .when_some(self.title, |d, title| {
                let close = self.on_close.clone();
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_between()
                        .pt(px(14.))
                        .pr(px(12.))
                        .pb(px(6.))
                        .pl(px(18.))
                        .child(label(title, t.text, 15.).line_height(px(19.)))
                        .child(
                            icon_btn("modal-close", "x", "Close (esc)", false, &t)
                                .on_click(move |_, window, cx| close(window, cx)),
                        ),
                )
            })
            .child(body);
        let panel: AnyElement = if self.motion {
            panel
                .with_animation(self.id, Motion::open(), |el, d| {
                    let rise = Motion::spring()(d);
                    el.opacity(d).mt(px((1. - rise) * Motion::RISE))
                })
                .into_any_element()
        } else {
            panel.into_any_element()
        };
        div()
            .id("modal-backdrop")
            .absolute()
            .inset_0()
            .p(px(24.))
            .bg(t.scrim())
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| close_bg(window, cx))
            .child(panel)
            .into_any_element()
    }
}
