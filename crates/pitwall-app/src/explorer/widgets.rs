//! The explorer's own marks, drawn with the kit: the change letter
//! (`.status-letter`) and the refresh control (`Freshness.tsx`, `.fresh`).

use gpui::{
    div, prelude::*, px, AnyElement, App, ClickEvent, ElementId, FontWeight, Hsla, SharedString,
    Window,
};

use pitwall_proto::FileStatus;

use super::logic::{ago, status_letter, status_title};
use crate::kit::{tooltip_view, MONO_FONT};
use crate::theme::Theme;

/// A change letter's colour (`.status-letter`): only where it means
/// something (A/U green, D red).
pub fn status_color(t: &Theme, s: FileStatus) -> Hsla {
    match s {
        FileStatus::A | FileStatus::U => t.green,
        FileStatus::D => t.red,
        FileStatus::M | FileStatus::R => t.text_2,
    }
}

/// `.status-letter` with its tooltip ("Modified").
pub fn status_mark(t: &Theme, id: impl Into<ElementId>, s: FileStatus) -> AnyElement {
    let title = SharedString::from(status_title(s));
    div()
        .id(id)
        .w(px(10.))
        .flex_none()
        .flex()
        .justify_center()
        .font_family(MONO_FONT)
        .text_size(px(11.))
        .font_weight(FontWeight::BOLD)
        .text_color(status_color(t, s))
        .child(status_letter(s))
        .tooltip(move |_, cx| tooltip_view(title.clone(), cx))
        .into_any_element()
}

/// The refresh control (`.fresh`): "updated … ago" (or "refreshing…") and ↻.
pub fn refresh_control(
    t: &Theme,
    id: impl Into<SharedString>,
    refreshing: bool,
    updated_at: Option<u64>,
    now_ms: u64,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let label: SharedString = if refreshing {
        "refreshing…".into()
    } else if let Some(at) = updated_at {
        format!("updated {}", ago(now_ms, at)).into()
    } else {
        "".into()
    };
    crate::kit::refresh_control(id, refreshing, Some(label), t, on_click)
        .into_any_element()
}
