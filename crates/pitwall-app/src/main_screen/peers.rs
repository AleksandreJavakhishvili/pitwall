//! The user's other machines on the main screen (docs/spec/tailnet-peers.md,
//! "The app"): their machine headings in the sidebar say how the link is
//! ("studio · connected", "offline · last seen 5 min ago"; a paired machine
//! without agents still gets its section, `model::with_paired`); the
//! sidebar card offering to show them;
//! the one-time offer toast per machine; and the pane of one of their
//! agents, a read-only summary (their terminals stay on their machine).

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, IntoElement, SharedString};

use pitwall_proto::peer::{LinkState, LinkView};
use pitwall_proto::AgentView;

use super::strip::{self, Toast, ToastAction};
use super::MainScreen;
use crate::agents::status_word;
use crate::kit::{tooltip, BtnKind, MONO_FONT};
use crate::peers::{link_text, now_ms, offer_title, PeersEvent};
use crate::theme::{Theme, RADIUS};

/// The dot's colour for a link state.
pub(super) fn link_color(state: LinkState, t: &Theme) -> gpui::Hsla {
    match state {
        LinkState::Connected => t.green,
        LinkState::Connecting => t.text_3,
        LinkState::Offline => t.text_4,
        LinkState::IdentityChanged => t.amber,
    }
}

/// Under a machine's name in its heading: "this Mac", "agw machine", or
/// for another machine's Pitwall how the link is and that it's view only.
pub(super) fn machine_sub(label: &str, home: bool, link: Option<&LinkView>, t: &Theme) -> AnyElement {
    let base = div().min_w_0().text_size(px(10.5)).text_color(t.text_4);
    let Some(link) = link else {
        return base.child(if home { "this Mac" } else { "agw machine" }).into_any_element();
    };
    let words = link_text(link, now_ms());
    base.id(SharedString::from(format!("peer-link-{}-{label}", link.peer)))
        .debug_selector({
            let peer = link.peer.clone();
            move || format!("peer-heading-{peer}")
        })
        .tooltip(tooltip(format!("{label}'s Pitwall: {words}. You can watch its agents here, not drive them.")))
        .child(crate::kit::one_line(format!("{words} · view only")))
        .into_any_element()
}

impl MainScreen {
    /// An offer from the peers model, as a toast with Show / Not now.
    pub(super) fn peer_event(&mut self, e: &PeersEvent, cx: &mut Context<Self>) {
        match e {
            PeersEvent::Offer(c) => {
                let (yes, no) = (c.clone(), c.clone());
                let actions = vec![
                    ToastAction {
                        label: "Show".into(),
                        primary: true,
                        run: std::rc::Rc::new(move |cx| {
                            if let Some(m) = crate::peers::model(cx) {
                                m.update(cx, |m, cx| m.answer(&yes, true, cx));
                            }
                        }),
                    },
                    ToastAction {
                        label: "Not now".into(),
                        primary: false,
                        run: std::rc::Rc::new(move |cx| {
                            if let Some(m) = crate::peers::model(cx) {
                                m.update(cx, |m, cx| m.answer(&no, false, cx));
                            }
                        }),
                    },
                ];
                let toast = Toast::ask(
                    format!("peer-offer-{}", c.stable_id),
                    format!("{} — show them here?", offer_title(c)),
                    Some("Read-only: their status and needs-you, next to your agents.".into()),
                    actions,
                );
                self.toast(toast, cx);
            }
            PeersEvent::Answered(id) => strip::drop_key(self, &format!("peer-offer-{id}"), cx),
        }
    }

    /// Below the agents: the card offering to show the other machines'
    /// agents.
    pub(super) fn render_peers(&mut self, t: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(model) = crate::peers::model(cx) else {
            return vec![];
        };
        let m = model.read(cx);
        let mut out = Vec::new();
        if m.card() {
            let names: Vec<String> = m.machines.iter().map(|n| n.label.clone()).collect();
            let who = match names.len() {
                1 => names[0].clone(),
                2 => format!("{} and {}", names[0], names[1]),
                n => format!("{} and {} more", names[0], n - 1),
            };
            let (on, off) = (model.downgrade(), model.downgrade());
            out.push(
                div()
                    .id("peer-card")
                    .debug_selector(|| "peer-card".into())
                    .mx(px(8.))
                    .mt(px(10.))
                    .p(px(10.))
                    .rounded(RADIUS)
                    .border_1()
                    .border_color(t.line_strong)
                    .bg(t.surface_2)
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .text_size(px(12.5))
                    .child(div().font_weight(FontWeight::SEMIBOLD).text_color(t.text).child("Your other machines"))
                    .child(div().text_color(t.text_2).child(format!(
                        "{who} {} on your Tailscale. Show their Pitwall agents here, read-only?",
                        if names.len() == 1 { "is" } else { "are" }
                    )))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(
                                crate::kit::text_button("peer-card-show", "Show them", BtnKind::Small, false, t)
                                    .debug_selector(|| "peer-card-show".into())
                                    .tooltip(tooltip("Turns on “Show my other machines' agents” (Settings → Peers)"))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        let _ = on.update(cx, |m, cx| m.answer_card(true, cx));
                                    })),
                            )
                            .child(
                                crate::kit::text_button("peer-card-no", "Not now", BtnKind::GhostSm, false, t)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        let _ = off.update(cx, |m, cx| m.answer_card(false, cx));
                                    })),
                            ),
                    )
                    .into_any_element(),
            );
        }
        out
    }
}

/// The pane of an agent on another machine: what is known of it, read-only
/// (its terminal stays on its machine in this version).
pub(super) fn remote_details(a: &AgentView, t: &Theme) -> AnyElement {
    let link = a.machine.link.as_ref().map(|l| link_text(l, now_ms())).unwrap_or_default();
    let row = |k: &str, v: String| {
        div()
            .flex()
            .gap(px(10.))
            .child(div().flex_none().w(px(84.)).text_color(t.text_4).child(k.to_string()))
            .child(div().flex_1().min_w_0().text_color(t.text_2).child(v))
    };
    let mut d = div()
        .id(SharedString::from(format!("peer-details-{}", a.id)))
        .debug_selector({
            let id = a.id.clone();
            move || format!("peer-details-{id}")
        })
        .flex_1()
        .flex()
        .flex_col()
        .gap(px(6.))
        .p(px(18.))
        .text_size(px(12.5))
        .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(14.)).text_color(t.text).child(a.name.clone()))
        .child(row("Status", match &a.status_detail {
            Some(d) => format!("{} · {d}", status_word(a.status)),
            None => status_word(a.status).to_string(),
        }))
        .child(row("Machine", format!("{} · {link}", a.machine.label)))
        .child(row("Agent", a.kind_name.clone()))
        .child(row("Folder", a.cwd_display.clone()));
    if let Some(b) = &a.branch {
        d = d.child(row("Branch", b.clone()));
    }
    if a.added + a.removed > 0 {
        d = d.child(row("Changes", format!("+{} −{} in {} files", a.added, a.removed, a.files_changed)));
    }
    if let Some(s) = &a.last_sent {
        d = d.child(row("Last sent", s.clone()));
    }
    d.child(
        div()
            .pt(px(8.))
            .text_size(px(12.))
            .text_color(t.text_3)
            .font_family(MONO_FONT)
            .child(format!("Read-only: open it on {} to type, stop or review.", a.machine.label.split(" › ").next().unwrap_or(&a.machine.label))),
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext};
    use pitwall_core::testing::TempDir;
    use pitwall_peer::fake::FakeWorld;
    use pitwall_peer::{Candidate, NullPeerSink, Peers, PeersConfig, Probe};
    use pitwall_proto::peer::{LinkState, LinkView, Pairing};

    use super::super::{init, MainScreen};
    use crate::agents::tests::agent;
    use crate::agents::AgentStore;
    use crate::peers::{ModelHandle, PeersEvent, PeersModel};

    /// `studio`'s agent as the mirror shows it: every cap off, its link.
    pub fn mirrored(id: &str, status: &str, state: LinkState) -> pitwall_proto::AgentView {
        let mut a = agent(&format!("peer:studio:{id}"), "/Users/me/api", status, 5, false);
        a.name = id.into();
        a.machine.provider = "peer:studio".into();
        a.machine.id = "studio-mac".into();
        a.machine.label = "studio".into();
        a.machine.link = Some(LinkView { peer: "studio".into(), state, since: 0, last_seen: Some(1) });
        a
    }

    struct Setup {
        _dir: TempDir,
        model: Entity<PeersModel>,
    }

    /// Peers on a fake tailnet (never started: nothing dials or listens).
    fn peers_model(cx: &mut TestAppContext) -> Setup {
        let dir = TempDir::new("app-peers-screen");
        let w = FakeWorld::new();
        w.add("nLAPTOP", "laptop", "alice@example.com").add("nSTUDIO", "studio", "alice@example.com");
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
            let m = cx.new(|_| PeersModel::new(peers, tailnet, root));
            cx.set_global(ModelHandle(m.clone()));
            m
        });
        Setup { _dir: dir, model }
    }

    fn screen(cx: &mut TestAppContext) -> (Setup, Entity<MainScreen>, Entity<AgentStore>, &mut VisualTestContext) {
        cx.update(|cx| {
            cx.set_global(crate::theme::Theme::dark());
            init(cx);
        });
        let setup = peers_model(cx);
        let store = cx.new(|_| {
            AgentStore::new(vec![
                agent("api", "/work/alpha", "idle", 1, true),
                mirrored("a1", "blocked", LinkState::Connected),
            ])
        });
        let s = store.clone();
        let (view, cx) = cx.add_window_view(move |window, cx| MainScreen::new(s.clone(), window, cx));
        cx.simulate_resize(gpui::size(gpui::px(1400.), gpui::px(900.)));
        frame(cx);
        (setup, view, store, cx)
    }

    fn frame(cx: &mut VisualTestContext) {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }

    #[gpui::test]
    fn another_machines_agents_sit_under_its_heading_read_only(cx: &mut TestAppContext) {
        let (_s, view, _store, cx) = screen(cx);
        assert!(cx.debug_bounds("peer-heading-studio").is_some(), "studio's heading, with its link");
        // Opened: what is known of it, no terminal.
        view.update_in(cx, |s, window, cx| s.show_agent("peer:studio:a1", window, cx));
        frame(cx);
        assert!(cx.debug_bounds("peer-details-peer:studio:a1").is_some(), "the read-only details");
        // Nothing to rename or remove here.
        view.update_in(cx, |s, window, cx| {
            let a = s.store.read(cx).agent("peer:studio:a1").cloned().unwrap();
            s.agent_menu(&a, false, gpui::point(gpui::px(10.), gpui::px(10.)), window, cx);
            assert!(s.menu_labels().is_empty(), "{:?}", s.menu_labels());
            s.open_remove("peer:studio:a1", cx);
            assert!(s.modal.is_none(), "no Remove dialog");
            s.start_agent_rename("peer:studio:a1", window, cx);
            assert!(s.agent_rename.is_none(), "no rename");
        });
        // A local agent still has both.
        view.update_in(cx, |s, window, cx| {
            let a = s.store.read(cx).agent("api").cloned().unwrap();
            s.agent_menu(&a, true, gpui::point(gpui::px(10.), gpui::px(10.)), window, cx);
            assert!(s.menu_labels().contains(&"Rename…".to_string()));
        });
    }

    #[gpui::test]
    fn an_offer_asks_once_and_not_now_is_remembered(cx: &mut TestAppContext) {
        let (s, view, _store, cx) = screen(cx);
        let c = Candidate {
            stable_id: "nSTUDIO".into(),
            label: "studio".into(),
            dns: "studio.example.ts.net".into(),
            os: "macOS".into(),
            online: true,
            paired: None,
            probe: Probe::Pitwall { version: "0.2.1".into(), label: "studio".into(), pairing: Pairing::Open, agents: 3 },
        };
        s.model.update(cx, |_, cx| cx.emit(PeersEvent::Offer(c)));
        frame(cx);
        let titles = view.read_with(cx, |s, _| s.toast_titles());
        assert_eq!(titles, ["studio is running Pitwall with 3 agents — show them here?"]);
        // Not now (the second button).
        let b = cx.debug_bounds("toast-act-1").expect("Not now");
        cx.simulate_click(b.center(), Modifiers::none());
        frame(cx);
        assert!(view.read_with(cx, |s, _| s.toast_titles()).is_empty());
        s.model.read_with(cx, |m, _| assert!(m.offers.asked.contains("nSTUDIO")));
        assert!(crate::peers::Offers::load(s._dir.path()).asked.contains("nSTUDIO"), "kept on disk");
    }

    #[gpui::test]
    fn a_paired_machine_without_agents_keeps_its_heading(cx: &mut TestAppContext) {
        let (s, _view, store, cx) = screen(cx);
        store.update(cx, |st, cx| {
            st.set_peers(vec![], 0);
            cx.notify();
        });
        s.model.update(cx, |m, cx| {
            m.entries = vec![pitwall_peer::PeerEntry {
                paired: pitwall_peer::Paired {
                    id: "studio".into(),
                    label: "studio".into(),
                    stable_id: "nSTUDIO".into(),
                    login: "alice@example.com".into(),
                    public_key: "ed25519:x".into(),
                    added_at: 0,
                    last_seen: Some(1),
                },
                link: Some(LinkView { peer: "studio".into(), state: LinkState::Offline, since: 0, last_seen: Some(1) }),
                agents: 0,
                blocked: 0,
            }];
            cx.notify();
        });
        frame(cx);
        assert!(cx.debug_bounds("peer-heading-studio").is_some());
    }

    #[gpui::test]
    fn this_mac_and_a_peer_are_sections_and_the_peers_is_read_only(cx: &mut TestAppContext) {
        let (_s, view, store, cx) = screen(cx);
        store.update(cx, |st, cx| {
            st.set_peers(vec![mirrored("a1", "blocked", LinkState::Connected), mirrored("a2", "idle", LinkState::Connected)], 1);
            cx.notify();
        });
        frame(cx);
        let mine = cx.debug_bounds("machine-head-machine:this-mac").expect("this Mac's heading");
        let theirs = cx.debug_bounds("machine-head-machine:peer:studio:studio-mac").expect("studio's");
        let row = cx.debug_bounds("row-body-peer:studio:a1").expect("studio's agent");
        let api = cx.debug_bounds("row-body-api").expect("this Mac's agent");
        assert!(mine.origin.y < api.origin.y && api.origin.y < theirs.origin.y && theirs.origin.y < row.origin.y, "this Mac's first, studio's under its heading");
        // Nothing is dragged into, out of or within studio's section.
        view.update_in(cx, |s, window, cx| {
            assert!(!s.drop_agent_on_row("peer:studio:a2", "peer:studio:a1", true, window, cx), "within");
            assert!(!s.drop_agent_on_row("peer:studio:a1", "api", true, window, cx), "out");
            assert!(!s.drop_agent_on_row("api", "peer:studio:a1", true, window, cx), "in");
            assert!(s.ui.sidebar_order.is_empty());
        });
        // Folding studio folds its agents (keyboard moves skip them).
        let rows = |view: &Entity<MainScreen>, cx: &mut VisualTestContext| view.read_with(cx, |s, cx| s.visible_rows(s.agents(cx), cx));
        assert_eq!(rows(&view, cx).len(), 3);
        view.update(cx, |s, cx| s.update_ui(|u| u.toggle_collapsed("machine:peer:studio:studio-mac"), cx));
        assert_eq!(rows(&view, cx), ["api"]);
        view.read_with(cx, |s, _| assert!(s.ui.collapsed.contains(&"machine:peer:studio:studio-mac".to_string()), "remembered"));
    }

    #[gpui::test]
    fn a_peers_agents_say_view_only_and_the_rail_groups_by_machine(cx: &mut TestAppContext) {
        let (_s, _view, store, cx) = screen(cx);
        store.update(cx, |st, cx| {
            st.set_peers(vec![mirrored("a1", "blocked", LinkState::Connected)], 1);
            cx.notify();
        });
        frame(cx);
        assert!(cx.debug_bounds("peer-heading-studio").is_some(), "studio's heading says how the link is");
        // The narrow rail: each machine's badge above its agents.
        cx.simulate_resize(gpui::size(gpui::px(800.), gpui::px(900.)));
        frame(cx);
        let mine = cx.debug_bounds("rail-m-machine:this-mac").expect("this Mac's badge");
        let theirs = cx.debug_bounds("rail-m-machine:peer:studio:studio-mac").expect("studio's badge");
        let row = cx.debug_bounds("rail-peer:studio:a1").expect("studio's agent");
        let api = cx.debug_bounds("rail-api").expect("this Mac's agent");
        assert!(mine.origin.y < api.origin.y && api.origin.y < theirs.origin.y && theirs.origin.y < row.origin.y);
    }

    #[gpui::test]
    fn grouped_by_account_a_peers_agents_keep_its_accounts(cx: &mut TestAppContext) {
        let (_s, view, store, cx) = screen(cx);
        store.update(cx, |st, cx| {
            let mut work = mirrored("a2", "idle", LinkState::Connected);
            work.account = Some("default-looking-id".into());
            work.account_name = Some("Work".into());
            st.set_peers(vec![mirrored("a1", "blocked", LinkState::Connected), work], 1);
            cx.notify();
        });
        view.update(cx, |s, cx| s.set_sidebar_group(crate::main_screen::workspace::SidebarGroup::Account, cx));
        frame(cx);
        let sections = view.read_with(cx, |s, cx| s.machine_sections(s.agents(cx), cx));
        assert_eq!(sections.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(), ["This Mac", "studio"]);
        let keys: Vec<&str> = sections[0].accounts.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, ["acct:default"], "not under this Mac's Default");
        assert_eq!(sections[0].accounts[0].agents, 1);
        let names: Vec<Option<&str>> = sections[1].accounts.iter().map(|a| a.peer_name.as_deref()).collect();
        assert_eq!(names, [Some("Default"), Some("Work")], "studio's own accounts, by the names it gave");
        assert!(cx.debug_bounds("row-body-peer:studio:a2").is_some());
        assert!(cx.debug_bounds("machine-head-machine:peer:studio:studio-mac").is_some());
    }

    #[gpui::test]
    fn the_card_offers_showing_until_answered(cx: &mut TestAppContext) {
        let (s, _view, _store, cx) = screen(cx);
        assert!(cx.debug_bounds("peer-card").is_none(), "no machines known yet");
        s.model.update(cx, |m, cx| {
            m.machines = vec![crate::peers::Machine { stable_id: "nSTUDIO".into(), label: "studio".into(), online: true }];
            cx.notify();
        });
        frame(cx);
        let b = cx.debug_bounds("peer-card-show").expect("the card");
        cx.simulate_click(b.center(), Modifiers::none());
        frame(cx);
        // (gpui's test bounds keep what was drawn once: the state says it.)
        s.model.read_with(cx, |m, _| assert!(m.offers.card_done && m.settings.show && !m.card()));
    }
}
