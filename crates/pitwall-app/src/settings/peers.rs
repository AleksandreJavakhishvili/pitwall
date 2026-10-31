//! Settings → Peers (docs/spec/tailnet-peers.md): the user's other
//! machines over Tailscale. The two switches ("Let my other machines see
//! this Mac's agents", "Show my other machines' agents") and the port; one
//! list of the user's machines (this Mac first): online or not, its
//! Pitwall, the pairing and its link, with Pair / Pair again / Unpair, and
//! Refresh to ask Tailscale and the machines again; machines watching this
//! one with Revoke; the recent audit log. All of it is [`crate::peers::PeersModel`]; changes the
//! `pitwall peers` CLI makes show up here live.

use gpui::{div, prelude::*, px, Context, Div, Entity, SharedString, Subscription};

use pitwall_peer::{Candidate, PeerEntry, PeerSettings, Probe, TailnetState};
use pitwall_proto::peer::LinkState;

use super::view::SettingsView;
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
            ts = ts.child(div().id("peers-tailnet-empty").debug_selector(|| "peers-tailnet-empty".into()).child(crate::kit::muted(words, t)));
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
        out.push(sw);

        // The machines: this Mac, then the user's others.
        let mr = model.downgrade();
        let checking = discovering || info.is_none();
        let mut sec = section().child(
            head(ui, "Machines").child(
                crate::kit::text_button("peers-refresh", if checking { "Checking…" } else { "Refresh" }, BtnKind::GhostSm, checking, t)
                    .debug_selector(|| "peers-refresh".into())
                    .tooltip(tooltip("Ask Tailscale again and look for your machines' Pitwall"))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let _ = mr.update(cx, |m, cx| m.refresh_now(cx));
                    })),
            ),
        );
        if s.show && found.is_none() && !discovering {
            model.update(cx, |m, cx| m.discover(false, cx));
        }
        let here = info.as_ref().and_then(|i| i.machine.clone()).unwrap_or_else(|| "This Mac".into());
        let here_words = match (s.share, listening.is_empty(), &listen_error) {
            (false, ..) => "this Mac · not shared".to_string(),
            (true, false, _) => format!("this Mac · shared on {}", listening.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")),
            (true, true, Some(e)) => format!("this Mac · not listening: {e}"),
            (true, true, None) => "this Mac · starting…".to_string(),
        };
        let ready = info.as_ref().is_some_and(|i| i.state == TailnetState::Ready);
        sec = sec.child(listed_row(ui, "peers-this-mac".into(), here, here_words, ready).debug_selector(|| "peers-this-mac".into()));
        let rows = machine_rows(&machines, found.as_deref(), &entries, s.show, now);
        if let Some(e) = &discover_error {
            sec = sec.child(crate::kit::error_text(e.clone(), t));
        }
        for (i, r) in rows.iter().enumerate() {
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
            sec = sec.child(row);
        }
        if rows.is_empty() && ready {
            sec = sec.child(div().id("peers-no-machines").debug_selector(|| "peers-no-machines".into()).child(crate::kit::muted(
                "No other machines of yours are on this tailnet. Sign in to Tailscale with the same account on your other Mac to see its agents here.",
                t,
            )));
        } else if !rows.is_empty() {
            sec = sec.child(crate::kit::hint(
                "A machine can be paired once its Pitwall has “Let my other machines see this Mac's agents” on.",
                t,
            ));
        }
        out.push(sec);

        // Machines watching this one.
        let mut sec = section().child(head(ui, "Can see this Mac"));
        if viewers.is_empty() {
            sec = sec.child(div().id("peers-viewers-empty").debug_selector(|| "peers-viewers-empty".into()).child(crate::kit::hint("No machine sees this Mac's agents.", t)));
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

    use gpui::{AppContext as _, Modifiers, TestAppContext, VisualTestContext};
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
        let root = dir.path().to_path_buf();
        let model = cx.update(|cx| {
            cx.set_global(crate::theme::Theme::dark());
            crate::ui_state::init(None, cx);
            crate::settings::data::set_page(cx, Page::Peers);
            let m = cx.new(|_| PeersModel::new(peers, tailnet, root));
            cx.set_global(ModelHandle(m.clone()));
            m
        });
        let (_view, cx) = cx.add_window_view(|_, cx| SettingsView::new(true, cx));
        cx.simulate_resize(gpui::size(gpui::px(1600.), gpui::px(1000.)));
        frame(cx);
        (dir, model, cx)
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
