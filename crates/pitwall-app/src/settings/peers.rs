//! Settings → Peers (docs/spec/tailnet-peers.md): the user's other
//! machines over Tailscale. The two switches ("Let my other machines see
//! this Mac's agents", "Show my other machines' agents") and the port;
//! machines found running a Pitwall that shares, with Pair; paired machines
//! with their link and Unpair; machines watching this one with Revoke; the
//! recent audit log. All of it is [`crate::peers::PeersModel`]; changes the
//! `pitwall peers` CLI makes show up here live.

use gpui::{div, prelude::*, px, Context, Div, Entity, SharedString, Subscription};

use pitwall_peer::{Candidate, PeerSettings, Probe, TailnetState};
use pitwall_proto::peer::LinkState;

use super::view::SettingsView;
use super::widgets::Ui;
use crate::kit::{tooltip, BtnKind, InputEvent, TextInput, Tone};
use crate::peers::{ago, link_text, now_ms, PeersModel};

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

    /// The Peers page's sections.
    pub(super) fn peers_page(&mut self, ui: &Ui, cx: &mut Context<Self>) -> Vec<Div> {
        let Some(model) = crate::peers::model(cx) else {
            return vec![section().child(crate::kit::muted("Your other machines need Pitwall's engine, which isn't running.", &ui.t))];
        };
        if !self.peers.checked {
            self.peers.checked = true;
            self.peers._subs.push(cx.observe(&model, |_, _, cx| cx.notify()));
            model.update(cx, |m, cx| m.check(cx));
        }
        let t = &ui.t;
        let m = model.read(cx);
        let s = m.settings;
        let info = m.info.clone();
        let machines = m.machines.clone();
        let (entries, viewers, audit) = (m.entries.clone(), m.viewers.clone(), m.audit.clone());
        let (found, discovering, discover_error) = (m.found.clone(), m.discovering, m.discover_error.clone());
        let (busy, error) = (m.busy.clone(), m.error.clone());
        let port_input = self.peers_port(&model, s.port, cx);
        let now = now_ms();
        let mut out = Vec::new();

        // Tailscale.
        let mut ts = section().child(head(ui, "Tailscale").child(match &info {
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
        if let Some(words) = info.as_ref().and_then(|i| tailnet_words(i.state)) {
            ts = ts.child(div().id("peers-tailnet-empty").child(crate::kit::muted(words, t)));
        } else if info.is_some() && machines.is_empty() {
            ts = ts.child(div().id("peers-no-machines").child(crate::kit::muted(
                "No other machines of yours are on this tailnet. Sign in to Tailscale with the same account on your other Mac to see its agents here.",
                t,
            )));
        }
        ts = ts.child(crate::kit::hint(
            "Only your own machines take part: the same Tailscale user, never tagged devices or other people's machines. Their agents are read-only here.",
            t,
        ));
        out.push(ts);

        // The switches and the port.
        let m2 = model.downgrade();
        let m3 = model.downgrade();
        let listening = info.as_ref().map(|i| i.listening.clone()).unwrap_or_default();
        let listen_error = info.as_ref().and_then(|i| i.listen_error.clone());
        let mut sw = section()
            .child(
                head(ui, "Let my other machines see this Mac's agents").child(
                    crate::kit::toggle("peers-share", s.share, t)
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
                    (true, true) => listen_error.clone().unwrap_or_else(|| "Starting…".into()),
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
        out.push(sw);

        // Discovered machines.
        if s.show {
            let md = model.downgrade();
            let mut sec = section().child(
                head(ui, "Your machines").child(
                    crate::kit::text_button("peers-discover", if discovering { "Looking…" } else { "Look again" }, BtnKind::GhostSm, discovering, t)
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let _ = md.update(cx, |m, cx| m.discover(false, cx));
                        })),
                ),
            );
            if found.is_none() && !discovering {
                model.update(cx, |m, cx| m.discover(false, cx));
            }
            match (&found, &discover_error) {
                (_, Some(e)) => sec = sec.child(crate::kit::error_text(e.clone(), t)),
                (None, None) => sec = sec.child(crate::kit::hint("Looking for your machines…", t)),
                (Some(list), None) if list.is_empty() => {
                    sec = sec.child(div().id("peers-found-empty").child(crate::kit::muted("No other machines of yours on Tailscale.", t)))
                }
                (Some(list), None) => {
                    for (i, c) in list.iter().enumerate() {
                        let mut r = machine_row(ui, SharedString::from(format!("peers-found-{i}")), c.label.clone(), found_words(c));
                        let can_pair = matches!(c.probe, Probe::Pitwall { .. }) && c.paired.is_none();
                        if c.paired.is_some() {
                            r = r.child(crate::kit::chip_tone("paired", Tone::Ok, t));
                        } else if can_pair {
                            let (id, mm) = (c.stable_id.clone(), model.downgrade());
                            let b = busy.contains(&c.stable_id);
                            r = r.child(
                                crate::kit::text_button(("peers-pair", i), if b { "Pairing…" } else { "Pair" }, BtnKind::Small, b, t)
                                    .tooltip(tooltip("Show its agents here (read-only)"))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        let _ = mm.update(cx, |m, cx| m.pair(&id, cx));
                                    })),
                            );
                        }
                        sec = sec.child(r);
                    }
                    sec = sec.child(crate::kit::hint(
                        "A machine shows up here once its Pitwall has “Let my other machines see this Mac's agents” on.",
                        t,
                    ));
                }
            }
            out.push(sec);
        }

        // Paired machines.
        let mut sec = section().child(head(ui, "Shown here"));
        if entries.is_empty() {
            sec = sec.child(div().id("peers-paired-empty").child(crate::kit::hint("No paired machines.", t)));
        }
        for (i, e) in entries.iter().enumerate() {
            let p = &e.paired;
            let state = match &e.link {
                Some(l) => link_text(l, now),
                None => "not shown (showing is off)".into(),
            };
            let sub = format!("{state} · {} agent{}", e.agents, if e.agents == 1 { "" } else { "s" });
            let (id, mm) = (p.id.clone(), model.downgrade());
            let b = busy.contains(&p.id);
            let mut r = machine_row(ui, SharedString::from(format!("peers-paired-{i}")), p.label.clone(), sub);
            if e.link.as_ref().is_some_and(|l| l.state == LinkState::IdentityChanged) {
                let (sid, mp) = (p.stable_id.clone(), model.downgrade());
                r = r.child(
                    crate::kit::text_button(("peers-repair", i), "Pair again", BtnKind::Small, b, t).on_click(cx.listener(move |_, _, _, cx| {
                        let _ = mp.update(cx, |m, cx| m.pair(&sid, cx));
                    })),
                );
            }
            r = r.child(
                crate::kit::text_button(("peers-unpair", i), "Unpair", BtnKind::GhostSm, b, t)
                    .tooltip(tooltip("Stop showing its agents and forget its key"))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let _ = mm.update(cx, |m, cx| m.unpair(&id, cx));
                    })),
            );
            sec = sec.child(r);
        }
        out.push(sec);

        // Machines watching this one.
        let mut sec = section().child(head(ui, "Can see this Mac"));
        if viewers.is_empty() {
            sec = sec.child(div().id("peers-viewers-empty").child(crate::kit::hint("No machine sees this Mac's agents.", t)));
        }
        for (i, v) in viewers.iter().enumerate() {
            let seen = v.last_seen.map(|t| format!("last connected {}", ago(t, now))).unwrap_or_else(|| "never connected".into());
            let (id, mm) = (v.id.clone(), model.downgrade());
            let b = busy.contains(&v.id);
            sec = sec.child(
                machine_row(ui, SharedString::from(format!("peers-viewer-{i}")), v.label.clone(), format!("{} · {seen}", v.login)).child(
                    crate::kit::text_button(("peers-revoke", i), "Revoke", BtnKind::GhostSm, b, t)
                        .tooltip(tooltip("Disconnect it now and forget it; it must pair again"))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let _ = mm.update(cx, |m, cx| m.revoke(&id, cx));
                        })),
                ),
            );
        }
        out.push(sec);

        // The audit log.
        if !audit.is_empty() {
            let mut sec = section().child(head(ui, "Recent activity"));
            for e in audit.iter().take(12) {
                let who = e.peer.clone().or_else(|| e.login.clone()).or_else(|| e.addr.clone()).unwrap_or_default();
                let what = [Some(e.event.replace('_', " ")), Some(who).filter(|w| !w.is_empty()), e.method.clone(), e.detail.clone()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ");
                sec = sec.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .child(div().flex_none().w(px(76.)).text_color(t.text_4).child(ago(e.at, now)))
                        .child(div().flex_1().min_w_0().text_color(t.text_2).child(what)),
                );
            }
            out.push(sec);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
