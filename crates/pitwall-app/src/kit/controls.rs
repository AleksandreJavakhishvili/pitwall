//! Form controls (`base.css`, `overlays.css`): checkbox, labelled checkbox,
//! toggle switch, radio dot, segmented control, dropdown (trigger and
//! menu) and the spinner. Rows around them take the click.

use super::text::Ellipsis as _;
use std::rc::Rc;

use crate::kit::HoverText;
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
        .hover_text(hover, move |s| s)
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

/// `.toggle-track`: the switch, 26×15. Attach `.on_click`. The track's
/// colour and the knob ease over 0.15 s when it flips.
pub fn toggle(id: impl Into<ElementId>, on: bool, t: &Theme) -> Stateful<Div> {
    use super::motion::{mix, tween, Ease, TOGGLE};
    let (track_off, track_on) = (t.surface_3, t.text);
    let (knob_off, knob_on) = (t.text_3, t.bg);
    div()
        .id(id)
        .flex_none()
        .w(px(26.))
        .h(px(15.))
        .rounded(px(10.))
        .border_1()
        .cursor_pointer()
        .border_color(if on { t.text } else { t.line_strong })
        .relative()
        .child(tween("knob", if on { 1. } else { 0. }, TOGGLE, Ease::Ease, move |v| {
            div()
                .absolute()
                .inset_0()
                .rounded(px(10.))
                .bg(mix(track_off, track_on, v))
                .child(
                    div()
                        .absolute()
                        .top(px(2.))
                        .left(px(2. + 11. * v))
                        .size(px(9.))
                        .rounded_full()
                        .bg(mix(knob_off, knob_on, v)),
                )
                .into_any_element()
        }))
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
        .rounded(RADIUS)
        .bg(bg)
        // Glass: `inset 0 0 0 1px` as a border inside the 2 px padding.
        .map(|d| {
            if t.is_glass() {
                d.p(px(1.)).border_1().border_color(t.g.line)
            } else {
                d.p(px(2.))
            }
        })
        .children(items.into_iter().enumerate().map(move |(i, item)| {
            let pick = on_pick.clone();
            let eid = ElementId::from((id, i));
            let el = div()
                .id(eid.clone())
                .h(px(24.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(RADIUS_SM)
                .text_size(px(12.))
                .cursor_pointer()
                .text_color(if item.on { fg } else { fg3 })
                .when(item.on, |d| d.bg(on_bg).font_weight(super::fonts::WEIGHT_550))
                .hover_text(fg, move |s| s);
            let skin = super::motion::Skin::new(item.on.then_some(on_bg), None, RADIUS_SM);
            super::motion::pressable(el, &eid, false, skin)
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
        .hover_probed(move |s| s.border_color(border))
        .child(
            div()
                .max_w(px(260.))
                .ellipsis()
                .child(crate::kit::one_line(text.into())),
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
                        .hover_probed(move |s| s.bg(hover))
                        .on_click(move |_, window, cx| pick(i, window, cx))
                        .child(label)
                })),
        ),
    ))
}

/// The spinning ring (`onb-spin`, `fresh-spin`): a faint track with a
/// bright quarter turning; still when `motion` is off.
pub fn spinner(id: impl Into<ElementId>, size: f32, motion: bool, t: &Theme) -> AnyElement {
    spinner_ms(id, size, 900, motion, t)
}

/// [`spinner`] turning once every `ms` (`onb-spin` 0.7 s, `fresh-spin`
/// 0.9 s).
pub fn spinner_ms(
    id: impl Into<ElementId>,
    size: f32,
    ms: u64,
    motion: bool,
    t: &Theme,
) -> AnyElement {
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
                gpui::Animation::new(std::time::Duration::from_millis(ms)).repeat(),
                move |_, d| ring(d),
            )
            .into_any_element()
    } else {
        ring(0.).into_any_element()
    }
}

/// `select.input` (`.rules-kind`: 28 px, 12.5 px; New agent: 34 px, 13
/// px): the value, a caret, and its menu under it while `open`.
#[allow(clippy::too_many_arguments)]
pub fn select(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    height: f32,
    size: f32,
    full: bool,
    open: bool,
    items: Vec<(SharedString, bool)>,
    t: &Theme,
    on_toggle: impl Fn(&mut Window, &mut App) + 'static,
    on_pick: OnPick,
    on_close: OnClose,
) -> Div {
    let id: ElementId = id.into();
    let hover = t.text_4;
    let trigger = div()
        .id(id.clone())
        .h(px(height))
        .pl(px(9.))
        .pr(px(10.))
        .flex()
        .items_center()
        .gap(px(10.))
        .rounded(RADIUS)
        .bg(t.bg)
        .border_1()
        .border_color(if open { t.text_3 } else { t.line_strong })
        .text_size(px(size))
        .text_color(t.text)
        .cursor_pointer()
        .when(!open, |d| d.hover_probed(move |s| s.border_color(hover)))
        .when(full, |d| d.w_full())
        // A native select is as wide as its longest option.
        .when(!full, |d| d.min_w(px(widest(&items, size))))
        .on_click(move |_, window, cx| on_toggle(window, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .ellipsis()
                .child(super::text::one_line(text.into())),
        )
        .child(icon("chevron-down", 11., t.text_3));
    div()
        .relative()
        .flex_none()
        .when(full, |d| d.w_full())
        .child(trigger)
        .when(open, |d| {
            // The kit's menu sits 28 px under its box's top.
            d.child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(height - 26.))
                    .child(select_menu(
                        ElementId::Name(format!("{id}-menu").into()),
                        items,
                        t,
                        on_pick,
                        on_close,
                    )),
            )
        })
}

/// About how wide a native select draws for these options: the longest
/// label, the padding and the caret.
pub fn widest(items: &[(SharedString, bool)], size: f32) -> f32 {
    let chars = items
        .iter()
        .map(|(l, _)| l.chars().count())
        .max()
        .unwrap_or(0);
    chars as f32 * size * 0.55 + 40.
}

/// `.check`: a checkbox and its text (a title and a `.hint.block`), the
/// whole row clickable unless `disabled`.
pub fn check_row(
    id: impl Into<ElementId>,
    on: bool,
    disabled: bool,
    title: impl IntoElement,
    hint: Option<SharedString>,
    t: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_start()
        .gap(px(10.))
        .when(!disabled, |d| d.cursor_pointer().on_click(on_click))
        .child(
            div()
                .mt(px(2.))
                .child(checkbox_box(on, disabled, 14., t)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(title)
                .when_some(hint, |d, h| d.child(super::text::hint(h, t))),
        )
}


/// What a textarea's resize grip drags (`textarea { resize: vertical }`):
/// listen with `on_drag_move::<GripDrag>` on the box and set its height.
#[derive(Debug, Clone, Copy)]
pub struct GripDrag;

/// Nothing follows the pointer while the grip drags.
struct NoGhost;

impl gpui::Render for NoGhost {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The grip a browser draws in a resizable textarea's bottom-right corner:
/// two short diagonal strokes. Put it in the (relative) box.
pub fn resize_grip(id: impl Into<ElementId>, color: gpui::Hsla) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .right(px(1.))
        .bottom(px(1.))
        .size(px(11.))
        .cursor_row_resize()
        .on_drag(GripDrag, |_, _, _, cx| cx.new(|_| NoGhost))
        .child(
            canvas(
                |_, _, _| {},
                move |b, _, window, _| {
                    let (r, btm) = (b.right() - px(2.), b.bottom() - px(2.));
                    for len in [8., 4.] {
                        let mut p = PathBuilder::stroke(px(1.));
                        p.move_to(point(r, btm - px(len)));
                        p.line_to(point(r - px(len), btm));
                        if let Ok(path) = p.build() {
                            window.paint_path(path, color);
                        }
                    }
                },
            )
            .size_full(),
        )
}
