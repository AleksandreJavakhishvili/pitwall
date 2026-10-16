//! A shared session on the owner's side (collaboration/ui.md §11): when
//! the focused agent is one caller's conversation with a shared agent, the
//! right panel starts with its strip ("Shared with bob@example.com · read
//! only · sandboxed · Take over") and **Messages**: what was typed, the
//! replies (with their source) and results, as a thread. The terminal
//! above is the ordinary pane (watch); typing into it takes over, as for
//! any agent. Nothing is shown for other agents.

use std::collections::HashMap;

use gpui::{div, prelude::*, px, AnyView, App, AppContext, Context, Entity, Global, IntoElement, Window, WindowId};

use pitwall_proto::api::collab::capability_words;
use pitwall_proto::AgentView;

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
    _sub: Option<gpui::Subscription>,
}

impl SessionPanel {
    fn new(cx: &mut Context<Self>) -> SessionPanel {
        let m = super::model_or_new(cx);
        SessionPanel { agent: None, _sub: Some(cx.observe(&m, |_, _, cx| cx.notify())) }
    }

    fn set_agent(&mut self, a: &AgentView, cx: &mut Context<Self>) {
        if self.agent.as_deref() != Some(a.id.as_str()) {
            self.agent = Some(a.id.clone());
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
        // The turns themselves come from the core's conversation log (a
        // seam: `SharedSessionView` carries the counts today).
        let last = s.last_at.as_deref().map(|at| format!(" · last message {} ago", super::sidebar::ago(at))).unwrap_or_default();
        let thread = div().text_size(px(12.)).text_color(t.text_3).child(format!(
            "{} messages, {} results{}{last}",
            s.turns,
            s.results,
            if s.held > 0 { format!(", {} held for your Send", s.held) } else { String::new() }
        ));
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
            .child(div().text_size(px(11.5)).text_color(t.text_3).child("Type in the terminal to take over; your lines go to the agent like any input."))
            .child(label_t("MESSAGES", t.text_3, 11., 0.08))
            .child(thread)
            .into_any_element()
    }
}
