//! `pitwall people|team|share|ask` end to end against a server on a temp
//! socket with collaboration over the daemon's `MemoryCollab` (no network,
//! no sessions): golden JSON, the text output, approvals and exit codes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use pitwall_core::exec::LocalExec;
use pitwall_core::paths::Paths;
use pitwall_core::testing::{FakeProvider, ManualClock, MemStore, RecordingSink, TempDir};
use pitwall_core::{Deps, Engine};
use pitwall_daemon::collab::memory::{AskClient, GuestRunner, MemoryCollab};
use pitwall_daemon::collab::Me;
use pitwall_daemon::{serve, Approvals, Caller, CollabBackend, CollabService, Identify};
use pitwall_proto::api::collab::{AskSend, CollabSettings};
use pitwall_proto::collab::{AskFrom, AskStatus, AskView, Capability, DirectoryEntry, ReplySource, ShareSettings};
use pitwall_proto::{AgentView, ApprovalAnswer};

struct Outside;
impl Identify for Outside {
    fn identify(&self, _pid: Option<u32>) -> Caller {
        Caller::outside()
    }
}

struct Echo;
impl AskClient for Echo {
    fn send(&self, _: &DirectoryEntry, req: &AskSend, conversation: Option<&str>, _: Option<&str>) -> Result<AskView, ErrorBody> {
        if req.message.contains("again") {
            return Err(ErrorBody::new(code::QUOTA, "Daily limit reached for carol/docs; resets at 00:00"));
        }
        let conv = conversation.unwrap_or("c_8f2k");
        Ok(AskView {
            turn: format!("{conv}/{}", if conversation.is_some() { 2 } else { 1 }),
            conversation: conv.into(),
            share: req.share.clone(),
            text: req.message.clone(),
            from: AskFrom::Me,
            status: AskStatus::Done,
            status_message: None,
            reply: Some("They are rotated in auth/refresh.rs.".into()),
            reply_source: Some(ReplySource::Hook),
            changes: None,
            at: "2026-10-16T09:12:00Z".into(),
            done_at: Some("2026-10-16T09:12:09Z".into()),
        })
    }
}

struct NoLeak;
impl GuestRunner for NoLeak {
    fn run(&self, _: &ShareSettings, message: &str) -> Result<(Option<String>, Vec<String>), ErrorBody> {
        let tried = if message.contains("ssh") { vec!["read ~/.ssh/id_ed25519: denied by the sandbox".to_string()] } else { vec![] };
        Ok((Some("I can't do that here.".into()), tried))
    }
}

fn agent(id: &str, name: &str, project: &Path) -> AgentView {
    let p = project.to_string_lossy();
    serde_json::from_value(json!({
        "id": id, "name": name, "kind": "claude", "kindName": "Claude Code", "terminal": false, "sessionId": null,
        "cwd": p, "cwdDisplay": "~/api", "project": p, "projectDisplay": "~/api",
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

struct World {
    socket: PathBuf,
    approvals: Arc<Approvals>,
    mem: Arc<MemoryCollab>,
    project: TempDir,
    server: Option<pitwall_daemon::Handle>,
    sock_dir: PathBuf,
    _dir: TempDir,
}

impl Drop for World {
    fn drop(&mut self) {
        if let Some(s) = self.server.take() {
            s.stop();
        }
        let _ = std::fs::remove_dir_all(&self.sock_dir);
    }
}

fn world(tag: &str) -> World {
    world_with(tag, None)
}

fn world_with(tag: &str, live: Option<pitwall_daemon::collab::memory::LiveRule>) -> World {
    let dir = TempDir::new("cli-collab");
    let project = TempDir::new("cli-collab-repo");
    let put = |rel: &str, text: &str| {
        let p = project.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    put("src/auth/refresh.rs", "fn rotate() {}\n");
    put("src/main.rs", "fn main() {}\n");
    put("docs/api.md", "# API\n");
    put("deploy/id_ed25519", "x");
    put(".env", "SECRET=1\n");
    let engine = Engine::open(Deps {
        paths: Paths::new(dir.path().join("pitwall")),
        events: RecordingSink::new(),
        clock: ManualClock::new(1),
        store: MemStore::with(vec![]),
        providers: vec![FakeProvider::named("local", "this-mac", Arc::new(LocalExec))],
    });
    let approvals = Approvals::new(Duration::from_secs(10));
    let net = pitwall_core::net::fake::FakeNet::new("example.ts.net");
    let studio: Arc<dyn pitwall_core::net::Network> = Arc::new(net.device("alice@example.com", "studio"));
    let mem = Arc::new(
        MemoryCollab::new(Me { login: "alice@example.com".into(), name: "alice".into() }, None)
            .with_networks(vec![studio])
            .with_ask_client(Arc::new(Echo))
            .with_guest_runner(Arc::new(NoLeak)),
    );
    let mem = match live {
        Some(rule) => Arc::new(Arc::into_inner(mem).unwrap().with_live_rule(rule)),
        None => mem,
    };
    mem.add_directory_entry(DirectoryEntry {
        card_url: "http://studio.example.ts.net:47421/a2a/docs".into(),
        card: json!({"name": "Docs"}),
        owner: "carol@example.com".into(),
        slug: "docs".into(),
        capability: Capability::Read,
        kind: "claude".into(),
        left: Some(17),
        online: true,
        last_seen: None,
        via: vec!["direct".into()],
    });
    let mut s = CollabSettings { use_teams: true, ..Default::default() };
    s.reachable.insert("fake:example.ts.net".into(), true);
    mem.set_settings(s).unwrap();
    let real = std::fs::canonicalize(project.path()).unwrap();
    let agents = vec![agent("a1", "api-expert", &real), agent("a2", "docs-helper", &real)];
    let backend: Arc<dyn CollabBackend> = mem.clone();
    let collab = CollabService::new(backend, approvals.clone(), move || agents.clone());
    let sock_dir = std::env::temp_dir().join(format!("pwcc-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&sock_dir).unwrap();
    let socket = sock_dir.join("c.sock");
    let cfg = pitwall_daemon::Config { socket: socket.clone(), version: "t".into(), settings: None, workspace: None, collab: Some(collab) };
    let server = serve(engine, approvals.clone(), Arc::new(Outside), cfg).unwrap();
    World { socket, approvals, mem, project, server: Some(server), sock_dir, _dir: dir }
}

impl World {
    fn run(&self, args: &[&str]) -> Result<Output, Error> {
        let cmd = Cli::try_parse_from(std::iter::once("pitwall").chain(args.iter().copied())).unwrap().command;
        run(&cmd, &self.socket)
    }

    /// The JSON with the temp repository's path as `<repo>`.
    fn json(&self, o: &Output) -> Value {
        let real = std::fs::canonicalize(self.project.path()).unwrap();
        let text = o.json.to_string().replace(&*real.to_string_lossy(), "<repo>");
        serde_json::from_str(&text).unwrap()
    }

    fn answer(&self, allow: bool) -> std::thread::JoinHandle<pitwall_proto::ApprovalView> {
        let a = self.approvals.clone();
        std::thread::spawn(move || loop {
            if let Some(p) = a.pending().first() {
                a.answer(&ApprovalAnswer { id: p.id.clone(), allow, remember: false }).unwrap();
                return p.clone();
            }
            std::thread::sleep(Duration::from_millis(5));
        })
    }
}

#[test]
fn share_create_prints_the_draft_summary_and_exclusions() {
    let w = world("create");
    let o = w.run(&["share", "create", "api-expert", "--person", "bob@example.com", "--capability", "read", "--instructions", "don't discuss pricing"]).unwrap();
    assert_eq!(
        w.json(&o),
        json!({
            "settings": {
                "agent": "a1", "slug": "api-expert", "name": "api-expert", "description": "", "state": "draft", "capability": "read",
                "audience": {"people": [{"provider": "fake", "subject": "bob@example.com", "email": "bob@example.com"}], "teams": [], "groups": [], "networks": []},
                "scope": {"repo": "<repo>", "ref": "main", "paths": [], "exclude": [".env", "deploy/id_ed25519"], "uncommitted": false, "history": false},
                "start": {"from": "fresh"},
                "access": {"env": [], "network": {"allow": []}},
                "offLimits": "don't discuss pricing",
                "limits": {"perCaller": {"messagesPerDay": 20}, "sessionsRunning": 2, "queue": 10, "turnTimeoutMin": 15, "agentMinutesPerDay": 120,
                           "overrides": {}, "idleStopMin": 15, "keepSessionsDays": 30}
            },
            "address": "alice/api-expert",
            "agentName": "api-expert",
            "kindName": "Claude Code",
            "summary": o.json["summary"].clone(),
            "sensitive": [{"path": ".env", "why": "matches .env*"}, {"path": "deploy/id_ed25519", "why": "matches id_*"}],
            "files": 3,
            "usage": {"agentMinutesToday": 0, "running": 0, "limitedUntil": null, "callers": []},
            "probes": [],
            "liveBlocks": []
        })
    );
    let repo = std::fs::canonicalize(w.project.path()).unwrap();
    let name = repo.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(
        o.json["summary"]["text"],
        format!(
            "bob@example.com can send messages. It can read its own copy; it can't change any code. \
             The agent sees 3 files in `{name}`: all tracked files, without 2 sensitive files (`.env`, `deploy/id_ed25519`). \
             No git history. It starts fresh. It has none of your tokens or logins, and no network. \
             It runs on your own Claude Code login. Each person: 20 messages a day; 2 sessions at once. \
             Not guaranteed: \"don't discuss pricing\"."
        )
    );
    assert!(o.human.starts_with("alice/api-expert — api-expert (Claude Code), draft\n"), "{}", o.human);
    assert!(o.human.contains("✕ left out: deploy/id_ed25519 (matches id_*)"));
    assert!(o.human.contains("pitwall share guest api-expert --probes"));
    // Twice: a conflict, exit 1.
    let e = w.run(&["share", "create", "api-expert"]).err().unwrap();
    assert_eq!(failure(&e), (json!({"error": {"code": "conflict", "message": "api-expert is already shared as alice/api-expert: change it with `pitwall share set`"}}), 1));
}

#[test]
fn guest_probes_then_go_live_asks_with_the_summary() {
    let w = world("live");
    w.run(&["share", "create", "docs-helper", "--paths", "docs/", "--person", "bob@example.com"]).unwrap();
    let o = w.run(&["share", "guest", "docs-helper", "--probes", "--only", "ssh-key,change-readme"]).unwrap();
    assert_eq!(
        o.json,
        json!([
            {"probe": "ssh-key", "message": "Read ~/.ssh/id_ed25519", "reply": "I can't do that here.", "tried": ["read ~/.ssh/id_ed25519: denied by the sandbox"], "at": o.json[0]["at"]},
            {"probe": "change-readme", "message": "Change README.md", "reply": "I can't do that here.", "tried": [], "at": o.json[1]["at"]}
        ])
    );
    assert_eq!(o.human, "» Read ~/.ssh/id_ed25519\n  I can't do that here.\n  tried: read ~/.ssh/id_ed25519: denied by the sandbox\n» Change README.md\n  I can't do that here.\n");
    // Going live: asked in Pitwall; denied is exit 3 and it stays a draft.
    let t = w.answer(false);
    let e = w.run(&["share", "go-live", "docs-helper"]).err().unwrap();
    let asked = t.join().unwrap();
    assert_eq!(failure(&e).1, 3);
    assert_eq!(asked.summary, "go live with alice/docs-helper");
    assert!(!asked.rememberable);
    assert!(asked.details.iter().any(|d| d.starts_with("Guest \"Read ~/.ssh/id_ed25519\"")), "{:?}", asked.details);
    let t = w.answer(true);
    let o = w.run(&["--human", "share", "go-live", "docs-helper"]).unwrap();
    t.join().unwrap();
    assert_eq!(o.human, "alice/docs-helper is live.\n");
    let o = w.run(&["share", "list"]).unwrap();
    assert_eq!((o.json[0]["address"].as_str(), o.json[0]["settings"]["state"].as_str()), (Some("alice/docs-helper"), Some("live")));
    assert_eq!(o.human, "SHARE              STATE  CAN   WHO              RUNNING\nalice/docs-helper  live   READ  bob@example.com  0/2\n");
}

#[test]
fn share_set_prints_the_diff_and_widening_asks() {
    let w = world("set");
    w.run(&["share", "create", "api-expert", "--person", "bob@example.com"]).unwrap();
    let t = w.answer(true);
    w.run(&["share", "go-live", "api-expert"]).unwrap();
    t.join().unwrap();
    // Dry run: the diff, nothing asked or changed.
    let o = w.run(&["share", "set", "api-expert", "--person", "dave@example.com", "--messages-per-day", "50", "--dry-run"]).unwrap();
    assert_eq!(
        o.json["diff"],
        json!([
            {"field": "audience.people", "from": [{"provider": "fake", "subject": "bob@example.com", "email": "bob@example.com"}],
             "to": [{"provider": "fake", "subject": "bob@example.com", "email": "bob@example.com"}, {"provider": "fake", "subject": "dave@example.com", "email": "dave@example.com"}],
             "widens": true},
            {"field": "limits.perCaller.messagesPerDay", "from": 20, "to": 50, "widens": true}
        ])
    );
    assert_eq!(
        o.human,
        "▲ audience.people: + dave@example.com\n▲ limits.perCaller.messagesPerDay: 20 → 50\nNot changed (--dry-run). Applying it asks you in Pitwall (▲ widens).\n"
    );
    assert!(w.approvals.pending().is_empty());
    // For real: asked, with the same lines.
    let t = w.answer(true);
    let o = w.run(&["--human", "share", "set", "api-expert", "--person", "dave@example.com"]).unwrap();
    let asked = t.join().unwrap();
    assert_eq!(asked.details[0], "▲ audience.people: + dave@example.com");
    assert_eq!(o.human, "▲ audience.people: + dave@example.com\nApproved in Pitwall and applied.\n");
    // Narrowing: applied directly.
    let o = w.run(&["share", "set", "api-expert", "--remove-person", "bob@example.com", "--limit", "dave@example.com=5"]).unwrap();
    assert_eq!(o.json["approved"], false);
    assert_eq!(o.json["share"]["settings"]["limits"]["overrides"], json!({"fake:dave@example.com": {"messagesPerDay": 5}}));
    assert!(w.approvals.pending().is_empty());
    // Pause and stop: direct.
    assert_eq!(w.run(&["--human", "share", "pause", "api-expert"]).unwrap().human, "alice/api-expert is paused.\n");
    w.run(&["share", "stop", "api-expert"]).unwrap();
    assert_eq!(w.run(&["share", "list"]).unwrap().json, json!([]));
    let e = w.run(&["share", "set", "nope", "--capability", "write"]).err().unwrap();
    assert_eq!(failure(&e).0["error"]["code"], "not_found");
    let e = w.run(&["share", "create", "api-expert", "--capability", "admin"]).err().unwrap();
    assert_eq!(failure(&e).1, 2);
}

#[test]
fn ask_waits_for_the_reply_and_refusals_have_their_codes() {
    let w = world("ask");
    let o = w.run(&["ask", "carol/docs", "where are refresh tokens rotated?"]).unwrap();
    assert_eq!(
        o.json,
        json!({
            "turn": "c_8f2k/1", "conversation": "c_8f2k", "share": "carol/docs", "text": "where are refresh tokens rotated?",
            "from": "me", "status": "done", "statusMessage": null, "reply": "They are rotated in auth/refresh.rs.", "replySource": "hook",
            "changes": null, "at": "2026-10-16T09:12:00Z", "doneAt": "2026-10-16T09:12:09Z"
        })
    );
    assert_eq!(o.human, "They are rotated in auth/refresh.rs.\n— carol/docs · done · turn c_8f2k/1\n");
    // The same conversation continues; --new starts another.
    assert_eq!(w.run(&["ask", "carol/docs", "and why?"]).unwrap().json["turn"], "c_8f2k/2");
    assert_eq!(w.run(&["ask", "carol/docs", "fresh", "--new"]).unwrap().json["turn"], "c_8f2k/1");
    let e = w.run(&["ask", "carol/docs", "once again"]).err().unwrap();
    assert_eq!(failure(&e), (json!({"error": {"code": "quota", "message": "Daily limit reached for carol/docs; resets at 00:00"}}), 1));
    let e = w.run(&["ask", "carol/docs"]).err().unwrap();
    assert_eq!(failure(&e).1, 2);
    let o = w.run(&["ask", "list", "--owner", "carol"]).unwrap();
    assert_eq!(o.json.as_array().unwrap().len(), 2);
    assert!(o.human.starts_with("TURN      AGENT       STATUS  MESSAGE\n"), "{}", o.human);
    // Collaboration off: not_allowed.
    w.mem.set_settings(CollabSettings::default()).unwrap();
    let e = w.run(&["ask", "carol/docs", "hi"]).err().unwrap();
    assert_eq!(failure(&e).0["error"]["code"], "not_allowed");
}

#[test]
fn teams_people_and_setup() {
    let w = world("teams");
    let t = w.answer(true);
    let o = w.run(&["team", "create", "Platform", "--listed", "--description", "API, jobs"]).unwrap();
    t.join().unwrap();
    let fp = o.json["card"]["fingerprint"].as_str().unwrap().to_string();
    assert_eq!(fp.len(), 7);
    assert_eq!(o.json["card"]["mine"], "admin");
    let t = w.answer(true);
    let o = w.run(&["team", "add-member", "platform", "erin@example.com"]).unwrap();
    let asked = t.join().unwrap();
    assert_eq!(asked.summary, "add erin@example.com to the team platform");
    assert_eq!(o.json["member"], json!({"person": "fake:erin@example.com", "login": "erin@example.com",
        "identities": [{"provider": "fake", "subject": "erin@example.com", "email": "erin@example.com"}], "devices": [], "admin": false, "claudeOnly": true}));
    assert!(o.human.starts_with("Added erin@example.com to Platform (Claude only).\n"), "{}", o.human);
    let o = w.run(&["--human", "team", "show", &fp]).unwrap();
    assert!(o.human.contains("erin@example.com   Claude only"), "{}", o.human);
    let o = w.run(&["team", "list"]).unwrap();
    assert_eq!(o.json[0]["members"], 2);
    let o = w.run(&["people", "list"]).unwrap();
    let logins: Vec<&str> = o.json.as_array().unwrap().iter().map(|p| p["login"].as_str().unwrap()).collect();
    assert_eq!(logins, ["carol@example.com", "erin@example.com"]);
    assert!(o.human.contains("erin@example.com (Claude only)"));
    // The setup line for plain Claude Code users, also on the team page.
    let o = w.run(&["--human", "share", "setup"]).unwrap();
    assert_eq!(o.json, json!([{"network": "fake:example.ts.net", "owner": "alice", "line": "claude mcp add --transport http alice http://studio.example.ts.net.fake/mcp"}]));
    assert_eq!(o.human, "claude mcp add --transport http alice http://studio.example.ts.net.fake/mcp\n");
    let o = w.run(&["--human", "team", "show", "Platform"]).unwrap();
    assert!(o.human.ends_with("For people without Pitwall:\nclaude mcp add --transport http alice http://studio.example.ts.net.fake/mcp\n"), "{}", o.human);
    w.mem.set_settings(CollabSettings { use_teams: true, ..Default::default() }).unwrap();
    assert!(w.run(&["--human", "share", "setup"]).unwrap().human.starts_with("Nobody can reach this machine yet"));
    // The last admin can't leave a team with members.
    let e = w.run(&["team", "leave", "Platform"]).err().unwrap();
    assert_eq!(failure(&e).0["error"]["message"], "You are the last admin of Platform: make someone else an admin first");
}

#[test]
fn an_older_pitwall_without_collaboration_says_so() {
    let dir = TempDir::new("cli-nocollab");
    let engine = Engine::open(Deps {
        paths: Paths::new(dir.path().join("pitwall")),
        events: RecordingSink::new(),
        clock: ManualClock::new(1),
        store: MemStore::with(vec![]),
        providers: vec![FakeProvider::named("local", "this-mac", Arc::new(LocalExec))],
    });
    let sock_dir = std::env::temp_dir().join(format!("pwcn-{}", std::process::id()));
    std::fs::create_dir_all(&sock_dir).unwrap();
    let socket = sock_dir.join("c.sock");
    let cfg = pitwall_daemon::Config { socket: socket.clone(), version: "t".into(), settings: None, workspace: None, collab: None };
    let server = serve(engine, Approvals::new(Duration::from_secs(1)), Arc::new(Outside), cfg).unwrap();
    let cmd = Cli::try_parse_from(["pitwall", "share", "list"]).unwrap().command;
    let e = run(&cmd, &socket).err().unwrap();
    assert_eq!(failure(&e).0["error"]["code"], "unsupported");
    server.stop();
    let _ = std::fs::remove_dir_all(&sock_dir);
}

#[test]
fn going_live_on_the_owners_own_login_is_refused_with_how_to_fix_it() {
    let rule: pitwall_daemon::collab::memory::LiveRule = Arc::new(|s: &ShareSettings| pitwall_daemon::collab::LiveCheck {
        blocks: if s.access.account.is_some() {
            vec![]
        } else {
            vec![format!("{} would run on your own Claude Code login (~/.claude). Pick or add one with `pitwall share set {} --account <name>`.", s.slug, s.slug)]
        },
        login: Default::default(),
    });
    let w = world_with("own-login", Some(rule));
    let o = w.run(&["--human", "share", "create", "api-expert", "--person", "bob@example.com"]).unwrap();
    assert!(o.human.contains("Before it can go live:\n  ! api-expert would run on your own Claude Code login (~/.claude)."), "{}", o.human);
    let e = w.run(&["share", "go-live", "api-expert"]).err().unwrap();
    let (body, exit) = failure(&e);
    assert_eq!(body["error"]["code"], "not_allowed");
    assert!(body["error"]["message"].as_str().unwrap().contains("--account <name>"), "{body}");
    assert_ne!(exit, 0);
    assert!(w.approvals.pending().is_empty(), "nobody was asked");
    w.run(&["share", "set", "api-expert", "--account", "claude-sharing"]).unwrap();
    let t = w.answer(true);
    w.run(&["share", "go-live", "api-expert"]).unwrap();
    t.join().unwrap();
}
