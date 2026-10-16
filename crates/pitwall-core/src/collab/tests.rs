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

// ------------------------------------------------------------------ shares and the audit log

fn draft(slug: &str) -> ShareSettings {
    serde_json::from_value(json!({
        "agent": "a1", "slug": slug, "name": "API expert", "description": "",
        "scope": {"repo": "/code/api", "ref": "main", "paths": ["src/"], "exclude": [".env"]}
    }))
    .unwrap()
}

#[test]
fn share_changes_say_exactly_what_widens() {
    use pitwall_proto::collab::{LimitOverride, Programs, ShareDiff};
    let a = draft("api");
    let widened = |f: &dyn Fn(&mut ShareSettings)| -> ShareDiff {
        let mut b = a.clone();
        f(&mut b);
        shares::diff(&a, &b)
    };
    let one = |d: ShareDiff| -> (String, bool) {
        assert_eq!(d.0.len(), 1, "{d:?}");
        (d.0[0].field.clone(), d.0[0].widens)
    };
    let carol = Identity::new("tailscale", "carol@example.com");
    assert_eq!(one(widened(&|s| s.audience.people.push(carol.clone()))), ("audience.people".into(), true));
    assert_eq!(one(widened(&|s| s.audience.networks.push(crate::net::NetworkId::new("tailscale", "x")))), ("audience.networks".into(), true));
    assert_eq!(one(widened(&|s| s.scope.paths.push("docs/".into()))), ("scope.paths".into(), true));
    assert_eq!(one(widened(&|s| s.scope.paths = vec!["src/api/".into()])), ("scope.paths".into(), false));
    assert_eq!(one(widened(&|s| s.scope.paths.clear())), ("scope.paths".into(), true), "no paths is the whole repository");
    assert_eq!(one(widened(&|s| s.scope.exclude.clear())), ("scope.exclude".into(), true));
    assert_eq!(one(widened(&|s| s.scope.exclude.push("secrets/".into()))), ("scope.exclude".into(), false));
    assert_eq!(one(widened(&|s| s.capability = Capability::WriteApproval)), ("capability".into(), true));
    assert_eq!(one(widened(&|s| s.start = Start::Summary { text: "x".into(), approved_at: "t".into() })), ("start".into(), true));
    assert_eq!(one(widened(&|s| s.access.env.push("NPM_TOKEN".into()))), ("access.env".into(), true));
    assert_eq!(one(widened(&|s| s.access.network.allow.push("registry.npmjs.org".into()))), ("access.network.allow".into(), true));
    assert_eq!(one(widened(&|s| s.access.programs = Some(Programs::Deny(vec!["curl".into()])))), ("access.programs".into(), true));
    assert_eq!(one(widened(&|s| s.limits.per_caller.messages_per_day = 50)), ("limits.perCaller.messagesPerDay".into(), true));
    assert_eq!(one(widened(&|s| s.limits.queue = 3)), ("limits.queue".into(), false));
    assert_eq!(one(widened(&|s| s.limits.agent_minutes_per_day = 600)), ("limits.agentMinutesPerDay".into(), true));
    assert_eq!(one(widened(&|s| s.off_limits = "pricing".into())), ("offLimits".into(), false));
    assert_eq!(one(widened(&|s| s.state = ShareState::Live)), ("state".into(), true));
    assert_eq!(
        one(widened(&|s| {
            s.limits.overrides.insert("tailscale:eve@example.com".into(), LimitOverride { blocked: true, ..LimitOverride::default() });
        })),
        ("limits.overrides.tailscale:eve@example.com".into(), false)
    );
    assert!(shares::diff(&a, &a).0.is_empty());
}

#[test]
fn shares_are_stored_drafted_published_and_stopped() {
    let dir = crate::testing::TempDir::new("shares");
    let paths = store::CollabPaths::new(dir.path().join("collab"));
    let store = shares::ShareStore::open(paths.clone());
    let mut s = draft("api");
    s.state = ShareState::Live;
    assert_eq!(store.create(s.clone()).unwrap().state, ShareState::Draft, "always a draft first");
    assert!(store.create(draft("api")).unwrap_err().is(crate::error::ErrorCode::Conflict));
    assert!(store.create(draft("Bad Slug")).is_err());
    assert!(store.pause("api").is_err(), "a draft isn't live");
    store.go_live("api").unwrap();
    let mut next = store.get("api").unwrap();
    next.state = ShareState::Draft;
    next.slug = "other".into();
    next.description = "Questions".into();
    let (d, after) = store.set("api", next).unwrap();
    assert_eq!(d.0.len(), 1, "state and slug can't be changed by set");
    assert_eq!(after.state, ShareState::Live);
    store.pause("api").unwrap();
    store.resume("api").unwrap();
    // Reloaded from disk.
    let again = shares::ShareStore::open(paths.clone());
    assert_eq!(again.get("api").unwrap().description, "Questions");
    assert_eq!(again.of_agent("a1").unwrap().slug, "api");
    again.stop("api").unwrap();
    assert!(shares::ShareStore::open(paths).list().is_empty());
}

#[test]
fn the_audit_log_rotates_keeping_three_files() {
    let dir = crate::testing::TempDir::new("audit");
    let log = audit::AuditLog::with_rotation(dir.path().join("audit.jsonl"), 300);
    for n in 0..40 {
        let mut line = audit::AuditLine::new(format!("2026-10-16T09:00:{n:02}Z"), "ask", "ok");
        line.agent = Some("api-expert".into());
        log.write(&line);
    }
    let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().into_string().unwrap()).collect();
    assert_eq!(files.len(), 3, "{files:?}");
    let lines = log.lines();
    assert!(lines.len() < 40 && !lines.is_empty());
    assert_eq!(lines.last().unwrap().at, "2026-10-16T09:00:39Z");
    assert!(lines.windows(2).all(|w| w[0].at < w[1].at), "oldest first");
    let raw = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
    let first: serde_json::Value = serde_json::from_str(raw.lines().next().unwrap()).unwrap();
    assert_eq!((first["kind"].as_str(), first["peer"].as_str(), first["login"].as_str()), (Some("collab"), Some("local"), Some("owner")));
}
