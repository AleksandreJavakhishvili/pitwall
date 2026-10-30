//! Peers in the app on a [`FakeTailnet`](pitwall_peer::fake) (no Tailscale,
//! everything on 127.0.0.1): offers, link words, the store's merge of the
//! peers' agents, attention, the badge count and CLI changes applied live.

use super::*;
use pitwall_proto::peer::Pairing;

fn cand(id: &str, label: &str, probe: Probe, paired: Option<&str>) -> Candidate {
    Candidate {
        stable_id: id.into(),
        label: label.into(),
        dns: format!("{label}.example.ts.net"),
        os: "macOS".into(),
        online: true,
        paired: paired.map(Into::into),
        probe,
    }
}

fn pitwall(agents: u32) -> Probe {
    Probe::Pitwall { version: "0.2.1".into(), label: "x".into(), pairing: Pairing::Open, agents }
}

#[test]
fn only_new_unpaired_pitwalls_are_offered_once() {
    let found = vec![
        cand("n1", "studio", pitwall(3), None),
        cand("n2", "mini", Probe::NoAnswer { message: "no".into() }, None),
        cand("n3", "old", pitwall(1), Some("old")),
        cand("n4", "air", pitwall(1), None),
    ];
    let mut offers = Offers::default();
    let ids: Vec<String> = to_offer(&found, &offers).into_iter().map(|c| c.stable_id).collect();
    assert_eq!(ids, ["n1", "n4"]);
    offers.asked.insert("n1".into());
    assert_eq!(to_offer(&found, &offers).len(), 1, "asked once, never again");
    assert_eq!(offer_title(&found[0]), "studio is running Pitwall with 3 agents");
    assert_eq!(offer_title(&found[3]), "air is running Pitwall with 1 agent");
}

#[test]
fn the_card_needs_other_machines_and_showing_off() {
    let off = PeerSettings::default();
    let on = PeerSettings { show: true, ..off };
    let m = vec![Machine { stable_id: "n1".into(), label: "studio".into(), online: false }];
    let mut offers = Offers::default();
    assert!(card(&off, &m, &offers));
    assert!(!card(&off, &[], &offers), "no other machines: nothing to offer");
    assert!(!card(&on, &m, &offers), "already showing");
    offers.card_done = true;
    assert!(!card(&off, &m, &offers), "answered");
}

#[test]
fn offers_are_remembered_in_the_data_folder() {
    let dir = pitwall_core::testing::TempDir::new("peers-offers");
    assert_eq!(Offers::load(dir.path()), Offers::default());
    let mut o = Offers::default();
    o.asked.insert("n1".into());
    o.card_done = true;
    o.save(dir.path());
    assert_eq!(Offers::load(dir.path()), o);
}

#[test]
fn link_words() {
    let now = 10 * 3_600_000;
    let link = |state, last_seen| LinkView { peer: "studio".into(), state, since: 0, last_seen };
    assert_eq!(link_text(&link(LinkState::Connected, None), now), "connected");
    assert_eq!(link_text(&link(LinkState::Connecting, None), now), "connecting…");
    assert_eq!(link_text(&link(LinkState::Offline, Some(now - 5 * 60_000)), now), "offline · last seen 5 min ago");
    assert_eq!(link_text(&link(LinkState::Offline, None), now), "offline");
    assert!(link_text(&link(LinkState::IdentityChanged, None), now).contains("pair again"));
    assert_eq!(ago(now - 2 * 3_600_000, now), "2 h ago");
    assert_eq!(ago(now, now), "just now");
}

mod live {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use gpui::{AppContext as _, Entity, TestAppContext};
    use pitwall_core::events::{Event, EventSink, NullSink};
    use pitwall_core::model::Attention;
    use pitwall_core::testing::TempDir;
    use pitwall_daemon::events::{self, Hub, Queue};
    use pitwall_peer::fake::FakeWorld;
    use pitwall_peer::{Feed, PeerSettings, Peers, PeersConfig, Timing};
    use pitwall_proto::AgentView;

    use crate::agents::tests::agent;
    use crate::agents::{AgentStore, StoreEvent};
    use crate::bridge::Bridge;

    /// `studio`'s agents without an engine: a list and the hub.
    struct TestFeed {
        views: Mutex<Vec<AgentView>>,
        hub: Arc<Hub>,
    }

    impl Feed for TestFeed {
        fn views(&self) -> Vec<AgentView> {
            self.views.lock().unwrap().clone()
        }

        fn subscribe(&self, topics: Vec<&'static str>) -> Arc<Queue> {
            let q = self.hub.subscribe(topics);
            events::snapshot_of(&self.views.lock().unwrap(), &q);
            q
        }
    }

    fn timing() -> Timing {
        let ms = Duration::from_millis;
        Timing { backoff: vec![ms(20), ms(50)], ping: ms(200), dead_after: Duration::from_secs(5), status_poll: ms(100), ..Timing::default() }
    }

    fn peers(world: &Arc<FakeWorld>, node: &str, dir: &TempDir, sink: Arc<dyn pitwall_peer::PeerSink>, feed: Option<Arc<dyn Feed>>) -> Arc<Peers> {
        Peers::new(PeersConfig {
            root: dir.path().to_path_buf(),
            tailnet: world.tailnet(node),
            version: "0.2.1".into(),
            sink,
            feed,
            timing: timing(),
            limits: Default::default(),
        })
        .unwrap()
    }

    /// Settle the app while the peers' threads work, until `f` holds.
    fn until(cx: &mut TestAppContext, what: &str, mut f: impl FnMut(&mut TestAppContext) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            cx.run_until_parked();
            if f(cx) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting until {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The laptop shows studio's agents next to its own, notifies for them
    /// named with the machine, counts them on the badge; the CLI's changes
    /// to `peers/` apply live.
    #[gpui::test]
    fn the_laptop_shows_studios_agents_live(cx: &mut TestAppContext) {
        let world = FakeWorld::new();
        world.add("nLAPTOP", "laptop", "alice@example.com").add("nSTUDIO", "studio", "alice@example.com");
        let (sdir, ldir) = (TempDir::new("app-peer-studio"), TempDir::new("app-peer-laptop"));
        let feed = Arc::new(TestFeed {
            views: Mutex::new(vec![agent("a1", "/work/api", "working", 1, true)]),
            hub: Hub::new(Arc::new(NullSink)),
        });
        let studio = peers(&world, "nSTUDIO", &sdir, Arc::new(pitwall_peer::NullPeerSink), Some(feed.clone()));
        studio.set_settings(PeerSettings { share: true, show: false, port: 0 }).unwrap();
        studio.start();

        let (bridge, rx) = Bridge::new();
        let laptop = peers(&world, "nLAPTOP", &ldir, Arc::new(bridge), None);
        laptop.set_settings(PeerSettings { share: false, show: true, port: 0 }).unwrap();
        let store: Entity<AgentStore> = cx.new(|_| AgentStore::new(vec![agent("mine", "/work/web", "idle", 1, true)]));
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let s = seen.clone();
        let model = cx.update(|cx| {
            AgentStore::listen(&store, rx, cx);
            cx.subscribe(&store, move |_, e: &StoreEvent, _| {
                if let StoreEvent::Attention { name, reason, .. } = e {
                    s.lock().unwrap().push(format!("{name} {reason}"));
                }
            })
            .detach();
            super::init(laptop.clone(), world.tailnet("nLAPTOP"), ldir.path().to_path_buf(), &store, cx)
        });
        laptop.pair("studio").unwrap();

        until(cx, "studio's agent is in the store", |cx| store.read_with(cx, |s, _| s.agents.len() == 2));
        store.read_with(cx, |s, _| {
            let a = s.agent("peer:studio:a1").expect("mirrored");
            assert_eq!(a.machine.label, "studio");
            assert_eq!(a.caps, pitwall_proto::AgentCaps::default(), "every cap off");
            assert_eq!(s.local().count(), 1);
        });
        until(cx, "the model has the paired machine", |cx| model.read_with(cx, |m, _| m.entries.len() == 1));

        // Needs you on studio: notified as "api on studio", badged.
        let mut blocked = agent("a1", "/work/api", "blocked", 1, true);
        blocked.name = "api".into();
        *feed.views.lock().unwrap() = vec![blocked.clone()];
        feed.hub.emit(Event::AgentsChanged(vec![blocked]));
        feed.hub.emit(Event::Attention(Attention { agent_id: "a1".into(), name: "api".into(), reason: "blocked", detail: None }));
        feed.hub.emit(Event::BlockedCount(1));
        until(cx, "the badge counts studio's agent", |cx| store.read_with(cx, |s, _| s.blocked == 1));
        until(cx, "the attention arrives", |_| !seen.lock().unwrap().is_empty());
        assert_eq!(seen.lock().unwrap()[0], "api on studio blocked");
        // This Mac's own count adds to it.
        let (b2, rx2) = Bridge::new();
        cx.update(|cx| AgentStore::listen(&store, rx2, cx));
        b2.send(crate::bridge::AppEvent::Engine(Event::BlockedCount(2)));
        until(cx, "both count", |cx| store.read_with(cx, |s, _| s.blocked == 3));
        // The engine's list again keeps studio's agents.
        b2.send(crate::bridge::AppEvent::Engine(Event::AgentsChanged(vec![agent("mine", "/work/web", "idle", 1, true)])));
        cx.run_until_parked();
        store.read_with(cx, |s, _| assert!(s.agent("peer:studio:a1").is_some()));

        // `pitwall peers unpair studio` in a terminal: applied live.
        let cli = peers(&world, "nLAPTOP", &ldir, Arc::new(pitwall_peer::NullPeerSink), None);
        std::thread::sleep(Duration::from_millis(20));
        cli.unpair("studio").unwrap();
        for _ in 0..3 {
            cx.executor().advance_clock(super::WATCH);
            cx.run_until_parked();
        }
        until(cx, "studio's agents are gone", |cx| store.read_with(cx, |s, _| s.agents.len() == 1 && s.blocked == 2));
        until(cx, "the model forgot it", |cx| model.read_with(cx, |m, _| m.entries.is_empty()));
        laptop.stop();
        studio.stop();
    }
}
