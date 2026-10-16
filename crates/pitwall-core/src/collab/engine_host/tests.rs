//! A shared session through the real engine, on the fake provider: an
//! ordinary agent with `AgentRecord.shared`, launched exactly as wrapped,
//! never restarted the ordinary way, its reply read from the `Stop` hook.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::*;
use crate::engine::lifecycle;
use crate::model::SharedSession;
use crate::sandbox::Wrapped;
use crate::testing::{FakeProvider, Harness};
use pitwall_proto::collab::ReplySource;

fn launch(cwd: &str) -> HostLaunch {
    let mut env = BTreeMap::new();
    env.insert("HOME".to_string(), "/s/home".to_string());
    HostLaunch {
        agent: "shared-1".into(),
        name: "api-expert · bob@example.com".into(),
        kind: "claude".into(),
        kind_name: "Claude Code".into(),
        cwd: PathBuf::from(cwd),
        project: "/code/api".into(),
        wrapped: Wrapped { argv: vec!["sandbox-exec".into(), "-f".into(), "/p.sb".into(), "claude".into(), "--session-id".into(), "s-1".into()], env },
        shared: SharedSession { share: "api-expert".into(), agent: "a1".into(), conversation: "c_1".into(), caller: "bob@example.com".into() },
        session_id: Some("s-1".into()),
        resumed: false,
    }
}

#[test]
fn a_shared_session_is_an_ordinary_agent_run_exactly_as_wrapped() {
    let h = Harness::new(vec![]);
    let mut caps = FakeProvider::local_like();
    caps.local_process = true;
    h.provider.set_caps(caps);
    let host = EngineHost::new(h.engine.clone());
    let events: Arc<Mutex<Vec<(String, SessionEvent)>>> = Arc::default();
    let seen = events.clone();
    host.subscribe(Arc::new(move |a: &str, e: SessionEvent| seen.lock().unwrap().push((a.to_string(), e))));
    let cwd = h.dir.path().to_string_lossy().into_owned();
    host.launch(&launch(&cwd)).unwrap();

    let rec = h.engine.records().into_iter().find(|r| r.id == "shared-1").unwrap();
    assert_eq!(rec.shared.as_ref().unwrap().share, "api-expert");
    assert_eq!((rec.kind.as_str(), rec.session_id.as_deref()), ("claude", Some("s-1")));
    let line = &h.provider.launched()[0].command_line;
    assert!(line.starts_with("/usr/bin/env -i HOME=/s/home sandbox-exec -f /p.sb claude --session-id s-1"), "{line}");
    assert!(!line.contains("--settings"), "hooks come from the wrapped launch, not the engine");
    // The engine never restarts it unconfined.
    assert!(lifecycle::restart(&h.engine, "shared-1", None).unwrap_err().contains("shared session"));
    // It isn't an agent that can be shared itself.
    assert!(host.agent("shared-1").is_none());

    // A message goes in as a prompt; the Stop hook's text is the reply.
    host.send("shared-1", "From bob@example.com:\nhi").unwrap();
    // Typing goes through the terminal's writer: wait for it.
    let ctl = h.provider.ctl("shared-1").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !String::from_utf8_lossy(&ctl.input()).contains("From bob@example.com:") {
        assert!(std::time::Instant::now() < deadline, "the message reached the terminal");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.engine.ingest_hook("shared-1", &serde_json::json!({"hook_event_name": "Stop", "last_assistant_message": "Hello Bob."}));
    let got = events.lock().unwrap().clone();
    assert!(got.contains(&(
        "shared-1".to_string(),
        SessionEvent::TurnEnded(Some(Reply { text: "Hello Bob.".into(), source: ReplySource::Hook }))
    )));
    // Hooks of other agents are not this host's business.
    h.engine.ingest_hook("someone-else", &serde_json::json!({"hook_event_name": "Stop", "last_assistant_message": "x"}));
    assert_eq!(events.lock().unwrap().len(), got.len());

    // Started again in place: same record, same agent.
    let mut again = launch(&cwd);
    again.resumed = true;
    host.launch(&again).unwrap();
    assert_eq!(h.engine.records().iter().filter(|r| r.id == "shared-1").count(), 1);
    assert!(h.engine.records().iter().find(|r| r.id == "shared-1").unwrap().has_conversation);
    assert_eq!(h.provider.launched().len(), 2);
}

#[test]
fn the_owners_agent_is_described_for_sharing() {
    let mut rec = crate::testing::record("a1", "/code/api");
    rec.kind = "claude".into();
    rec.kind_name = "Claude Code".into();
    let h = Harness::new(vec![rec]);
    let host = EngineHost::new(h.engine.clone());
    let a = host.agent("a1").unwrap();
    assert_eq!(a.kind.id, "claude");
    assert_eq!(a.project, "/code/api");
    assert!(a.kind.share.as_ref().is_some_and(|s| s.reply == ReplyTier::Hook));
    let env = host.account_env(&a.kind, None);
    assert_eq!(env[0].0, "CLAUDE_CONFIG_DIR");
}
