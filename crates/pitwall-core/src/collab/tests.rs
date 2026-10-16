//! Golden JSON for team records and turns, and the contract's shape.

use serde_json::json;

use super::team::*;
use super::turn::{ChangesState, Front};
use super::*;
use crate::net::fake::FakeNet;
use crate::net::Identity;

/// §4.2's example record (version 7, Carol added), elisions filled in.
fn golden_record() -> serde_json::Value {
    json!({
        "team": "t_9f2c",
        "version": 7,
        "prev": "sha256:61b1",
        "ops": [{"op": "add_member", "member": "p_carol",
                 "identities": [{"provider": "tailscale", "subject": "carol@example.com"}],
                 "devices": [{"key": "ed25519:C1", "addr": [{"network": "tailscale:example.ts.net", "node": "nXyz"}]}]}],
        "state": {
            "name": "Platform", "description": "API, jobs, infra",
            "listed": true, "join": "request",
            "admins": ["p_alice", "p_dave"],
            "members": [
                {"member": "p_alice",
                 "identities": [{"provider": "tailscale", "subject": "alice@example.com"},
                                {"provider": "cloud", "subject": "u_3Fz9"}],
                 "devices": [{"key": "ed25519:A1", "addr": [{"network": "tailscale:example.ts.net", "node": "nStudio"},
                                                           {"network": "cloud:acme", "node": "d_77"}]}]},
                {"member": "p_carol",
                 "identities": [{"provider": "tailscale", "subject": "carol@example.com"}],
                 "devices": [{"key": "ed25519:C1", "addr": [{"network": "tailscale:example.ts.net", "node": "nXyz"}]}]},
                {"member": "p_erin",
                 "identities": [{"provider": "tailscale", "subject": "erin@example.com"}],
                 "devices": []}
            ],
            "removed": [{"member": "p_eve", "at": 5}],
            "binding": null
        },
        "by": {"member": "p_alice", "key": "ed25519:A1"},
        "at": "2026-10-16T09:12:00Z",
        "sig": "ed25519:5c0d"
    })
}

#[test]
fn team_records_match_the_spec_shape_both_ways() {
    let golden = golden_record();
    let record: TeamRecord = serde_json::from_value(golden.clone()).unwrap();
    assert!(!record.is_genesis());
    assert_eq!(record.state.join, JoinPolicy::Request);
    assert!(record.state.is_admin(&MemberId("p_dave".into())));
    let bob_cloud = Identity::new("cloud", "u_3Fz9");
    assert_eq!(record.state.member_with(&bob_cloud).unwrap().member, MemberId("p_alice".into()));
    assert!(record.state.member(&MemberId("p_erin".into())).unwrap().devices.is_empty());
    assert!(record.state.member_with(&Identity::new("cloud", "carol@example.com")).is_none());
    assert_eq!(serde_json::to_value(&record).unwrap(), golden);
}

#[test]
fn a_genesis_record_has_no_team_or_prev() {
    let mut golden = golden_record();
    let obj = golden.as_object_mut().unwrap();
    obj.remove("team");
    obj.remove("prev");
    obj.insert("version".into(), json!(1));
    obj.insert("ops".into(), json!([]));
    let record: TeamRecord = serde_json::from_value(golden.clone()).unwrap();
    assert!(record.is_genesis());
    assert_eq!(serde_json::to_value(&record).unwrap(), golden);
}

#[test]
fn every_op_has_its_spec_name() {
    let member = || MemberId("p_bob".into());
    let key = |k: &str| DeviceKey(format!("ed25519:{k}"));
    let device = TeamDevice { key: key("B1"), addr: vec![] };
    let binding = Binding {
        network: crate::net::NetworkId::new("tailscale", "example.ts.net"),
        group: "platform".into(),
        admin_group: Some("platform-admins".into()),
    };
    let ops = vec![
        TeamOp::AddMember { member: member(), identities: vec![Identity::new("tailscale", "bob@example.com")], devices: vec![] },
        TeamOp::RemoveMember { member: member() },
        TeamOp::AddIdentity { member: member(), identity: Identity::new("cloud", "u_8KQ2") },
        TeamOp::RemoveIdentity { member: member(), identity: Identity::new("cloud", "u_8KQ2") },
        TeamOp::AddDevice { member: member(), device },
        TeamOp::RemoveDevice { member: member(), key: key("B1") },
        TeamOp::RotateDevice { member: member(), old: key("B1"), new: key("B2") },
        TeamOp::AddAdmin { member: member() },
        TeamOp::RemoveAdmin { member: member() },
        TeamOp::SetInfo { name: Some("Platform".into()), description: None, listed: Some(false) },
        TeamOp::SetJoin { join: JoinPolicy::Closed },
        TeamOp::Bind { binding },
        TeamOp::Unbind,
    ];
    let names: Vec<String> =
        ops.iter().map(|op| serde_json::to_value(op).unwrap()["op"].as_str().unwrap().to_string()).collect();
    assert_eq!(
        names,
        [
            "add_member",
            "remove_member",
            "add_identity",
            "remove_identity",
            "add_device",
            "remove_device",
            "rotate_device",
            "add_admin",
            "remove_admin",
            "set_info",
            "set_join",
            "bind",
            "unbind"
        ]
    );
    for op in &ops {
        let back: TeamOp = serde_json::from_value(serde_json::to_value(op).unwrap()).unwrap();
        assert_eq!(&back, op);
    }
    assert_eq!(serde_json::to_value(&ops[9]).unwrap(), json!({"op": "set_info", "name": "Platform", "listed": false}));
    assert_eq!(serde_json::to_value(&ops[11]).unwrap()["binding"]["adminGroup"], "platform-admins");
}

#[test]
fn turn_ids_name_their_conversation() {
    let c = ConversationId("c_8f2k".into());
    let t = TurnId::new(&c, 7);
    assert_eq!(t.as_str(), "c_8f2k/7");
    assert_eq!(t.conversation(), c);
    assert_eq!(t.number(), Some(7));
    assert_eq!(TurnId("odd".into()).number(), None);
}

#[test]
fn turns_serialize_with_their_status_inline() {
    let turn = Turn {
        id: TurnId("c_8f2k/7".into()),
        conversation: ConversationId("c_8f2k".into()),
        share: "api-expert".into(),
        status: TurnStatus::Refused(Refusal {
            reason: RefusalReason::Quota,
            message: "Daily limit reached for alice/api-expert; resets at 00:00".into(),
            resets_at: Some("2026-10-17T00:00:00+02:00".into()),
            until: None,
        }),
        status_message: None,
        reply: None,
        changes: None,
        left: Some(Left { messages_today: 0, resets_at: "2026-10-17T00:00:00+02:00".into() }),
        at: "2026-10-16T09:12:00Z".into(),
        done_at: None,
    };
    let v = serde_json::to_value(&turn).unwrap();
    assert_eq!(
        v["status"],
        json!({"state": "refused", "reason": "quota",
               "message": "Daily limit reached for alice/api-expert; resets at 00:00",
               "resetsAt": "2026-10-17T00:00:00+02:00"})
    );
    assert_eq!(v["left"], json!({"messagesToday": 0, "resetsAt": "2026-10-17T00:00:00+02:00"}));
    assert_eq!(serde_json::from_value::<Turn>(v).unwrap(), turn);

    let statuses = [
        (TurnStatus::Queued { position: 2 }, json!({"state": "queued", "position": 2})),
        (TurnStatus::WaitingForOwner, json!({"state": "waiting_for_owner"})),
        (TurnStatus::ChangesHeld, json!({"state": "changes_held"})),
        (TurnStatus::Failed { reason: Failure::NoAnswer }, json!({"state": "failed", "reason": "no_answer"})),
    ];
    for (status, golden) in statuses {
        assert_eq!(serde_json::to_value(&status).unwrap(), golden);
        assert!(!status.is_final() || matches!(status, TurnStatus::Failed { .. }));
    }
    assert!(TurnStatus::Done.is_final() && TurnStatus::Canceled.is_final());
    assert!(!TurnStatus::Working.is_final());

    let changes = Changes {
        state: ChangesState::Held,
        base_commit: "3f2a1c9".into(),
        files: 2,
        added: 14,
        removed: 3,
        patches: 1,
        parts: 1,
    };
    assert_eq!(
        serde_json::to_value(&changes).unwrap(),
        json!({"state": "held", "baseCommit": "3f2a1c9", "files": 2, "added": 14, "removed": 3, "patches": 1, "parts": 1})
    );
}

/// The service traits are object-safe, so the handler can hold them as
/// `Arc<dyn …>`, and an asker carries what a network proved.
#[test]
fn the_contract_is_usable_as_trait_objects() {
    fn _hold(_: std::sync::Arc<dyn Conversations>, _: std::sync::Arc<dyn Teams>) {}

    let net = FakeNet::new("example");
    net.device("erin@example.com", "erin-mbp");
    let asker = Asker {
        who: Who::Network(Box::new(net.caller("erin-mbp").unwrap())),
        agent: None,
        client: Some("Claude Code".into()),
        chain: vec![],
        front: Front::Mcp,
    };
    let Who::Network(caller) = &asker.who else { panic!("a network caller") };
    assert_eq!(caller.identity().subject, "erin@example.com");
    let ask: Ask = serde_json::from_value(json!({"share": "api-expert", "message": "what does 409 mean?"})).unwrap();
    assert!(!ask.new_conversation && ask.conversation.is_none());
}
