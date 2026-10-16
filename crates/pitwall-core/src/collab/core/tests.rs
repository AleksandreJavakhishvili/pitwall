//! The conversation core on the Fake network, a scripted session host, a
//! fake sandbox, copies and proxy, and fake agent kinds: conversations,
//! every limit, the queue, needs-you, cancel, audiences.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::collab::session::{HostSink, SharedAgent, Snapshot};
use crate::collab::turn::Reply;
use crate::collab::teams::keys::{Ed25519, Ed25519Signer};
use crate::collab::teams::{TeamStore, TeamsDeps};
use crate::collab::time::ManualTime;
use crate::error::ErrorCode;
use crate::net::fake::FakeNet;
use crate::sandbox::{Support, Wrapped};
use crate::testing::TempDir;
use pitwall_proto::collab::{Audience, JoinPolicy, LimitOverride, ReplySource, Scope};

// ------------------------------------------------------------------ fakes

/// A session host whose agents do nothing until the test says so.
#[derive(Default)]
pub(crate) struct FakeHost {
    sink: Mutex<Option<HostSink>>,
    pub launches: Mutex<Vec<HostLaunch>>,
    pub sent: Mutex<Vec<(String, String)>>,
    pub interrupts: Mutex<Vec<String>>,
    pub stops: Mutex<Vec<String>>,
    pub removed: Mutex<Vec<String>>,
    partial: Mutex<BTreeMap<String, Reply>>,
    /// Agents come up at once (`Ready` from inside `launch`).
    pub manual_ready: Mutex<bool>,
    pub fail_launch: Mutex<bool>,
    kinds: Mutex<BTreeMap<String, AgentKind>>,
    /// The owner's variables a share may pass.
    pub env: Mutex<BTreeMap<String, String>>,
}

impl FakeHost {
    pub fn emit(&self, agent: &str, event: SessionEvent) {
        let sink = self.sink.lock().unwrap().clone().expect("subscribed");
        sink(agent, event);
    }

    pub fn reply(&self, agent: &str, text: &str) {
        self.emit(agent, SessionEvent::TurnEnded(Some(Reply { text: text.into(), source: ReplySource::Hook })));
    }

    pub fn set_partial(&self, agent: &str, text: &str) {
        self.partial.lock().unwrap().insert(agent.into(), Reply { text: text.into(), source: ReplySource::Terminal });
    }

    pub fn last_launch(&self) -> HostLaunch {
        self.launches.lock().unwrap().last().cloned().expect("a launch")
    }

    pub fn sent_to(&self, agent: &str) -> Vec<String> {
        self.sent.lock().unwrap().iter().filter(|(a, _)| a == agent).map(|(_, t)| t.clone()).collect()
    }

    pub fn launches_of(&self, agent: &str) -> Vec<HostLaunch> {
        self.launches.lock().unwrap().iter().filter(|l| l.agent == agent).cloned().collect()
    }

    fn add_kind(&self, id: &str, kind: AgentKind) {
        self.kinds.lock().unwrap().insert(id.into(), kind);
    }
}

impl SessionHost for FakeHost {
    fn subscribe(&self, sink: HostSink) {
        *self.sink.lock().unwrap() = Some(sink);
    }

    fn launch(&self, launch: &HostLaunch) -> Result<()> {
        if *self.fail_launch.lock().unwrap() {
            return Err(PwError::other("no agent here"));
        }
        self.launches.lock().unwrap().push(launch.clone());
        if !*self.manual_ready.lock().unwrap() {
            if let Some(id) = &launch.session_id {
                self.emit(&launch.agent, SessionEvent::SessionId(id.clone()));
            }
            self.emit(&launch.agent, SessionEvent::Ready);
        }
        Ok(())
    }

    fn send(&self, agent: &str, text: &str) -> Result<()> {
        self.sent.lock().unwrap().push((agent.into(), text.into()));
        Ok(())
    }

    fn interrupt(&self, agent: &str) -> Result<()> {
        self.interrupts.lock().unwrap().push(agent.into());
        Ok(())
    }

    fn stop(&self, agent: &str) -> Result<()> {
        self.stops.lock().unwrap().push(agent.into());
        Ok(())
    }

    fn remove(&self, agent: &str) -> Result<()> {
        self.removed.lock().unwrap().push(agent.into());
        Ok(())
    }

    fn partial_reply(&self, agent: &str) -> Option<Reply> {
        self.partial.lock().unwrap().get(agent).cloned()
    }
}

impl OwnerAgents for FakeHost {
    fn agent(&self, id: &str) -> Option<SharedAgent> {
        let kind = self.kinds.lock().unwrap().get(id).cloned()?;
        Some(SharedAgent { kind, project: "/code/api".into(), account: None })
    }

    fn account_env(&self, kind: &AgentKind, _account: Option<&str>) -> Vec<(String, String)> {
        match &kind.account_env {
            Some(var) => vec![(var.clone(), "/accounts/share".into())],
            None => vec![],
        }
    }

    /// One separate account for sharing, `fake-share` ("Share").
    fn account(&self, _kind: &AgentKind, key: &str) -> Option<crate::collab::live::AccountInfo> {
        (key == "fake-share" || key.eq_ignore_ascii_case("share"))
            .then(|| crate::collab::live::AccountInfo { id: "fake-share".into(), name: "Share".into(), dir: "/accounts/share".into() })
    }

    /// Never the developer's own environment.
    fn env_value(&self, name: &str) -> Option<String> {
        self.env.lock().unwrap().get(name).cloned()
    }
}

/// Wraps argv as `sandbox <argv>` and keeps every policy.
#[derive(Default)]
pub(crate) struct FakeSandbox {
    pub policies: Mutex<Vec<SandboxPolicy>>,
}

impl SessionSandbox for FakeSandbox {
    fn name(&self) -> &str {
        "fake"
    }

    fn support(&self) -> Support {
        Support::Ready
    }

    fn wrap(&self, policy: &SandboxPolicy, argv: &[String]) -> Result<Wrapped> {
        self.policies.lock().unwrap().push(policy.clone());
        let mut out = vec!["sandbox".to_string()];
        out.extend(argv.iter().cloned());
        Ok(Wrapped { argv: out, env: policy.env.clone() })
    }
}

#[derive(Default)]
pub(crate) struct FakeCopies {
    pub made: Mutex<Vec<(Scope, PathBuf)>>,
}

impl Snapshots for FakeCopies {
    fn make(&self, scope: &Scope, dest: &Path) -> Result<Snapshot> {
        std::fs::write(dest.join("README.md"), "copy").unwrap();
        self.made.lock().unwrap().push((scope.clone(), dest.to_path_buf()));
        Ok(Snapshot { base_commit: "3f2a1c9".into(), files: 1 })
    }
}

pub(crate) struct FakeProxy;

impl EgressProxies for FakeProxy {
    fn port(&self, _share: &ShareSettings, _hosts: &[String]) -> Result<Option<u16>> {
        Ok(Some(4242))
    }
}

/// A fake agent kind: `fake --id <session>` / `--resume <session>`, hooks
/// per launch, forks with `--fork <base>`.
fn fake_kind(resume: bool, fork: bool) -> AgentKind {
    let mut src = String::from(
        r#"
id = "fake"
name = "Fake agent"
command = "fake"
new_args = ["--id", "{session_id}"]
assign_session_id = true
hooks = "claude-settings"
account_env = "FAKE_CONFIG_DIR"
"#,
    );
    if resume {
        src.push_str("resume_args = [\"--resume\", \"{session_id}\"]\n");
    }
    src.push_str("[share]\nreply = \"hook\"\nmodel_hosts = [\"api.fake.dev\"]\ninstruction_files = [\"FAKE.md\"]\n");
    if fork {
        src.push_str("fork_args = [\"--fork\", \"{base}\", \"--id\", \"{session_id}\"]\n");
    }
    toml::from_str(&src).unwrap()
}

// ------------------------------------------------------------------ world

pub(crate) struct World {
    pub collab: Arc<Collab>,
    pub host: Arc<FakeHost>,
    pub sandbox: Arc<FakeSandbox>,
    pub copies: Arc<FakeCopies>,
    pub clock: Arc<ManualTime>,
    pub net: FakeNet,
    pub teams: Arc<TeamStore>,
    pub dir: TempDir,
}

impl World {
    pub(crate) fn new() -> World {
        World::with(CollabSettings { max_sessions: 4, owner_name: "alice".into() })
    }

    fn with(settings: CollabSettings) -> World {
        let dir = TempDir::new("collab");
        let paths = CollabPaths::new(dir.path().join("collab"));
        let clock = Arc::new(ManualTime::at("2026-10-16T09:00:00+02:00"));
        let net = FakeNet::new("example");
        let alice = net.device("alice@example.com", "studio");
        let host = Arc::new(FakeHost::default());
        host.add_kind("a1", fake_kind(true, false));
        host.add_kind("a2", fake_kind(false, false));
        host.add_kind("a3", fake_kind(true, true));
        let teams = TeamStore::open(TeamsDeps {
            paths: paths.clone(),
            signer: Arc::new(Ed25519Signer::from_seed([7; 32])),
            check: Arc::new(Ed25519),
            clock: clock.clone(),
            audit: Arc::new(AuditLog::new(paths.audit_file())),
            version: "0.3.0".into(),
            label: "studio".into(),
        });
        teams.set_networks(vec![Arc::new(alice.clone())]);
        let sandbox = Arc::new(FakeSandbox::default());
        let copies = Arc::new(FakeCopies::default());
        let collab = Collab::open(CollabDeps {
            paths,
            clock: clock.clone(),
            host: host.clone(),
            agents: host.clone(),
            sandbox: sandbox.clone(),
            snapshots: copies.clone(),
            egress: Arc::new(FakeProxy),
            skills: None,
            teams: Some(teams.clone()),
            hooks: Some(HookWiring { command: "'/pw/pitwall-hook'".into(), socket: PathBuf::from("/pw/hooks.sock"), programs: vec![PathBuf::from("/pw/pitwall-hook")] }),
            settings,
            run: RunMode::Inline,
        });
        collab.set_networks(vec![Arc::new(alice.clone())]);
        World { collab, host, sandbox, copies, clock, net, teams, dir }
    }

    /// A live Read only share of agent `agent`, for `audience`.
    pub(crate) fn share(&self, slug: &str, agent: &str, audience: Audience) -> ShareSettings {
        let share: ShareSettings = serde_json::from_value(serde_json::json!({
            "agent": agent, "slug": slug, "name": slug, "description": "Questions about the API",
            "scope": {"repo": "/code/api", "ref": "main", "paths": ["src/"]},
            "access": {"account": "fake-share"},
        }))
        .unwrap();
        let share = ShareSettings { audience, ..share };
        self.collab.create_share(share).unwrap();
        self.collab.go_live(slug, false).unwrap()
    }

    fn set(&self, slug: &str, f: impl FnOnce(&mut ShareSettings)) -> ShareDiff {
        let mut s = self.collab.share(slug).unwrap();
        f(&mut s);
        self.collab.set_share(slug, s).unwrap()
    }

    pub(crate) fn everyone(&self) -> Audience {
        Audience { networks: vec![self.net.id()], ..Audience::default() }
    }

    /// A caller asking from device `node` (signed in as `login`).
    fn asker(&self, login: &str, node: &str) -> Asker {
        self.net.device(login, node);
        Asker { who: Who::Network(Box::new(self.net.caller(node).unwrap())), agent: None, client: Some("Claude Code".into()), chain: vec![], front: Front::Mcp }
    }

    fn ask(&self, asker: &Asker, share: &str, message: &str) -> Turn {
        self.collab.ask(asker, Ask { share: share.into(), message: message.into(), ..Ask::default() }).unwrap()
    }

    fn agent_of(&self, turn: &Turn) -> String {
        self.collab.agent_of(&turn.conversation).unwrap()
    }

    fn status(&self, asker: &Asker, turn: &Turn) -> TurnStatus {
        self.collab.turn(asker, &turn.id).unwrap().status
    }

    fn advance(&self, minutes: u64) {
        self.clock.advance(Duration::from_secs(minutes * 60));
        self.collab.tick();
    }
}

fn reason(status: &TurnStatus) -> Option<RefusalReason> {
    match status {
        TurnStatus::Refused(r) => Some(r.reason),
        _ => None,
    }
}

// ------------------------------------------------------------------ conversations

#[test]
fn a_message_starts_a_sandboxed_session_and_its_reply_comes_back() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let turn = w.ask(&bob, "api-expert", "Where are refresh tokens rotated?");
    assert_eq!(turn.status, TurnStatus::Working);
    assert_eq!(turn.left.as_ref().unwrap().messages_today, 19);
    assert_eq!(turn.left.as_ref().unwrap().resets_at, "2026-10-17T00:00:00+02:00");

    // An ordinary agent of the shared kind, wrapped in the sandbox, in its copy.
    let launch = w.host.last_launch();
    assert_eq!(launch.kind, "fake");
    assert_eq!(launch.shared.share, "api-expert");
    assert_eq!(launch.shared.agent, "a1");
    assert_eq!(launch.shared.conversation, turn.conversation.0);
    assert!(launch.cwd.ends_with(format!("shares/api-expert/s/{}/work", turn.conversation.0)));
    assert_eq!(&launch.wrapped.argv[..3], &["sandbox".to_string(), "fake".into(), "--id".into()]);
    assert!(!launch.resumed);
    assert_eq!(w.copies.made.lock().unwrap()[0].0.paths, vec!["src/".to_string()]);

    // The message as ordinary input, with the factual "From" line (§8.3.1).
    let agent = w.agent_of(&turn);
    assert_eq!(w.host.sent_to(&agent), vec!["From bob@example.com (Claude Code over MCP):\nWhere are refresh tokens rotated?".to_string()]);
    assert_eq!(w.collab.delivered(&turn.id).unwrap(), w.host.sent_to(&agent)[0]);

    w.host.reply(&agent, "In auth/refresh.rs.");
    let done = w.collab.turn(&bob, &turn.id).unwrap();
    assert_eq!(done.status, TurnStatus::Done);
    assert_eq!(done.reply.unwrap().text, "In auth/refresh.rs.");
    assert!(done.done_at.is_some());
}

#[test]
fn the_sandbox_policy_is_built_from_nothing() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let turn = w.ask(&bob, "api-expert", "hi");
    let policy = w.sandbox.policies.lock().unwrap()[0].clone();
    let folder = w.dir.path().join("collab/shares/api-expert/s").join(&turn.conversation.0);
    assert_eq!(policy.env["HOME"], folder.join("home").to_string_lossy());
    assert_eq!(policy.env["TMPDIR"].trim_end_matches('/'), folder.join("tmp").to_string_lossy());
    assert_eq!(policy.env["HTTPS_PROXY"], "http://127.0.0.1:4242");
    assert_eq!(policy.env["NO_PROXY"], "", "everything goes through the proxy");
    assert_eq!(policy.env["FAKE_CONFIG_DIR"], "/accounts/share");
    assert!(policy.env.keys().all(|k| !k.starts_with("PITWALL")), "{:?}", policy.env.keys());
    assert!(!policy.env.contains_key("SSH_AUTH_SOCK"));
    assert_eq!(policy.egress.proxy_port, Some(4242));
    // Read only: the copy is readable, never writable.
    assert!(policy.read.contains(&folder.join("work")));
    assert!(!policy.read.iter().any(|r| policy.state_dir.starts_with(r)), "the profile is outside what it reads");
    assert!(!policy.write.contains(&folder.join("work")));
    assert!(policy.write.contains(&folder.join("home")) && policy.write.contains(&PathBuf::from("/accounts/share")));
    let ExecRule::Allow(programs) = &policy.exec else { panic!("an allowlist at Read only") };
    // The hook relay is the pitwall-hook binary: runnable and readable, and
    // no curl among the programs.
    assert!(programs.contains(&PathBuf::from("/pw/pitwall-hook")) && policy.read.contains(&PathBuf::from("/pw/pitwall-hook")));
    assert!(!programs.iter().any(|p| p.ends_with("curl")), "{programs:?}");
    assert!(programs.iter().any(|p| p.ends_with("git")));
    assert_eq!(policy.connect, vec![LocalEndpoint::UnixSocket(PathBuf::from("/pw/hooks.sock"))]);
    // The hook relay carries the agent id itself: no PITWALL_* in the environment.
    let argv = &w.host.last_launch().wrapped.argv;
    let settings = argv.iter().find(|a| a.contains("\"hooks\"")).unwrap();
    assert!(settings.contains(&format!("PITWALL_AGENT_ID={}", w.agent_of(&turn))));
}

#[test]
fn the_owners_instructions_and_off_limits_go_into_the_kinds_files() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    w.set("api-expert", |s| s.off_limits = "Don't discuss pricing.".into());
    w.collab.set_instructions("api-expert", "You answer questions about the API.").unwrap();
    let bob = w.asker("bob@example.com", "bob-mbp");
    let turn = w.ask(&bob, "api-expert", "hi");
    let folder = w.dir.path().join("collab/shares/api-expert/s").join(&turn.conversation.0);
    let text = std::fs::read_to_string(folder.join("FAKE.md")).unwrap();
    assert_eq!(text, "You answer questions about the API.\n\nDon't discuss pricing.\n");
}

#[test]
fn the_same_caller_resumes_one_conversation() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t1 = w.ask(&bob, "api-expert", "first");
    let agent = w.agent_of(&t1);
    let session = w.host.last_launch().session_id.unwrap();
    w.host.reply(&agent, "one");

    // Idle stop after idleStopMin; the next message resumes the same session.
    w.advance(16);
    assert_eq!(w.host.stops.lock().unwrap().last().unwrap(), &agent);
    let t2 = w.ask(&bob, "api-expert", "second");
    assert_eq!(t2.conversation, t1.conversation);
    assert_eq!(t2.id.number(), Some(2));
    let launches = w.host.launches_of(&agent);
    assert_eq!(launches.len(), 2);
    assert!(launches[1].resumed);
    assert_eq!(&launches[1].wrapped.argv[1..4], &["fake".to_string(), "--resume".into(), session]);
    assert_eq!(w.copies.made.lock().unwrap().len(), 1, "a resumed session keeps its copy");

    // Same caller from another device (and over A2A): still one conversation.
    w.host.reply(&agent, "two");
    let mut again = w.asker("bob@example.com", "bob-ipad");
    again.front = Front::A2a;
    let t3 = w.ask(&again, "api-expert", "third");
    assert_eq!(t3.conversation, t1.conversation);
    assert_eq!(w.collab.conversations(&bob).unwrap().len(), 1);
}

#[test]
fn a_new_conversation_is_explicit_and_the_old_one_is_kept() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t1 = w.ask(&bob, "api-expert", "first");
    w.host.reply(&w.agent_of(&t1), "one");
    let t2 = w.collab.ask(&bob, Ask { share: "api-expert".into(), message: "fresh".into(), new_conversation: true, ..Ask::default() }).unwrap();
    assert_ne!(t2.conversation, t1.conversation);
    let convs = w.collab.conversations(&bob).unwrap();
    assert_eq!(convs.len(), 2);
    assert!(convs.iter().find(|c| c.id == t2.conversation).unwrap().current);
    assert!(!convs.iter().find(|c| c.id == t1.conversation).unwrap().current);
    // Naming the old one continues it.
    w.host.reply(&w.agent_of(&t2), "two");
    let t3 = w.collab.ask(&bob, Ask { share: "api-expert".into(), message: "back".into(), conversation: Some(t1.conversation.clone()), ..Ask::default() }).unwrap();
    assert_eq!(t3.conversation, t1.conversation);
}

#[test]
fn different_callers_are_isolated() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let carol = w.asker("carol@example.com", "carol-mbp");
    let tb = w.ask(&bob, "api-expert", "bob's question");
    let tc = w.ask(&carol, "api-expert", "carol's question");
    assert_ne!(tb.conversation, tc.conversation);
    let (ab, ac) = (w.agent_of(&tb), w.agent_of(&tc));
    assert_ne!(ab, ac);
    assert_ne!(w.host.launches_of(&ab)[0].cwd, w.host.launches_of(&ac)[0].cwd);
    assert_ne!(w.host.launches_of(&ab)[0].session_id, w.host.launches_of(&ac)[0].session_id);
    // Someone else's turn, conversation or forget look like nothing at all.
    assert!(w.collab.turn(&carol, &tb.id).unwrap_err().is(ErrorCode::NotFound));
    assert!(w.collab.watch(&carol, &tb.id).unwrap_err().is(ErrorCode::NotFound));
    assert!(w.collab.cancel(&carol, &tb.id).unwrap_err().is(ErrorCode::NotFound));
    assert!(w.collab.forget(&carol, &tb.conversation).unwrap_err().is(ErrorCode::NotFound));
    let named = Ask { share: "api-expert".into(), message: "x".into(), conversation: Some(tb.conversation.clone()), ..Ask::default() };
    assert!(w.collab.ask(&carol, named).unwrap_err().is(ErrorCode::NotFound));
    assert_eq!(w.collab.turns(&carol, None).unwrap().len(), 1);
}

#[test]
fn agents_of_one_person_get_their_own_conversations() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let plain = w.asker("bob@example.com", "bob-mbp");
    let mut agent = w.asker("bob@example.com", "bob-mbp");
    agent.agent = Some("client-fix@bob-mbp".into());
    agent.front = Front::A2a;
    let t1 = w.ask(&plain, "api-expert", "from my plain client");
    let t2 = w.ask(&agent, "api-expert", "from my agent");
    assert_ne!(t1.conversation, t2.conversation);
    assert_eq!(w.host.sent_to(&w.agent_of(&t2))[0], "From bob@example.com (agent client-fix on bob-mbp):\nfrom my agent");
    // Both are Bob's: he sees both.
    assert_eq!(w.collab.conversations(&plain).unwrap().len(), 2);
}

#[test]
fn linked_identities_share_a_conversation_and_limits() {
    let w = World::new();
    let cloud = FakeNet::new("cloud");
    cloud.device("u_8kq2", "bob-cloud");
    let mut audience = w.everyone();
    audience.networks.push(cloud.id());
    w.share("api-expert", "a1", audience);
    w.set("api-expert", |s| s.limits.per_caller.messages_per_day = 2);
    let bob = w.asker("bob@example.com", "bob-mbp");
    let bob_cloud = Asker { who: Who::Network(Box::new(cloud.caller("bob-cloud").unwrap())), agent: None, client: Some("Claude Code".into()), chain: vec![], front: Front::Mcp };

    // Unlinked: two people.
    let t1 = w.ask(&bob, "api-expert", "over the tailnet");
    w.host.reply(&w.agent_of(&t1), "ok");
    let t2 = w.ask(&bob_cloud, "api-expert", "over the cloud");
    assert_ne!(t1.conversation, t2.conversation);
    w.host.reply(&w.agent_of(&t2), "ok");

    // The owner links them: one person, one conversation, one quota.
    w.collab.people().link(bob.network_identity(), bob_cloud.network_identity()).unwrap();
    assert_eq!(w.collab.person_of(bob.network_identity()), w.collab.person_of(bob_cloud.network_identity()));
    let t3 = w.ask(&bob_cloud, "api-expert", "again");
    assert!(t3.conversation == t1.conversation || t3.conversation == t2.conversation);
    assert_eq!(reason(&t3.status), Some(RefusalReason::Quota), "2 messages a day across both identities");
}

#[test]
fn team_records_link_identities_too() {
    let w = World::new();
    let team = w.teams.create("Platform", "", true, JoinPolicy::Request).unwrap();
    let bob_ts = w.net.person("bob@example.com");
    w.teams.add_member(&team, bob_ts.clone()).unwrap();
    let member = w.teams.state(&team).unwrap().member_with(&bob_ts).unwrap().member.clone();
    let cloud_id = Identity::new("cloud", "u_8kq2");
    assert_ne!(w.collab.person_of(&bob_ts), w.collab.person_of(&cloud_id));
    w.teams.add_identity(&team, &member, cloud_id.clone()).unwrap();
    assert_eq!(w.collab.person_of(&bob_ts), w.collab.person_of(&cloud_id));
}

trait NetworkIdentity {
    fn network_identity(&self) -> &Identity;
}

impl NetworkIdentity for Asker {
    fn network_identity(&self) -> &Identity {
        match &self.who {
            Who::Network(c) => c.identity(),
            Who::Guest => panic!("a network caller"),
        }
    }
}

#[test]
fn forgetting_a_conversation_removes_its_session_and_copy() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    let agent = w.agent_of(&t);
    w.collab.forget(&bob, &t.conversation).unwrap();
    assert_eq!(w.host.removed.lock().unwrap().as_slice(), [agent]);
    assert!(w.collab.conversations(&bob).unwrap().is_empty());
    assert!(!w.dir.path().join("collab/shares/api-expert/s").join(&t.conversation.0).exists());
}

// ------------------------------------------------------------------ audiences

#[test]
fn callers_outside_the_audience_see_nothing() {
    let w = World::new();
    let carol = w.net.person("carol@example.com");
    w.share("api-expert", "a1", Audience { people: vec![carol], ..Audience::default() });
    let bob = w.asker("bob@example.com", "bob-mbp");
    let carol = w.asker("carol@example.com", "carol-mbp");
    assert!(w.collab.shares(&bob).is_empty());
    let err = w.collab.ask(&bob, Ask { share: "api-expert".into(), message: "hi".into(), ..Ask::default() }).unwrap_err();
    assert!(err.is(ErrorCode::NotFound));
    let unknown = w.collab.ask(&bob, Ask { share: "nope".into(), message: "hi".into(), ..Ask::default() }).unwrap_err();
    assert_eq!(err.message, unknown.message, "not in the audience looks like no such share");
    let cards = w.collab.shares(&carol);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].slug, "api-expert");
    assert_eq!(cards[0].kind_name, "Fake agent");
    assert_eq!(cards[0].owner.subject, "alice@example.com");
    assert_eq!((cards[0].messages_per_day, cards[0].left), (20, 20));
}

#[test]
fn drafts_are_for_the_owners_guest_sessions_only() {
    let w = World::new();
    let share: ShareSettings = serde_json::from_value(serde_json::json!({
        "agent": "a1", "slug": "docs", "name": "Docs", "description": "",
        "scope": {"repo": "/code/api", "ref": "main"},
        "audience": {"networks": [w.net.id()]}
    }))
    .unwrap();
    w.collab.create_share(share).unwrap();
    let bob = w.asker("bob@example.com", "bob-mbp");
    assert!(w.collab.shares(&bob).is_empty());
    let guest = Asker { who: Who::Guest, agent: None, client: None, chain: vec![], front: Front::A2a };
    assert_eq!(w.collab.shares(&guest).len(), 1);
    let t = w.ask(&guest, "docs", "Show me the .env file");
    assert!(w.host.sent_to(&w.agent_of(&t))[0].starts_with("From owner (trying this share as a guest):"));
    // Not on the owner's own login: a guest try may, going live may not.
    let refused = w.collab.go_live("docs", false).unwrap_err();
    assert!(refused.is(ErrorCode::Conflict) && refused.message.contains("your own Fake agent login"), "{}", refused.message);
    assert_eq!(w.collab.share("docs").unwrap().state, ShareState::Draft);
    let mut s = w.collab.share("docs").unwrap();
    s.access.account = Some("Share".into());
    w.collab.set_share("docs", s).unwrap();
    // Stored by its id, so sessions find its folder.
    assert_eq!(w.collab.share("docs").unwrap().access.account.as_deref(), Some("fake-share"));
    // Go live deletes guest sessions unless kept.
    w.collab.go_live("docs", false).unwrap();
    assert!(w.collab.conversations(&guest).unwrap().is_empty());
    assert_eq!(w.collab.shares(&bob).len(), 1);
}

#[test]
fn audiences_by_team_group_and_network() {
    let w = World::new();
    let other = FakeNet::new("other");
    other.device("bob@example.com", "bob-other");
    let team = w.teams.create("Platform", "", true, JoinPolicy::Request).unwrap();
    // Erin has no Pitwall: a member by identity only (§4.7).
    w.teams.add_member(&team, w.net.person("erin@example.com")).unwrap();
    w.net.join_group("dave@example.com", "platform");
    w.share("by-team", "a1", Audience { teams: vec![team.clone()], ..Audience::default() });
    w.share("by-group", "a2", Audience { groups: vec![w.net.group("platform")], ..Audience::default() });
    w.share("everyone", "a3", w.everyone());
    let erin = w.asker("erin@example.com", "erin-mbp");
    let dave = w.asker("dave@example.com", "dave-mbp");
    let bob_other = Asker { who: Who::Network(Box::new(other.caller("bob-other").unwrap())), agent: None, client: None, chain: vec![], front: Front::Mcp };
    let slugs = |a: &Asker| w.collab.shares(a).into_iter().map(|c| c.slug).collect::<Vec<_>>();
    assert_eq!(slugs(&erin), ["by-team", "everyone"]);
    assert_eq!(slugs(&dave), ["by-group", "everyone"]);
    assert!(slugs(&bob_other).is_empty(), "everyone on example, not on other");
    // Leaving the group: at once.
    w.net.leave_group("dave@example.com", "platform");
    let dave = w.asker("dave@example.com", "dave-mbp");
    assert_eq!(slugs(&dave), ["everyone"]);
}

#[test]
fn blocked_people_see_nothing_and_lose_running_turns() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let eve = w.asker("eve@example.com", "eve-mbp");
    let t = w.ask(&eve, "api-expert", "hi");
    w.collab.block(eve.network_identity()).unwrap();
    assert_eq!(w.status(&eve, &t), TurnStatus::Canceled);
    assert_eq!(w.host.interrupts.lock().unwrap().len(), 1);
    assert!(w.collab.shares(&eve).is_empty());
    assert!(w.collab.ask(&eve, Ask { share: "api-expert".into(), message: "x".into(), ..Ask::default() }).unwrap_err().is(ErrorCode::NotFound));
    w.collab.unblock(eve.network_identity()).unwrap();
    assert_eq!(w.collab.shares(&eve).len(), 1);

    // An override's `blocked`: the tool disappears for that person only.
    let key = eve.network_identity().key();
    w.set("api-expert", |s| {
        s.limits.overrides.insert(key, LimitOverride { blocked: true, ..LimitOverride::default() });
    });
    assert!(w.collab.shares(&eve).is_empty());
    assert_eq!(w.collab.shares(&w.asker("bob@example.com", "bob-mbp")).len(), 1);
}

#[test]
fn removing_someone_from_the_audience_ends_their_turns_at_once() {
    let w = World::new();
    let bob_id = w.net.person("bob@example.com");
    let carol_id = w.net.person("carol@example.com");
    w.share("api-expert", "a1", Audience { people: vec![bob_id, carol_id.clone()], ..Audience::default() });
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    let d = w.set("api-expert", |s| s.audience.people = vec![carol_id]);
    assert!(!d.widens());
    let after = w.collab.turn(&bob, &t.id).unwrap();
    assert_eq!(after.status, TurnStatus::Canceled);
    assert_eq!(after.status_message.as_deref(), Some("no longer shared with you"));
}

#[test]
fn revoked_identities_lose_what_they_had() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    w.collab.on_revocation(&Revocation::Identity { identity: bob.network_identity().clone() });
    assert_eq!(w.status(&bob, &t), TurnStatus::Canceled);
}

// ------------------------------------------------------------------ limits

#[test]
fn the_daily_quota_resets_at_the_owners_midnight() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    w.set("api-expert", |s| s.limits.per_caller.messages_per_day = 2);
    let bob = w.asker("bob@example.com", "bob-mbp");
    for n in 0..2 {
        let t = w.ask(&bob, "api-expert", &format!("q{n}"));
        w.host.reply(&w.agent_of(&t), "a");
    }
    let refused = w.ask(&bob, "api-expert", "one more");
    let TurnStatus::Refused(r) = &refused.status else { panic!("refused: {:?}", refused.status) };
    assert_eq!(r.reason, RefusalReason::Quota);
    assert_eq!(r.resets_at.as_deref(), Some("2026-10-17T00:00:00+02:00"));
    assert_eq!(r.message, "Daily limit reached for alice/api-expert; resets at 00:00");
    assert_eq!(refused.left.as_ref().unwrap().messages_today, 0);
    assert_eq!(w.collab.shares(&bob)[0].left, 0);
    // Refusals don't count, and other people have their own.
    let carol = w.asker("carol@example.com", "carol-mbp");
    assert_eq!(w.ask(&carol, "api-expert", "hi").status, TurnStatus::Working);

    w.clock.set("2026-10-16T23:59:00+02:00");
    assert_eq!(reason(&w.ask(&bob, "api-expert", "still today").status), Some(RefusalReason::Quota));
    w.clock.set("2026-10-17T00:00:01+02:00");
    let t = w.ask(&bob, "api-expert", "tomorrow");
    assert!(!matches!(t.status, TurnStatus::Refused(_)), "{:?}", t.status);
    let rows = w.collab.usage(Some("2026-10-16"));
    assert_eq!(rows.iter().find(|r| r.person == "fake:bob@example.com").unwrap().messages, 2);
}

#[test]
fn overrides_raise_or_lower_one_persons_limit() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let dave = w.asker("dave@example.com", "dave-mbp");
    let key = dave.network_identity().key();
    w.set("api-expert", |s| s.limits.per_caller.messages_per_day = 1);
    let d = w.set("api-expert", |s| {
        s.limits.overrides.insert(key.clone(), LimitOverride { messages_per_day: Some(3), blocked: false });
    });
    assert!(d.0.iter().any(|l| l.field == format!("limits.overrides.{key}") && l.widens));
    assert_eq!(w.collab.shares(&dave)[0].messages_per_day, 3);
    for n in 0..3 {
        let t = w.ask(&dave, "api-expert", &format!("q{n}"));
        assert!(!matches!(t.status, TurnStatus::Refused(_)));
        w.host.reply(&w.agent_of(&t), "a");
    }
    assert_eq!(reason(&w.ask(&dave, "api-expert", "4th").status), Some(RefusalReason::Quota));
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "1st");
    w.host.reply(&w.agent_of(&t), "a");
    assert_eq!(reason(&w.ask(&bob, "api-expert", "2nd").status), Some(RefusalReason::Quota));
}

#[test]
fn the_running_cap_queues_in_order_and_a_full_queue_is_busy() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    w.set("api-expert", |s| {
        s.limits.sessions_running = 1;
        s.limits.queue = 2;
    });
    let callers: Vec<Asker> = ["bob", "carol", "dave", "erin"].iter().map(|n| w.asker(&format!("{n}@example.com"), &format!("{n}-mbp"))).collect();
    let t0 = w.ask(&callers[0], "api-expert", "first");
    assert_eq!(t0.status, TurnStatus::Working);
    let t1 = w.ask(&callers[1], "api-expert", "second");
    assert_eq!(t1.status, TurnStatus::Queued { position: 1 });
    assert_eq!(t1.status_message.as_deref(), Some("1st in line"));
    let t2 = w.ask(&callers[2], "api-expert", "third");
    assert_eq!(t2.status, TurnStatus::Queued { position: 2 });
    assert_eq!(t2.status_message.as_deref(), Some("2nd in line"));
    let t3 = w.ask(&callers[3], "api-expert", "fourth");
    assert_eq!(reason(&t3.status), Some(RefusalReason::Busy));

    // FIFO: when bob's turn ends his idle session makes way for carol's.
    let watch = w.collab.watch(&callers[2], &t2.id).unwrap();
    w.host.reply(&w.agent_of(&t0), "done");
    assert_eq!(w.status(&callers[1], &t1), TurnStatus::Working);
    assert_eq!(w.status(&callers[2], &t2), TurnStatus::Queued { position: 1 });
    assert!(w.host.stops.lock().unwrap().contains(&w.agent_of(&t0)), "the least recently used idle session stopped");
    w.host.reply(&w.agent_of(&t1), "done");
    assert_eq!(w.status(&callers[2], &t2), TurnStatus::Working);
    let mut seen: Vec<TurnStatus> = watch.try_iter().map(|t| t.status).collect();
    seen.dedup();
    assert_eq!(seen, [TurnStatus::Queued { position: 2 }, TurnStatus::Queued { position: 1 }, TurnStatus::Working]);
}

#[test]
fn one_message_at_a_time_per_session() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t1 = w.ask(&bob, "api-expert", "first");
    let t2 = w.ask(&bob, "api-expert", "second");
    assert_eq!(t2.conversation, t1.conversation);
    assert_eq!(t2.status, TurnStatus::Queued { position: 1 });
    let agent = w.agent_of(&t1);
    assert_eq!(w.host.sent_to(&agent).len(), 1);
    w.host.reply(&agent, "one");
    assert_eq!(w.status(&bob, &t2), TurnStatus::Working);
    assert_eq!(w.host.sent_to(&agent).len(), 2);
    assert_eq!(w.host.launches_of(&agent).len(), 1, "the same session, not a new one");
}

#[test]
fn the_machine_wide_cap_counts_every_share() {
    let w = World::with(CollabSettings { max_sessions: 1, owner_name: "alice".into() });
    w.share("one", "a1", w.everyone());
    w.share("two", "a2", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let carol = w.asker("carol@example.com", "carol-mbp");
    let t1 = w.ask(&bob, "one", "hi");
    assert_eq!(t1.status, TurnStatus::Working);
    let t2 = w.ask(&carol, "two", "hi");
    assert_eq!(t2.status, TurnStatus::Queued { position: 1 });
    w.host.reply(&w.agent_of(&t1), "ok");
    assert_eq!(w.status(&carol, &t2), TurnStatus::Working);
    w.collab.set_settings(CollabSettings { max_sessions: 2, owner_name: "alice".into() });
    let dave = w.asker("dave@example.com", "dave-mbp");
    assert_eq!(w.ask(&dave, "one", "hi").status, TurnStatus::Working);
}

#[test]
fn a_turn_over_its_time_fails_with_the_partial_reply() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    w.set("api-expert", |s| s.limits.turn_timeout_min = 15);
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "a long one");
    let agent = w.agent_of(&t);
    w.host.set_partial(&agent, "So far: the tokens are");
    w.advance(14);
    assert_eq!(w.status(&bob, &t), TurnStatus::Working);
    w.advance(1);
    let after = w.collab.turn(&bob, &t.id).unwrap();
    assert_eq!(after.status, TurnStatus::Failed { reason: Failure::Timeout });
    assert_eq!(after.reply.unwrap().text, "So far: the tokens are");
    assert_eq!(*w.host.interrupts.lock().unwrap(), vec![agent.clone()]);
    // The session takes the next message.
    let t2 = w.ask(&bob, "api-expert", "next");
    assert_eq!(t2.status, TurnStatus::Working);
    let rows = w.collab.usage(None);
    assert_eq!(rows[0].agent_ms, 15 * 60_000);
}

#[test]
fn the_daily_agent_time_pauses_the_share_and_tells_the_owner() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    w.set("api-expert", |s| {
        s.limits.agent_minutes_per_day = 10;
        s.limits.turn_timeout_min = 60;
    });
    let notices = w.collab.notices();
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "work");
    w.clock.advance(Duration::from_secs(11 * 60));
    w.host.reply(&w.agent_of(&t), "done");
    let carol = w.asker("carol@example.com", "carol-mbp");
    let refused = w.ask(&carol, "api-expert", "hi");
    let TurnStatus::Refused(r) = &refused.status else { panic!("{:?}", refused.status) };
    assert_eq!(r.reason, RefusalReason::ShareTotal);
    assert_eq!(r.resets_at.as_deref(), Some("2026-10-17T00:00:00+02:00"));
    let told: Vec<Notice> = notices.try_iter().collect();
    assert_eq!(told, [Notice::ShareTotal { share: "api-expert".into() }]);
    w.ask(&bob, "api-expert", "me too");
    assert!(notices.try_iter().next().is_none(), "told once a day");
    w.clock.set("2026-10-17T08:00:00+02:00");
    assert!(!matches!(w.ask(&carol, "api-expert", "tomorrow").status, TurnStatus::Refused(_)));
}

#[test]
fn the_agents_own_vendor_limit_passes_through() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    w.set("api-expert", |s| s.limits.sessions_running = 1);
    let bob = w.asker("bob@example.com", "bob-mbp");
    let carol = w.asker("carol@example.com", "carol-mbp");
    let t1 = w.ask(&bob, "api-expert", "hi");
    let t2 = w.ask(&carol, "api-expert", "queued");
    assert_eq!(t2.status, TurnStatus::Queued { position: 1 });
    let notices = w.collab.notices();
    let agent = w.agent_of(&t1);
    w.host.emit(&agent, SessionEvent::Limited { until: Some("09:10".into()) });
    // Queued messages are told the same, with the time the agent showed.
    let TurnStatus::Refused(r) = w.status(&carol, &t2) else { panic!("refused") };
    assert_eq!(r.reason, RefusalReason::AgentLimit);
    assert_eq!(r.until.as_deref(), Some("09:10"));
    assert!(r.message.contains("unavailable until 09:10"));
    let dave = w.asker("dave@example.com", "dave-mbp");
    assert_eq!(reason(&w.ask(&dave, "api-expert", "hi").status), Some(RefusalReason::AgentLimit));
    assert_eq!(notices.try_iter().next(), Some(Notice::AgentLimit { share: "api-expert".into(), until: Some("09:10".into()) }));
    // Until the time it showed.
    w.clock.set("2026-10-16T09:11:00+02:00");
    w.collab.tick();
    w.host.reply(&agent, "back");
    assert!(!matches!(w.ask(&dave, "api-expert", "hi").status, TurnStatus::Refused(_)));
    // Or until its screen clears.
    w.host.emit(&agent, SessionEvent::Limited { until: None });
    let erin = w.asker("erin@example.com", "erin-mbp");
    assert_eq!(reason(&w.ask(&erin, "api-expert", "hi").status), Some(RefusalReason::AgentLimit));
    w.host.emit(&agent, SessionEvent::LimitCleared);
    assert!(!matches!(w.ask(&erin, "api-expert", "hi").status, TurnStatus::Refused(_)));
}

#[test]
fn pausing_refuses_and_interrupts_at_once() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    let changed = w.collab.shares_changed();
    w.collab.pause("api-expert").unwrap();
    assert!(changed.try_recv().is_ok());
    assert_eq!(w.status(&bob, &t), TurnStatus::Canceled);
    assert!(w.collab.shares(&bob).is_empty());
    assert_eq!(reason(&w.ask(&bob, "api-expert", "still there?").status), Some(RefusalReason::Paused));
    w.collab.resume("api-expert").unwrap();
    assert!(!matches!(w.ask(&bob, "api-expert", "back").status, TurnStatus::Refused(_)));
}

#[test]
fn chains_through_this_share_are_loops() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let mut bob = w.asker("bob@example.com", "bob-mbp");
    bob.chain = vec!["c_17".into(), "alice@example.com/api-expert".into()];
    assert_eq!(reason(&w.ask(&bob, "api-expert", "hi").status), Some(RefusalReason::Loop));
    // The fronts' rule: only share entries count, more than two is a loop.
    bob.chain = vec!["bob/fixer".into(), "carol/reviewer".into(), "dave/docs".into()];
    assert_eq!(reason(&w.ask(&bob, "api-expert", "hi").status), Some(RefusalReason::Loop));
    bob.chain = vec!["c_1/1".into(), "c_2/4".into(), "c_3/2".into()];
    assert!(!matches!(w.ask(&bob, "api-expert", "hi").status, TurnStatus::Refused(_)), "turn ids don't count");
    bob.chain = vec!["c_1".into(), "dave@example.com/docs".into()];
    assert!(!matches!(w.ask(&bob, "api-expert", "hi").status, TurnStatus::Refused(_)));
}

// ------------------------------------------------------------------ the owner in the loop

#[test]
fn a_blocked_session_waits_for_the_owner() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let notices = w.collab.notices();
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "should we change pricing?");
    let agent = w.agent_of(&t);
    w.host.emit(&agent, SessionEvent::Blocked { detail: Some("Allow edit?".into()) });
    let waiting = w.collab.turn(&bob, &t.id).unwrap();
    assert_eq!(waiting.status, TurnStatus::WaitingForOwner);
    assert_eq!(waiting.status_message.as_deref(), Some("waiting for alice"));
    assert_eq!(
        notices.try_iter().collect::<Vec<_>>(),
        [Notice::NeedsYou { share: "api-expert".into(), conversation: t.conversation.clone(), caller: "bob@example.com".into(), detail: Some("Allow edit?".into()) }]
    );
    assert_eq!(w.collab.sessions("api-expert")[0].state, SessionState::Blocked);
    w.host.emit(&agent, SessionEvent::Working);
    assert_eq!(w.status(&bob, &t), TurnStatus::Working);
    w.host.reply(&agent, "alice says no");
    assert_eq!(w.status(&bob, &t), TurnStatus::Done);
}

#[test]
fn nothing_captured_is_no_answer_and_flags_the_session() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let notices = w.collab.notices();
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    w.host.emit(&w.agent_of(&t), SessionEvent::TurnEnded(None));
    assert_eq!(w.status(&bob, &t), TurnStatus::Failed { reason: Failure::NoAnswer });
    assert!(w.collab.flagged(&t.conversation));
    assert!(w.collab.sessions("api-expert")[0].flagged, "the owner's sidebar flags it");
    assert_eq!(notices.try_iter().next(), Some(Notice::NoAnswer { share: "api-expert".into(), conversation: t.conversation.clone() }));
    // The owner's view of the conversation: what was typed and how it ended.
    let turns = w.collab.owner_turns("api-expert", &t.conversation).unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].text, "hi");
    assert!(turns[0].delivered.as_deref().unwrap().starts_with("From bob@example.com"));
    assert!(w.collab.owner_turns("other", &t.conversation).is_err(), "only through its own share");
    assert_eq!(w.collab.caller_of(&t.conversation).map(|(login, _)| login).as_deref(), Some("bob@example.com"));
    // The next turn with a reply clears the flag.
    let t2 = w.ask(&bob, "api-expert", "again?");
    w.host.reply(&w.agent_of(&t2), "Here.");
    assert!(!w.collab.flagged(&t.conversation));
    assert_eq!(w.collab.owner_turns("api-expert", &t.conversation).unwrap()[1].turn.reply.as_ref().unwrap().text, "Here.");
}

#[test]
fn an_agent_that_exits_fails_its_turn_and_resumes_next_time() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    let agent = w.agent_of(&t);
    w.host.emit(&agent, SessionEvent::Exited);
    assert_eq!(w.status(&bob, &t), TurnStatus::Failed { reason: Failure::Crashed });
    let t2 = w.ask(&bob, "api-expert", "again");
    assert_eq!(t2.status, TurnStatus::Working);
    assert!(w.host.launches_of(&agent)[1].resumed);
}

#[test]
fn a_session_that_cant_start_fails_its_turn() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    *w.host.fail_launch.lock().unwrap() = true;
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    let after = w.collab.turn(&bob, &t.id).unwrap();
    assert_eq!(after.status, TurnStatus::Failed { reason: Failure::Crashed });
    assert!(after.status_message.unwrap().contains("couldn't start"));
}

#[test]
fn cancel_drops_a_queued_turn_or_stops_a_running_one() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t1 = w.ask(&bob, "api-expert", "first");
    let t2 = w.ask(&bob, "api-expert", "second");
    let agent = w.agent_of(&t1);
    assert_eq!(w.collab.cancel(&bob, &t2.id).unwrap().status, TurnStatus::Canceled);
    assert!(w.host.interrupts.lock().unwrap().is_empty(), "a queued turn is just dropped");
    w.host.set_partial(&agent, "Partly");
    let canceled = w.collab.cancel(&bob, &t1.id).unwrap();
    assert_eq!(canceled.status, TurnStatus::Canceled);
    assert_eq!(canceled.reply.unwrap().text, "Partly");
    assert_eq!(*w.host.interrupts.lock().unwrap(), vec![agent.clone()]);
    // A late end of the canceled turn changes nothing; final turns stay.
    w.host.reply(&agent, "late");
    assert_eq!(w.status(&bob, &t1), TurnStatus::Canceled);
    assert_eq!(w.collab.cancel(&bob, &t1.id).unwrap().status, TurnStatus::Canceled);
    assert_eq!(w.host.sent_to(&agent).len(), 1, "the dropped one never reached the agent");
}

#[test]
fn kinds_without_resume_keep_running_and_say_when_memory_is_lost() {
    let w = World::new();
    w.share("aider-like", "a2", w.everyone());
    w.set("aider-like", |s| s.limits.sessions_running = 1);
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "aider-like", "hi");
    let agent = w.agent_of(&t);
    w.host.reply(&agent, "ok");
    w.advance(60);
    assert!(w.host.stops.lock().unwrap().is_empty(), "not idle-stopped: it couldn't resume");
    // Another caller needs the only slot: Bob's session stops, losing its memory.
    let carol = w.asker("carol@example.com", "carol-mbp");
    let tc = w.ask(&carol, "aider-like", "hi");
    assert_eq!(tc.status, TurnStatus::Working);
    w.host.reply(&w.agent_of(&tc), "ok");
    let t2 = w.ask(&bob, "aider-like", "remember me?");
    assert_eq!(t2.status_message.as_deref(), Some("new conversation (the agent can't resume)"));
    assert!(!w.host.launches_of(&agent)[1].resumed);
}

#[test]
fn settings_changes_restart_sessions_and_rebuild_copies() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    let agent = w.agent_of(&t);
    w.host.reply(&agent, "ok");
    let d = w.set("api-expert", |s| s.scope.paths.push("docs/".into()));
    assert!(d.widens());
    let t2 = w.ask(&bob, "api-expert", "again");
    assert_eq!(t2.status, TurnStatus::Working);
    let launches = w.host.launches_of(&agent);
    assert_eq!(launches.len(), 2);
    assert!(launches[1].resumed, "restarted with the kind's resume");
    assert_eq!(w.copies.made.lock().unwrap().len(), 2, "the copy was rebuilt");
    let sent = w.host.sent_to(&agent);
    assert_eq!(sent[1], "From bob@example.com (Claude Code over MCP):\nFiles updated to main\nagain");
}

#[test]
fn a_summary_start_forks_the_base_where_the_kind_can() {
    let w = World::new();
    w.share("api-expert", "a3", w.everyone());
    w.set("api-expert", |s| s.start = Start::Summary { text: "We use JWTs.".into(), approved_at: "2026-10-16T08:00:00Z".into() });
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    // The base goes first: a fresh session whose first message is the summary.
    let base = w.host.launches.lock().unwrap()[0].clone();
    assert_eq!(base.shared.caller, "briefing");
    assert_eq!(w.host.sent_to(&base.agent), vec!["We use JWTs.".to_string()]);
    assert!(matches!(w.status(&bob, &t), TurnStatus::Queued { .. }));
    w.host.reply(&base.agent, "Understood.");
    assert!(w.host.stops.lock().unwrap().contains(&base.agent), "the base frees its slot");
    // Then Bob's session forks it.
    let mine = w.host.last_launch();
    assert_eq!(mine.agent, w.agent_of(&t));
    let base_id = base.session_id.unwrap();
    assert_eq!(&mine.wrapped.argv[1..4], &["fake".to_string(), "--fork".into(), base_id.clone()]);
    assert_eq!(w.status(&bob, &t), TurnStatus::Working);
    // The next caller forks the same base; it isn't made again.
    let carol = w.asker("carol@example.com", "carol-mbp");
    w.ask(&carol, "api-expert", "hi");
    assert_eq!(w.host.last_launch().wrapped.argv[3], base_id);
    assert_eq!(w.host.launches.lock().unwrap().iter().filter(|l| l.shared.caller == "briefing").count(), 1);
    // Callers never see the base.
    assert_eq!(w.collab.sessions("api-expert").len(), 2);
}

#[test]
fn a_summary_start_without_fork_sends_the_summary_first() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    w.set("api-expert", |s| s.start = Start::Summary { text: "We use JWTs.".into(), approved_at: "2026-10-16T08:00:00Z".into() });
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    let agent = w.agent_of(&t);
    assert_eq!(w.host.sent_to(&agent), vec!["We use JWTs.".to_string()]);
    assert_eq!(w.collab.turn(&bob, &t.id).unwrap().status_message.as_deref(), Some("starting"));
    w.host.reply(&agent, "Understood.");
    assert_eq!(w.host.sent_to(&agent)[1], "From bob@example.com (Claude Code over MCP):\nhi");
    w.host.reply(&agent, "hello");
    let turns = w.collab.turns(&bob, Some(&t.conversation)).unwrap();
    assert_eq!(turns.len(), 1, "the summary turn is Pitwall's, not the caller's");
    assert_eq!(turns[0].reply.as_ref().unwrap().text, "hello");
}

// ------------------------------------------------------------------ owner views, audit, persistence

#[test]
fn the_owner_sees_sessions_usage_and_the_audit_log() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    w.clock.advance(Duration::from_secs(90));
    w.host.reply(&w.agent_of(&t), "ok");
    w.collab.ask(&bob, Ask { share: "nope".into(), message: "x".into(), ..Ask::default() }).unwrap_err();
    let sessions = w.collab.sessions("api-expert");
    assert_eq!(sessions.len(), 1);
    let s = &sessions[0];
    assert_eq!((s.caller.login.as_str(), s.caller.front, s.turns, s.usage.messages_today, s.usage.minutes_today), ("bob@example.com", Front::Mcp, 1, 1, 1));
    assert_eq!(s.state, SessionState::Ready);
    let lines = w.collab.audit_log().lines();
    let methods: Vec<(&str, &str)> = lines.iter().map(|l| (l.method.as_str(), l.outcome.as_str())).collect();
    assert!(methods.contains(&("share.create", "ok")) && methods.contains(&("share.go_live", "ok")));
    assert!(methods.contains(&("ask", "ok")) && methods.contains(&("turn", "done")));
    let ask = lines.iter().find(|l| l.method == "ask").unwrap();
    assert_eq!((ask.kind.as_str(), ask.login.as_str(), ask.peer.as_str(), ask.front.as_deref()), ("collab", "fake:bob@example.com", "fake:example", Some("mcp")));
    assert_eq!(ask.node.as_deref(), Some("bob-mbp"));
    let set = lines.iter().find(|l| l.method == "share.go_live").unwrap();
    assert_eq!(set.agent.as_deref(), Some("api-expert"));
}

#[test]
fn conversations_survive_a_restart_and_resume() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    let agent = w.agent_of(&t);
    w.host.reply(&agent, "ok");
    let t2 = w.ask(&bob, "api-expert", "in flight");
    let session = w.host.last_launch().session_id.unwrap();
    drop(w.collab.clone());
    // Pitwall restarts.
    let host = Arc::new(FakeHost::default());
    host.add_kind("a1", fake_kind(true, false));
    let collab = Collab::open(CollabDeps {
        paths: CollabPaths::new(w.dir.path().join("collab")),
        clock: w.clock.clone(),
        host: host.clone(),
        agents: host.clone(),
        sandbox: Arc::new(FakeSandbox::default()),
        snapshots: Arc::new(FakeCopies::default()),
        egress: Arc::new(FakeProxy),
        skills: None,
        teams: None,
        hooks: None,
        settings: CollabSettings::default(),
        run: RunMode::Inline,
    });
    let after = collab.turn(&bob, &t2.id).unwrap();
    assert_eq!(after.status, TurnStatus::Failed { reason: Failure::Crashed });
    assert_eq!(collab.turn(&bob, &t.id).unwrap().reply.unwrap().text, "ok");
    let t3 = collab.ask(&bob, Ask { share: "api-expert".into(), message: "after".into(), ..Ask::default() }).unwrap();
    assert_eq!(t3.conversation, t.conversation);
    assert_eq!(t3.id.number(), Some(3));
    let l = host.last_launch();
    assert!(l.resumed);
    assert_eq!(l.session_id.as_deref(), Some(session.as_str()));
}

#[test]
fn stop_sharing_ends_every_conversation() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    let agent = w.agent_of(&t);
    w.collab.stop_share("api-expert").unwrap();
    assert!(w.collab.share("api-expert").is_none());
    assert!(w.host.removed.lock().unwrap().contains(&agent));
    assert!(w.collab.turn(&bob, &t.id).unwrap_err().is(ErrorCode::NotFound));
    assert!(!w.dir.path().join("collab/shares/api-expert.json").exists());
    assert!(!w.dir.path().join("collab/shares/api-expert").exists());
}

#[test]
fn only_read_only_shares_go_live_in_c1a() {
    let w = World::new();
    let share: ShareSettings = serde_json::from_value(serde_json::json!({
        "agent": "a1", "slug": "fixer", "name": "Fixer", "description": "", "capability": "write",
        "scope": {"repo": "/code/api", "ref": "main"}, "access": {"account": "fake-share"}
    }))
    .unwrap();
    w.collab.create_share(share).unwrap();
    assert!(w.collab.go_live("fixer", false).unwrap_err().is(ErrorCode::Unsupported));
    assert!(w.collab.create_share(serde_json::from_value(serde_json::json!({
        "agent": "ghost", "slug": "ghost", "name": "Ghost", "description": "", "scope": {"repo": "/x", "ref": "main"}
    })).unwrap()).unwrap_err().is(ErrorCode::NotFound));
}

#[test]
fn watching_streams_a_turn_until_it_ends() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "api-expert", "hi");
    let rx: Receiver<Turn> = w.collab.watch(&bob, &t.id).unwrap();
    let agent = w.agent_of(&t);
    w.host.emit(&agent, SessionEvent::Blocked { detail: None });
    w.host.emit(&agent, SessionEvent::Working);
    w.host.reply(&agent, "done");
    let mut seen: Vec<TurnStatus> = rx.iter().map(|t| t.status).collect();
    seen.dedup();
    assert_eq!(seen, [TurnStatus::Working, TurnStatus::WaitingForOwner, TurnStatus::Working, TurnStatus::Done]);
    // A final turn: the current state, then the channel closes.
    let rx = w.collab.watch(&bob, &t.id).unwrap();
    assert_eq!(rx.iter().count(), 1);
}

// ------------------------------------------------------------------ the agent account (§6.3)

#[test]
fn a_live_share_never_moves_onto_the_owners_own_login() {
    let w = World::new();
    w.share("api-expert", "a1", w.everyone());
    let before = w.collab.share("api-expert").unwrap();
    let mut s = before.clone();
    s.access.account = None;
    let e = w.collab.set_share("api-expert", s).unwrap_err();
    assert!(e.is(ErrorCode::Conflict) && e.message.contains("needs a separate account"), "{}", e.message);
    assert_eq!(w.collab.share("api-expert").unwrap(), before, "nothing changed");
    let mut s = before.clone();
    s.access.account = Some("default".into());
    assert!(w.collab.set_share("api-expert", s).is_err());
    // Other changes still apply.
    w.set("api-expert", |s| s.description = "Ask about the API".into());
    // Paused, it resumes only on a separate account too.
    w.collab.pause("api-expert").unwrap();
    w.collab.resume("api-expert").unwrap();
}

/// A kind that logs in only with an API key (as Gemini CLI does).
fn key_kind() -> AgentKind {
    toml::from_str(
        r#"
id = "keyed"
name = "Keyed agent"
command = "keyed"
hooks = "none"
[share]
login_dir = "~/.keyed"
api_key_env = ["KEYED_API_KEY"]
"#,
    )
    .unwrap()
}

#[test]
fn an_api_key_kind_goes_live_only_with_its_key_passed_and_set() {
    let w = World::new();
    w.host.add_kind("k1", key_kind());
    let share: ShareSettings = serde_json::from_value(serde_json::json!({
        "agent": "k1", "slug": "docs", "name": "Docs", "description": "",
        "scope": {"repo": "/code/docs", "ref": "main"}, "audience": {"networks": [w.net.id()]}
    }))
    .unwrap();
    w.collab.create_share(share).unwrap();
    let e = w.collab.go_live("docs", false).unwrap_err();
    assert!(e.message.starts_with("Keyed agent keeps its login in ~/.keyed, which shared sessions can't use; add an API key for sharing"), "{}", e.message);
    let mut s = w.collab.share("docs").unwrap();
    s.access.env = vec!["KEYED_API_KEY".into()];
    w.collab.set_share("docs", s).unwrap();
    assert!(w.collab.go_live("docs", false).unwrap_err().message.contains("KEYED_API_KEY is passed to docs, but it isn't set"));
    w.host.env.lock().unwrap().insert("KEYED_API_KEY".into(), "not-a-real-key".into());
    w.collab.go_live("docs", false).unwrap();

    // The key reaches the session's environment, and nothing of ~/.keyed.
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "docs", "What's in the docs?");
    let policy = w.sandbox.policies.lock().unwrap().last().cloned().unwrap();
    assert_eq!(policy.env["KEYED_API_KEY"], "not-a-real-key");
    let home = crate::paths::home().join(".keyed");
    assert!(!policy.read.iter().chain(policy.write.iter()).any(|p| p.starts_with(&home)), "{policy:?}");
    w.host.reply(&w.agent_of(&t), "Here.");

    // The key gone from where Pitwall looks: a new session can't start.
    w.host.env.lock().unwrap().clear();
    let carol = w.asker("carol@example.com", "carol-mbp");
    let t = w.ask(&carol, "docs", "Hello?");
    let TurnStatus::Failed { .. } = w.status(&carol, &t) else { panic!("{:?}", w.status(&carol, &t)) };
    assert!(w.collab.turn(&carol, &t.id).unwrap().status_message.unwrap().contains("KEYED_API_KEY is passed to docs, but it isn't set"));
}

// ------------------------------------------------------------------ reply tiers (§7.2)

#[test]
fn a_non_interactive_kind_gets_sandboxed_turn_commands_and_its_own_id() {
    let w = World::new();
    let kind: AgentKind = toml::from_str(
        r#"
id = "headless"
name = "Headless agent"
command = "headless"
resume_args = ["--resume", "{session_id}"]
hooks = "none"
[share]
reply = "non-interactive"
session_args = ["--trust"]
[share.non_interactive]
first_args = ["--new", "{session_id}", "-p", "{message}"]
next_args = ["--resume", "{session_id}", "-p", "{message}"]
reply = "/response"
"#,
    )
    .unwrap();
    w.host.add_kind("h1", kind);
    let share: ShareSettings = serde_json::from_value(serde_json::json!({
        "agent": "h1", "slug": "docs", "name": "Docs", "description": "",
        "scope": {"repo": "/code/docs", "ref": "main"}, "audience": {"networks": [w.net.id()]}
    }))
    .unwrap();
    w.collab.create_share(share).unwrap();
    w.collab.go_live("docs", false).unwrap();
    let bob = w.asker("bob@example.com", "bob-mbp");
    let t = w.ask(&bob, "docs", "Where are the docs?");
    let l = w.host.last_launch();
    let turns = l.turns.clone().expect("turn commands");
    let id = l.session_id.clone().expect("Pitwall picks the conversation's id");
    assert_eq!(turns.first.argv, ["sandbox", "headless", "--new", id.as_str(), "-p", "{message}", "--trust"]);
    assert_eq!(turns.next.argv, ["sandbox", "headless", "--resume", id.as_str(), "-p", "{message}", "--trust"]);
    assert_eq!(turns.first.env, turns.next.env, "one sandbox policy for both");
    assert!(!l.resumed);
    // The message goes in as usual; the host runs it.
    assert!(w.host.sent_to(&l.agent)[0].ends_with("Where are the docs?"));
    w.host.reply(&l.agent, "In docs/.");
    assert_eq!(w.status(&bob, &t), TurnStatus::Done);
}
