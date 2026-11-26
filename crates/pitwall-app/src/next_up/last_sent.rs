//! Last sent, as one line at the foot of the Next up popover: the last
//! prompt sent through Pitwall, how long ago (the absolute time in its
//! tooltip), and who queued it when an extension did.

use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{div, prelude::*, px, AnyElement, FontWeight, IntoElement};

use pitwall_proto::AgentView;

use crate::kit::{tooltip, Ellipsis as _, MONO_FONT};
use crate::theme::Theme;

/// Compact relative time (`relTime`): "just now", "42s ago", "4m ago",
/// "2h ago", "3d ago".
pub fn rel_time(ms: u64, now: u64) -> String {
    let s = (now.saturating_sub(ms) as f64 / 1000.).round() as u64;
    if s < 10 {
        return "just now".into();
    }
    if s < 60 {
        return format!("{s}s ago");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}m ago");
    }
    let h = m / 60;
    if h < 24 {
        return format!("{h}h ago");
    }
    format!("{}d ago", h / 24)
}

/// The local date and time (`toLocaleString`, en-US): "10/8/2026, 3:04:05 PM".
pub fn absolute_time(ms: u64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms as i64) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => {
            t.format("%-m/%-d/%Y, %-I:%M:%S %p").to_string()
        }
        chrono::LocalResult::None => String::new(),
    }
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// "Last sent · 4m ago · from CI · <the prompt, on one line>", or nothing
/// when nothing was sent through Pitwall yet.
pub(super) fn line(a: &AgentView, now: u64, t: &Theme) -> Option<AnyElement> {
    let text = a.last_sent.as_ref()?;
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(
        div()
            .id("last-sent")
            .debug_selector(|| "next-up-last-sent".into())
            .flex()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .px(px(10.))
            .py(px(7.))
            .border_t_1()
            .border_color(t.line)
            .text_size(px(11.5))
            .line_height(px(15.))
            .text_color(t.text_3)
            .tooltip(tooltip(text.clone()))
            .child(div().flex_none().font_weight(FontWeight::MEDIUM).child("Last sent"))
            .children(a.last_sent_at.map(|at| {
                div()
                    .id("last-sent-at")
                    .flex_none()
                    .font_family(MONO_FONT)
                    .tooltip(tooltip(absolute_time(at)))
                    .child(rel_time(at, now))
            }))
            .children(a.last_sent_by.as_ref().map(|o| {
                div().flex_none().text_color(t.focus).child(format!("from {}", o.name))
            }))
            .child(div().min_w_0().flex_1().ellipsis().text_color(t.text_2).child(crate::kit::one_line(flat)))
            .into_any_element(),
    )
}
