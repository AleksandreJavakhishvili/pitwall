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
        turns: None,
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

/// Codex (tier (a), session files): the turn ends when its session file
/// says so, with the reply from that line, and the file names the
/// conversation for its resume.
#[test]
fn a_codex_turn_ends_with_the_reply_in_its_session_file() {
    let h = Harness::new(vec![]);
    let mut caps = FakeProvider::local_like();
    caps.local_process = true;
    h.provider.set_caps(caps);
    let host = EngineHost::new(h.engine.clone());
    let events: Arc<Mutex<Vec<(String, SessionEvent)>>> = Arc::default();
    let seen = events.clone();
    host.subscribe(Arc::new(move |a: &str, e: SessionEvent| seen.lock().unwrap().push((a.to_string(), e))));
    let account = h.dir.path().join("codex-sharing");
    let work = h.dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let mut l = launch(&work.to_string_lossy());
    l.kind = "codex".into();
    l.kind_name = "Codex".into();
    l.session_id = None;
    l.wrapped.env.insert("CODEX_HOME".into(), account.to_string_lossy().into_owned());
    host.launch(&l).unwrap();

    let real = std::fs::canonicalize(&work).unwrap();
    let rollout = include_str!("../fixtures/codex-rollout.jsonl").replace("/pw/collab/shares/api-expert/s/c_1/work", &real.to_string_lossy());
    let all: Vec<&str> = rollout.lines().collect();
    let end = all.iter().position(|x| x.contains("task_complete")).unwrap();
    let day = account.join("sessions/2026/10/16");
    std::fs::create_dir_all(&day).unwrap();
    let file = day.join("rollout-2026-10-16T22-53-27-01a1460f-dcd9-7cd2-9dcc-336889d84012.jsonl");
    // Codex starts its file with the session's line; the message goes in.
    std::fs::write(&file, format!("{}\n", all[0])).unwrap();
    host.send("shared-1", "From bob@example.com:\nWhere is the entry point?").unwrap();
    let id = SessionEvent::SessionId("01a1460f-dcd9-7cd2-9dcc-336889d84012".into());
    assert!(events.lock().unwrap().iter().any(|(_, e)| *e == id), "the id comes from the file before the turn");
    let rest: String = all[1..=end].iter().map(|x| format!("{x}\n")).collect();
    std::fs::OpenOptions::new().append(true).open(&file).and_then(|mut f| std::io::Write::write_all(&mut f, rest.as_bytes())).unwrap();
    let want = SessionEvent::TurnEnded(Some(Reply { text: "Mock reply 1: the entry point is src/lib.rs.".into(), source: ReplySource::SessionFile }));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !events.lock().unwrap().iter().any(|(_, e)| *e == want) {
        assert!(std::time::Instant::now() < deadline, "the turn ended from the file: {:?}", events.lock().unwrap());
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // Once.
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert_eq!(events.lock().unwrap().iter().filter(|(_, e)| matches!(e, SessionEvent::TurnEnded(_))).count(), 1);
}

fn wait_event(events: &Arc<Mutex<Vec<(String, SessionEvent)>>>, want: &SessionEvent) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !events.lock().unwrap().iter().any(|(_, e)| e == want) {
        assert!(std::time::Instant::now() < deadline, "waited for {want:?}: {:?}", events.lock().unwrap());
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Gemini CLI (tier (b), its non-interactive mode): nothing runs between
/// turns; each message is one sandboxed run in the pane, the first one
/// starting the conversation with Pitwall's id, the next ones resuming it,
/// and the reply is the JSON's `response` (recorded from Gemini CLI 0.64.0).
#[test]
fn a_gemini_turn_is_one_run_and_its_reply_is_the_jsons_response() {
    let h = Harness::new(vec![]);
    let mut caps = FakeProvider::local_like();
    caps.local_process = true;
    h.provider.set_caps(caps);
    let host = EngineHost::new(h.engine.clone());
    let events: Arc<Mutex<Vec<(String, SessionEvent)>>> = Arc::default();
    let seen = events.clone();
    host.subscribe(Arc::new(move |a: &str, e: SessionEvent| seen.lock().unwrap().push((a.to_string(), e))));
    let folder = h.dir.path().join("s/c_1");
    let work = folder.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let wrap = |args: &[&str]| {
        let mut argv: Vec<String> = ["sandbox-exec", "-f", "/p.sb", "gemini", "--skip-trust"].iter().map(|s| s.to_string()).collect();
        argv.extend(args.iter().map(|s| s.to_string()));
        Wrapped { argv, env: BTreeMap::from([("HOME".to_string(), "/s/home".to_string()), ("GEMINI_API_KEY".to_string(), "not-a-real-key".to_string())]) }
    };
    let mut l = launch(&work.to_string_lossy());
    l.kind = "gemini".into();
    l.kind_name = "Gemini CLI".into();
    l.session_id = Some("g-1".into());
    l.turns = Some(crate::collab::session::TurnArgs {
        first: wrap(&["--session-id", "g-1", "-p", "{message}", "--output-format", "json"]),
        next: wrap(&["--resume", "g-1", "-p", "{message}", "--output-format", "json"]),
    });
    l.wrapped = l.turns.as_ref().unwrap().first.clone();
    host.launch(&l).unwrap();
    assert!(h.provider.launched().is_empty(), "nothing runs before a message");
    wait_event(&events, &SessionEvent::Ready);

    host.send("shared-1", "From bob@example.com:\nWhere are the docs?").unwrap();
    let line = h.provider.launched()[0].command_line.clone();
    assert!(line.starts_with("/bin/sh -c "), "{line}");
    assert!(line.contains("/usr/bin/env -i GEMINI_API_KEY=not-a-real-key HOME=/s/home sandbox-exec -f /p.sb gemini --skip-trust --session-id g-1 -p"), "{line}");
    assert!(line.contains("'From bob@example.com:\nWhere are the docs?'"), "the message is one argument: {line}");
    assert!(!line.contains("{message}"));
    let out = folder.join("turn/stdout");
    std::fs::write(&out, include_str!("../fixtures/gemini-headless.json")).unwrap();
    std::fs::write(folder.join("turn/stderr"), "Warning: Basic terminal detected (TERM=dumb).\nRipgrep is not available. Falling back to GrepTool.\n").unwrap();
    h.provider.ctl("shared-1").unwrap().exit(0);
    let reply = Reply { text: "Mock Gemini reply 12: the docs are in docs/.".into(), source: ReplySource::NonInteractive };
    wait_event(&events, &SessionEvent::TurnEnded(Some(reply)));

    // The next message resumes the conversation, in the same record.
    host.send("shared-1", "And the tokens?").unwrap();
    let line = h.provider.launched()[1].command_line.clone();
    assert!(line.contains("--resume g-1 -p 'And the tokens?'"), "{line}");
    assert!(!out.exists(), "each run's output starts empty");
    assert_eq!(h.engine.records().iter().filter(|r| r.id == "shared-1").count(), 1);
    // No login in the session: the run's error, no answer.
    std::fs::write(folder.join("turn/stderr"), include_str!("../fixtures/gemini-headless-no-login.stderr")).unwrap();
    h.provider.ctl("shared-1").unwrap().exit(41);
    wait_event(&events, &SessionEvent::TurnEnded(None));
    assert!(!events.lock().unwrap().iter().any(|(_, e)| *e == SessionEvent::Exited), "a run ending isn't the session exiting");
}
