//! The sidebar's "Shared" group (collaboration/ui.md §11, owner side):
//! after the local projects, before agents elsewhere. Each shared agent is
//! a heading with its capability chip, running / cap (`1/2`) and its draft
//! or paused state; under it one row per conversation, named by caller
//! (`bob`, `bob · client-fix`, `erin · Claude Code (MCP)`, `guest`), with
//! its flag, last message and use today. Needs-you and held results flag
//! amber. The heading opens the Share dialog; its gauge opens Usage.

use gpui::{div, prelude::*, px, AnyElement, App, Context, IntoElement, SharedString, Window};

use pitwall_proto::api::collab::{capability_chip, ShareView};
use pitwall_proto::collab::{SessionState, ShareState, SharedSessionView};

use crate::kit::{chevron, chip, chip_tone, icon_btn, label_fit, tooltip, HoverText as _, Tone, MONO_FONT};
use crate::main_screen::MainScreen;
use crate::theme::{Theme, RADIUS};

/// A caller as a row's name.
pub fn caller_name(s: &SharedSessionView) -> String {
    let c = &s.caller;
    if c.guest {
        return "guest".into();
    }
    let who = super::short(&c.login).to_string();
    match (&c.agent, &c.client) {
        (Some(agent), _) => format!("{who} · {}", agent.split('@').next().unwrap_or(agent)),
        (None, Some(client)) => format!("{who} · {client} ({})", if c.front == pitwall_proto::collab::Front::Mcp { "MCP" } else { "A2A" }),
        (None, None) => who,
    }
}

/// The session needs the owner: blocked, or results held for Send.
pub fn needs_you(s: &SharedSessionView) -> bool {
    s.state == SessionState::Blocked || s.held > 0
}

/// `2026-10-16T09:12:00Z` as "5 m", "2 h", "3 d" ago.
pub fn ago(at: &str) -> String {
    let Ok(t) = chrono::DateTime::parse_from_rfc3339(at) else { return String::new() };
    let secs = (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds().max(0);
    match secs {
        s if s < 60 => "now".into(),
        s if s < 3600 => format!("{} m", s / 60),
        s if s < 86_400 => format!("{} h", s / 3600),
        s => format!("{} d", s / 86_400),
    }
}

fn state_dot(s: &SharedSessionView, t: &Theme) -> gpui::Div {
    let color = match s.state {
        _ if needs_you(s) => t.amber,
        SessionState::Working | SessionState::Starting => t.green,
        SessionState::Ready | SessionState::New => t.text_3,
        SessionState::Stopped | SessionState::Closed => t.text_4,
        SessionState::Blocked => t.amber,
    };
    div().flex_none().size(px(7.)).rounded_full().bg(color)
}

/// The group's rows (nothing while nothing is shared).
pub fn render(screen: &mut MainScreen, t: &Theme, _window: &mut Window, cx: &mut Context<MainScreen>) -> Vec<AnyElement> {
    let Some(m) = super::model(cx) else { return vec![] };
    let (data, collapsed) = {
        let s = m.read(cx);
        (s.data.clone(), s.collapsed.clone())
    };
    if data.shares.is_empty() {
        return vec![];
    }
    let _ = screen;
    let mut out = vec![group_head(data.shares.len(), collapsed.contains(GROUP), t).into_any_element()];
    if collapsed.contains(GROUP) {
        return out;
    }
    for (i, v) in data.shares.iter().enumerate() {
        let folded = collapsed.contains(&v.settings.slug);
        let sessions = data.sessions.get(&v.settings.slug).cloned().unwrap_or_default();
        out.push(share_head(i, v, &sessions, folded, t).into_any_element());
        if folded {
            continue;
        }
        for (j, s) in sessions.iter().enumerate() {
            out.push(session_row(i, j, s, t, cx).into_any_element());
        }
    }
    out
}

/// Fold key of the whole group.
const GROUP: &str = "\u{0}shared";

fn toggle(key: String, cx: &mut App) {
    if let Some(m) = super::model(cx) {
        m.update(cx, |s, cx| {
            if !s.collapsed.remove(&key) {
                s.collapsed.insert(key);
            }
            cx.notify();
        });
    }
}

fn group_head(n: usize, collapsed: bool, t: &Theme) -> impl IntoElement {
    div()
        .id("shared-head")
        .group("shared-head")
        .flex()
        .items_center()
        .pl(px(8.))
        .pr(px(6.))
        .pt(px(6.))
        .pb(px(2.))
        .child(
            div()
                .id("shared-toggle")
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(4.))
                .py(px(3.))
                .rounded(px(4.))
                .cursor_pointer()
                .tooltip(tooltip("Your agents shared with people and teams"))
                .on_click(|_, _, cx| toggle(GROUP.into(), cx))
                .child(chevron("chev", !collapsed, 12., t.text_4))
                .child(label_fit("Shared", t.text_4, 12., 22).group_hover_text("shared-head", t.text_2, |s| s))
                .child(div().flex_none().font_family(MONO_FONT).text_size(px(11.)).text_color(t.text_3).child(n.to_string())),
        )
}

fn share_head(i: usize, v: &ShareView, sessions: &[SharedSessionView], folded: bool, t: &Theme) -> impl IntoElement {
    let s = &v.settings;
    let slug = s.slug.clone();
    let (agent, agent2) = (s.agent.clone(), s.agent.clone());
    let hover = t.surface_2;
    let group = SharedString::from(format!("share-head-{i}"));
    let needs = sessions.iter().filter(|x| needs_you(x)).count();
    div()
        .id(("share-head", i))
        .group(group.clone())
        .mx(px(6.))
        .flex()
        .items_center()
        .gap(px(6.))
        .pl(px(4.))
        .pr(px(4.))
        .py(px(4.))
        .rounded(RADIUS)
        .text_color(t.text_2)
        .hover_probed(move |s| s.bg(hover))
        .cursor_pointer()
        .tooltip(tooltip(format!("{}\n{}", v.address, v.summary.text)))
        .on_click(move |_, window, cx| super::share_dialog::open(&agent, window, cx))
        .child(
            div()
                .id(("share-fold", i))
                .flex_none()
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    toggle(slug.clone(), cx)
                })
                .child(chevron(("share-chev", i), !folded, 11., t.text_4)),
        )
        // (Not on the row: its hover probe names it.)
        .child(div().debug_selector(move || format!("share-head-{i}")).flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_size(px(12.5)).child(s.name.clone()))
        .child(chip(capability_chip(s.capability), t))
        .when(s.state == ShareState::Draft, |d| d.child(chip_tone("draft", Tone::Subtle, t)))
        .when(s.state == ShareState::Paused, |d| d.child(chip_tone("paused", Tone::Warn, t)))
        .when(needs > 0, |d| d.child(chip_tone(format!("{needs} needs you"), Tone::Warn, t)))
        .when(s.state == ShareState::Live, |d| {
            d.child(
                div()
                    .flex_none()
                    .font_family(MONO_FONT)
                    .text_size(px(11.))
                    .text_color(t.text_3)
                    .child(format!("{}/{}", v.usage.running, s.limits.sessions_running)),
            )
        })
        .child(
            icon_btn(format!("share-usage-{i}"), "grid", "Usage…", true, t).on_click(move |_, window, cx| {
                cx.stop_propagation();
                super::share_dialog::open_usage(&agent2, window, cx)
            }),
        )
}

fn session_row(i: usize, j: usize, s: &SharedSessionView, t: &Theme, cx: &mut Context<MainScreen>) -> impl IntoElement {
    let hover = t.surface_2;
    let id = s.agent_id.clone();
    let mut meta = Vec::new();
    if let Some(at) = s.last_at.as_deref() {
        meta.push(ago(at));
    }
    meta.push(format!("{} msg", s.usage.messages_today));
    if s.held > 0 {
        meta.push(format!("{} held", s.held));
    }
    let tip = format!(
        "{} — {}\n{} turns, {} min today{}",
        s.caller.login,
        serde_json::to_value(s.state).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
        s.turns,
        s.usage.minutes_today,
        if needs_you(s) { "\nNeeds you" } else { "" }
    );
    div()
        .id(SharedString::from(format!("shared-row-{i}-{j}")))
        .mx(px(6.))
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(24.))
        .pr(px(8.))
        .py(px(4.))
        .rounded(RADIUS)
        .text_size(px(12.))
        .text_color(if needs_you(s) { t.amber } else { t.text_2 })
        .hover_probed(move |st| st.bg(hover))
        .cursor_pointer()
        .tooltip(tooltip(tip))
        .on_click(cx.listener(move |this, _, window, cx| this.show_agent(&id, window, cx)))
        .child(state_dot(s, t))
        .child(div().debug_selector(move || format!("shared-row-{i}-{j}")).flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(caller_name(s)))
        .child(div().flex_none().font_family(MONO_FONT).text_size(px(10.5)).text_color(t.text_4).child(meta.join(" · ")))
}
