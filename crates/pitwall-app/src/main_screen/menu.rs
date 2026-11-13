//! A small popup menu at a point (React `Menu.tsx`): kept on screen, ↑/↓
//! move, ⏎ runs, Esc or a click outside closes. In-house; a candidate for
//! the kit's popover/menu.
//!
//! Under Liquid Glass it is a glass pop-up above the window
//! (`kit::pop`): the same rows, with the keyboard, kept on the screen
//! rather than in the window.

use crate::kit::HoverText as _;
use std::rc::Rc;

use gpui::{
    anchored, deferred, div, prelude::*, px, AnyElement, App, Context, FocusHandle, IntoElement, MouseButton,
    Pixels, Point, SharedString, WeakEntity, Window,
};

use super::{Confirm, MainScreen, SelectNext, SelectPrev};
use crate::kit::pop;
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
    /// Which opening this is: a pop-up that closes late (it lost the
    /// keyboard to the click that opened the next menu) closes only its own.
    serial: usize,
}

/// What a menu shows at one moment, as plain values.
struct Shown {
    /// Icon, label, section title.
    rows: Vec<(&'static str, SharedString, bool)>,
    /// The keyboard's row.
    cursor: Option<usize>,
    focus: FocusHandle,
}

impl CtxMenu {
    fn shown(&self) -> Shown {
        Shown {
            rows: self.items.iter().map(|it| (it.icon, it.label.clone(), it.header)).collect(),
            cursor: self.keyboard.then_some(self.selected),
            focus: self.focus.clone(),
        }
    }
}

/// The menu's rows with its keys: the panel's content, drawn in the
/// screen's window or in the menu's pop-up. `this` is only touched when a
/// row is chosen or a key pressed.
fn menu_rows(m: &Shown, t: &Theme, this: &WeakEntity<MainScreen>) -> gpui::Stateful<gpui::Div> {
    let rows = m.rows.iter().enumerate().map(|(i, (icon, label, header))| {
        if *header {
            return div()
                .id(("menu-item", i))
                .px(px(8.))
                .pt(px(8.))
                .pb(px(2.))
                .border_t_1()
                .border_color(t.line)
                .text_size(px(10.5))
                .text_color(t.text_3)
                .child(label.clone());
        }
        let this = this.clone();
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
            .when(m.cursor == Some(i), |d| d.bg(t.surface_3))
            .hover_probed(|s| s.bg(t.surface_3))
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                let _ = this.update(cx, |this, cx| this.choose_menu_item(i, window, cx));
            })
            .child(div().w(px(16.)).flex().when(!icon.is_empty(), |d| d.child(crate::kit::icon(icon, 14., t.text_3))))
            .child(div().flex_1().child(label.clone()))
    });
    let (prev, next, confirm) = (this.clone(), this.clone(), this.clone());
    div()
        .id("ctx-menu")
        .key_context("PwMenu")
        .track_focus(&m.focus)
        .on_action(move |_: &SelectPrev, _, cx| {
            let _ = prev.update(cx, |this, cx| this.step_menu(-1, cx));
        })
        .on_action(move |_: &SelectNext, _, cx| {
            let _ = next.update(cx, |this, cx| this.step_menu(1, cx));
        })
        .on_action(move |_: &Confirm, window, cx| {
            let _ = confirm.update(cx, |this, cx| {
                let i = this.menu.as_ref().map(|m| m.selected).unwrap_or(0);
                this.choose_menu_item(i, window, cx)
            });
        })
        .min_w(px(190.))
        .p(px(4.))
        .font_family(crate::kit::UI_FONT)
        .children(rows)
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
        // A pop-up has the keyboard in its own window: this one's focus
        // stays where it is.
        if !pop::hosted(cx) {
            window.focus(&focus, cx);
        }
        self.menu_serial += 1;
        self.menu = Some(CtxMenu {
            at,
            items,
            selected: 0,
            keyboard: false,
            focus,
            serial: self.menu_serial,
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
        window.focus(&self.focus, cx);
        run(self, window, cx);
        cx.notify();
    }

    /// Row `i` was clicked, or Enter pressed on it. In a pop-up (`window`
    /// is then its window): the pop-up closes and the row runs in this
    /// screen's window.
    fn choose_menu_item(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.menu.as_ref().and_then(|m| m.items.get(i)).is_some_and(|it| !it.header) {
            return;
        }
        if pop::parent_of(window).is_none() {
            return self.run_menu_item(i, window, cx);
        }
        let this = cx.weak_entity();
        pop::pick(window, cx, move |window, cx| {
            let _ = this.update(cx, |this, cx| this.run_menu_item(i, window, cx));
        });
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

    /// The menu as a glass pop-up: where it would be drawn, the anchor
    /// that keeps its pop-up open.
    fn hosted_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let m = self.menu.as_ref()?;
        let (serial, this, closer) = (m.serial, cx.weak_entity(), cx.weak_entity());
        // The pop-up draws what the menu shows now, from values: its
        // window must not touch the screen while it draws (see `pop::Build`).
        let shown = m.shown();
        let build: pop::Build =
            Rc::new(move |_: &mut Window, cx: &mut App| menu_rows(&shown, &pop::theme(cx), &this).into_any_element());
        // Esc, a press outside, the keyboard gone elsewhere.
        let on_dismiss: pop::OnDismiss = Rc::new(move |_: &mut Window, cx: &mut App| {
            let _ = closer.update(cx, |this, cx| {
                if this.menu.as_ref().is_some_and(|m| m.serial == serial) {
                    this.menu = None;
                    cx.notify();
                }
            });
        });
        let spec = pop::Pop { at: Some(m.at), radius: pop::MENU_RADIUS, focus: Some(m.focus.clone()), build, on_dismiss };
        Some(pop::anchor(("ctx-menu-pop", serial), spec).into_any_element())
    }

    pub(super) fn render_menu(
        &mut self,
        t: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if pop::hosted(cx) {
            return self.hosted_menu(cx);
        }
        let m = self.menu.as_ref()?;
        let (at, rows) = (m.at, menu_rows(&m.shown(), t, &cx.weak_entity()));
        Some(
            deferred(
                anchored()
                    .position(at)
                    .snap_to_window_with_margin(px(4.))
                    .child(crate::kit::motion::enter(
                        "ctx-menu-in",
                        crate::kit::motion::Fx::MENU,
                        rows.on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.notify();
                        }))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .rounded(RADIUS)
                        .bg(t.raised)
                        .shadow(t.float_shadow()),
                    )),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}
