//! A small popup menu at a point (React `Menu.tsx`): kept on screen, ↑/↓
//! move, ⏎ runs, Esc or a click outside closes. In-house; a candidate for
//! the kit's popover/menu.

use crate::kit::HoverText as _;
use std::rc::Rc;

use gpui::{
    anchored, deferred, div, prelude::*, px, Context, FocusHandle, IntoElement, MouseButton,
    Pixels, Point, SharedString, Window,
};

use super::{Confirm, MainScreen, SelectNext, SelectPrev};
use crate::theme::{Theme, RADIUS};

type Run = Rc<dyn Fn(&mut MainScreen, &mut Window, &mut Context<MainScreen>)>;

pub struct MenuItem {
    icon: &'static str,
    label: SharedString,
    run: Run,
    /// A section's title: shown, never chosen.
    header: bool,
}

impl MenuItem {
    pub fn new(
        icon: &'static str,
        label: &str,
        run: impl Fn(&mut MainScreen, &mut Window, &mut Context<MainScreen>) + 'static,
    ) -> MenuItem {
        MenuItem {
            icon,
            label: label.to_string().into(),
            run: Rc::new(run),
            header: false,
        }
    }

    /// A section's title ("RUN ON ACCOUNT").
    pub fn header(label: &str) -> MenuItem {
        MenuItem { icon: "", label: label.to_string().into(), run: Rc::new(|_, _, _| {}), header: true }
    }
}

pub struct CtxMenu {
    at: Point<Pixels>,
    items: Vec<MenuItem>,
    selected: usize,
    /// ↑/↓ was used: the keyboard row shows (as `:focus-visible` does; a
    /// menu opened with the pointer shows none).
    keyboard: bool,
    focus: FocusHandle,
}

impl MainScreen {
    /// The open menu's labels (tests).
    #[cfg(test)]
    pub(super) fn menu_labels(&self) -> Vec<String> {
        self.menu.as_ref().map(|m| m.items.iter().map(|i| i.label.to_string()).collect()).unwrap_or_default()
    }

    /// Open a menu at `at` with keyboard focus in it.
    pub(super) fn open_menu(
        &mut self,
        at: Point<Pixels>,
        items: Vec<MenuItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = cx.focus_handle();
        window.focus(&focus);
        self.menu = Some(CtxMenu {
            at,
            items,
            selected: 0,
            keyboard: false,
            focus,
        });
        cx.notify();
    }

    fn run_menu_item(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self
            .menu
            .as_ref()
            .and_then(|m| m.items.get(i))
            .filter(|it| !it.header)
            .map(|it| it.run.clone())
        else {
            return;
        };
        self.menu = None;
        window.focus(&self.focus);
        run(self, window, cx);
        cx.notify();
    }

    fn step_menu(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(m) = self.menu.as_mut() {
            let n = m.items.len() as isize;
            // (Past section titles.)
            for _ in 0..n.max(1) {
                m.selected = (m.selected as isize + delta).rem_euclid(n.max(1)) as usize;
                if !m.items.get(m.selected).is_some_and(|it| it.header) {
                    break;
                }
            }
            m.keyboard = true;
            cx.notify();
        }
    }

    pub(super) fn render_menu(
        &mut self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let m = self.menu.as_ref()?;
        let focus = m.focus.clone();
        let rows = m.items.iter().enumerate().map(|(i, it)| {
            let on = m.keyboard && i == m.selected;
            if it.header {
                return div()
                    .id(("menu-item", i))
                    .px(px(8.))
                    .pt(px(8.))
                    .pb(px(2.))
                    .border_t_1()
                    .border_color(t.line)
                    .text_size(px(10.5))
                    .text_color(t.text_3)
                    .child(it.label.clone());
            }
            div()
                .id(("menu-item", i))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(6.))
                .rounded(crate::theme::RADIUS_SM)
                .text_size(px(13.))
                .line_height(px(18.))
                .text_color(t.text)
                .when(on, |d| d.bg(t.surface_3))
                .hover_probed(|s| s.bg(t.surface_3))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| this.run_menu_item(i, window, cx)))
                .child(div().w(px(16.)).flex().when(!it.icon.is_empty(), |d| {
                    d.child(crate::kit::icon(it.icon, 14., t.text_3))
                }))
                .child(div().flex_1().child(it.label.clone()))
        });
        Some(
            deferred(
                anchored()
                    .position(m.at)
                    .snap_to_window_with_margin(px(4.))
                    .child(crate::kit::motion::enter(
                        "ctx-menu-in",
                        crate::kit::motion::Fx::MENU,
                        div()
                            .id("ctx-menu")
                            .key_context("PwMenu")
                            .track_focus(&focus)
                            .on_action(
                                cx.listener(|this, _: &SelectPrev, _, cx| this.step_menu(-1, cx)),
                            )
                            .on_action(
                                cx.listener(|this, _: &SelectNext, _, cx| this.step_menu(1, cx)),
                            )
                            .on_action(cx.listener(|this, _: &Confirm, window, cx| {
                                let i = this.menu.as_ref().map(|m| m.selected).unwrap_or(0);
                                this.run_menu_item(i, window, cx)
                            }))
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.menu = None;
                                cx.notify();
                            }))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .min_w(px(190.))
                            .p(px(4.))
                            .font_family(crate::kit::UI_FONT)
                            .rounded(RADIUS)
                            .bg(t.raised)
                            .shadow(t.float_shadow())
                            .children(rows),
                    )),
            )
            .with_priority(2),
        )
    }
}
