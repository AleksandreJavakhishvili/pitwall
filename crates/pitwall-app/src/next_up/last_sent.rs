//! Last sent (React `LastSent.tsx`): the last prompt sent through Pitwall,
//! quoted, with how long ago (the absolute time in its tooltip), or a hint
//! when nothing was sent yet. The relative time refreshes every 15 s
//! (`useNow`).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{div, prelude::*, px, Context, IntoElement, Task, Window};

use pitwall_proto::AgentView;

use crate::kit::{hint, label, tooltip, MONO_FONT};
use crate::theme::{self, RADIUS};

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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub struct LastSent {
    agent: Option<AgentView>,
    _tick: Task<()>,
}

impl LastSent {
    pub(super) fn new(cx: &mut Context<Self>) -> LastSent {
        let tick = cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_secs(15))
                .await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        });
        LastSent {
            agent: None,
            _tick: tick,
        }
    }

    pub(super) fn set_agent(&mut self, a: &AgentView, cx: &mut Context<Self>) {
        let same = self.agent.as_ref().is_some_and(|x| {
            x.id == a.id && x.last_sent == a.last_sent && x.last_sent_at == a.last_sent_at
        });
        if !same {
            self.agent = Some(a.clone());
            cx.notify();
        }
    }
}

impl Render for LastSent {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::panel(theme::theme(cx), cx);
        let Some(a) = &self.agent else {
            return div().into_any_element();
        };
        let head = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .pt(px(12.))
            .px(px(14.))
            .pb(px(6.))
            .line_height(px(17.))
            .child(label("Last sent", t.text_3, 12.))
            .child(div().flex_1())
            .children(a.last_sent_at.map(|at| {
                div()
                    .id("last-sent-at")
                    .font_family(MONO_FONT)
                    .text_size(px(12.))
                    .text_color(t.text_3)
                    .tooltip(tooltip(absolute_time(at)))
                    .child(rel_time(at, now_ms()))
            }));
        let body = match &a.last_sent {
            Some(text) => {
                let text = text.clone();
                crate::kit::scroll_area("last-sent-area", &t, move |h| {
                    div()
                        .id("last-sent")
                        .track_scroll(h)
                        .max_h(px(180.))
                        .overflow_y_scroll()
                        .py(px(9.))
                        .pr(px(11.))
                        .pl(px(11.))
                        .child(text)
                        .into_any_element()
                })
                .mx(px(14.))
                .max_h(px(180.))
                .rounded(RADIUS)
                .bg(t.surface_2)
                .border_l_2()
                .border_color(t.line_strong)
                .text_size(px(12.5))
                .line_height(px(18.))
                .text_color(t.text)
                .into_any_element()
            }
            None => hint(
                "Nothing sent through Pitwall yet. Prompts you type in the terminal aren't tracked.",
                &t,
            )
            .mx(px(14.))
            .line_height(px(17.))
            .into_any_element(),
        };
        div()
            .flex_none()
            .flex()
            .flex_col()
            .pb(px(12.))
            // Changes above draws no bottom line of its own.
            .border_t_1()
            .border_color(t.line)
            .child(head)
            .child(body)
            .into_any_element()
    }
}
