//! Settings → Extensions, running agents that use an extension's previous
//! setup (docs/extensions/updates.md): what Pitwall does with them after
//! an update (`extensions.restartAfterUpdate`, and per extension on its
//! card), and on the card the agents themselves, with Restart now and
//! Restart all idle.

use gpui::{div, prelude::*, px, AnyElement, Context, Div, SharedString};
use pitwall_core::extensions::update::Restart;
use pitwall_core::extensions::Manifest;

use super::super::view::SettingsView;
use super::super::widgets::Ui;
use crate::extensions::stale;
use crate::kit::BtnKind;

fn btn(id: String, text: impl Into<SharedString>, kind: BtnKind, ui: &Ui) -> gpui::Stateful<Div> {
    let sel = id.clone();
    crate::kit::text_button(SharedString::from(id), text, kind, false, &ui.t).debug_selector(move || sel)
}

impl SettingsView {
    /// The global choice, on the Extensions page.
    pub(super) fn restart_setting(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Div {
        let now = stale::global_mode(cx);
        let mut row = div().flex().flex_wrap().items_center().gap(px(6.)).child(div().text_size(px(12.)).text_color(ui.t.text_2).child("After an update, restart agents that use the old setup:")).child(div().flex_1());
        for r in Restart::ALL {
            let kind = if r == now { BtnKind::Small } else { BtnKind::GhostSm };
            row = row.child(btn(format!("extensions-restart-{}", r.id()), r.label(), kind, ui).tooltip(crate::kit::tooltip(r.words())).on_click(cx.listener(move |_, _, _, cx| {
                cx.defer(move |cx| stale::set_global_mode(r, cx));
                cx.notify();
            })));
        }
        div().flex().flex_col().gap(px(4.)).child(row).child(crate::kit::hint(now.words(), &ui.t))
    }

    /// On `m`'s card (an extension whose setup or variables reach agents):
    /// its own choice over the global one, and the running agents that use
    /// its previous setup.
    pub(super) fn stale_notes(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if m.agent_env.is_empty() && !m.actions.iter().any(|a| a.setup) {
            return out;
        }
        let own = crate::ui_state::get(cx).extension(&m.id).restart_after_update.as_deref().and_then(Restart::parse);
        let global = stale::global_mode(cx);
        let list = stale::stale_agents(&m.id, cx);
        if !list.is_empty() {
            let n = list.len();
            let head = if n == 1 { "1 running agent uses the previous setup".to_string() } else { format!("{n} running agents use the previous setup") };
            let sel = format!("extension-stale-{}", m.id);
            let warn = m.update.agents == pitwall_core::extensions::update::Agents::Restart;
            let sub = if warn { format!("They don't work with {} until restarted.", m.name) } else { "They keep working with the version they started with until restarted.".to_string() };
            let mut col = div()
                .debug_selector(move || sel)
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(ui.t.surface_2)
                .child(div().text_size(px(13.)).text_color(if warn { ui.t.amber } else { ui.t.text }).child(head))
                .child(crate::kit::hint(sub, &ui.t));
            for a in &list {
                let status = crate::agents::status_word(a.status);
                let note = if stale::resumable(a) { "" } else { " · starts a new conversation" };
                let (aid, ext) = (a.id.clone(), m.id.clone());
                col = col.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().flex_1().min_w_0().text_size(px(12.)).text_color(ui.t.text_2).child(format!("{} — {status}{note}", a.name)))
                        .child(btn(format!("extension-stale-restart-{}-{}", m.id, a.id), "Restart now", BtnKind::GhostSm, ui).tooltip(crate::kit::tooltip("Restart it now, continuing its conversation where it can")).on_click(cx.listener(move |_, _, _, cx| {
                            let (aid, ext) = (aid.clone(), ext.clone());
                            cx.defer(move |cx| stale::restart_now(&aid, vec![ext], cx));
                        }))),
                );
            }
            let ext = m.id.clone();
            col = col.child(div().flex().child(div().flex_1()).child(btn(format!("extension-stale-idle-{}", m.id), "Restart all idle", BtnKind::Small, ui).tooltip(crate::kit::tooltip("Restart each one that is waiting for you now, one at a time")).on_click(cx.listener(move |_, _, _, cx| {
                let ext = ext.clone();
                cx.defer(move |cx| {
                    stale::restart_idle(&ext, cx);
                });
            }))));
            out.push(col.into_any_element());
        }
        let mut row = div().flex().flex_wrap().items_center().gap(px(6.)).child(div().text_size(px(12.)).text_color(ui.t.text_3).child("After its updates, restart agents that use the old setup:")).child(div().flex_1());
        let choices: [(Option<Restart>, String); 4] = [
            (None, format!("As set ({})", global.label())),
            (Some(Restart::Ask), Restart::Ask.label().into()),
            (Some(Restart::WhenIdle), Restart::WhenIdle.label().into()),
            (Some(Restart::Never), Restart::Never.label().into()),
        ];
        for (r, label) in choices {
            let kind = if r == own { BtnKind::Small } else { BtnKind::GhostSm };
            let ext = m.id.clone();
            let id = format!("extension-restart-{}-{}", m.id, r.map_or("default", Restart::id));
            row = row.child(btn(id, label, kind, ui).on_click(cx.listener(move |_, _, _, cx| {
                let ext = ext.clone();
                cx.defer(move |cx| stale::set_mode(&ext, r, cx));
                cx.notify();
            })));
        }
        out.push(row.into_any_element());
        out
    }
}
