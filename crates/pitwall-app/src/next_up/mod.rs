//! The right panel's Next up and Last sent sections (React `NextUp.tsx`,
//! `LastSent.tsx`, `panel.css`; docs/spec/gpui inventory §10), registered
//! with the main screen as [`PanelSlot::Top`] and [`PanelSlot::Bottom`].
//!
//! Next up: the queue count, the Auto-send toggle, the numbered queue with
//! "Send now" (only while the agent runs) and "Remove from queue", and the
//! compose box ("queue a prompt…", ⌘↵ / Ctrl+↵ or Queue). Text is queued
//! verbatim and put back when the call fails; unsent drafts are kept per
//! agent for the window's session. The engine sends the next item by
//! itself when Auto-send is on and the agent is free.
//!
//! One view of each per window, following the window's focused agent.

mod last_sent;
#[cfg(test)]
mod tests;

use crate::kit::HoverText as _;
use std::collections::HashMap;

use gpui::{
    div, prelude::*, px, AnyView, App, AppContext, Context, Entity, FontWeight, Global,
    IntoElement, SharedString, Subscription, Window, WindowId,
};

use pitwall_core::Shared;
use pitwall_proto::AgentView;

use crate::code_view::input::{TextField, TextFieldEvent};
use crate::kit::{
    button, hint, icon_btn_state, kbd, label, resize_grip, toggle, tooltip, BtnKind, GripDrag,
    MONO_FONT,
};
use crate::main_screen::{register_panel_section, EngineHandle, MainScreen, PanelSlot};
use crate::theme::{self, Theme, RADIUS};

pub use last_sent::{absolute_time, rel_time, LastSent};

/// The compose box's text: 12.5 px at line height 1.45 (`.queue-compose
/// textarea`).
const COMPOSE_SIZE: f32 = 12.5;
const COMPOSE_LINE: f32 = 18.125;

/// The views of every window, so each keeps its drafts and focus.
#[derive(Default)]
struct Views {
    next_up: HashMap<WindowId, Entity<NextUp>>,
    last_sent: HashMap<WindowId, Entity<LastSent>>,
}

impl Global for Views {}

/// Put Next up above Changes and Last sent below it (call once at start).
pub fn register(cx: &mut App) {
    register_panel_section(cx, PanelSlot::Top, |a, window, cx| {
        let id = window.window_handle().window_id();
        prune(cx);
        let view = match cx.default_global::<Views>().next_up.get(&id).cloned() {
            Some(v) => v,
            None => {
                let v = cx.new(|cx| NextUp::new(window, cx));
                cx.default_global::<Views>().next_up.insert(id, v.clone());
                v
            }
        };
        view.update(cx, |v, cx| v.set_agent(a, cx));
        AnyView::from(view)
    });
    register_panel_section(cx, PanelSlot::Bottom, |a, window, cx| {
        let id = window.window_handle().window_id();
        let view = match cx.default_global::<Views>().last_sent.get(&id).cloned() {
            Some(v) => v,
            None => {
                let v = cx.new(LastSent::new);
                cx.default_global::<Views>().last_sent.insert(id, v.clone());
                v
            }
        };
        view.update(cx, |v, cx| v.set_agent(a, cx));
        AnyView::from(view)
    });
}

/// Forget the views of closed windows.
fn prune(cx: &mut App) {
    let open: Vec<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
    let views = cx.default_global::<Views>();
    views.next_up.retain(|id, _| open.contains(id));
    views.last_sent.retain(|id, _| open.contains(id));
}

/// Tell the window's main screen a call failed ("Couldn't <what>" toast).
fn report(what: &str, err: String, window: &mut Window, cx: &mut App) {
    let screen = window
        .root::<crate::ui::MainView>()
        .flatten()
        .and_then(|v| v.read(cx).screen.clone())
        .or_else(|| window.root::<MainScreen>().flatten());
    match screen {
        Some(s) => s.update(cx, |s, cx| s.failed(what, err, cx)),
        None => eprintln!("pitwall: couldn't {what}: {err}"),
    }
}

/// The ⌘↵ label as this desktop types it (`Kbd submit`: Ctrl+↵ elsewhere).
pub fn submit_keys(mac: bool) -> &'static str {
    if mac {
        "⌘↵"
    } else {
        "Ctrl+↵"
    }
}

/// What a queue call does.
#[derive(Debug, Clone, PartialEq)]
enum Op {
    Add(String),
    Remove(String),
    SendNow(String),
    AutoSend(bool),
}

impl Op {
    /// The words after "Couldn't" (React `run(…, what)`).
    fn what(&self) -> &'static str {
        match self {
            Op::Add(_) => "queue prompt",
            Op::Remove(_) => "remove queue item",
            Op::SendNow(_) => "send prompt",
            Op::AutoSend(_) => "change auto-send",
        }
    }

    fn run(&self, e: &Shared, agent: &str) -> Result<AgentView, String> {
        match self {
            Op::Add(text) => e.queue_add(agent, text.clone()),
            Op::Remove(item) => e.queue_remove(agent, item),
            Op::SendNow(item) => pitwall_core::engine::input::queue_send_now(e, agent, item),
            Op::AutoSend(on) => e.set_auto_send(agent, *on),
        }
    }
}

pub struct NextUp {
    agent: Option<AgentView>,
    /// Unsent text per agent (kept while the window is open).
    drafts: HashMap<String, String>,
    field: Entity<TextField>,
    /// The compose box's height once its grip was dragged (min 64 px).
    height: Option<f32>,
    _sub: Subscription,
}

impl NextUp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> NextUp {
        let field = cx.new(|cx| TextField::multi("queue a prompt…", 3, cx));
        let sub = cx.subscribe_in(
            &field,
            window,
            |this, field, e: &TextFieldEvent, window, cx| match e {
                TextFieldEvent::Changed => {
                    if let Some(a) = &this.agent {
                        let text = field.read(cx).text().to_string();
                        this.drafts.insert(a.id.clone(), text);
                    }
                    cx.notify();
                }
                TextFieldEvent::Submit => this.add(window, cx),
                TextFieldEvent::Enter { .. } => {}
            },
        );
        NextUp {
            agent: None,
            drafts: HashMap::new(),
            field,
            height: None,
            _sub: sub,
        }
    }

    /// Follow the panel's agent; its draft comes back into the box.
    fn set_agent(&mut self, a: &AgentView, cx: &mut Context<Self>) {
        let switched = self.agent.as_ref().map(|x| x.id.as_str()) != Some(a.id.as_str());
        if !switched && self.agent.as_ref() == Some(a) {
            return;
        }
        if switched {
            // Keep what the box holds for the agent it showed.
            if let Some(old) = &self.agent {
                self.drafts.insert(old.id.clone(), self.draft(cx));
            }
        }
        self.agent = Some(a.clone());
        if switched {
            let draft = self.drafts.get(&a.id).cloned().unwrap_or_default();
            self.field.update(cx, |f, cx| f.set_text(draft, cx));
        }
        cx.notify();
    }

    fn draft(&self, cx: &App) -> String {
        self.field.read(cx).text().to_string()
    }

    /// Queue the box's text, verbatim; it comes back if the call fails.
    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.draft(cx);
        if text.trim().is_empty() {
            return;
        }
        let Some(id) = self.agent.as_ref().map(|a| a.id.clone()) else {
            return;
        };
        // `set_text` sends no Changed, so the saved draft is cleared here too.
        self.drafts.insert(id.clone(), String::new());
        self.field.update(cx, |f, cx| f.set_text("", cx));
        self.run(id, Op::Add(text), window, cx);
    }

    fn run(&mut self, agent: String, op: Op, window: &mut Window, cx: &mut Context<Self>) {
        let Some(engine) = cx.try_global::<EngineHandle>().map(|e| e.0.clone()) else {
            return;
        };
        let (id, call) = (agent.clone(), op.clone());
        let task = cx
            .background_executor()
            .spawn(async move { call.run(&engine, &id) });
        cx.spawn_in(window, async move |this, cx| {
            let err = match task.await {
                Ok(view) => {
                    let _ = cx.update(|_, cx| crate::agents::patch(view, cx));
                    return;
                }
                Err(err) => err,
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if let Op::Add(text) = &op {
                    this.drafts.insert(agent.clone(), text.clone());
                    if this.agent.as_ref().is_some_and(|a| a.id == agent) {
                        this.field.update(cx, |f, cx| f.set_text(text.clone(), cx));
                    }
                }
                report(op.what(), err, window, cx);
            });
        })
        .detach();
    }

    fn head(&self, a: &AgentView, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let on = a.auto_send;
        let id = a.id.clone();
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .pt(px(12.))
            .px(px(14.))
            .pb(px(6.))
            .line_height(px(17.))
            .child(label("Next up", t.text_3, 12.))
            .when(!a.queue.is_empty(), |d| {
                d.child(
                    div()
                        .font_family(MONO_FONT)
                        .text_size(px(11.))
                        .text_color(t.text_3)
                        .child(a.queue.len().to_string()),
                )
            })
            .child(div().flex_1())
            .child(
                div()
                    .id("auto-send")
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(t.text_2)
                    .tooltip(tooltip(
                        "Send the next item when the agent is idle or done and you're not typing",
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run(id.clone(), Op::AutoSend(!on), window, cx)
                    }))
                    .child(toggle("auto-send-track", on, t))
                    .child("Auto-send"),
            )
    }

    fn queue(&self, a: &AgentView, t: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .mx(px(8.))
            .mb(px(8.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(a.queue.iter().enumerate().map(|(i, q)| {
                let group = SharedString::from(format!("queue-item-{i}"));
                let (send_id, rm_id) = (q.id.clone(), q.id.clone());
                let (agent_send, agent_rm) = (a.id.clone(), a.id.clone());
                let running = a.running;
                div()
                    .id(("queue-item", i))
                    .group(group.clone())
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .pt(px(7.))
                    .pb(px(7.))
                    .pl(px(8.))
                    .pr(px(6.))
                    .rounded(RADIUS)
                    .bg(t.surface_2)
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
                            .text_size(px(12.5))
                            .line_height(px(18.))
                            .text_color(t.text)
                            .line_clamp(4)
                            .child(q.text.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(1.))
                            .opacity(0.45)
                            .group_hover_probed(group, |s| s.opacity(1.))
                            .child(
                                icon_btn_state(
                                    format!("queue-send-{i}"),
                                    "send",
                                    "Send now",
                                    true,
                                    14.,
                                    None,
                                    t,
                                )
                                .when(!running, |d| d.opacity(0.45).cursor_default())
                                .when(running, |d| {
                                    d.on_click(cx.listener(move |this, _, window, cx| {
                                        this.run(
                                            agent_send.clone(),
                                            Op::SendNow(send_id.clone()),
                                            window,
                                            cx,
                                        )
                                    }))
                                }),
                            )
                            .child(
                                icon_btn_state(
                                    format!("queue-rm-{i}"),
                                    "x",
                                    "Remove from queue",
                                    true,
                                    14.,
                                    None,
                                    t,
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.run(
                                            agent_rm.clone(),
                                            Op::Remove(rm_id.clone()),
                                            window,
                                            cx,
                                        )
                                    },
                                )),
                            ),
                    )
            }))
    }

    fn compose(
        &self,
        a: &AgentView,
        t: &Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let empty = self.draft(cx).trim().is_empty();
        let focused = self.field.read(cx).is_focused(window);
        let field = self.field.clone();
        div()
            .mx(px(14.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .id("queue-compose")
                    .relative()
                    .w_full()
                    .min_h(px(64.))
                    .when_some(self.height, |d, h| d.h(px(h)).overflow_hidden())
                    // `resize: vertical`: the corner grip sets the height.
                    .on_drag_move(cx.listener(|this, e: &gpui::DragMoveEvent<GripDrag>, _, cx| {
                        let h = f32::from(e.event.position.y - e.bounds.top()) + 4.;
                        this.height = Some(h.max(64.));
                        cx.notify();
                    }))
                    .px(px(9.))
                    .py(px(7.))
                    .rounded(RADIUS)
                    .bg(t.bg)
                    .border_1()
                    .border_color(if focused { t.text_3 } else { t.line_strong })
                    .text_size(px(COMPOSE_SIZE))
                    .line_height(px(COMPOSE_LINE))
                    .on_click(move |_, window, cx| field.read(cx).focus(window))
                    .child(self.field.clone())
                    .child(resize_grip("queue-grip", t.text_4)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        hint(
                            if a.auto_send {
                                "Sends when the agent is free"
                            } else {
                                "Auto-send off — send items manually"
                            },
                            t,
                        )
                        .flex_1()
                        .min_w_0()
                        .line_height(px(17.)),
                    )
                    .child(
                        button("queue-add", BtnKind::Small, empty, t)
                            .when(!empty, |d| {
                                d.on_click(cx.listener(|this, _, window, cx| this.add(window, cx)))
                            })
                            .child("Queue")
                            .child(kbd(submit_keys(cfg!(target_os = "macos")), t)),
                    ),
            )
    }
}

impl gpui::Render for NextUp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::panel(theme::theme(cx), cx);
        let Some(a) = self.agent.clone() else {
            return div().into_any_element();
        };
        div()
            .flex_none()
            .flex()
            .flex_col()
            .pb(px(12.))
            .border_b_1()
            .border_color(t.line)
            .font_weight(FontWeight::NORMAL)
            .child(self.head(&a, &t, cx))
            .when(!a.queue.is_empty(), |d| d.child(self.queue(&a, &t, cx)))
            .child(self.compose(&a, &t, window, cx))
            .into_any_element()
    }
}
