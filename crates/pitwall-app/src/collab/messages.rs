//! A shared session on the owner's side (collaboration/ui.md §11): when
//! the focused agent is one caller's conversation with a shared agent, the
//! right panel starts with its strip ("Shared with bob@example.com · read
//! only · sandboxed · Take over") and **Messages**: what was typed, the
//! replies (with their source) and results, as a thread. The terminal
//! above is the ordinary pane (watch); typing into it takes over, as for
//! any agent. Nothing is shown for other agents.

use std::collections::HashMap;

use gpui::{div, prelude::*, px, AnyView, App, AppContext, Context, Entity, Global, IntoElement, Window, WindowId};

use pitwall_proto::api::collab::{capability_words, OwnerTurnView};
use pitwall_proto::collab::{AskStatus, ReplySource, SessionState};
use pitwall_proto::{method, AgentView};

use crate::kit::{chip_tone, label_t, Tone};
use crate::main_screen::{register_panel_section, PanelSlot};
use crate::theme::{self, RADIUS};

#[derive(Default)]
struct Views(HashMap<WindowId, Entity<SessionPanel>>);

impl Global for Views {}

pub fn register(cx: &mut App) {
    register_panel_section(cx, PanelSlot::Shared, |a, window, cx| {
        let id = window.window_handle().window_id();
        let open: Vec<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
        cx.default_global::<Views>().0.retain(|w, _| open.contains(w));
        let view = match cx.default_global::<Views>().0.get(&id).cloned() {
            Some(v) => v,
            None => {
                let v = cx.new(SessionPanel::new);
                cx.default_global::<Views>().0.insert(id, v.clone());
                v
            }
        };
        view.update(cx, |v, cx| v.set_agent(a, cx));
        AnyView::from(view)
    });
}

pub struct SessionPanel {
    agent: Option<String>,
    /// The conversation's turns as last read (`share.turns`).
    pub turns: Vec<OwnerTurnView>,
    /// What they were read for: the conversation and how it looked then.
    read_for: Option<(String, u32, Option<String>, SessionState, bool)>,
    _sub: Option<gpui::Subscription>,
}

impl SessionPanel {
    pub(crate) fn new(cx: &mut Context<Self>) -> SessionPanel {
        let m = super::model_or_new(cx);
        SessionPanel {
            agent: None,
            turns: Vec::new(),
            read_for: None,
            _sub: Some(cx.observe(&m, |this, _, cx| {
                this.reload_if_changed(cx);
                cx.notify();
            })),
        }
    }

    /// Read the turns again when the conversation moved on.
    fn reload_if_changed(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.agent.clone() else { return };
        let snap = super::snapshot(cx);
        let Some((share, s)) = snap.session_of(&id) else { return };
        let key = (s.conversation.clone(), s.turns, s.last_at.clone(), s.state, s.flagged);
        if self.read_for.as_ref() == Some(&key) {
            return;
        }
        self.read_for = Some(key);
        let this = cx.entity().downgrade();
        let params = serde_json::json!({"share": share.settings.slug, "conversation": s.conversation});
        super::call::<Vec<OwnerTurnView>>(method::SHARE_TURNS, params, cx, move |r, cx| {
            let _ = this.update(cx, |p, cx| {
                if let Ok(turns) = r {
                    p.turns = turns;
                }
                cx.notify();
            });
        });
    }

    pub(crate) fn set_agent(&mut self, a: &AgentView, cx: &mut Context<Self>) {
        if self.agent.as_deref() != Some(a.id.as_str()) {
            self.agent = Some(a.id.clone());
            self.turns.clear();
            self.read_for = None;
            self.reload_if_changed(cx);
            cx.notify();
        }
    }
}

impl gpui::Render for SessionPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(id) = self.agent.clone() else { return div().into_any_element() };
        let snap = super::snapshot(cx);
        let Some((share, s)) = snap.session_of(&id) else { return div().into_any_element() };
        let t = theme::theme(cx).chrome();
        let who = if s.caller.guest { "you, as a guest".to_string() } else { s.caller.login.clone() };
        let strip = format!("Shared with {who} · {} · sandboxed", capability_words(share.settings.capability));
        let last = s.last_at.as_deref().map(|at| format!(" · last message {} ago", super::sidebar::ago(at))).unwrap_or_default();
        let counts = div().text_size(px(12.)).text_color(t.text_3).child(format!(
            "{} messages, {} results{}{last}",
            s.turns,
            s.results,
            if s.held > 0 { format!(", {} held for your Send", s.held) } else { String::new() }
        ));
        let caller = if s.caller.guest { "you (guest)".to_string() } else { super::short(&s.caller.login).to_string() };
        let mut thread = div().flex().flex_col().gap(px(8.)).child(counts);
        for (i, turn) in self.turns.iter().enumerate() {
            thread = thread.child(turn_card(i, turn, &caller, &t));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(14.))
            .py(px(10.))
            .border_b_1()
            .border_color(t.line)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(RADIUS)
                    .bg(t.surface_2)
                    .text_size(px(12.))
                    .text_color(t.text_2)
                    .child(div().flex_1().child(strip))
                    .when(super::sidebar::needs_you(s), |d| d.child(chip_tone("needs you", Tone::Warn, &t))),
            )
            .when(s.flagged, |d| {
                d.child(
                    div()
                        .px(px(8.))
                        .py(px(5.))
                        .rounded(RADIUS)
                        .bg(t.amber_soft)
                        .text_size(px(12.))
                        .text_color(t.amber)
                        .child("The reply to its last message couldn't be read (no answer): look at the terminal, and answer in it if needed."),
                )
            })
            .child(div().text_size(px(11.5)).text_color(t.text_3).child("Type in the terminal to take over; your lines go to the agent like any input."))
            .child(label_t("MESSAGES", t.text_3, 11., 0.08))
            .child(thread)
            .into_any_element()
    }
}

/// Where a reply was read from, as the thread says it (§7.2).
pub fn source_words(s: ReplySource) -> &'static str {
    match s {
        ReplySource::Hook => "from its hook",
        ReplySource::SessionFile => "from its session file",
        ReplySource::NonInteractive => "from its non-interactive run",
        ReplySource::Terminal => "from terminal output",
    }
}

/// One turn: the caller's message, then the reply (or how the turn ended).
fn turn_card(i: usize, turn: &OwnerTurnView, caller: &str, t: &theme::Theme) -> gpui::Div {
    let who = if turn.internal { "briefing (your approved summary)".to_string() } else { caller.to_string() };
    let ended = match turn.status {
        AskStatus::Done => None,
        AskStatus::Queued | AskStatus::Working | AskStatus::WaitingForOwner => {
            Some(turn.status_message.clone().unwrap_or_else(|| "working".into()))
        }
        _ => Some(turn.status_message.clone().unwrap_or_else(|| format!("{:?}", turn.status).to_lowercase())),
    };
    let amber = matches!(turn.status, AskStatus::Failed | AskStatus::Refused | AskStatus::WaitingForOwner);
    let mut card = div()
        .debug_selector(move || format!("owner-turn-{i}"))
        .flex()
        .flex_col()
        .gap(px(4.))
        .px(px(8.))
        .py(px(6.))
        .rounded(RADIUS)
        .bg(t.surface_2)
        .child(div().text_size(px(11.)).text_color(t.text_4).child(format!("{who} · {}", super::sidebar::ago(&turn.at))))
        .child(div().text_size(px(12.5)).text_color(t.text).child(turn.text.clone()));
    if let Some(reply) = &turn.reply {
        let from = turn.reply_source.map(source_words).unwrap_or("");
        card = card
            .child(div().text_size(px(12.5)).text_color(t.text_2).child(reply.clone()))
            .child(div().text_size(px(10.5)).text_color(t.text_4).child(format!("reply {from}")));
    }
    if let Some(e) = ended {
        card = card.child(div().text_size(px(11.5)).text_color(if amber { t.amber } else { t.text_3 }).child(e));
    }
    card
}
