//! Next up, in a popover on the agent's tile (docs/spec/gpui inventory
//! §10): its tile header shows "2 queued" while something waits, and the
//! chip (or the palette's "Show Next up for <agent>") opens this.
//!
//! The queue count, the Auto-send toggle, the numbered queue with "Send
//! now" (only while the agent runs) and "Remove from queue", and one Last
//! sent line. Nothing is typed here: prompts are queued from the palette
//! ("name: prompt"), Review, `pitwall queue` or an extension. The engine
//! sends the next item by itself when Auto-send is on and the agent is
//! free.
//!
//! An item an extension put there (`agents.queue`) is drawn apart: a
//! tinted border, "From <name>" with its icon, and, unless the user lets
//! that extension send without asking, "Waits for you: press Send"
//! (Auto-send skips it). The user sends or removes it like any other.
//!
//! The popover is built from values ([`Shown`], taken when the screen
//! draws): under Liquid Glass it is a pop-up window of its own
//! (`kit::pop`), which must not touch the screen while it draws. Its
//! buttons reach the engine, then the agent store, and the screen only to
//! say a call failed.

mod last_sent;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use gpui::{div, prelude::*, px, AnyElement, App, FontWeight, IntoElement, SharedString, Stateful, Div, WeakEntity};

use pitwall_core::Shared;
use pitwall_proto::AgentView;

use crate::kit::{icon_btn_state, label, toggle, tooltip, HoverText as _, MONO_FONT};
use crate::main_screen::{EngineHandle, MainScreen};
use crate::theme::{Theme, RADIUS};

pub use last_sent::{absolute_time, rel_time};

/// The popover's width.
pub const WIDTH: f32 = 340.;

/// "2 queued": the tile header's chip, `None` with nothing queued.
pub fn chip_label(a: &AgentView) -> Option<String> {
    (!a.queue.is_empty()).then(|| format!("{} queued", a.queue.len()))
}

/// What a queue call does.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Remove(String),
    SendNow(String),
    AutoSend(bool),
}

impl Op {
    /// The words after "Couldn't" (React `run(…, what)`).
    pub fn what(&self) -> &'static str {
        match self {
            Op::Remove(_) => "remove queue item",
            Op::SendNow(_) => "send prompt",
            Op::AutoSend(_) => "change auto-send",
        }
    }

    fn run(&self, e: &Shared, agent: &str) -> Result<AgentView, String> {
        match self {
            Op::Remove(item) => e.queue_remove(agent, item),
            Op::SendNow(item) => pitwall_core::engine::input::queue_send_now(e, agent, item),
            Op::AutoSend(on) => e.set_auto_send(agent, *on),
        }
    }
}

/// Run `op` on the agent's queue; the agent it returns goes into the
/// store, and a failure is the screen's toast ("Couldn't …").
pub fn run(agent: String, op: Op, screen: WeakEntity<MainScreen>, cx: &mut App) {
    let Some(engine) = cx.try_global::<EngineHandle>().map(|e| e.0.clone()) else {
        return;
    };
    let what = op.what();
    let task = cx.background_executor().spawn(async move { op.run(&engine, &agent) });
    cx.spawn(async move |cx| match task.await {
        Ok(view) => cx.update(|cx| crate::agents::patch(view, cx)),
        Err(err) => {
            let _ = screen.update(cx, |s, cx| s.failed(what, err, cx));
        }
    })
    .detach();
}

/// What the popover shows, as plain values.
#[derive(Clone)]
pub struct Shown {
    pub agent: AgentView,
    /// The extensions whose items Auto-send sends like the user's own.
    pub senders: Vec<String>,
    /// Each queueing extension's icon.
    pub icons: HashMap<String, String>,
    /// Now, for "4m ago".
    pub now: u64,
}

impl Shown {
    pub fn of(a: &AgentView, cx: &App) -> Shown {
        let icons = a
            .queue
            .iter()
            .filter_map(|q| q.origin.as_ref())
            .map(|o| {
                let icon = crate::extensions::manifest_of(&o.extension, cx)
                    .map(|m| crate::extensions::icon_of(&m).to_string())
                    .unwrap_or_else(|| crate::extensions::DEFAULT_ICON.into());
                (o.extension.clone(), icon)
            })
            .collect();
        Shown { agent: a.clone(), senders: crate::extensions::calls::auto_senders(cx), icons, now: last_sent::now_ms() }
    }
}

/// The popover's content (the caller gives it its surface).
pub fn body(s: &Shown, t: &Theme, screen: &WeakEntity<MainScreen>) -> Stateful<Div> {
    let a = &s.agent;
    div()
        .id("next-up")
        .debug_selector(|| "next-up".into())
        .w(px(WIDTH))
        .flex()
        .flex_col()
        .font_family(crate::kit::UI_FONT)
        .font_weight(FontWeight::NORMAL)
        .child(head(a, t, screen))
        .child(if a.queue.is_empty() {
            div()
                .px(px(12.))
                .pb(px(10.))
                .text_size(px(12.))
                .line_height(px(17.))
                .text_color(t.text_3)
                .child("Nothing queued. Type “name: prompt” in the palette (⌘K) to add one.")
                .into_any_element()
        } else {
            list(s, t, screen)
        })
        .children(last_sent::line(a, s.now, t))
}

fn head(a: &AgentView, t: &Theme, screen: &WeakEntity<MainScreen>) -> impl IntoElement {
    let on = a.auto_send;
    let (id, screen) = (a.id.clone(), screen.clone());
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .pt(px(10.))
        .px(px(12.))
        .pb(px(8.))
        .line_height(px(17.))
        .child(label("Next up", t.text_2, 12.).font_weight(FontWeight::MEDIUM))
        .when(!a.queue.is_empty(), |d| {
            d.child(div().font_family(MONO_FONT).text_size(px(11.)).text_color(t.text_3).child(a.queue.len().to_string()))
        })
        .child(div().flex_1())
        .child(
            div()
                .id("auto-send")
                .debug_selector(|| "next-up-auto-send".into())
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(t.text_2)
                .tooltip(tooltip("Send the next item when the agent is idle or done and you're not typing"))
                .on_click(move |_, _, cx| run(id.clone(), Op::AutoSend(!on), screen.clone(), cx))
                .child(toggle("auto-send-track", on, t))
                .child("Auto-send"),
        )
}

fn list(s: &Shown, t: &Theme, screen: &WeakEntity<MainScreen>) -> AnyElement {
    let a = &s.agent;
    let items = a.queue.iter().enumerate().map(|(i, q)| {
        let group = SharedString::from(format!("queue-item-{i}"));
        let (send_id, rm_id) = (q.id.clone(), q.id.clone());
        let (agent_send, agent_rm) = (a.id.clone(), a.id.clone());
        let (screen_send, screen_rm) = (screen.clone(), screen.clone());
        let running = a.running;
        // An extension's item: whose, and whether it waits for Send.
        let from = q.origin.as_ref().map(|o| {
            let waits = !s.senders.contains(&o.extension);
            let icon = s.icons.get(&o.extension).cloned().unwrap_or_default();
            let sel = format!("queue-from-{i}");
            div()
                .debug_selector(move || sel)
                .flex()
                .items_center()
                .gap(px(5.))
                .mb(px(3.))
                .min_w_0()
                .text_size(px(11.))
                .line_height(px(14.))
                .child(crate::kit::icon(&icon, 11., t.focus))
                .child(div().flex_none().text_color(t.focus).font_weight(FontWeight::MEDIUM).child(format!("From {}", o.name)))
                .child(
                    crate::kit::Ellipsis::ellipsis(div().min_w_0())
                        .text_color(if waits { t.text_3 } else { t.amber })
                        .child(crate::kit::one_line(if waits { "· waits for you: press Send" } else { "· sends without asking" })),
                )
        });
        let ext = q.origin.is_some();
        div()
            .id(("queue-item", i))
            .debug_selector(move || format!("queue-item-{i}"))
            .group(group.clone())
            .flex()
            .items_start()
            .gap(px(8.))
            .py(px(7.))
            .pl(px(8.))
            .pr(px(6.))
            .rounded(RADIUS)
            .bg(t.surface_2)
            .when(ext, |d| d.border_1().border_color(t.focus.opacity(0.45)).bg(t.focus.opacity(0.06)))
            .child(
                div()
                    .mt(px(1.))
                    .font_family(MONO_FONT)
                    .text_size(px(11.))
                    .line_height(px(15.))
                    .text_color(t.text_4)
                    .child((i + 1).to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .children(from)
                    .child(div().text_size(px(12.5)).line_height(px(18.)).text_color(t.text).line_clamp(4).child(q.text.clone())),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(px(1.))
                    .opacity(0.6)
                    .group_hover_probed(group, |s| s.opacity(1.))
                    .when(running, |d| {
                        d.child(
                            icon_btn_state(format!("queue-send-{i}"), "send", "Send now", true, 14., None, t)
                                .debug_selector(move || format!("queue-send-{i}"))
                                .on_click(
                                move |_, _, cx| run(agent_send.clone(), Op::SendNow(send_id.clone()), screen_send.clone(), cx),
                            ),
                        )
                    })
                    .child(
                        icon_btn_state(format!("queue-rm-{i}"), "x", "Remove from queue", true, 14., None, t)
                            .on_click(move |_, _, cx| run(agent_rm.clone(), Op::Remove(rm_id.clone()), screen_rm.clone(), cx)),
                    ),
            )
    });
    div()
        .id("next-up-list")
        .max_h(px(320.))
        .overflow_y_scroll()
        .mx(px(8.))
        .mb(px(8.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .children(items)
        .into_any_element()
}
