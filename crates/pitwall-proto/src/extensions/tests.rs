//! The status's shape in the main states, with and without a service, and
//! the next step picked from it.

use super::*;
use serde_json::json;

fn step(kind: StepKind, key: &str, label: &str, state: StepState) -> ExtStep {
    ExtStep { kind, key: Some(key.into()).filter(|k: &String| !k.is_empty()), label: label.into(), state, ..Default::default() }
}

/// An inbox with a server address, an API key, a setup action and an
/// adapter: no service.
fn inbox() -> ExtStatus {
    ExtStatus {
        proto: EXT_PROTO,
        id: "inbox".into(),
        name: "Inbox".into(),
        version: Some("1.0.0".into()),
        installed: true,
        on: true,
        source: "comes with Pitwall".into(),
        state: "Needs setup".into(),
        steps: vec![
            step(StepKind::Setting, "server", "Server address", StepState::Todo),
            ExtStep { secret: true, ..step(StepKind::Setting, "apiKey", "API key", StepState::Todo) },
            step(StepKind::Action, "setup", "Set up my agents", StepState::Todo),
            ExtStep { blocked_by: Some("Set Server address first.".into()), ..step(StepKind::Connected, "", "Connected", StepState::Blocked) },
        ],
        settings: vec![
            ExtSettingInfo { key: "server".into(), label: "Server address".into(), kind: "url".into(), required: true, hint: Some("like https://inbox.example.com".into()), ..Default::default() },
            ExtSettingInfo { key: "apiKey".into(), label: "API key".into(), kind: "secret".into(), required: true, ..Default::default() },
        ],
        connections: vec![ExtConnection { id: "c1".into(), name: "Inbox".into(), default: true, values: BTreeMap::new(), secrets: BTreeMap::from([("apiKey".into(), false)]), state: "Needs setup".into() }],
        adapter: Some(ExtAdapter { state: AdapterState::NeedsSetup, reason: "Set Server address first.".into() }),
        ..Default::default()
    }
}

/// An extension with a service (install, confirm and start, first run).
fn served(state: ServiceState) -> ExtStatus {
    let done = |b: bool| if b { StepState::Done } else { StepState::Todo };
    let installed = !matches!(state, ServiceState::NotInstalled | ServiceState::TooOld);
    let up = matches!(state, ServiceState::Running | ServiceState::RunningElsewhere);
    ExtStatus {
        proto: EXT_PROTO,
        id: "kit".into(),
        name: "Kit".into(),
        installed: true,
        on: true,
        source: "comes with Pitwall".into(),
        steps: vec![
            step(StepKind::ServiceInstall, "", "Install Kit's server", done(installed)),
            ExtStep { blocked_by: (!installed).then(|| "Install Kit's server first.".into()), ..step(StepKind::ServiceStart, "", "Confirm and start Kit's server", if !installed { StepState::Blocked } else { done(up) }) },
            step(StepKind::ServiceFirstRun, "", "Sign in", if up { StepState::Todo } else { StepState::Blocked }),
        ],
        service: Some(ExtService {
            label: "Kit's server".into(),
            state,
            detail: match state {
                ServiceState::TooOld => "Kit's server 0.5.0; this extension needs 0.6.0 or later.".into(),
                ServiceState::RunningElsewhere => "Something else is already listening on 127.0.0.1:8787.".into(),
                _ => String::new(),
            },
            install_action: Some("install".into()),
            first_run: Some(ExtFirstRun { label: "Sign in".into(), pending: true, opens: true, ..Default::default() }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn the_next_step_is_the_first_not_done() {
    // Not installed, off, a problem: before any step.
    let mut s = inbox();
    s.installed = false;
    assert_eq!(next_of(&s).command.as_deref(), Some("pitwall extension install inbox"));
    s.installed = true;
    s.on = false;
    let n = next_of(&s);
    assert_eq!((n.who, n.command.as_deref()), (Who::Agent, Some("pitwall extension on inbox")));
    s.on = true;
    // A plain setting: the agent asks the user, then sets it.
    let n = next_of(&s);
    assert_eq!((n.who, n.step, n.command.as_deref()), (Who::Agent, Some(1), Some("pitwall extension set inbox server <value>")));
    assert!(n.text.contains("like https://inbox.example.com"), "{}", n.text);
    // A secret: only the user, in Pitwall; never in the chat.
    s.steps[0].state = StepState::Done;
    let n = next_of(&s);
    assert_eq!((n.who, n.step, n.command.as_deref()), (Who::User, Some(2), Some("pitwall extension secret inbox apiKey")));
    assert!(n.text.contains("Never ask for it in the chat"), "{}", n.text);
    // A setup action; done but not for every account.
    s.steps[1].state = StepState::Done;
    s.steps[2].not_yet_for = vec!["Claude Code, account Home".into()];
    let n = next_of(&s);
    assert_eq!(n.command.as_deref(), Some("pitwall extension setup inbox --action setup"));
    assert!(n.text.contains("not yet for: Claude Code, account Home"), "{}", n.text);
    // Connected: signed out is the user's, with what may fix it.
    s.steps[2].state = StepState::Done;
    s.steps[3].state = StepState::Todo;
    s.adapter = Some(ExtAdapter { state: AdapterState::SignedOut, reason: "The key was refused.".into() });
    let n = next_of(&s);
    assert_eq!(n.who, Who::User);
    assert!(n.text.contains("The key was refused") && n.text.contains("pitwall extension secret inbox apiKey"), "{}", n.text);
    s.adapter = Some(ExtAdapter { state: AdapterState::Connecting, reason: String::new() });
    assert_eq!(next_of(&s).who, Who::Wait);
    s.steps[3].state = StepState::Done;
    let n = next_of(&s);
    assert_eq!((n.who, n.text.as_str()), (Who::Nobody, "All set."));
    // With several connections, the one shown is named.
    s.steps[0].state = StepState::Todo;
    s.connection = Some("Work stuff".into());
    s.connections.push(ExtConnection { name: "Work stuff".into(), ..Default::default() });
    assert_eq!(next_of(&s).command.as_deref(), Some("pitwall extension set inbox server <value> --connection 'Work stuff'"));
}

#[test]
fn a_service_moves_from_install_to_its_first_run() {
    let n = next_of(&served(ServiceState::NotInstalled));
    assert_eq!((n.step, n.command.as_deref()), (Some(1), Some("pitwall extension setup kit --action install")));
    let n = next_of(&served(ServiceState::TooOld));
    assert_eq!(n.command.as_deref(), Some("pitwall extension service kit update"));
    assert!(n.text.contains("0.5.0") && n.text.contains("0.6.0"), "{}", n.text);
    let n = next_of(&served(ServiceState::NeedsConfirmation));
    assert_eq!((n.step, n.command.as_deref()), (Some(2), Some("pitwall extension service kit confirm")));
    assert_eq!(next_of(&served(ServiceState::Starting)).who, Who::Wait);
    assert_eq!(next_of(&served(ServiceState::Stopped)).command.as_deref(), Some("pitwall extension service kit start"));
    let n = next_of(&served(ServiceState::Running));
    assert_eq!((n.step, n.command.as_deref()), (Some(3), Some("pitwall extension first-run kit")));
    assert!(n.text.contains("you never see it"), "{}", n.text);
    // Someone else's server: never started or stopped by the agent.
    let n = next_of(&served(ServiceState::RunningElsewhere));
    assert_eq!((n.who, n.command), (Who::User, None));
    // Without an install action, the user installs it.
    let mut s = served(ServiceState::NotInstalled);
    s.service.as_mut().unwrap().install_action = None;
    assert_eq!(next_of(&s).who, Who::User);
}

/// The wire shape: camelCase, a secret only as whether it is set.
#[test]
fn the_status_on_the_wire() {
    let mut s = inbox();
    s.next = next_of(&s);
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["steps"][1], json!({"kind": "setting", "key": "apiKey", "label": "API key", "state": "todo", "secret": true}));
    assert_eq!(v["steps"][3]["blockedBy"], json!("Set Server address first."));
    assert_eq!(v["connections"][0]["secrets"], json!({"apiKey": false}));
    assert_eq!(v["adapter"], json!({"state": "needsSetup", "reason": "Set Server address first."}));
    assert_eq!(v["next"]["who"], json!("agent"));
    assert!(v.get("service").is_none());
    let back: ExtStatus = serde_json::from_value(v).unwrap();
    assert_eq!(back, s);
    let mut k = served(ServiceState::RunningElsewhere);
    k.next = next_of(&k);
    let v = serde_json::to_value(&k).unwrap();
    assert_eq!(v["service"]["state"], json!("runningElsewhere"));
    assert_eq!(v["service"]["firstRun"]["pending"], json!(true));
    assert!(status_text(&k).contains("Next (the user)"), "{}", status_text(&k));
    // A newer Pitwall's answer, with fields unknown here, still reads.
    let newer: ExtStatus = serde_json::from_value(json!({"proto": 9, "id": "x", "badge": 3, "steps": [{"kind": "setting", "state": "done", "later": 1}]})).unwrap();
    assert_eq!(newer.steps[0].state, StepState::Done);
}

/// What an agent may ask, on the wire.
#[test]
fn acts_on_the_wire() {
    let a = ExtAct { id: "kit".into(), op: ExtOp::Service { what: ServiceOp::Confirm } };
    let v = serde_json::to_value(&a).unwrap();
    assert_eq!(v, json!({"id": "kit", "op": "service", "what": "confirm"}));
    assert_eq!(serde_json::from_value::<ExtAct>(v).unwrap(), a);
    let a: ExtAct = serde_json::from_value(json!({"id": "inbox", "op": "setup"})).unwrap();
    assert_eq!(a.op, ExtOp::Setup { action: None, connection: None });
    let a: ExtAct = serde_json::from_value(json!({"id": "inbox", "op": "set", "key": "server", "value": "https://x"})).unwrap();
    assert_eq!(a.op.word(), "set");
    assert!(serde_json::from_value::<ExtAct>(json!({"id": "inbox", "op": "grant", "permission": "agents.queue"})).is_err(), "granting isn't an act");
    assert_eq!(ServiceOp::parse("restart").map(ServiceOp::word), Some("restart"));
    assert_eq!(ServiceOp::parse("kill"), None);
}
