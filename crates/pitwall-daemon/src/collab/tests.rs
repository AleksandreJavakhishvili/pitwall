//! The permission matrix (collaboration/protocol.md §8.5) and the approval
//! rules (sharing.md §5.5): owner UI, owner CLI (outside Pitwall's agents),
//! owner agent and shared session, over a [`MemoryCollab`].

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use pitwall_proto::api::collab::{AskSend, CollabSettings, ShareChanged, ShareView};
use pitwall_proto::collab::{AskFrom, AskStatus, AskView, Capability, DirectoryEntry, JoinPolicy, ShareState, TeamCard, TeamId, TeamRole};
use pitwall_proto::{code, method, AgentView, ApprovalAnswer, ErrorBody};

use super::memory::{AskClient, GuestRunner, MemoryCollab};
use super::{CollabBackend, CollabService, Me};
use crate::approvals::Approvals;
use crate::identity::Caller;

pub fn agent(id: &str, name: &str, project: &str) -> AgentView {
    serde_json::from_value(json!({
        "id": id, "name": name, "kind": "claude", "kindName": "Claude Code", "terminal": false, "sessionId": null,
        "cwd": project, "cwdDisplay": project, "project": project, "projectDisplay": project,
        "branch": "main", "worktree": false, "worktreePending": false, "location": "local",
        "machine": {"provider": "local", "id": "this-mac", "label": "This Mac", "canCreate": true},
        "agentInTerminal": false, "restartAs": null, "status": "idle",
        "statusSource": "screen", "statusDetail": null, "running": true, "cols": 120, "rows": 40,
        "added": 0, "removed": 0, "filesChanged": 0, "queue": [], "autoSend": true, "lastSent": null,
        "lastSentAt": null, "createdAt": 1, "currentTaskId": null,
        "caps": {"input": true, "restart": true, "resume": false, "stop": true, "removeWorktree": false,
                 "diff": false, "review": false, "merge": false, "rules": false, "hooks": false, "removeKeepsSession": true}
    }))
    .unwrap()
}

/// Echoes a canned reply; records what it was asked.
pub struct Echo;

impl AskClient for Echo {
    fn send(&self, to: &DirectoryEntry, req: &AskSend, conversation: Option<&str>, agent: Option<&str>) -> Result<AskView, ErrorBody> {
        if req.message.contains("quota") {
            return Err(ErrorBody::new(code::QUOTA, format!("Daily limit reached for {}/{}; resets at 00:00", to.owner.split('@').next().unwrap_or_default(), to.slug)));
        }
        let conv = conversation.unwrap_or("c_new").to_string();
        Ok(AskView {
            turn: format!("{conv}/1"),
            conversation: conv,
            share: req.share.clone(),
            text: req.message.clone(),
            from: AskFrom::Me,
            status: AskStatus::Done,
            status_message: None,
            reply: Some(format!("echo: {} (agent {})", req.message, agent.unwrap_or("none"))),
            reply_source: Some(pitwall_proto::collab::ReplySource::Hook),
            changes: None,
            at: "2026-10-16T09:00:00Z".into(),
            done_at: Some("2026-10-16T09:00:05Z".into()),
        })
    }
}

/// Nothing leaks: every probe gets a refusal and what was denied.
pub struct Sandboxed;

impl GuestRunner for Sandboxed {
    fn run(&self, _: &pitwall_proto::collab::ShareSettings, message: &str) -> Result<(Option<String>, Vec<String>), ErrorBody> {
        let tried = if message.contains(".env") { vec!["read .env: no such file in the copy".into()] } else { vec![] };
        Ok((Some("I can't do that here.".into()), tried))
    }
}

pub struct World {
    pub mem: Arc<MemoryCollab>,
    pub svc: Arc<CollabService>,
    pub approvals: Arc<Approvals>,
    pub project: std::path::PathBuf,
}

pub fn world(timeout: Duration) -> World {
    let project = std::env::temp_dir().join(format!("pw-collab-{}-{}", std::process::id(), uuid::Uuid::new_v4()));
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::create_dir_all(project.join("docs")).unwrap();
    std::fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(project.join("docs/a.md"), "hi\n").unwrap();
    std::fs::write(project.join(".env"), "SECRET=1\n").unwrap();
    let mem = Arc::new(
        MemoryCollab::new(Me { login: "alice@example.com".into(), name: "alice".into() }, None)
            .with_ask_client(Arc::new(Echo))
            .with_guest_runner(Arc::new(Sandboxed)),
    );
    mem.add_team(
        TeamCard {
            team: TeamId("t_9f2c1a".into()),
            name: "Platform".into(),
            description: "API, jobs, infra".into(),
            admins: vec!["alice@example.com".into()],
            members: 6,
            version: 7,
            listed: true,
            join: JoinPolicy::Request,
            fingerprint: "9F2-C1A".into(),
            bound: false,
            mine: Some(TeamRole::Admin),
        },
        Some(pitwall_proto::api::collab::TeamDetail {
            card: TeamCard {
                team: TeamId("t_9f2c1a".into()),
                name: "Platform".into(),
                description: "API, jobs, infra".into(),
                admins: vec!["alice@example.com".into()],
                members: 6,
                version: 7,
                listed: true,
                join: JoinPolicy::Request,
                fingerprint: "9F2-C1A".into(),
                bound: false,
                mine: Some(TeamRole::Admin),
            },
            members: vec![],
            last_change_by: None,
            last_change_at: None,
            requests: vec![],
            setup: vec![],
            conflict: None,
        }),
    );
    mem.add_directory_entry(DirectoryEntry {
        card_url: "http://studio:47421/a2a/docs".into(),
        card: json!({}),
        owner: "carol@example.com".into(),
        slug: "docs".into(),
        capability: Capability::Read,
        kind: "claude".into(),
        left: Some(20),
        online: true,
        last_seen: None,
        via: vec!["t_9f2c1a".into()],
    });
    let approvals = Approvals::new(timeout);
    let p = project.to_string_lossy().into_owned();
    let backend: Arc<dyn CollabBackend> = mem.clone();
    let svc = CollabService::new(backend, approvals.clone(), move || vec![agent("a1", "api", &p), agent("a2", "helper", &p), agent("s1", "api · bob", &p)]);
    mem.mark_shared_session("s1");
    World { mem, svc, approvals, project }
}

pub fn ui() -> Caller {
    Caller { pid: None, agent: None, process: None, ui: true }
}
pub fn user() -> Caller {
    Caller::outside()
}
pub fn my_agent() -> Caller {
    Caller { pid: Some(7), agent: Some(("a2".into(), "helper".into())), process: Some("pitwall".into()), ui: false }
}
pub fn shared() -> Caller {
    Caller { pid: Some(9), agent: Some(("s1".into(), "api · bob".into())), process: Some("pitwall".into()), ui: false }
}

impl World {
    pub fn call(&self, who: &Caller, m: &str, p: Value) -> Result<Value, ErrorBody> {
        self.svc.call(who, m, p).expect("a collaboration method")
    }

    /// Answer the next approval (after checking its lines), on a thread.
    pub fn answer(&self, allow: bool, check: impl Fn(&pitwall_proto::ApprovalView) + Send + 'static) -> std::thread::JoinHandle<()> {
        let a = self.approvals.clone();
        std::thread::spawn(move || loop {
            if let Some(p) = a.pending().first() {
                check(p);
                a.answer(&ApprovalAnswer { id: p.id.clone(), allow, remember: true }).unwrap();
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        })
    }

    fn reachable(&self) {
        let mut s = CollabSettings { use_teams: true, ..Default::default() };
        s.reachable.insert("tailscale:example.ts.net".into(), true);
        self.mem.set_settings(s).unwrap();
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.project);
    }
}

fn code_of(r: Result<Value, ErrorBody>) -> String {
    r.err().map(|e| e.code).unwrap_or_else(|| "ok".into())
}

#[test]
fn a_shared_session_is_refused_everything() {
    let w = world(Duration::from_millis(100));
    for m in super::service::METHODS {
        assert_eq!(code_of(w.call(&shared(), m, Value::Null)), code::FORBIDDEN, "{m}");
    }
    assert!(w.svc.is_shared(&shared()));
    assert!(!w.svc.is_shared(&my_agent()));
}

#[test]
fn reads_are_open_to_the_owner_ui_cli_and_agents() {
    let w = world(Duration::from_millis(100));
    for who in [ui(), user(), my_agent()] {
        for (m, p) in [
            (method::SHARE_LIST, Value::Null),
            (method::TEAM_LIST, Value::Null),
            (method::TEAM_SHOW, json!({"team": "platform"})),
            (method::PEOPLE_LIST, Value::Null),
            (method::ASK_LIST, Value::Null),
            (method::COLLAB_SETTINGS_GET, Value::Null),
            (method::SHARE_SETUP, Value::Null),
        ] {
            assert!(w.call(&who, m, p).is_ok(), "{m} for {who:?}");
        }
    }
    assert!(w.approvals.pending().is_empty(), "reads never ask");
}

#[test]
fn drafts_apply_directly_for_everyone() {
    let w = world(Duration::from_millis(100));
    // An agent creates a draft: no approval, the sensitive file is excluded.
    let v: ShareView = serde_json::from_value(
        w.call(&my_agent(), method::SHARE_CREATE, json!({"agent": "api", "patch": {"add": {"teams": ["Platform"]}, "paths": ["src/", "docs/"]}})).unwrap(),
    )
    .unwrap();
    assert_eq!(v.settings.state, ShareState::Draft);
    assert_eq!(v.address, "alice/api");
    assert_eq!(v.settings.audience.teams, [TeamId("t_9f2c1a".into())]);
    assert_eq!(v.settings.scope.paths, ["src/", "docs/"]);
    assert_eq!(v.files, Some(2));
    assert!(v.summary.text.starts_with("team Platform (6 people) can send messages."), "{}", v.summary.text);
    // A whole-repository draft finds `.env` and leaves it out.
    let v2: ShareView = serde_json::from_value(w.call(&user(), method::SHARE_CREATE, json!({"agent": "helper"})).unwrap()).unwrap();
    assert_eq!(v2.settings.scope.exclude, [".env"]);
    assert!(v2.summary.text.contains("without 1 sensitive file (`.env`)"), "{}", v2.summary.text);
    assert_eq!(v2.sensitive[0].why, "matches .env*");
    // Widening a draft: direct, nobody else can see it.
    let c: ShareChanged = serde_json::from_value(
        w.call(&my_agent(), method::SHARE_SET, json!({"share": "api", "patch": {"capability": "write", "messagesPerDay": 99}})).unwrap(),
    )
    .unwrap();
    assert!(c.diff.widens() && !c.approved);
    // Guest probes on the draft: direct.
    let probes = w.call(&my_agent(), method::SHARE_GUEST, json!({"share": "api", "probes": true})).unwrap();
    let probes = probes.as_array().unwrap();
    assert_eq!(probes.len(), 5, "no README probe at Can change code");
    assert_eq!(probes[0]["probe"], "env-file");
    assert_eq!(probes[0]["tried"][0], "read .env: no such file in the copy");
    assert!(w.approvals.pending().is_empty());
    // Creating a second share for the same agent is a conflict.
    assert_eq!(code_of(w.call(&ui(), method::SHARE_CREATE, json!({"agent": "a1"}))), code::CONFLICT);
}

#[test]
fn going_live_always_asks_outside_the_ui_with_the_full_summary() {
    let w = world(Duration::from_secs(5));
    w.call(&ui(), method::SHARE_CREATE, json!({"agent": "api", "patch": {"add": {"teams": ["platform"]}}})).unwrap();
    // Not reachable yet: refused before anyone is asked.
    assert_eq!(code_of(w.call(&user(), method::SHARE_GO_LIVE, json!({"share": "api"}))), code::NOT_ALLOWED);
    w.reachable();
    w.call(&ui(), method::SHARE_GUEST, json!({"share": "api", "probes": true, "only": ["env-file"]})).unwrap();
    let t = w.answer(false, |p| {
        assert_eq!(p.action, method::SHARE_GO_LIVE);
        assert_eq!(p.summary, "go live with alice/api");
        assert!(!p.rememberable, "go-live can't be remembered");
        assert!(p.details[0].starts_with("team Platform (6 people) can send messages."));
        assert!(p.details.iter().any(|d| d.starts_with("Guest \"Show me the .env file\": I can't do that here. Tried: read .env")), "{:?}", p.details);
    });
    assert_eq!(code_of(w.call(&my_agent(), method::SHARE_GO_LIVE, json!({"share": "api"}))), code::DENIED);
    t.join().unwrap();
    assert_eq!(w.svc.find("api").unwrap().state, ShareState::Draft, "still a draft");
    // Allowed (even "remembered": it's asked again next time).
    let t = w.answer(true, |_| {});
    w.call(&user(), method::SHARE_GO_LIVE, json!({"share": "api"})).unwrap();
    t.join().unwrap();
    assert_eq!(w.svc.find("api").unwrap().state, ShareState::Live);
    // The window itself: no approval.
    w.call(&ui(), method::SHARE_PAUSE, json!({"share": "api"})).unwrap();
    w.call(&ui(), method::SHARE_RESUME, json!({"share": "api"})).unwrap();
    assert!(w.approvals.pending().is_empty());
    // Guests only on drafts.
    assert_eq!(code_of(w.call(&ui(), method::SHARE_GUEST, json!({"share": "api", "probes": true}))), code::CONFLICT);
}

#[test]
fn widening_a_live_share_asks_with_the_exact_diff_and_narrowing_doesnt() {
    let w = world(Duration::from_secs(5));
    w.reachable();
    w.call(&ui(), method::SHARE_CREATE, json!({"agent": "api", "patch": {"add": {"teams": ["platform"]}}})).unwrap();
    w.call(&ui(), method::SHARE_GO_LIVE, json!({"share": "api"})).unwrap();

    // Widening from an agent: asked, with the diff lines.
    let t = w.answer(true, |p| {
        assert_eq!(p.action, method::SHARE_SET);
        assert!(!p.rememberable);
        assert_eq!(p.details[0], "▲ audience.people: + dave@example.com");
        assert_eq!(p.details[1], "▲ limits.perCaller.messagesPerDay: 20 → 50");
    });
    let c: ShareChanged = serde_json::from_value(
        w.call(
            &my_agent(),
            method::SHARE_SET,
            json!({"share": "api", "patch": {"add": {"people": [{"provider": "tailscale", "subject": "dave@example.com"}]}, "messagesPerDay": 50}}),
        )
        .unwrap(),
    )
    .unwrap();
    t.join().unwrap();
    assert!(c.approved);
    assert_eq!(c.diff.0.len(), 2);

    // Denied: nothing changes, exit 3 on the CLI.
    let t = w.answer(false, |_| {});
    let e = w.call(&user(), method::SHARE_SET, json!({"share": "api", "patch": {"capability": "write_approval"}})).unwrap_err();
    t.join().unwrap();
    assert_eq!(e.code, code::DENIED);
    assert_eq!(w.svc.find("api").unwrap().capability, Capability::Read);

    // Narrowing: direct for everyone.
    for (who, patch) in [
        (my_agent(), json!({"remove": {"people": [{"provider": "tailscale", "subject": "dave@example.com"}]}})),
        (user(), json!({"messagesPerDay": 5})),
        (my_agent(), json!({"exclude": ["docs/"]})),
    ] {
        let c: ShareChanged = serde_json::from_value(w.call(&who, method::SHARE_SET, json!({"share": "api", "patch": patch})).unwrap()).unwrap();
        assert!(!c.diff.widens() && !c.approved);
    }
    // A dry run shows the diff and changes nothing (the dialog's Save).
    let c: ShareChanged = serde_json::from_value(w.call(&my_agent(), method::SHARE_SET, json!({"share": "api", "patch": {"queue": 20}, "dryRun": true})).unwrap()).unwrap();
    assert!(c.dry_run && c.diff.widens());
    assert_eq!(w.svc.find("api").unwrap().limits.queue, 10);
    // Pause and stop: narrowing, direct; resume widens and asks.
    w.call(&my_agent(), method::SHARE_PAUSE, json!({"share": "api"})).unwrap();
    assert!(w.approvals.pending().is_empty());
    let t = w.answer(true, |p| assert_eq!(p.details, ["▲ state: paused → live"]));
    w.call(&my_agent(), method::SHARE_RESUME, json!({"share": "api"})).unwrap();
    t.join().unwrap();
    // Blocking narrows; letting in again asks.
    w.call(&my_agent(), method::SHARE_BLOCK, json!({"person": "tailscale:eve@example.com"})).unwrap();
    assert_eq!(w.mem.blocked(), ["tailscale:eve@example.com"]);
    let t = w.answer(true, |p| assert!(p.summary.contains("again")));
    w.call(&my_agent(), method::SHARE_BLOCK, json!({"person": "tailscale:eve@example.com", "blocked": false})).unwrap();
    t.join().unwrap();
    w.call(&user(), method::SHARE_STOP, json!({"share": "api"})).unwrap();
    assert!(w.mem.shares().is_empty());
    assert!(w.approvals.pending().is_empty());
}

#[test]
fn team_changes_ask_outside_the_ui() {
    let w = world(Duration::from_millis(100));
    let e = w.call(&my_agent(), method::TEAM_ADD_MEMBER, json!({"team": "platform", "login": "erin@example.com", "network": "tailscale"})).unwrap_err();
    assert_eq!(e.code, code::APPROVAL_TIMEOUT, "asked, nobody answered");
    let added = w.call(&ui(), method::TEAM_ADD_MEMBER, json!({"team": "9f2c1a", "login": "erin@example.com", "network": "tailscale"})).unwrap();
    assert_eq!(added["member"]["claudeOnly"], true);
    assert_eq!(added["member"]["person"], "tailscale:erin@example.com");
    let created = w.call(&ui(), method::TEAM_CREATE, json!({"name": "Mobile", "listed": false})).unwrap();
    assert_eq!(created["card"]["mine"], "admin");
    assert_eq!(created["card"]["members"], 1);
    assert_eq!(code_of(w.call(&user(), method::TEAM_CREATE, json!({"name": "Ops"}))), code::APPROVAL_TIMEOUT);
    // Leaving narrows: direct.
    w.call(&my_agent(), method::TEAM_LEAVE, json!({"team": "Mobile"})).unwrap();
    assert_eq!(code_of(w.call(&ui(), method::TEAM_SHOW, json!({"team": "nope"}))), code::NOT_FOUND);
}

#[test]
fn asking_needs_teams_and_for_agents_the_switch_and_a_budget() {
    let w = world(Duration::from_millis(100));
    let ask = json!({"share": "carol/docs", "message": "where are tokens rotated?"});
    assert_eq!(code_of(w.call(&user(), method::ASK_SEND, ask.clone())), code::NOT_ALLOWED, "Use teams is off");
    w.mem.set_settings(CollabSettings { use_teams: true, ..Default::default() }).unwrap();
    let v = w.call(&user(), method::ASK_SEND, ask.clone()).unwrap();
    assert_eq!(v["reply"], "echo: where are tokens rotated? (agent none)");
    let e = w.call(&my_agent(), method::ASK_SEND, ask.clone()).unwrap_err();
    assert_eq!(e.code, code::NOT_ALLOWED);
    assert!(e.message.contains("Let my agents ask shared agents"));
    // Turning the switch on from an agent asks the owner.
    let t = w.answer(true, |p| assert_eq!(p.details, ["▲ Let my agents ask shared agents: off → on"]));
    w.call(&my_agent(), method::COLLAB_SETTINGS_SET, json!({"useTeams": true, "agentsAsk": true, "agentBudgetPerHour": 2})).unwrap();
    t.join().unwrap();
    let v = w.call(&my_agent(), method::ASK_SEND, ask.clone()).unwrap();
    assert_eq!(v["reply"], "echo: where are tokens rotated? (agent helper)", "labelled from the process tree");
    w.call(&my_agent(), method::ASK_SEND, ask.clone()).unwrap();
    assert_eq!(code_of(w.call(&my_agent(), method::ASK_SEND, ask.clone())), code::QUOTA, "2 per hour");
    // Turning it off again: direct.
    w.call(&my_agent(), method::COLLAB_SETTINGS_SET, json!({"useTeams": true, "agentBudgetPerHour": 2})).unwrap();
    assert!(w.approvals.pending().is_empty());
    // The owner's refusals come through with their codes.
    assert_eq!(code_of(w.call(&user(), method::ASK_SEND, json!({"share": "carol/docs", "message": "over quota"}))), code::QUOTA);
    assert_eq!(code_of(w.call(&user(), method::ASK_SEND, json!({"share": "dave/x", "message": "hi"}))), code::NOT_FOUND);
    assert_eq!(code_of(w.call(&user(), method::ASK_SEND, json!({"share": "docs", "message": "hi"}))), code::BAD_PARAMS);
    let list = w.call(&user(), method::ASK_LIST, json!({"owner": "carol"})).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1, "one turn, kept once");
    assert_eq!(w.call(&user(), method::ASK_GET, json!({"turn": "c_new/1"})).unwrap()["share"], "carol/docs");
}

/// Sessions on the owner's own login can't go live (sessions.md §6.3).
fn own_login_rule() -> super::memory::LiveRule {
    Arc::new(|s: &pitwall_proto::collab::ShareSettings| super::LiveCheck {
        blocks: if s.access.account.as_deref() == Some("claude-sharing") {
            vec![]
        } else {
            vec![format!("{} would run on your own Claude Code login (~/.claude). …", s.slug)]
        },
        login: super::summary::Login::Account { name: (s.access.account.as_deref() == Some("claude-sharing")).then(|| "Sharing".into()) },
    })
}

#[test]
fn going_live_on_the_owners_own_login_is_refused_before_anyone_is_asked() {
    let mut w = world(Duration::from_secs(5));
    let mem = Arc::new(
        MemoryCollab::new(Me { login: "alice@example.com".into(), name: "alice".into() }, None)
            .with_guest_runner(Arc::new(Sandboxed))
            .with_live_rule(own_login_rule()),
    );
    let p = w.project.to_string_lossy().into_owned();
    w.svc = CollabService::new(mem.clone(), w.approvals.clone(), move || vec![agent("a1", "api", &p)]);
    w.mem = mem;
    w.reachable();
    let people = json!({"people": [{"provider": "tailscale", "subject": "bob@example.com"}]});
    let v: ShareView = serde_json::from_value(w.call(&user(), method::SHARE_CREATE, json!({"agent": "api", "patch": {"add": people}})).unwrap()).unwrap();
    assert_eq!(v.live_blocks, ["api would run on your own Claude Code login (~/.claude). …"]);
    assert!(v.summary.text.contains("It would run on your own Claude Code login, which holds your other conversations"), "{}", v.summary.text);
    // Guests may try it on any login.
    w.call(&user(), method::SHARE_GUEST, json!({"share": "api", "probes": true, "only": ["env-file"]})).unwrap();
    // Going live: refused with the reason, nobody asked, from the CLI and the window.
    for who in [user(), ui(), my_agent()] {
        let e = w.call(&who, method::SHARE_GO_LIVE, json!({"share": "api"})).unwrap_err();
        assert_eq!(e.code, code::NOT_ALLOWED);
        assert!(e.message.contains("your own Claude Code login"), "{}", e.message);
    }
    assert!(w.approvals.pending().is_empty());
    // A separate account: live.
    w.call(&user(), method::SHARE_SET, json!({"share": "api", "patch": {"account": "claude-sharing"}})).unwrap();
    let t = w.answer(true, |_| {});
    w.call(&user(), method::SHARE_GO_LIVE, json!({"share": "api"})).unwrap();
    t.join().unwrap();
    // Moving the live share back onto the owner's login: refused before asking.
    let e = w.call(&my_agent(), method::SHARE_SET, json!({"share": "api", "patch": {"account": "default"}})).unwrap_err();
    assert_eq!(e.code, code::NOT_ALLOWED);
    assert!(w.approvals.pending().is_empty());
    assert_eq!(w.svc.find("api").unwrap().access.account.as_deref(), Some("claude-sharing"));
}
