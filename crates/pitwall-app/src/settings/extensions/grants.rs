//! An extension's permissions, activity and typed settings in its card
//! (docs/spec/extension-api.md, "Permissions" and "How the user sees and
//! stops it").
//!
//! - **The permission screen**: turning on an extension that asks for
//!   permissions shows, in its card, what it asks for with its own reason
//!   and what each gives; the user grants or refuses each, and only then
//!   is it on. A permission this Pitwall doesn't serve is shown as not
//!   available and never granted.
//! - **Permissions**, in a card that is on: each with its reason and when
//!   it was granted or refused, a switch to revoke (at once: the next call
//!   is refused) or grant; one an update added waits, not granted, until
//!   the user answers.
//! - **Activity**, next to Log: what it called and what was refused, with
//!   when.
//! - Fields for `text`, `number`, `choice` and `bool` settings.

use std::collections::BTreeMap;
use std::time::Duration;

use gpui::{div, prelude::*, px, AnyElement, Context, Div, Entity, SharedString};
use pitwall_core::accounts::Account;
use pitwall_core::extensions::activity::{Outcome, Via};
use pitwall_core::extensions::permissions::{self, Decision};
use pitwall_core::extensions::{Manifest, SettingDef, SettingKind};

use super::super::view::SettingsView;
use super::super::widgets::Ui;
use crate::extensions::{calls, connections};
use crate::kit::{BtnKind, InputEvent, TextInput, Tone};

/// The most activity entries shown (the newest).
const SHOWN: usize = 200;

/// A button tests can find by its id.
fn btn(id: String, text: impl Into<SharedString>, kind: BtnKind, disabled: bool, ui: &Ui) -> gpui::Stateful<Div> {
    let sel = id.clone();
    crate::kit::text_button(SharedString::from(id), text, kind, disabled, &ui.t).debug_selector(move || sel)
}

/// A switch tests can find by its id.
fn switch(id: String, on: bool, ui: &Ui) -> gpui::Stateful<Div> {
    let sel = id.clone();
    crate::kit::toggle(SharedString::from(id), on, &ui.t).debug_selector(move || sel)
}

/// A date as this computer shows it ("14 Nov 2026").
fn date_of(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0).map(|d| d.with_timezone(&chrono::Local).format("%-d %b %Y").to_string()).unwrap_or_default()
}

/// A time of day and date ("14 Nov 09:41:07").
fn when_of(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| d.with_timezone(&chrono::Local).format("%-d %b %H:%M:%S").to_string()).unwrap_or_default()
}

/// What the permission screen starts with for `m`: each permission it
/// asks for that can be granted, on when it is a read (and granted before,
/// when it was).
pub fn initial_picks(m: &Manifest, kept: &permissions::Decisions) -> BTreeMap<String, bool> {
    m.permissions.iter().filter(|p| permissions::available(&p.name)).map(|p| (p.name.clone(), kept.get(&p.name).map_or(permissions::on_by_default(&p.name), |d| d.granted))).collect()
}

/// Turn extension `id` on with `picks` answered (each with today's date),
/// the decisions of permissions `m` no longer asks for forgotten.
pub fn turn_on_with(m: &Manifest, picks: &BTreeMap<String, bool>, cx: &mut gpui::App) {
    let id = m.id.clone();
    let m = m.clone();
    let picks = picks.clone();
    crate::ui_state::update(cx, |s| {
        *s = s.clone().with_extension(&id, |e| {
            let mut kept = permissions::kept(&m, &e.grants);
            for (name, on) in &picks {
                kept.insert(name.clone(), Decision::now(*on));
            }
            e.grants = kept;
            e.enabled = true;
        })
    });
}

use calls::decide;

impl SettingsView {
    /// The switch was pressed to turn `m` on: with permissions to answer,
    /// the permission screen first; else on at once.
    pub(super) fn ask_to_turn_on(&mut self, m: &Manifest, cx: &mut Context<Self>) {
        let kept = crate::ui_state::get(cx).extension(&m.id).grants;
        if m.permissions.is_empty() {
            crate::ui_state::update(cx, |s| *s = s.clone().set_extension(&m.id, true));
            return;
        }
        self.exts.asking = Some((m.id.clone(), initial_picks(m, &kept)));
        self.exts.open.insert(m.id.clone(), true);
        cx.notify();
    }

    /// Bench and debug builds: `ext`'s permission screen, as its switch
    /// shows it.
    pub fn debug_permissions(&mut self, ext: &str, cx: &mut Context<Self>) {
        let Some(m) = crate::extensions::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == ext).cloned() else { return };
        self.ask_to_turn_on(&m, cx);
    }

    /// Bench and debug builds: Turn on pressed on the permission screen
    /// that is open.
    pub fn debug_permissions_on(&mut self, cx: &mut Context<Self>) {
        let Some((id, picks)) = self.exts.asking.take() else { return };
        if let Some(m) = crate::extensions::found(cx).iter().filter_map(|f| f.manifest.as_ref().ok()).find(|m| m.id == id).cloned() {
            turn_on_with(&m, &picks, cx);
        }
        cx.notify();
    }

    /// Bench and debug builds: `ext`'s activity shown (`on`) or hidden.
    pub fn debug_activity(&mut self, ext: &str, on: bool, cx: &mut Context<Self>) {
        self.exts_open(ext);
        if on {
            self.exts.activity.insert(ext.to_string());
            self.tick_activity(cx);
        } else {
            self.exts.activity.remove(ext);
        }
        cx.notify();
    }

    /// One permission's words: its title (or its name, unknown here), its
    /// name, the extension's reason, and what it gives.
    pub(super) fn permission_words(ui: &Ui, name: &str, reason: &str) -> Div {
        let known = permissions::find(name);
        let title = known.map_or(name.to_string(), |p| p.title.to_string());
        let mut col = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(div().flex().items_center().gap(px(6.)).child(div().text_size(px(13.)).font_weight(gpui::FontWeight::MEDIUM).text_color(ui.t.text).child(title)).child(crate::kit::mono_text(name.to_string(), &ui.t).text_color(ui.t.text_3)))
            .child(div().text_size(px(12.)).text_color(ui.t.text_2).child(format!("Its reason: “{reason}”")));
        if let Some(p) = known {
            col = col.child(crate::kit::hint(p.detail, &ui.t));
            // One that acts on agents says, in plain words, what can go
            // wrong.
            if let Some(note) = p.risk_note.filter(|_| p.available) {
                let sel = format!("extension-permission-risk-{name}");
                col = col.child(
                    div()
                        .debug_selector(move || sel)
                        .mt(px(4.))
                        .flex()
                        .gap(px(6.))
                        .items_start()
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(ui.t.amber.opacity(0.35))
                        .bg(ui.t.amber.opacity(0.08))
                        .child(div().flex_none().mt(px(1.)).child(crate::kit::icon("alert", 12., ui.t.amber)))
                        .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(gpui::relative(1.4)).text_color(ui.t.text_2).child(note)),
                );
            }
        }
        col
    }

    /// [`Self::permission_words`], with what `m` names for it: the
    /// servers `net` may reach, each as written.
    pub(super) fn permission_words_of(ui: &Ui, m: &Manifest, name: &str, reason: &str) -> Div {
        let col = Self::permission_words(ui, name, reason);
        let listed = permissions::listed(m, name);
        if listed.is_empty() {
            return col;
        }
        let sel = format!("extension-permission-listed-{}-{name}", m.id);
        col.child(
            div()
                .debug_selector(move || sel)
                .mt(px(2.))
                .flex()
                .flex_wrap()
                .gap(px(4.))
                .children(listed.into_iter().map(|h| div().px(px(6.)).py(px(1.)).rounded(px(4.)).bg(ui.t.surface_3).child(crate::kit::mono_text(h, &ui.t).text_color(ui.t.text_2)))),
        )
    }

    /// What a permission granted with the extension shows in place of a
    /// switch: how much (`memory`: "256 MiB").
    fn amount_chip(ui: &Ui, m: &Manifest, name: &str, amount: String) -> Div {
        let sel = format!("extension-permission-amount-{}-{name}", m.id);
        div()
            .flex_none()
            .mt(px(1.))
            .debug_selector(move || sel)
            .child(div().id(SharedString::from(format!("extension-permission-amount-{}-{name}", m.id))).tooltip(crate::kit::tooltip("Granted with the extension: not a switch")).child(crate::kit::mono_text(amount, &ui.t).text_color(ui.t.text_2)))
    }

    /// Whether installed extension `id` came from a preview folder
    /// (unsigned).
    pub(super) fn is_preview(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        self.origins_now(cx).get(id).is_some_and(|(o, _)| o.as_ref().is_some_and(|o| o.is_preview()))
    }

    /// The "send without asking" switch of an extension granted
    /// `agents.queue`: off unless the user turns it on, with a warning.
    fn send_without_asking(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Div {
        let on = crate::ui_state::get(cx).extension(&m.id).send_without_asking.is_some_and(|d| d.granted);
        let id = m.id.clone();
        div()
            .flex()
            .gap(px(12.))
            .items_start()
            .pl(px(14.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(13.)).font_weight(gpui::FontWeight::MEDIUM).text_color(ui.t.text).child(format!("Send what {} queues without asking", m.name)))
                    .child(crate::kit::hint(
                        if on {
                            "On: Auto-send sends its text to the agent like your own, without you reading it first."
                        } else {
                            "Off: its text waits in Next up, marked with its name, until you press Send."
                        },
                        &ui.t,
                    ))
                    .child(div().text_size(px(12.)).text_color(ui.t.amber).child("Turn on only if you trust it and the service behind it with what your agents do: no one reads its text before the agent acts on it.")),
            )
            .child(div().flex_none().mt(px(2.)).child(switch(format!("extension-send-without-asking-{}", m.id), on, ui).tooltip(crate::kit::tooltip(if on { "On: press to make its text wait for Send" } else { "Off: press to let it send without asking" })).on_click(cx.listener(move |_, _, _, cx| calls::set_send_without_asking(&id, !on, cx)))))
    }

    /// The permission screen, in `m`'s card while it is being turned on.
    pub(super) fn permission_screen(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (_, picks) = self.exts.asking.as_ref().filter(|(e, _)| *e == m.id)?.clone();
        let mut col = div()
            .id(SharedString::from(format!("extension-permissions-ask-{}", m.id)))
            .debug_selector({
                let sel = format!("extension-permissions-ask-{}", m.id);
                move || sel
            })
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(14.))
            .rounded(px(8.))
            .border_1()
            .border_color(ui.t.line_strong)
            .bg(ui.t.surface_2)
            .child(div().text_size(px(14.)).font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child(format!("{} asks for these permissions", m.name)))
            .child(crate::kit::hint(
                if m.adapter.is_none() && m.service.is_none() && crate::tiles::has_tiles(m) {
                    "Grant or refuse each. Its tiles run sealed off from your computer: they get only what you grant here. You can change these later here."
                } else {
                    "Grant or refuse each. A permission bounds what Pitwall gives it; its program runs as you either way. You can change these later here, and every call it makes is in its Activity."
                },
                &ui.t,
            ));
        if self.is_preview(&m.id, cx) {
            col = col.child(preview_warning(&m.id, ui));
        }
        for p in &m.permissions {
            let mut row = div().flex().gap(px(12.)).items_start().child(Self::permission_words_of(ui, m, &p.name, &p.reason));
            match (permissions::unavailable_why(&p.name), permissions::amount(m, &p.name).filter(|_| permissions::shown_only(&p.name))) {
                (Some(why), _) => row = row.child(div().flex_none().mt(px(1.)).child(crate::kit::chip_tone(why, Tone::Subtle, &ui.t))),
                // Granted with it, shown with its size.
                (None, Some(amount)) => row = row.child(Self::amount_chip(ui, m, &p.name, amount)),
                (None, None) => {
                    let on = picks.get(&p.name).copied().unwrap_or(false);
                    let name = p.name.clone();
                    row = row.child(div().flex_none().mt(px(2.)).child(switch(format!("extension-permission-pick-{}-{}", m.id, p.name), on, ui).tooltip(crate::kit::tooltip(if on { "Granted: press to refuse" } else { "Refused: press to grant" })).on_click(cx.listener(move |v, _, _, cx| {
                        if let Some((_, picks)) = v.exts.asking.as_mut() {
                            picks.insert(name.clone(), !on);
                        }
                        cx.notify();
                    }))));
                }
            }
            col = col.child(row);
        }
        if let Some(adds) = Self::contributions(ui, m) {
            col = col.child(adds);
        }
        let m2 = m.clone();
        let buttons = div()
            .flex()
            .gap(px(8.))
            .child(div().flex_1())
            .child(btn(format!("extension-permissions-cancel-{}", m.id), "Cancel", BtnKind::GhostSm, false, ui).on_click(cx.listener(|v, _, _, cx| {
                v.exts.asking = None;
                cx.notify();
            })))
            .child(btn(format!("extension-permissions-on-{}", m.id), "Turn on", BtnKind::Small, false, ui).on_click(cx.listener(move |v, _, _, cx| {
                if let Some((_, picks)) = v.exts.asking.take() {
                    turn_on_with(&m2, &picks, cx);
                }
                v.exts.open.remove(&m2.id);
                cx.notify();
            })));
        Some(col.child(buttons).into_any_element())
    }

    /// What it adds to Pitwall while it is on (its views, commands,
    /// badges and notifications), one line each, and the program it keeps
    /// running (its service); nothing for an inbox alone.
    pub(super) fn contributions(ui: &Ui, m: &Manifest) -> Option<Div> {
        let c = &m.contributes;
        let mut lines = c.lines();
        if let Some(s) = &m.service {
            lines.push(format!("Runs a local program continuously while it is on: {} (you confirm its command first)", s.label));
            // Its tiles may reach it without `net`.
            if crate::tiles::has_tiles(m) {
                if let Some(Ok(addr)) = pitwall_core::extensions::service::listen_of(m, s, &Default::default()) {
                    lines.push(format!("Its tiles talk to its own server at {addr}"));
                }
            }
        }
        if lines.is_empty() {
            return None;
        }
        let sel = format!("extension-adds-{}", m.id);
        Some(
            div()
                .debug_selector(move || sel)
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(crate::kit::label("Adds to Pitwall", ui.t.text_3, 11.))
                .children(lines.into_iter().map(|l| div().text_size(px(12.)).text_color(ui.t.text_2).child(l))),
        )
    }

    /// "Show in toolbar", in a card that is on and has a toolbar button:
    /// what the user sets is kept (an update never shows it again).
    pub(super) fn toolbar_row(ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Option<Div> {
        m.contributes.toolbar.as_ref()?;
        let on = crate::main_screen::toolbar_items::visible(m, cx);
        let id = m.id.clone();
        Some(
            div()
                .flex()
                .gap(px(12.))
                .items_start()
                .pt(px(10.))
                .border_t_1()
                .border_color(ui.t.line)
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).child(div().text_size(px(13.)).font_weight(gpui::FontWeight::MEDIUM).text_color(ui.t.text).child("Show in toolbar")).child(crate::kit::hint("Its button in the top bar, after the counts. Right-click it to hide it.", &ui.t)))
                .child(div().flex_none().mt(px(2.)).child(switch(format!("extension-toolbar-{}", m.id), on, ui).on_click(cx.listener(move |_, _, _, cx| {
                    crate::ui_state::update(cx, |s| *s = std::mem::take(s).with_extension(&id, |e| e.toolbar = Some(!on)));
                })))),
        )
    }

    /// Permissions, in a card that is on: each with its reason, its state
    /// and when, and a switch; a new one (after an update) to answer.
    pub(super) fn permissions_view(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Option<Div> {
        if m.permissions.is_empty() {
            return None;
        }
        let decisions = crate::ui_state::get(cx).extension(&m.id).grants;
        let new = permissions::undecided(m, &decisions).len();
        let mut col = div().flex().flex_col().gap(px(10.)).pt(px(10.)).border_t_1().border_color(ui.t.line).child(crate::kit::label("Permissions", ui.t.text_3, 11.));
        if new > 0 {
            let text = if new == 1 { "It asks for a new permission since an update: not granted until you allow it.".to_string() } else { format!("It asks for {new} new permissions since an update: not granted until you allow them.") };
            col = col.child(div().text_size(px(12.)).text_color(ui.t.amber).child(text));
        }
        for p in &m.permissions {
            let mut row = div().flex().gap(px(12.)).items_start().child(Self::permission_words_of(ui, m, &p.name, &p.reason));
            let (ext, name) = (m.id.clone(), p.name.clone());
            let right = match (permissions::unavailable_why(&p.name), decisions.get(&p.name)) {
                (Some(why), _) => div().child(crate::kit::chip_tone(why, Tone::Subtle, &ui.t)),
                (None, _) if permissions::shown_only(&p.name) => match permissions::amount(m, &p.name) {
                    Some(amount) => Self::amount_chip(ui, m, &p.name, amount),
                    None => div(),
                },
                (None, None) => {
                    let (e2, n2) = (ext.clone(), name.clone());
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(crate::kit::chip_tone("new", Tone::Warn, &ui.t))
                        .child(btn(format!("extension-permission-refuse-{}-{}", m.id, p.name), "Don't allow", BtnKind::GhostSm, false, ui).on_click(cx.listener(move |_, _, _, cx| decide(&e2, &n2, false, cx))))
                        .child(btn(format!("extension-permission-allow-{}-{}", m.id, p.name), "Allow", BtnKind::Small, false, ui).on_click(cx.listener(move |_, _, _, cx| decide(&ext, &name, true, cx))))
                }
                (None, Some(d)) => {
                    let granted = d.granted;
                    let when = format!("{} {}", if granted { "Granted" } else { "Refused" }, date_of(d.at));
                    div().flex().items_center().gap(px(10.)).child(div().text_size(px(12.)).text_color(ui.t.text_3).child(when)).child(
                        switch(format!("extension-permission-{}-{}", m.id, p.name), granted, ui)
                            .tooltip(crate::kit::tooltip(if granted { "Revoke: its next call is refused" } else { "Grant" }))
                            .on_click(cx.listener(move |_, _, _, cx| decide(&ext, &name, !granted, cx))),
                    )
                }
            };
            row = row.child(div().flex_none().mt(px(1.)).child(right));
            col = col.child(row);
            if p.name == permissions::AGENTS_QUEUE && decisions.get(&p.name).is_some_and(|d| d.granted) {
                col = col.child(self.send_without_asking(ui, m, cx));
            }
        }
        Some(col)
    }

    /// "Agents can use <tile>", in a card that is on: a switch per tile
    /// agents may open (agent-tools.md §11.5), with its tools; off, its
    /// tools are gone from every agent's list at once.
    pub(super) fn agent_tiles_view(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Option<Div> {
        let tiles: Vec<_> = m.contributes.tiles.iter().filter(|t| t.agents).cloned().collect();
        if tiles.is_empty() {
            return None;
        }
        let all_on = crate::tiles::agents::enabled(cx);
        let sel = format!("extension-agents-{}", m.id);
        let mut col = div().debug_selector(move || sel).flex().flex_col().gap(px(10.)).pt(px(10.)).border_t_1().border_color(ui.t.line).child(crate::kit::label("Agents", ui.t.text_3, 11.));
        if !all_on {
            col = col.child(div().text_size(px(12.)).text_color(ui.t.amber).child("Off for every tile: Settings → Agents → Agents can use tiles."));
        }
        for t in tiles {
            let on = crate::tiles::agents::tile_on(&m.id, &t.id, cx);
            let mut names = vec![pitwall_proto::tile_tools::mcp_name(&m.id, None)];
            names.extend(t.tools.iter().map(|d| pitwall_proto::tile_tools::mcp_name(&m.id, Some(&d.name))));
            let detail = if on {
                format!("Agents can open {} next to their terminal and show you things in it. They get nothing of it back and can’t replace what you edited.", t.title)
            } else {
                "Off: agents don’t get its tools.".to_string()
            };
            let (ext, tile) = (m.id.clone(), t.id.clone());
            col = col.child(
                div()
                    .flex()
                    .gap(px(12.))
                    .items_start()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_size(px(13.)).font_weight(gpui::FontWeight::MEDIUM).text_color(ui.t.text).child(format!("Agents can use {}", t.title)))
                            .child(crate::kit::hint(detail, &ui.t))
                            .child(crate::kit::mono_text(names.join("  "), &ui.t).text_color(ui.t.text_3)),
                    )
                    .child(
                        div().flex_none().mt(px(2.)).child(
                            switch(format!("extension-agents-{}-{}", m.id, t.id), on && all_on, ui)
                                .tooltip(crate::kit::tooltip(if on { "On: press to take its tools from agents" } else { "Off: press to give agents its tools" }))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    crate::tiles::agents::set_tile_on(&ext, &tile, !on, cx);
                                    cx.notify();
                                })),
                        ),
                    ),
            );
        }
        Some(col)
    }

    /// The footer's Activity button: an extension with an adapter, or
    /// tiles agents may use (their calls).
    pub(super) fn activity_button(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Option<AnyElement> {
        if m.adapter.is_none() && !m.contributes.tiles.iter().any(|t| t.agents) {
            return None;
        }
        let shown = self.exts.activity.contains(&m.id);
        let id = m.id.clone();
        Some(
            btn(format!("extension-activity-{}", m.id), if shown { "Hide activity" } else { "Activity" }, BtnKind::GhostSm, false, ui)
                .tooltip(crate::kit::tooltip("What it asked Pitwall for, and what was refused"))
                .on_click(cx.listener(move |v, _, _, cx| {
                    if !v.exts.activity.remove(&id) {
                        v.exts.activity.insert(id.clone());
                        v.tick_activity(cx);
                    }
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    /// Redraw while an activity list is open, every second (calls on the
    /// socket come without a hub change).
    fn tick_activity(&mut self, cx: &mut Context<Self>) {
        if self.exts.activity_ticking {
            return;
        }
        self.exts.activity_ticking = true;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let open = this
                .update(cx, |v, cx| {
                    let open = !v.exts.activity.is_empty();
                    if open {
                        cx.notify();
                    } else {
                        v.exts.activity_ticking = false;
                    }
                    open
                })
                .unwrap_or(false);
            if !open {
                return;
            }
        })
        .detach();
    }

    /// Its activity, when shown: newest first, each call with when, from
    /// where and how it went.
    pub(super) fn activity_view(&mut self, ui: &Ui, m: &Manifest, cx: &mut Context<Self>) -> Option<Div> {
        if !self.exts.activity.contains(&m.id) {
            return None;
        }
        let entries = calls::entries(&m.id, cx);
        let refused = entries.iter().filter(|e| e.refused()).count();
        let head = match entries.len() {
            0 => "Nothing yet: what it asks Pitwall for shows here.".to_string(),
            n => format!("Its last {n} calls ({refused} refused), from its adapter and the programs it started. Kept {} days, at most {}.", pitwall_core::extensions::activity::KEEP_DAYS, pitwall_core::extensions::activity::MAX_ENTRIES),
        };
        let mut col = div().flex().flex_col().gap(px(6.)).child(div().text_size(px(12.)).text_color(ui.t.text_3).child(head));
        if entries.is_empty() {
            return Some(col);
        }
        let sel = format!("extension-activity-lines-{}", m.id);
        let mut list = div().id(SharedString::from(sel.clone())).debug_selector(move || sel).max_h(px(280.)).overflow_y_scroll().flex().flex_col().gap(px(2.)).px(px(10.)).py(px(8.)).rounded(px(8.)).bg(ui.t.surface_2);
        for e in entries.iter().rev().take(SHOWN) {
            let via = match e.via {
                Via::Adapter => "adapter",
                Via::Socket => "socket",
                Via::Pitwall => "Pitwall",
                Via::View => "button",
                Via::Tile => "tile",
                Via::Agent => "agent",
            };
            let (word, color) = match &e.outcome {
                Outcome::Ok => ("ok".to_string(), ui.t.green),
                Outcome::Refused { message, .. } => (format!("refused: {message}"), ui.t.amber),
                Outcome::Failed { message, .. } => (format!("failed: {message}"), ui.t.text_2),
            };
            // What a call that acts asked for (the agent, the text queued).
            let word = match &e.detail {
                Some(d) => format!("{word} · {d}"),
                None => word,
            };
            let row = div()
                .flex()
                .gap(px(8.))
                .items_start()
                .font_family(crate::kit::fonts::MONO_FONT)
                .text_size(px(11.5))
                .line_height(gpui::relative(1.4))
                .child(div().flex_none().text_color(ui.t.text_4).child(when_of(e.at)))
                .child(div().flex_none().w(px(52.)).text_color(ui.t.text_3).child(via))
                .child(div().flex_none().text_color(ui.t.text).child(e.method.clone()))
                .child(div().flex_1().min_w_0().text_color(color).child(word));
            list = list.child(row);
        }
        col = col.child(list);
        Some(col)
    }

    /// A `text` or `number` setting: its field, saved when it loses focus
    /// or on Return, checked first.
    pub(super) fn plain_row(&mut self, ui: &Ui, m: &Manifest, c: Option<&Account>, s: &SettingDef, cx: &mut Context<Self>) -> Div {
        let input = self.plain_input(m, c, s, cx);
        let k = (Self::scope(m, c), s.key.clone());
        let mut col = div().flex().flex_col().gap(px(5.)).child(Self::setting_title(ui, s)).child(div().max_w(px(if s.kind == SettingKind::Number { 180. } else { 520. })).child(input));
        if let Some(n) = self.exts.notes.get(&k) {
            col = col.child(div().text_size(px(12.)).text_color(ui.t.amber).child(n.clone()));
        } else if let Some(h) = Self::setting_hint(s) {
            col = col.child(crate::kit::hint(h, &ui.t));
        }
        col
    }

    /// A setting's hint, with a number's bounds and a default.
    fn setting_hint(s: &SettingDef) -> Option<String> {
        let mut parts: Vec<String> = s.hint.iter().cloned().collect();
        match (&s.min, &s.max) {
            (Some(lo), Some(hi)) => parts.push(format!("From {lo} to {hi}.")),
            (Some(lo), None) => parts.push(format!("At least {lo}.")),
            (None, Some(hi)) => parts.push(format!("At most {hi}.")),
            _ => {}
        }
        if let (Some(d), true) = (&s.default, matches!(s.kind, SettingKind::Text | SettingKind::Number)) {
            parts.push(format!("Empty: {d}."));
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    fn plain_input(&mut self, m: &Manifest, c: Option<&Account>, s: &SettingDef, cx: &mut Context<Self>) -> Entity<TextInput> {
        let k = (Self::scope(m, c), s.key.clone());
        if let Some(i) = self.exts.inputs.get(&k) {
            return i.clone();
        }
        let text = c.and_then(|c| c.values.get(&s.key).cloned()).unwrap_or_default();
        let (ext, conn, def) = (m.id.clone(), c.map(|c| c.id.clone()), s.clone());
        let placeholder = s.placeholder.clone().or_else(|| s.default.clone()).unwrap_or_default();
        let input = cx.new(|cx| TextInput::new(cx, &text, &placeholder));
        let key = k.clone();
        let sub = cx.subscribe(&input, move |v, input, e: &InputEvent, cx| {
            if !matches!(e, InputEvent::Blur | InputEvent::Submit) {
                return;
            }
            let text = input.read(cx).text().to_string();
            match def.check_value(&text) {
                Ok(val) => {
                    v.exts.notes.remove(&key);
                    if let Err(why) = connections::set_value(&ext, conn.as_deref(), &key.1, val, cx) {
                        v.exts.notes.insert(key.clone(), why);
                    }
                }
                Err(why) => {
                    v.exts.notes.insert(key.clone(), why);
                }
            }
            cx.notify();
        });
        self.exts._subs.push(sub);
        self.exts.inputs.insert(k, input.clone());
        input
    }

    /// A `choice` setting: one button per option, the one in use marked.
    pub(super) fn choice_row(&mut self, ui: &Ui, m: &Manifest, c: Option<&Account>, s: &SettingDef, cx: &mut Context<Self>) -> Div {
        let now = c.and_then(|c| c.values.get(&s.key).cloned()).or_else(|| s.default.clone());
        let mut opts = div().flex().flex_wrap().gap(px(6.));
        for o in &s.options {
            let on = now.as_deref() == Some(o.value.as_str());
            let (ext, conn, key, value) = (m.id.clone(), c.map(|c| c.id.clone()), s.key.clone(), o.value.clone());
            let scope = (Self::scope(m, c), s.key.clone());
            let id = format!("extension-choice-{}-{}-{}", m.id, s.key, o.value);
            let sel = id.clone();
            let mut b = div()
                .id(SharedString::from(id))
                .debug_selector(move || sel)
                .h(px(26.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .border_1()
                .cursor_pointer()
                .text_size(px(12.))
                .child(o.label.clone());
            b = if on { b.bg(ui.t.surface_3).border_color(ui.t.line_strong).text_color(ui.t.text).font_weight(crate::kit::WEIGHT_550) } else { b.border_color(ui.t.line).text_color(ui.t.text_2).hover(|d| d.bg(ui.t.surface_2)) };
            opts = opts.child(b.on_click(cx.listener(move |v, _, _, cx| {
                match connections::set_value(&ext, conn.as_deref(), &key, Some(value.clone()), cx) {
                    Ok(()) => v.exts.notes.remove(&scope),
                    Err(why) => v.exts.notes.insert(scope.clone(), why),
                };
                cx.notify();
            })));
        }
        let k = (Self::scope(m, c), s.key.clone());
        let mut col = div().flex().flex_col().gap(px(5.)).child(Self::setting_title(ui, s)).child(opts);
        if let Some(n) = self.exts.notes.get(&k) {
            col = col.child(div().text_size(px(12.)).text_color(ui.t.amber).child(n.clone()));
        } else if let Some(h) = &s.hint {
            col = col.child(crate::kit::hint(h.clone(), &ui.t));
        }
        col
    }

    /// A `bool` setting: a switch beside its title.
    pub(super) fn bool_row(&mut self, ui: &Ui, m: &Manifest, c: Option<&Account>, s: &SettingDef, cx: &mut Context<Self>) -> Div {
        let on = c.and_then(|c| c.values.get(&s.key).cloned()).or_else(|| s.default.clone()).as_deref() == Some("true");
        let (ext, conn, key) = (m.id.clone(), c.map(|c| c.id.clone()), s.key.clone());
        let scope = (Self::scope(m, c), s.key.clone());
        let line = div().flex().items_center().gap(px(10.)).child(Self::step_title(ui, s.label.clone())).child(div().flex_1()).child(switch(format!("extension-bool-{}-{}", m.id, s.key), on, ui).on_click(cx.listener(move |v, _, _, cx| {
            match connections::set_value(&ext, conn.as_deref(), &key, Some((!on).to_string()), cx) {
                Ok(()) => v.exts.notes.remove(&scope),
                Err(why) => v.exts.notes.insert(scope.clone(), why),
            };
            cx.notify();
        })));
        let k = (Self::scope(m, c), s.key.clone());
        let mut col = div().flex().flex_col().gap(px(5.)).max_w(px(520.)).child(line);
        if let Some(n) = self.exts.notes.get(&k) {
            col = col.child(div().text_size(px(12.)).text_color(ui.t.amber).child(n.clone()));
        } else if let Some(h) = &s.hint {
            col = col.child(crate::kit::hint(h.clone(), &ui.t));
        }
        col
    }
}

#[cfg(test)]
mod tests;

/// "Unsigned, preview: only install tiles from people you trust." in an
/// amber note (packaging.md §6.3).
pub(super) fn preview_warning(id: &str, ui: &Ui) -> Div {
    let sel = format!("extension-preview-warning-{id}");
    div()
        .debug_selector(move || sel)
        .flex()
        .gap(px(6.))
        .items_start()
        .px(px(8.))
        .py(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(ui.t.amber.opacity(0.35))
        .bg(ui.t.amber.opacity(0.08))
        .child(div().flex_none().mt(px(1.)).child(crate::kit::icon("alert", 12., ui.t.amber)))
        .child(div().flex_1().min_w_0().text_size(px(12.)).line_height(gpui::relative(1.4)).text_color(ui.t.text_2).child(pitwall_core::extensions::preview::WARNING))
}
