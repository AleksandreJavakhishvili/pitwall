//! Settings → Collaboration and Settings → Networks (collaboration/ui.md
//! §11, safety.md §9.3). Every switch is off by default; Pitwall's own
//! window changes them directly (an agent asking for the same goes through
//! an approval).

use std::rc::Rc;

use gpui::{div, prelude::*, px, App, ClipboardItem, Context, Div, SharedString};
use serde_json::json;

use pitwall_proto::api::collab::{CollabSettings, CollabSettingsView, LookFor};
use pitwall_proto::method;

use crate::kit::{checkbox_box, chip_tone, hint, muted, text_button, BtnKind, SegItem, Tone, MONO_FONT};
use crate::settings::pages::{head, row, section};
use crate::settings::view::SettingsView;
use crate::settings::widgets::Ui;

fn current(cx: &App) -> Option<CollabSettingsView> {
    super::snapshot(cx).settings
}

/// Save changed switches (then everything is read again).
fn change(cx: &mut App, f: impl FnOnce(&mut CollabSettings)) {
    let Some(mut s) = current(cx).map(|v| v.settings) else { return };
    f(&mut s);
    super::call::<CollabSettingsView>(method::COLLAB_SETTINGS_SET, json!(s), cx, |r, _| {
        if let Err(e) = r {
            eprintln!("pitwall: couldn't change collaboration settings: {}", e.message);
        }
    });
}

/// A switch row: title, hint, the box on the right (as Settings' others).
fn switch(id: impl Into<SharedString>, title: &str, about: &str, on: bool, amber: bool, ui: &Ui, run: impl Fn(&mut App) + 'static) -> Div {
    let id: SharedString = id.into();
    section().child(
        div()
            .id(id)
            .flex()
            .gap(px(12.))
            .items_start()
            .cursor_pointer()
            .on_click(move |_, _, cx| run(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(head(ui, title).when(amber && on, |d| d.child(chip_tone("open", Tone::Warn, &ui.t))))
                    .child(hint(about.to_string(), &ui.t)),
            )
            .child(div().mt(px(6.)).child(checkbox_box(on, false, 14., &ui.t))),
    )
}

fn not_running(ui: &Ui) -> Vec<Div> {
    vec![section().child(muted("Collaboration needs Pitwall's engine, which isn't running.", &ui.t))]
}

pub fn collaboration(ui: &Ui, cx: &mut Context<SettingsView>) -> Vec<Div> {
    let Some(v) = current(cx) else { return not_running(ui) };
    let s = v.settings.clone();
    let mut out = vec![switch(
        "collab-teams",
        "Use teams",
        "Discovery, joining teams and asking teammates' shared agents. Outgoing only: nobody can reach this machine because of it.",
        s.use_teams,
        false,
        ui,
        |cx| change(cx, |s| s.use_teams = !s.use_teams),
    )];
    if v.networks.is_empty() {
        out.push(section().child(head(ui, "Let people reach this machine")).child(hint("Pitwall isn't on a network yet (Settings → Networks).", &ui.t)));
    }
    for (i, n) in v.networks.iter().enumerate() {
        let id = n.id.as_str().to_string();
        let name = n.id.as_str().split_once(':').map_or(n.id.as_str(), |(_, x)| x).to_string();
        let on = n.reachable;
        out.push(switch(
            SharedString::from(format!("collab-reach-{i}")),
            &format!("Let people on {name} reach this machine"),
            "Serves your shared agents on this network. Sharing needs it; each share stays a draft until you press Go live.",
            on,
            true,
            ui,
            move |cx| {
                let id = id.clone();
                change(cx, move |s| {
                    let now = s.reachable.get(&id).copied().unwrap_or(false);
                    s.reachable.insert(id, !now);
                })
            },
        ));
    }
    if !v.can_host {
        out.push(section().child(hint("Shared sessions run on macOS for now; here you can ask, not share.", &ui.t)));
    }
    out.push(switch(
        "collab-agents-ask",
        "Let my agents ask shared agents",
        "Your agents may run `pitwall ask` (labelled with their name from the process tree), within the budget below.",
        s.agents_ask,
        false,
        ui,
        |cx| change(cx, |s| s.agents_ask = !s.agents_ask),
    ));
    let budget = s.agent_budget_per_hour;
    const BUDGETS: [u32; 3] = [5, 20, 50];
    out.push(
        section().child(row(ui, "Messages per hour per asking agent").child(crate::kit::seg(
            "collab-budget",
            BUDGETS.iter().map(|n| SegItem::new(n.to_string(), budget == *n)).collect(),
            &ui.t,
            Rc::new(|i, _, cx| change(cx, |s| s.agent_budget_per_hour = BUDGETS[i])),
        ))),
    );
    let cap = s.session_cap;
    const CAPS: [u32; 3] = [2, 4, 8];
    out.push(
        section()
            .child(row(ui, "Shared conversations at once, all shares").child(crate::kit::seg(
                "collab-cap",
                CAPS.iter().map(|n| SegItem::new(n.to_string(), cap == *n)).collect(),
                &ui.t,
                Rc::new(|i, _, cx| change(cx, |s| s.session_cap = CAPS[i])),
            )))
            .child(hint("More messages wait in each share's queue.", &ui.t)),
    );
    out.push(look_for(ui, s.look_for));
    let mut setup = section().child(head(ui, "My MCP setup line")).child(hint(
        "For people without Pitwall: one line in their terminal, and their Claude Code sees your shared agents as tools.",
        &ui.t,
    ));
    if v.setup.is_empty() {
        setup = setup.child(hint("Turn on “Let people on … reach this machine” first.", &ui.t));
    }
    for (i, l) in v.setup.iter().enumerate() {
        let text = l.line.clone();
        setup = setup.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().flex_1().min_w_0().font_family(MONO_FONT).text_size(px(11.5)).text_color(ui.t.text_2).child(l.line.clone()))
                .child(text_button(("collab-copy-setup", i), "Copy", BtnKind::Small, false, &ui.t).on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())))),
        );
    }
    out.push(setup);
    out
}

fn look_for(ui: &Ui, now: LookFor) -> Div {
    const ALL: [(LookFor, &str, &str); 3] = [
        (LookFor::Tagged, "Tagged devices", "Only devices tagged tag:pitwall"),
        (LookFor::Everyone, "Everyone", "Every online device of people on my networks (capped)"),
        (LookFor::Named, "Devices I name", "Only devices I add by name (very large networks)"),
    ];
    section()
        .child(row(ui, "Look for teams on").child(crate::kit::seg(
            "collab-look",
            ALL.iter().map(|(l, label, tip)| SegItem { label: (*label).into(), on: now == *l, tooltip: Some((*tip).into()) }).collect(),
            &ui.t,
            Rc::new(|i, _, cx| change(cx, |s| s.look_for = ALL[i].0)),
        )))
        .child(hint("Discovery runs while People is open, every 30 minutes while “Use teams” is on, and on Refresh.", &ui.t))
}

pub fn networks(ui: &Ui, cx: &mut Context<SettingsView>) -> Vec<Div> {
    let Some(v) = current(cx) else { return not_running(ui) };
    let mut out = Vec::new();
    if v.networks.is_empty() {
        out.push(section().child(head(ui, "Tailscale")).child(hint(
            "Collaboration runs on the networks Pitwall is on. Tailscale isn't set up here yet: install it and sign in, then come back.",
            &ui.t,
        )));
    }
    for (i, n) in v.networks.iter().enumerate() {
        let id = n.id.as_str().to_string();
        let name = n.id.as_str().split_once(':').map_or(n.id.as_str(), |(_, x)| x).to_string();
        let tone = if n.state == "ready" { Tone::Ok } else { Tone::Subtle };
        let mut sec = section()
            .child(head(ui, &format!("{} · {name}", n.label)).child(chip_tone(n.state.replace('_', " "), tone, &ui.t)))
            .children(n.me.clone().map(|me| hint(format!("You: {me}"), &ui.t)))
            .children(n.endpoint.clone().map(|e| div().font_family(MONO_FONT).text_size(px(11.5)).text_color(ui.t.text_3).child(e)));
        let on = n.reachable;
        sec = sec.child(
            div()
                .id(("net-reach", i))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    let id = id.clone();
                    change(cx, move |s| {
                        let now = s.reachable.get(&id).copied().unwrap_or(false);
                        s.reachable.insert(id, !now);
                    })
                })
                .child(checkbox_box(on, false, 14., &ui.t))
                .child(format!("Let people on {name} reach this machine")),
        );
        out.push(sec);
    }
    out.push(look_for(ui, v.settings.look_for));
    out
}
