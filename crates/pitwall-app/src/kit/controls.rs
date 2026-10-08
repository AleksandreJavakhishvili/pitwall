//! Form controls (`base.css`, `overlays.css`): checkbox, labelled checkbox,
//! toggle switch, radio dot, segmented control, dropdown (trigger and
//! menu) and the spinner. Rows around them take the click.

use std::rc::Rc;

use gpui::{
    anchored, canvas, deferred, div, point, prelude::*, px, AnimationExt, AnyElement, App,
    ClickEvent, Div, ElementId, FontWeight, IntoElement, PathBuilder, SharedString, Stateful,
    Window,
};

use crate::theme::{Theme, RADIUS, RADIUS_SM};

use super::icons::icon;
use super::tooltip::tooltip;

/// Called with the picked index (segmented controls, dropdowns).
pub type OnPick = Rc<dyn Fn(usize, &mut Window, &mut App)>;
/// Called when a popover closes.
pub type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

/// A checkbox square, 14 px (`input[type=checkbox]` with `accent-color:
/// var(--text)`).
pub fn check(checked: bool, t: &Theme) -> Div {
    checkbox_box(checked, false, 14., t)
}

/// A checkbox square of `size` px; `disabled` dims it.
pub fn checkbox_box(checked: bool, disabled: bool, size: f32, t: &Theme) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(3.))
        .border_1()
        .flex()
        .items_center()
        .justify_center()
        .border_color(if checked { t.text } else { t.text_4 })
        .bg(if checked { t.text } else { t.bg })
        .when(disabled, |d| d.opacity(0.45))
        .when(checked, |d| d.child(icon("check", size - 4., t.bg)))
}

/// `.ex-check` and the like: a small labelled checkbox (12 px box, 11.5 px
/// text-3), the whole row clickable.
pub fn checkbox(
    id: impl Into<ElementId>,
    on: bool,
    label: impl Into<SharedString>,
    t: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let hover = t.text_2;
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .text_size(px(11.5))
        .text_color(t.text_3)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |s| s.text_color(hover))
        .child(
            div()
                .size(px(12.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .border_color(if on { t.text_2 } else { t.text_4 })
                .bg(if on { t.text_2 } else { t.raised })
                .when(on, |d| d.child(icon("check", 10., t.raised))),
        )
        .child(label.into())
        .on_click(on_click)
}

/// `.toggle-track`: the switch, 26×15. Attach `.on_click`.
pub fn toggle(id: impl Into<ElementId>, on: bool, t: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .w(px(26.))
        .h(px(15.))
        .rounded(px(10.))
        .border_1()
        .cursor_pointer()
        .border_color(if on { t.text } else { t.line_strong })
        .bg(if on { t.text } else { t.surface_3 })
        .relative()
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(if on { 13. } else { 2. }))
                .size(px(9.))
                .rounded_full()
                .bg(if on { t.bg } else { t.text_3 }),
        )
}

/// A radio dot, 14 px.
pub fn radio(checked: bool, t: &Theme) -> Div {
    div()
        .flex_none()
        .size(px(14.))
        .rounded_full()
        .border_1()
        .flex()
        .items_center()
        .justify_center()
        .border_color(if checked { t.text } else { t.text_4 })
        .when(checked, |d| {
            d.child(div().size(px(6.)).rounded_full().bg(t.text))
        })
}

/// One button of a segmented control.
pub struct SegItem {
    pub label: SharedString,
    pub on: bool,
    pub tooltip: Option<SharedString>,
}

impl SegItem {
    pub fn new(label: impl Into<SharedString>, on: bool) -> SegItem {
        SegItem {
            label: label.into(),
            on,
            tooltip: None,
        }
    }
}

/// `.seg`: a row of radio buttons; `on_pick` gets the chosen index. Under
/// Glass it sits on glass surface-2 with a hairline ring.
pub fn seg(id: &'static str, items: Vec<SegItem>, t: &Theme, on_pick: OnPick) -> Div {
    let (bg, on_bg) = if t.is_glass() {
        (t.g.surface_2, t.g.surface_3)
    } else {
        (t.surface_2, t.surface_3)
    };
    let (fg, fg3) = (t.text, t.text_3);
    div()
        .flex()
        .flex_none()
        .gap(px(1.))
        .p(px(2.))
        .rounded(RADIUS)
        .bg(bg)
        .when(t.is_glass(), |d| d.border_1().border_color(t.g.line))
        .children(items.into_iter().enumerate().map(move |(i, item)| {
            let pick = on_pick.clone();
            div()
                .id((id, i))
                .h(px(24.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(RADIUS_SM)
                .text_size(px(12.))
                .cursor_pointer()
                .text_color(if item.on { fg } else { fg3 })
                .when(item.on, |d| d.bg(on_bg).font_weight(FontWeight::SEMIBOLD))
                .hover(move |s| s.text_color(fg))
                .when_some(item.tooltip, |d, tip| d.tooltip(tooltip(tip)))
                .on_click(move |_, window, cx| pick(i, window, cx))
                .child(item.label)
        }))
}

/// A dropdown's trigger (`select.input`): the value and a caret.
pub fn select_trigger(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    t: &Theme,
) -> Stateful<Div> {
    let border = t.text_4;
    div()
        .id(id)
        .h(px(26.))
        .pl(px(8.))
        .pr(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(RADIUS)
        .bg(t.bg)
        .border_1()
        .border_color(t.line_strong)
        .text_size(px(12.))
        .text_color(t.text)
        .cursor_pointer()
        .hover(move |s| s.border_color(border))
        .child(
            div()
                .max_w(px(260.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(text.into()),
        )
        .child(icon("chevron-down", 11., t.text_3))
}

/// The open dropdown under its trigger (put both in a `relative` box):
/// `on_pick(index)`, `on_close` on a click outside.
pub fn select_menu(
    id: impl Into<ElementId>,
    items: Vec<(SharedString, bool)>,
    t: &Theme,
    on_pick: OnPick,
    on_close: OnClose,
) -> impl IntoElement {
    let t = t.float();
    let id = id.into();
    div().absolute().left_0().top(px(28.)).child(deferred(
        anchored().snap_to_window().child(
            div()
                .id(id)
                .occlude()
                .min_w(px(180.))
                .max_h(px(320.))
                .overflow_y_scroll()
                .p(px(4.))
                .rounded(RADIUS)
                .bg(t.raised)
                .border_1()
                .border_color(t.line_strong)
                .shadow_lg()
                .on_mouse_down_out(move |_, window, cx| on_close(window, cx))
                .children(items.into_iter().enumerate().map(|(i, (label, on))| {
                    let pick = on_pick.clone();
                    let hover = t.surface_3;
                    div()
                        .id(i)
                        .px(px(8.))
                        .py(px(5.))
                        .rounded(RADIUS_SM)
                        .text_size(px(12.))
                        .text_color(if on { t.text } else { t.text_2 })
                        .when(on, |d| d.font_weight(FontWeight::MEDIUM))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .on_click(move |_, window, cx| pick(i, window, cx))
                        .child(label)
                })),
        ),
    ))
}

/// The spinning ring (`onb-spin`, `fresh-spin`): a faint track with a
/// bright quarter turning; still when `motion` is off.
pub fn spinner(id: impl Into<ElementId>, size: f32, motion: bool, t: &Theme) -> AnyElement {
    let (track, head) = (t.line_strong, t.text);
    let ring = move |turn: f32| {
        div()
            .flex_none()
            .size(px(size))
            .rounded_full()
            .border(px(1.5))
            .border_color(track)
            .child(
                canvas(
                    |_, _, _| {},
                    move |b, _, window, _| {
                        let r = f32::from(b.size.width) / 2. - 0.75;
                        let c = b.center();
                        let mut path = PathBuilder::stroke(px(1.5));
                        let start = turn * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                        for i in 0..=12 {
                            let a = start + i as f32 / 12. * std::f32::consts::FRAC_PI_2;
                            let pt = point(c.x + px(r * a.cos()), c.y + px(r * a.sin()));
                            if i == 0 {
                                path.move_to(pt);
                            } else {
                                path.line_to(pt);
                            }
                        }
                        if let Ok(path) = path.build() {
                            window.paint_path(path, head);
                        }
                    },
                )
                .absolute()
                .top(px(-1.5))
                .left(px(-1.5))
                .size(px(size)),
            )
    };
    if motion {
        ring(0.)
            .with_animation(
                id,
                gpui::Animation::new(std::time::Duration::from_millis(900)).repeat(),
                move |_, d| ring(d),
            )
            .into_any_element()
    } else {
        ring(0.).into_any_element()
    }
}
