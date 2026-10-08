//! The rules screens' own pieces (`rules.css`), drawn with the kit: the
//! `select.input` trigger at a given height, `.rules-mini` buttons, the
//! `.rules-list` box and its rows, `.check` rows and text with inline
//! `.mono` parts.

use std::rc::Rc;

use gpui::{
    div, font, prelude::*, px, AnyElement, App, ClickEvent, Div, ElementId, FontWeight, Hsla,
    SharedString, Stateful, StyledText, TextRun, Window,
};

use crate::kit::{self, BtnKind, OnClose, OnPick, MONO_FONT, UI_FONT};
use crate::theme::{Theme, RADIUS};

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
        .when(!open, |d| d.hover(move |s| s.border_color(hover)))
        .when(full, |d| d.w_full())
        // A native select is as wide as its longest option.
        .when(!full, |d| d.min_w(px(widest(&items, size))))
        .on_click(move |_, window, cx| on_toggle(window, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .line_clamp(1)
                .text_ellipsis()
                .child(text.into()),
        )
        .child(kit::icon("chevron-down", 11., t.text_3));
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
                    .child(kit::select_menu(
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

/// `.ghost-btn.rules-mini`: 24 px, 12 px type.
pub fn mini(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    disabled: bool,
    t: &Theme,
) -> Stateful<Div> {
    kit::button(id, BtnKind::GhostSm, disabled, t)
        .h(px(24.))
        .px(px(8.))
        .text_size(px(12.))
        .child(text.into())
}

/// `.rules-list`: hairline box on surface-2, at most 220 px tall.
pub fn list(id: impl Into<ElementId>, rows: Vec<AnyElement>, t: &Theme) -> Stateful<Div> {
    let n = rows.len();
    let line = t.line;
    div()
        .id(id)
        .flex()
        .flex_col()
        .max_h(px(220.))
        .overflow_y_scroll()
        .rounded(RADIUS)
        .border_1()
        .border_color(t.line)
        .bg(t.surface_2)
        .children(rows.into_iter().enumerate().map(move |(i, r)| {
            div()
                .flex_none()
                .when(i + 1 < n, |d| d.border_b_1().border_color(line))
                .child(r)
        }))
}

/// `.rules-list li`: 32 px tall, 12.5 px.
pub fn row(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .min_h(px(32.))
        .px(px(10.))
        .py(px(4.))
        .text_size(px(12.5))
}

/// `.rules-name`: 12 px text, in mono unless `ui`.
pub fn name(text: impl Into<SharedString>, mono: bool, t: &Theme) -> Div {
    div()
        .flex_none()
        .text_size(px(12.))
        .text_color(t.text)
        .whitespace_nowrap()
        .when(mono, |d| d.font_family(MONO_FONT))
        .child(text.into())
}

/// `.rules-desc`: the rest of the row, text-3, one line.
pub fn desc(text: impl Into<SharedString>, t: &Theme) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .line_clamp(1)
        .text_ellipsis()
        .text_color(t.text_3)
        .child(text.into())
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
                .child(kit::checkbox_box(on, disabled, 14., t)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(title)
                .when_some(hint, |d, h| d.child(kit::hint(h, t))),
        )
}

/// How a part of [`rich`] text is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Text,
    /// `.mono`
    Mono,
    /// `<b>`
    Bold,
}

/// Text with inline `.mono` and bold parts, wrapping as one paragraph.
pub fn rich(parts: &[(&str, Span)], color: Hsla) -> StyledText {
    let mut text = String::new();
    let mut runs = Vec::new();
    for (s, span) in parts {
        text.push_str(s);
        let mut f = font(if *span == Span::Mono {
            MONO_FONT
        } else {
            UI_FONT
        });
        f.weight = if *span == Span::Bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        };
        runs.push(TextRun {
            len: s.len(),
            font: f,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    StyledText::new(text).with_runs(runs)
}

/// Picks for a select from labels (`on` marks the current one).
pub fn items(labels: impl IntoIterator<Item = (String, bool)>) -> Vec<(SharedString, bool)> {
    labels.into_iter().map(|(l, on)| (l.into(), on)).collect()
}

/// `OnPick` from a closure.
pub fn on_pick(f: impl Fn(usize, &mut Window, &mut App) + 'static) -> OnPick {
    Rc::new(f)
}

/// `OnClose` from a closure.
pub fn on_close(f: impl Fn(&mut Window, &mut App) + 'static) -> OnClose {
    Rc::new(f)
}
