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
