//! Settings → Peers (docs/spec/tailnet-peers.md): the user's other
//! machines over Tailscale. The two switches ("Let my other machines see
//! this Mac's agents", "Show my other machines' agents") and the port; one
//! list of the user's machines (this Mac first): online or not, its
//! Pitwall, the pairing and its link, with Pair / Pair again / Unpair, and
//! Refresh to ask Tailscale and the machines again; machines watching this
//! one with Revoke; the recent audit log. All of it is [`crate::peers::PeersModel`]; changes the
//! `pitwall peers` CLI makes show up here live.

use std::rc::Rc;

use gpui::{div, prelude::*, px, AnyElement, App, Context, Div, Entity, SharedString, Subscription};

use pitwall_peer::{Candidate, PeerEntry, PeerSettings, Probe, TailnetState};
use pitwall_proto::peer::LinkState;

use super::view::{Part, SettingsView};
use super::widgets::Ui;
use crate::kit::{tooltip, BtnKind, InputEvent, TextInput, Tone};
use crate::peers::{ago, link_text, now_ms, Machine, PeersModel};

/// The page's own state.
#[derive(Default)]
pub struct Section {
    port: Option<Entity<TextInput>>,
    /// The page looked at Tailscale since it opened.
    checked: bool,
    _subs: Vec<Subscription>,
}

/// Why nothing can be shown, in words, when Tailscale isn't ready.
pub fn tailnet_words(state: TailnetState) -> Option<&'static str> {
    match state {
        TailnetState::Ready => None,
        TailnetState::NotInstalled => Some("Tailscale isn't installed on this Mac. Install it and sign in, on this Mac and your other machines, to see their agents here."),
        TailnetState::NotRunning => Some("Tailscale isn't running. Start it to reach your other machines."),
        TailnetState::NeedsLogin => Some("Tailscale is logged out. Sign in to reach your other machines."),
        TailnetState::Userspace => Some("Tailscale runs in userspace networking mode, which Pitwall can't use. Use the Tailscale app (or tailscaled with a tun device)."),
    }
}

/// A port the user typed, if it is one Pitwall may use.
pub fn parse_port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|p| *p >= 1024)
}

/// What a discovered machine's row says.
pub fn found_words(c: &Candidate) -> String {
    match &c.probe {
        Probe::Pitwall { agents, version, .. } => format!("Pitwall {version} · {agents} agent{}", if *agents == 1 { "" } else { "s" }),
        Probe::NoAnswer { .. } => "no Pitwall sharing there".into(),
        Probe::Incompatible { message } => message.clone(),
        Probe::NotProbed => "offline".into(),
    }
}

/// One machine in the list: Tailscale's word on it, its Pitwall (what a
/// probe found) and this Mac's pairing with it.
#[derive(Debug, Clone, PartialEq)]
pub struct MachineRow {
    pub stable_id: String,
    pub label: String,
    /// `None`: Tailscale doesn't list it (any more).
    pub online: Option<bool>,
    /// What it is, in words: "online · macOS · Pitwall 0.2.1 · 2 agents".
    pub words: String,
    pub entry: Option<PeerEntry>,
    /// Found running a Pitwall that shares, not paired yet.
    pub can_pair: bool,
}

/// The user's machines, from Tailscale's status (online first), then
/// paired or found ones it no longer lists. `show`: discovery may run (off,
/// nothing is probed, so their Pitwall is unknown).
pub fn machine_rows(machines: &[Machine], found: Option<&[Candidate]>, entries: &[PeerEntry], show: bool, now: u64) -> Vec<MachineRow> {
    let found = found.unwrap_or_default();
    let mut ids: Vec<(String, String, Option<bool>)> = machines.iter().map(|m| (m.stable_id.clone(), m.label.clone(), Some(m.online))).collect();
    for (id, label) in entries.iter().map(|e| (&e.paired.stable_id, &e.paired.label)).chain(found.iter().map(|c| (&c.stable_id, &c.label))) {
        if !ids.iter().any(|(i, ..)| i == id) {
            ids.push((id.clone(), label.clone(), None));
        }
    }
    ids.into_iter()
        .map(|(stable_id, label, online)| {
            let cand = found.iter().find(|c| c.stable_id == stable_id);
            let entry = entries.iter().find(|e| e.paired.stable_id == stable_id).cloned();
            let mut words: Vec<String> = Vec::new();
            match online.or(cand.map(|c| c.online)) {
                Some(true) => words.push("online".into()),
                Some(false) => words.push("offline".into()),
                None => words.push("not on Tailscale".into()),
            }
            if let Some(c) = cand.filter(|c| !c.os.is_empty()) {
                words.push(c.os.clone());
            }
            match (&entry, cand) {
                (Some(e), _) => {
                    let agents = format!("{} agent{}", e.agents, if e.agents == 1 { "" } else { "s" });
                    words.push(match &e.link {
                        Some(l) => format!("{} · {agents}", link_text(l, now)),
                        None => "paired · not shown (showing is off)".into(),
                    });
                }
                (None, Some(c)) if c.probe != Probe::NotProbed => {
                    words.push(found_words(c));
                    if matches!(c.probe, Probe::Pitwall { .. }) {
                        words.push("not paired".into());
                    }
                }
                (None, _) if !show => words.push("turn on “Show” to look for its Pitwall".into()),
                (None, None) if online == Some(true) => words.push("checking for Pitwall…".into()),
                _ => {}
            }
            let can_pair = entry.is_none() && matches!(cand.map(|c| &c.probe), Some(Probe::Pitwall { .. }));
            MachineRow { stable_id, label, online, words: words.join(" · "), entry, can_pair }
        })
        .collect()
}

fn section() -> Div {
    div().flex().flex_col().gap(px(8.))
}

fn head(ui: &Ui, title: &str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .min_h(px(26.))
        .child(div().font_weight(crate::kit::WEIGHT_550).text_color(ui.t.text).child(title.to_string()))
        .child(div().flex_1())
}

/// One machine's row: name, what's known, then its buttons.
fn machine_row(ui: &Ui, id: SharedString, name: String, sub: String) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .min_h(px(30.))
        .child(div().flex_none().font_weight(crate::kit::WEIGHT_550).child(name))
        .child(div().flex_1().min_w_0().child(crate::kit::hint(sub, &ui.t)))
}

/// A machine in the list: a dot (green online, grey not), its name, what's
/// known, then its buttons.
fn listed_row(ui: &Ui, id: SharedString, name: String, sub: String, online: bool) -> gpui::Stateful<Div> {
    let dot = div().flex_none().size(px(7.)).rounded_full().bg(if online { ui.t.green } else { ui.t.text_4 });
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .min_h(px(30.))
        .child(dot)
        .child(div().flex_none().font_weight(crate::kit::WEIGHT_550).child(name))
        .child(div().flex_1().min_w_0().child(crate::kit::hint(sub, &ui.t)))
}

impl SettingsView {
    fn peers_port(&mut self, model: &Entity<PeersModel>, port: u16, cx: &mut Context<Self>) -> Entity<TextInput> {
        if let Some(p) = &self.peers.port {
            return p.clone();
        }
        let input = cx.new(|cx| TextInput::new(cx, &port.to_string(), "47420"));
        let m = model.downgrade();
        let sub = cx.subscribe(&input, move |_, input, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Submit | InputEvent::Blur) {
                let text = input.read(cx).text().to_string();
                if let Some(port) = parse_port(&text) {
                    let _ = m.update(cx, |m, cx| {
                        if m.settings.port != port {
                            let s = PeerSettings { port, ..m.settings };
                            m.set_settings(s, cx);
                        }
                    });
                }
            }
        });
        self.peers._subs.push(sub);
        self.peers.port = Some(input.clone());
        input
    }

    /// The Peers page's parts: its sections, with each machine, each
    /// machine that can see this one and each audit line a row of its own.
    pub(super) fn peers_parts(&mut self, cx: &mut Context<Self>) -> Vec<Part> {
        let Some(model) = crate::peers::model(cx) else {
            return vec![Part::section("peers-off", |_, ui, _, _| {
                section().child(crate::kit::muted("Your other machines need Pitwall's engine, which isn't running.", &ui.t)).into_any_element()
            })];
        };
        if !self.peers.checked {
            self.peers.checked = true;
            self.peers._subs.push(cx.observe(&model, |_, _, cx| cx.notify()));
            model.update(cx, |m, cx| m.check(cx));
        }
        if model.read(cx).wants_discovery() {
            model.update(cx, |m, cx| m.discover(false, cx));
        }
        let m = model.read(cx);
        let rows = Rc::new(machine_rows(&m.machines, m.found.as_deref(), &m.entries, m.settings.show, now_ms()));
        let ready = m.info.as_ref().is_some_and(|i| i.state == TailnetState::Ready);
        let viewers: Vec<String> = m.viewers.iter().map(|v| v.id.clone()).collect();
        let audit = m.audit.len().min(12);
        let mut out = vec![
            Part::section("peers-tailscale", |_, ui, _, cx| with_model(cx, |m, _| peers_tailscale(ui, m))),
            Part::section("peers-switches", |v, ui, _, cx| v.peers_switches(ui, cx)),
            Part::section("peers-machines", |v, ui, _, cx| v.peers_machines_head(ui, cx)),
        ];
        for (i, r) in rows.iter().enumerate() {
            let rows = rows.clone();
            out.push(Part::row(("peers-machine", &r.stable_id), move |v, ui, _, cx| v.peers_machine(ui, i, &rows[i], cx)));
        }
        if !rows.is_empty() || ready {
            let any = !rows.is_empty();
            out.push(Part::row("peers-machines-foot", move |_, ui, _, _| {
                if any {
                    crate::kit::hint("A machine can be paired once its Pitwall has “Let my other machines see this Mac's agents” on.", &ui.t).into_any_element()
                } else {
                    div()
                        .id("peers-no-machines")
                        .debug_selector(|| "peers-no-machines".into())
                        .child(crate::kit::muted(
                            "No other machines of yours are on this tailnet. Sign in to Tailscale with the same account on your other Mac to see its agents here.",
                            &ui.t,
                        ))
                        .into_any_element()
                }
            }));
        }
        let none = viewers.is_empty();
        out.push(Part::section("peers-viewers", move |_, ui, _, _| {
            section()
                .child(head(ui, "Can see this Mac"))
                .when(none, |s| {
                    s.child(div().id("peers-viewers-empty").debug_selector(|| "peers-viewers-empty".into()).child(crate::kit::hint("No machine sees this Mac's agents.", &ui.t)))
                })
                .into_any_element()
        }));
        for (i, id) in viewers.into_iter().enumerate() {
            out.push(Part::row(("peers-viewer", id), move |v, ui, _, cx| v.peers_viewer(ui, i, cx)));
        }
        if audit > 0 {
            out.push(Part::section("peers-audit", |_, ui, _, _| section().child(head(ui, "Recent activity")).into_any_element()));
            for i in 0..audit {
                out.push(Part::row(("peers-audit", i), move |_, ui, _, cx| with_model(cx, |m, _| audit_line(ui, m, i))));
            }
        }
        out
    }

    /// The two switches and the port.
    fn peers_switches(&mut self, ui: &Ui, cx: &mut Context<Self>) -> AnyElement {
        let Some(model) = crate::peers::model(cx) else { return div().into_any_element() };
        let t = &ui.t;
        let m = model.read(cx);
        let s = m.settings;
        let listening = m.info.as_ref().map(|i| i.listening.clone()).unwrap_or_default();
        let listen_error = m.info.as_ref().and_then(|i| i.listen_error.clone());
        let error = m.error.clone();
        let port_input = self.peers_port(&model, s.port, cx);
        let (m2, m3) = (model.downgrade(), model.downgrade());
        let mut sw = section()
            .child(
                head(ui, "Let my other machines see this Mac's agents").child(
                    crate::kit::toggle("peers-share", s.share, t)
                        .debug_selector(|| "peers-share".into())
                        .tooltip(tooltip("Listen on this Mac's Tailscale addresses only, for your own machines"))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let _ = m2.update(cx, |m, cx| {
                                let s = PeerSettings { share: !m.settings.share, ..m.settings };
                                m.set_settings(s, cx);
                            });
                        })),
                ),
            )
            .child(crate::kit::hint(
                match (s.share, listening.is_empty()) {
                    (false, _) => "Off: nothing listens. Your other machines can't see this Mac's agents.".to_string(),
                    (true, true) => listen_error.unwrap_or_else(|| "Starting…".into()),
                    (true, false) => format!(
                        "Listening on {} (Tailscale only). They see names, status and needs-you; never your terminals.",
                        listening.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")
                    ),
                },
                t,
            ))
            .child(
                head(ui, "Show my other machines' agents").child(
                    crate::kit::toggle("peers-show", s.show, t)
                        .debug_selector(|| "peers-show".into())
                        .tooltip(tooltip("Find and connect to your machines that share theirs"))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let _ = m3.update(cx, |m, cx| {
                                let s = PeerSettings { show: !m.settings.show, ..m.settings };
                                m.set_settings(s, cx);
                            });
                        })),
                ),
            )
            .child(crate::kit::hint(
                if s.show {
                    "Paired machines' agents show in the sidebar under their machine's name, read-only, with their needs-you."
                } else {
                    "Off: nothing is looked for or connected to."
                },
                t,
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_h(px(28.))
                    .child(crate::kit::muted("Port", t))
                    .child(div().w(px(90.)).font_family(crate::kit::MONO_FONT).child(port_input))
                    .child(crate::kit::hint("The same on all your machines.", t)),
            );
        if let Some(e) = error {
            sw = sw.child(div().id("peers-error").child(crate::kit::error_text(e, t)));
        }
        sw.into_any_element()
    }

    /// "Machines" with Refresh, this Mac's row, and a failed discovery.
    fn peers_machines_head(&mut self, ui: &Ui, cx: &mut Context<Self>) -> AnyElement {
        let Some(model) = crate::peers::model(cx) else { return div().into_any_element() };
        let t = &ui.t;
        let m = model.read(cx);
        let (s, info) = (m.settings, m.info.as_ref());
        let checking = m.discovering || info.is_none();
        let here = info.and_then(|i| i.machine.clone()).unwrap_or_else(|| "This Mac".into());
        let listening = info.map(|i| i.listening.clone()).unwrap_or_default();
        let here_words = match (s.share, listening.is_empty(), info.and_then(|i| i.listen_error.as_ref())) {
            (false, ..) => "this Mac · not shared".to_string(),
            (true, false, _) => format!("this Mac · shared on {}", listening.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")),
            (true, true, Some(e)) => format!("this Mac · not listening: {e}"),
            (true, true, None) => "this Mac · starting…".to_string(),
        };
        let ready = info.is_some_and(|i| i.state == TailnetState::Ready);
        let discover_error = m.discover_error.clone();
        let mr = model.downgrade();
        section()
            .child(
                head(ui, "Machines").child(
                    crate::kit::text_button("peers-refresh", if checking { "Checking…" } else { "Refresh" }, BtnKind::GhostSm, checking, t)
                        .debug_selector(|| "peers-refresh".into())
                        .tooltip(tooltip("Ask Tailscale again and look for your machines' Pitwall"))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let _ = mr.update(cx, |m, cx| m.refresh_now(cx));
                        })),
                ),
            )
            .child(listed_row(ui, "peers-this-mac".into(), here, here_words, ready).debug_selector(|| "peers-this-mac".into()))
            .when_some(discover_error, |s, e| {
                s.child(div().id("peers-discover-error").debug_selector(|| "peers-discover-error".into()).child(crate::kit::error_text(e, t)))
            })
            .into_any_element()
    }

    /// Machine `i` of the list, with Pair, Pair again or Unpair.
    fn peers_machine(&mut self, ui: &Ui, i: usize, r: &MachineRow, cx: &mut Context<Self>) -> AnyElement {
        let Some(model) = crate::peers::model(cx) else { return div().into_any_element() };
        let t = &ui.t;
        let busy = &model.read(cx).busy;
        let b = busy.contains(&r.stable_id) || r.entry.as_ref().is_some_and(|e| busy.contains(&e.paired.id));
        let mut row = listed_row(ui, SharedString::from(format!("peers-machine-{i}")), r.label.clone(), r.words.clone(), r.online == Some(true))
            .debug_selector(move || format!("peers-machine-{i}"));
        if let Some(e) = &r.entry {
            if e.link.as_ref().is_some_and(|l| l.state == LinkState::IdentityChanged) {
                let (sid, mp) = (e.paired.stable_id.clone(), model.downgrade());
                row = row.child(
                    crate::kit::text_button(("peers-repair", i), "Pair again", BtnKind::Small, b, t).on_click(cx.listener(move |_, _, _, cx| {
                        let _ = mp.update(cx, |m, cx| m.pair(&sid, cx));
                    })),
                );
            }
            let (id, mm) = (e.paired.id.clone(), model.downgrade());
            row = row.child(
                crate::kit::text_button(("peers-unpair", i), "Unpair", BtnKind::GhostSm, b, t)
                    .tooltip(tooltip("Stop showing its agents and forget its key"))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let _ = mm.update(cx, |m, cx| m.unpair(&id, cx));
                    })),
            );
        } else if r.can_pair {
            let (id, mm) = (r.stable_id.clone(), model.downgrade());
            row = row.child(
                crate::kit::text_button(("peers-pair", i), if b { "Pairing…" } else { "Pair" }, BtnKind::Small, b, t)
                    .debug_selector(move || format!("peers-pair-{i}"))
                    .tooltip(tooltip("Show its agents here (read-only)"))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let _ = mm.update(cx, |m, cx| m.pair(&id, cx));
                    })),
            );
        }
        row.into_any_element()
    }

    /// Machine `i` of those that can see this one, with Revoke.
    fn peers_viewer(&mut self, ui: &Ui, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(model) = crate::peers::model(cx) else { return div().into_any_element() };
        let Some(v) = model.read(cx).viewers.get(i).cloned() else { return div().into_any_element() };
        let seen = v.last_seen.map(|t| format!("last connected {}", ago(t, now_ms()))).unwrap_or_else(|| "never connected".into());
        let b = model.read(cx).busy.contains(&v.id);
        let (id, mm) = (v.id.clone(), model.downgrade());
        machine_row(ui, SharedString::from(format!("peers-viewer-{i}")), v.label.clone(), format!("{} · {seen}", v.login))
            .child(
                crate::kit::text_button(("peers-revoke", i), "Revoke", BtnKind::GhostSm, b, &ui.t)
                    .tooltip(tooltip("Disconnect it now and forget it; it must pair again"))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let _ = mm.update(cx, |m, cx| m.revoke(&id, cx));
                    })),
            )
            .into_any_element()
    }
}

/// Draw with the peers model (an empty element without one).
fn with_model(cx: &mut Context<SettingsView>, f: impl FnOnce(&PeersModel, &App) -> AnyElement) -> AnyElement {
    match crate::peers::model(cx) {
        Some(model) => f(model.read(cx), cx),
        None => div().into_any_element(),
    }
}

/// Tailscale: its state, and why nothing shows when it isn't ready.
fn peers_tailscale(ui: &Ui, m: &PeersModel) -> AnyElement {
    let t = &ui.t;
    let info = m.info.as_ref();
    let mut ts = section().child(head(ui, "Tailscale").child(match info {
        None => crate::kit::hint("checking…", t),
        Some(i) if i.state == TailnetState::Ready => crate::kit::chip_tone(
            match (&i.machine, &i.login) {
                (Some(mc), Some(l)) => format!("{mc} · {l}"),
                (Some(mc), None) => mc.clone(),
                _ => "ready".into(),
            },
            Tone::Ok,
            t,
        ),
        Some(_) => crate::kit::chip_tone("not ready", Tone::Warn, t),
    }));
    if let Some(words) = info.and_then(|i| tailnet_words(i.state)) {
        ts = ts.child(div().id("peers-tailnet-empty").debug_selector(|| "peers-tailnet-empty".into()).child(crate::kit::muted(words, t)));
    }
    ts.child(crate::kit::hint(
        "Only your own machines take part: the same Tailscale user, never tagged devices or other people's machines. Their agents are read-only here.",
        t,
    ))
    .into_any_element()
}

/// Audit line `i` (newest first).
fn audit_line(ui: &Ui, m: &PeersModel, i: usize) -> AnyElement {
    let Some(e) = m.audit.get(i) else { return div().into_any_element() };
    let t = &ui.t;
    let who = e.peer.clone().or_else(|| e.login.clone()).or_else(|| e.addr.clone()).unwrap_or_default();
    let what = [Some(e.event.replace('_', " ")), Some(who).filter(|w| !w.is_empty()), e.method.clone(), e.detail.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    div()
        .flex()
        .gap(px(8.))
        .text_size(px(12.))
        .child(div().flex_none().w(px(76.)).text_color(t.text_4).child(ago(e.at, now_ms())))
        .child(div().flex_1().min_w_0().text_color(t.text_2).child(what))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_list_says_what_each_one_is() {
        use pitwall_peer::Paired;
        use pitwall_proto::peer::{LinkView, Pairing};
        let machine = |id: &str, label: &str, online| Machine { stable_id: id.into(), label: label.into(), online };
        let cand = |id: &str, label: &str, probe| Candidate {
            stable_id: id.into(),
            label: label.into(),
            dns: format!("{label}.example.ts.net"),
            os: "macOS".into(),
            online: true,
            paired: None,
            probe,
        };
        let pitwall = Probe::Pitwall { version: "0.2.1".into(), label: "studio".into(), pairing: Pairing::Open, agents: 2 };
        let machines = [machine("nS", "studio", true), machine("nA", "air", true), machine("nO", "old", false)];
        // Showing off: nothing probed, Tailscale's word only.
        let rows = machine_rows(&machines, None, &[], false, 0);
        assert_eq!(rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["studio", "air", "old"]);
        assert_eq!(rows[0].words, "online · turn on “Show” to look for its Pitwall");
        assert_eq!(rows[2].words, "offline · turn on “Show” to look for its Pitwall");
        assert!(!rows[0].can_pair);
        // Showing: studio shares, air has no Pitwall, old wasn't probed.
        let found = [cand("nS", "studio", pitwall), cand("nA", "air", Probe::NoAnswer { message: "refused".into() })];
        let rows = machine_rows(&machines, Some(&found), &[], true, 0);
        assert_eq!(rows[0].words, "online · macOS · Pitwall 0.2.1 · 2 agents · not paired");
        assert!(rows[0].can_pair && !rows[1].can_pair);
        assert_eq!(rows[1].words, "online · macOS · no Pitwall sharing there");
        assert_eq!(rows[2].words, "offline");
        // Paired: its link and agents; one Tailscale no longer lists stays.
        let paired = |id: &str, label: &str, link| PeerEntry {
            paired: Paired {
                id: label.into(),
                label: label.into(),
                stable_id: id.into(),
                login: "me@".into(),
                public_key: "ed25519:x".into(),
                added_at: 0,
                last_seen: None,
            },
            link,
            agents: 3,
            blocked: 0,
        };
        let connected = LinkView { peer: "studio".into(), state: LinkState::Connected, since: 0, last_seen: None };
        let entries = [paired("nS", "studio", Some(connected)), paired("nGone", "gone", None)];
        let rows = machine_rows(&machines, Some(&found), &entries, true, 0);
        assert_eq!(rows[0].words, "online · macOS · connected · 3 agents");
        assert!(!rows[0].can_pair && rows[0].entry.is_some());
        assert_eq!(rows[3].label, "gone");
        assert_eq!(rows[3].words, "not on Tailscale · paired · not shown (showing is off)");
    }

    #[test]
    fn ports_and_words() {
        assert_eq!(parse_port(" 47420 "), Some(47420));
        assert_eq!(parse_port("80"), None, "no privileged ports");
        assert_eq!(parse_port("nope"), None);
        assert!(tailnet_words(TailnetState::NotInstalled).unwrap().contains("isn't installed"));
        assert!(tailnet_words(TailnetState::NeedsLogin).unwrap().contains("logged out"));
        assert_eq!(tailnet_words(TailnetState::Ready), None);
    }
}

#[cfg(test)]
mod window_tests {
    use std::sync::Arc;

    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use pitwall_core::testing::TempDir;
    use pitwall_peer::fake::FakeWorld;
    use pitwall_peer::{NullPeerSink, Peers, PeersConfig};
    use pitwall_proto::peer::Pairing;
    use pitwall_proto::settings::Page;

    use super::*;
    use crate::peers::ModelHandle;

    fn frame(cx: &mut VisualTestContext) {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    /// Settings → Peers on a fake tailnet where this Mac is alone.
    fn open(cx: &mut TestAppContext) -> (TempDir, Entity<PeersModel>, &mut VisualTestContext) {
        let (dir, _, _, model, cx) = open_with(cx, |_, _| {});
        (dir, model, cx)
    }

    /// [`open`], `setup` first changing the tailnet or the saved settings.
    #[allow(clippy::type_complexity)]
    fn open_with(
        cx: &mut TestAppContext,
        setup: impl FnOnce(&Arc<FakeWorld>, &Arc<Peers>),
    ) -> (TempDir, Arc<FakeWorld>, Entity<SettingsView>, Entity<PeersModel>, &mut VisualTestContext) {
        let dir = TempDir::new("app-peers-settings");
        let w = FakeWorld::new();
        w.add("nLAPTOP", "laptop", "alice@example.com");
        let tailnet = w.tailnet("nLAPTOP");
        let peers = Peers::new(PeersConfig {
            root: dir.path().to_path_buf(),
            tailnet: tailnet.clone(),
            version: "0.2.1".into(),
            sink: Arc::new(NullPeerSink),
            feed: None,
            timing: Default::default(),
            limits: Default::default(),
        })
        .unwrap();
        setup(&w, &peers);
        let root = dir.path().to_path_buf();
        let model = cx.update(|cx| {
            cx.set_global(crate::theme::Theme::dark());
            crate::ui_state::init(None, cx);
            crate::settings::data::set_page(cx, Page::Peers);
            let m = cx.new(|_| PeersModel::new(peers, tailnet, root));
            cx.set_global(ModelHandle(m.clone()));
            m
        });
        let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(true, cx));
        cx.simulate_resize(gpui::size(gpui::px(1600.), gpui::px(1000.)));
        frame(cx);
        (dir, w, view, model, cx)
    }

    /// `n` machines of the user's, online, none probed (showing is off).
    fn machines(model: &Entity<PeersModel>, n: usize, cx: &mut VisualTestContext) {
        model.update(cx, |m, cx| {
            m.machines = (0..n).map(|i| Machine { stable_id: format!("n{i}"), label: format!("mac-{i:02}"), online: true }).collect();
            cx.notify();
        });
        cx.run_until_parked();
    }

    fn scroll(cx: &mut VisualTestContext, dy: f32) {
        let at = gpui::point(gpui::px(500.), gpui::px(300.));
        cx.simulate_mouse_move(at, None, Modifiers::none());
        cx.simulate_event(gpui::ScrollEvent {
            position: at,
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(dy))),
            ..Default::default()
        });
        cx.run_until_parked();
    }

    fn selector(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }

    /// The rows drawn now: index and bounds.
    fn drawn_rows(cx: &mut VisualTestContext, n: usize) -> Vec<(usize, gpui::Bounds<gpui::Pixels>)> {
        (0..n).filter_map(|i| cx.debug_bounds(selector(format!("peers-machine-{i}"))).map(|b| (i, b))).collect()
    }

    /// A long page: a scroll step builds only the parts on screen (the
    /// page isn't built again), and the page still follows its model.
    #[gpui::test]
    fn a_scroll_step_builds_only_the_parts_on_screen(cx: &mut TestAppContext) {
        let (_dir, _w, view, model, cx) = open_with(cx, |_, _| {});
        machines(&model, 40, cx);
        let parts = view.read_with(cx, |v, _| v.parts.len());
        assert!(parts > 40, "{parts} parts");
        let built = |cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.built);
        let before = drawn_rows(cx, 40);
        assert!(!before.is_empty() && before.len() < 20, "only some rows on screen: {}", before.len());
        let (first, top) = before[before.len() - 1];

        for step in 0..3 {
            let b = built(cx);
            scroll(cx, -40.);
            let n = built(cx) - b;
            assert!(n > 0 && n <= 20, "step {step}: {n} of {parts} parts built");
        }
        let moved = cx.debug_bounds(selector(format!("peers-machine-{first}"))).expect("still on screen");
        assert_eq!(moved.top(), top.top() - gpui::px(120.), "scrolled by three steps");

        // The model changes: the page follows (every part measured again).
        let b = built(cx);
        let id = format!("n{first}");
        model.update(cx, |m, cx| {
            m.found = Some(vec![Candidate {
                stable_id: id.clone(),
                label: format!("mac-{first:02}"),
                dns: "mac.example.ts.net".into(),
                os: "macOS".into(),
                online: true,
                paired: None,
                probe: Probe::Pitwall { version: "0.2.1".into(), label: "mac".into(), pairing: Pairing::Open, agents: 1 },
            }]);
            m.settings.show = true;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(built(cx) - b >= parts, "measured again: {}", built(cx) - b);
        assert!(cx.debug_bounds(selector(format!("peers-pair-{first}"))).is_some(), "Pair, where it scrolled to");
        let still = cx.debug_bounds(selector(format!("peers-machine-{first}"))).expect("on screen");
        assert_eq!(still.top(), moved.top(), "the change kept the scroll position");

        // Machines go: their rows leave the list.
        machines(&model, 3, cx);
        // (The machine found above stays, as "not on Tailscale".)
        assert_eq!(view.read_with(cx, |v, _| v.parts.len()), parts - 36);
        assert!(drawn_rows(cx, 40).iter().all(|(i, _)| *i < 4));
    }

    /// The port field keeps its text and focus scrolled out of view, and
    /// typing still reaches it.
    #[gpui::test]
    fn the_port_field_keeps_focus_scrolled_away_and_back(cx: &mut TestAppContext) {
        let (_dir, _w, view, model, cx) = open_with(cx, |_, _| {});
        machines(&model, 40, cx);
        let input = view.read_with(cx, |v, _| v.peers.port.clone().expect("the port field"));
        let focus = input.read_with(cx, gpui::Focusable::focus_handle);
        cx.update(|window, cx| window.focus(&focus, cx));
        cx.run_until_parked();
        cx.simulate_input("1");
        let share = cx.debug_bounds("peers-share").expect("the switches");
        for _ in 0..10 {
            scroll(cx, -80.);
        }
        // Its part is still drawn (gpui's list keeps a focused item drawn,
        // under the ones on screen, out of view).
        let away = cx.debug_bounds("peers-share").expect("kept drawn while its field has focus");
        assert!(away.top() >= gpui::px(super::super::view::HEIGHT), "out of view: {away:?}");
        assert!(cx.update(|window, _| focus.is_focused(window)), "the field kept focus");
        cx.simulate_input("2");
        assert_eq!(input.read_with(cx, |i, _| i.text().to_string()), "4742012");
        for _ in 0..10 {
            scroll(cx, 80.);
        }
        assert_eq!(cx.debug_bounds("peers-share"), Some(share), "back");
        assert!(cx.update(|window, _| focus.is_focused(window)));
        assert_eq!(input.read_with(cx, |i, _| i.text().to_string()), "4742012");
        // Same field, not a new one.
        assert_eq!(view.read_with(cx, |v, _| v.peers.port.as_ref().map(|p| p.entity_id())), Some(input.entity_id()));
    }

    /// Discovery that fails (Tailscale went away) isn't started again by
    /// each redraw; Refresh tries again.
    #[gpui::test]
    fn a_failed_discovery_waits_for_refresh(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let (_dir, w, _view, model, cx) = open_with(cx, |w, peers| {
            peers.set_settings(PeerSettings { show: true, ..peers.settings() }).unwrap();
            w.set_down(true);
        });
        for _ in 0..5 {
            frame(cx);
        }
        model.read_with(cx, |m, _| {
            assert!(m.discover_error.is_some(), "discovery failed");
            assert_eq!(m.discoveries, 1, "and wasn't started again");
        });
        assert!(cx.debug_bounds("peers-discover-error").is_some(), "the failure shows");

        w.set_down(false);
        let refresh = cx.debug_bounds("peers-refresh").expect("Refresh");
        cx.simulate_click(refresh.center(), Modifiers::none());
        for _ in 0..3 {
            frame(cx);
        }
        model.read_with(cx, |m, _| {
            assert_eq!(m.discoveries, 2, "Refresh looked again");
            assert!(m.discover_error.is_none() && m.found.is_some(), "{:?}", m.discover_error);
        });
        assert!(cx.debug_bounds("peers-discover-error").is_none());
    }

    #[gpui::test]
    fn alone_on_the_tailnet_it_says_so_and_the_switches_work(cx: &mut TestAppContext) {
        let (_dir, model, cx) = open(cx);
        model.read_with(cx, |m, _| {
            assert_eq!(m.info.as_ref().map(|i| i.state), Some(TailnetState::Ready));
            assert!(m.machines.is_empty());
        });
        assert!(cx.debug_bounds("peers-no-machines").is_some(), "no other machines");
        assert!(cx.debug_bounds("peers-this-mac").is_some() && cx.debug_bounds("peers-viewers-empty").is_some());
        assert!(cx.debug_bounds("peers-refresh").is_some(), "Refresh");
        let share = cx.debug_bounds("peers-share").expect("the share switch");
        cx.simulate_click(share.center(), Modifiers::none());
        frame(cx);
        model.read_with(cx, |m, _| assert!(m.settings.share && !m.settings.show));
        assert!(model.read_with(cx, |m, _| m.peers().settings().share), "saved");
    }

    #[gpui::test]
    fn tailscale_missing_and_a_machine_to_pair(cx: &mut TestAppContext) {
        let (_dir, model, cx) = open(cx);
        model.update(cx, |m, cx| {
            if let Some(i) = m.info.as_mut() {
                i.state = TailnetState::NotInstalled;
            }
            cx.notify();
        });
        frame(cx);
        assert!(cx.debug_bounds("peers-tailnet-empty").is_some());
        model.update(cx, |m, cx| {
            m.settings.show = true;
            m.found = Some(vec![Candidate {
                stable_id: "nSTUDIO".into(),
                label: "studio".into(),
                dns: "studio.example.ts.net".into(),
                os: "macOS".into(),
                online: true,
                paired: None,
                probe: Probe::Pitwall { version: "0.2.1".into(), label: "studio".into(), pairing: Pairing::Open, agents: 2 },
            }]);
            cx.notify();
        });
        frame(cx);
        assert!(cx.debug_bounds("peers-pair-0").is_some(), "Pair for studio");
        assert!(cx.debug_bounds("peers-machine-0").is_some(), "studio in the list");
        assert_eq!(found_words(&model.read_with(cx, |m, _| m.found.clone().unwrap())[0]), "Pitwall 0.2.1 · 2 agents");
    }
}
