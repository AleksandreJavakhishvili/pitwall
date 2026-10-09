//! A source-control view's freshness (`Freshness.tsx`, `base.css`
//! `.fresh`): "refreshing…" / "updated 12 s ago" and the ↻ button, whose
//! icon turns while a refresh runs (`fresh-spin` 0.9 s; dimmed instead with
//! Reduce motion).

use std::time::Duration;

use gpui::{
    div, prelude::*, px, Animation, AnimationExt, App, ClickEvent, Div, ElementId, SharedString,
    Window,
};

use crate::theme::{Theme, RADIUS};

use super::button::keys;
use super::icons::icon;
use super::tooltip::tooltip;
use super::HoverText;

/// "updated 12 s ago" from an age in seconds (`updatedLabel`).
pub fn updated_label(age_secs: u64) -> String {
    match age_secs {
        0..=4 => "updated just now".into(),
        5..=59 => format!("updated {age_secs} s ago"),
        60..=3599 => format!("updated {} min ago", age_secs / 60),
        _ => format!("updated {} h ago", age_secs / 3600),
    }
}

/// The note: "refreshing…", "updated …", or nothing yet.
pub fn fresh_note(refreshing: bool, age_secs: Option<u64>) -> SharedString {
    if refreshing {
        "refreshing…".into()
    } else {
        age_secs.map(updated_label).unwrap_or_default().into()
    }
}

/// `.fresh`: the note (when `note` is given) and the ↻ `.icon-btn-sm`.
pub fn refresh_control(
    id: impl Into<SharedString>,
    busy: bool,
    note: Option<SharedString>,
    t: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let id: SharedString = id.into();
    let group = SharedString::from(format!("fresh-{id}"));
    let (fg, hover_fg, hover_bg) = (t.text_3, t.text, t.surface_3);
    let moving = super::motion::on();
    let glyph = icon("refresh", 13., fg).group_hover_text(group.clone(), hover_fg, |s| s);
    let glyph = if busy && moving {
        glyph
            .with_animation(
                ElementId::Name(format!("{id}-spin").into()),
                Animation::new(Duration::from_millis(900)).repeat(),
                |svg, d| {
                    svg.with_transformation(gpui::Transformation::rotate(gpui::radians(
                        d * std::f32::consts::TAU,
                    )))
                },
            )
            .into_any_element()
    } else {
        glyph.when(busy, |s| s.opacity(0.6)).into_any_element()
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .min_w_0()
        .when_some(note, |d, n| {
            d.child(
                div()
                    .min_w_0()
                    .text_size(px(11.))
                    .text_color(t.text_3)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(super::text::one_line(n)),
            )
        })
        .child(
            div()
                .id(ElementId::Name(id))
                .group(group)
                .size(px(22.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(RADIUS)
                .cursor_pointer()
                .hover_probed(move |s| s.bg(hover_bg))
                .tooltip(tooltip(format!("Refresh ({})", keys("⌘⇧R", false))))
                .on_click(on_click)
                .child(glyph),
        )
}
