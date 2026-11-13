//! The modal frame (`Modal.tsx`, `overlays.css` `.modal`): a backdrop over
//! the window (fading in: [`scrim_in`]), a raised box with the app's shadow
//! that rises in (still with Reduce motion), an optional title row with ✕. A click on the backdrop
//! closes; Tab and Shift-Tab move between the fields inside.
//!
//! Under Liquid Glass (`pop::hosted`) the box is a glass pop-up over the
//! backdrop, which stays in the window: [`backdrop`] is the window's half
//! and [`Modal::render_in`], drawn in the pop-up, the box's content.
//! [`Modal::show`] is both, for a dialog its owner draws from values; a
//! dialog that is a view of its own is drawn by its pop-up alone (see
//! `pop::Build`).

use gpui::{
    actions, div, prelude::*, px, AnyElement, App, Context, ElementId, FocusHandle, IntoElement, KeyBinding,
    MouseButton, SharedString, WeakEntity, Window,
};

use crate::theme::Theme;

use super::motion::{enter, Fx};

use super::button::icon_btn;
use super::pop;
use super::text::label_t;

/// The box's corners.
pub const RADIUS: f32 = 12.;

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
            .on_action(|_: &NextField, window, cx| window.focus_next(cx))
            .on_action(|_: &PrevField, window, cx| window.focus_prev(cx))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .occlude()
            .w(px(self.width))
            .max_w_full()
            .max_h_full()
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(t.raised)
            .shadow(t.float_shadow())
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
                        .child(label_t(title, t.text, 15., 0.08).line_height(px(19.)))
                        .child(
                            icon_btn("modal-close", "x", "Close (esc)", false, &t)
                                .on_click(move |_, window, cx| close(window, cx)),
                        ),
                )
            })
            .child(body);
        // (Reduce motion: `enter` keeps the fade alone.)
        let panel: AnyElement = enter(self.id, Fx::MODAL, panel).into_any_element();
        let backdrop = div()
            .id("modal-backdrop")
            // Takes the click: nothing under a backdrop is clicked through it.
            .occlude()
            .absolute()
            .inset_0()
            .p(px(24.))
            .bg(t.scrim())
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| close_bg(window, cx))
            .child(panel);
        scrim_in("modal-backdrop", backdrop)
    }
}

impl Modal {
    /// [`Modal::render`] in the window that draws it. In a glass pop-up
    /// (the pop-up is the box and [`backdrop`] is drawn in its parent):
    /// only what is in the box, with the pop-up's theme; its ✕ closes
    /// in the parent window.
    pub fn render_in(self, window: &Window, cx: &App, t: &Theme, body: impl IntoElement) -> AnyElement {
        if pop::parent_of(window).is_none() {
            return self.render(t, body);
        }
        let t = pop::theme(cx);
        div()
            .id(self.id)
            .key_context(CONTEXT)
            .on_action(|_: &NextField, window, cx| window.focus_next(cx))
            .on_action(|_: &PrevField, window, cx| window.focus_prev(cx))
            .w(px(self.width))
            .max_w_full()
            .max_h_full()
            .flex()
            .flex_col()
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
                        .child(label_t(title, t.text, 15., 0.08).line_height(px(19.)))
                        .child(icon_btn("modal-close", "x", "Close (esc)", false, &t).on_click(move |_, window, cx| {
                            let close = close.clone();
                            pop::pick(window, cx, move |window, cx| close(window, cx))
                        })),
                )
            })
            .child(body)
            .into_any_element()
    }

    /// The whole dialog from where its owner draws it. `body` builds what
    /// is in the box with the theme it is given, in the window that shows
    /// it: this one, or (under Liquid Glass) the box's pop-up, where it
    /// must not touch a view this window draws (see `pop::Build`): it is
    /// built from values, with handlers that reach the owner when they
    /// run. `focus`: what has the keyboard in the pop-up.
    pub fn show(
        self,
        focus: Option<FocusHandle>,
        t: &Theme,
        window: &mut Window,
        cx: &mut App,
        body: impl Fn(&Theme, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> AnyElement {
        if !pop::hosted(cx) {
            let body = body(t, window, cx);
            return self.render(t, body);
        }
        let (id, width, title, on_close) = (self.id, self.width, self.title, self.on_close);
        let (close, t) = (on_close.clone(), t.clone());
        let build: pop::Build = std::rc::Rc::new(move |window: &mut Window, cx: &mut App| {
            let t = on_glass(&t);
            let body = body(&t, window, cx);
            let m = Modal { id, width, title: title.clone(), on_close: close.clone(), motion: false };
            m.render_in(window, cx, &t, body)
        });
        backdrop(id, focus, on_close, window, cx, build)
    }
}

/// The theme a dialog's body draws with in its pop-up: `t` (what it
/// draws with in the window), its dim text a step brighter as
/// `pop::theme`'s (too dim on glass over a bright page).
pub fn on_glass(t: &Theme) -> Theme {
    let mut t = t.clone();
    t.text_3 = t.text_2;
    t.text_2 = t.text;
    t
}

/// A handler in a dialog built from values ([`Modal::show`]) that reaches
/// its owner when it runs (`cx.listener` without a `Context`), in the
/// owner's window: the dialog's own, or the parent of its pop-up.
pub fn listener<T: 'static, E: 'static>(
    this: &WeakEntity<T>,
    f: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
) -> impl Fn(&E, &mut Window, &mut App) + 'static {
    let this = this.clone();
    move |_, window, cx| match pop::parent_of(window) {
        None => {
            let _ = this.update(cx, |t, cx| f(t, window, cx));
        }
        Some(parent) => {
            let _ = parent.update(cx, |_, window, cx| this.update(cx, |t, cx| f(t, window, cx)));
        }
    }
}

/// As [`listener`], for what closes the dialog: its pop-up closes first
/// (and its parent has the keyboard again when `f` runs).
pub fn leaver<T: 'static, E: 'static>(
    this: &WeakEntity<T>,
    f: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
) -> impl Fn(&E, &mut Window, &mut App) + 'static {
    let (this, f) = (this.clone(), std::rc::Rc::new(f));
    move |_, window, cx| {
        let (this, f) = (this.clone(), f.clone());
        pop::pick(window, cx, move |window, cx| {
            let _ = this.update(cx, |t, cx| f(t, window, cx));
        })
    }
}

/// A dialog on glass, in its parent window: the backdrop (a press on it
/// closes) and the anchor of the pop-up that `build` draws the box in
/// (with [`Modal::render_in`]). `on_close` also hears Esc in the pop-up
/// and a press beside it; it runs in this window.
pub fn backdrop(
    id: &'static str,
    focus: Option<FocusHandle>,
    on_close: super::OnClose,
    window: &Window,
    cx: &App,
    build: pop::Build,
) -> AnyElement {
    let scrim = crate::theme::theme(cx).float().scrim();
    let close = on_close.clone();
    let spec = pop::Pop { at: None, radius: RADIUS, focus, build, on_dismiss: on_close };
    let backdrop = div()
        .id("modal-backdrop")
        // Takes the click: nothing under a backdrop is clicked through it.
        .occlude()
        .absolute()
        .inset_0()
        .bg(scrim)
        .on_mouse_down(MouseButton::Left, move |_, window, cx| close(window, cx))
        .child(pop::anchor(id, spec).modal(window.viewport_size()));
    scrim_in("modal-backdrop", backdrop)
}

/// A backdrop (`.backdrop`, `.scrim`: [`Theme::scrim`]) fading in over
/// 0.12 s; its content fades with it, as in CSS. Every dialog, drawer and
/// the palette put their scrim through this.
pub fn scrim_in<E: IntoElement + gpui::Styled + 'static>(
    id: impl Into<ElementId>,
    el: E,
) -> AnyElement {
    enter(id, Fx::SCRIM, el).into_any_element()
}
