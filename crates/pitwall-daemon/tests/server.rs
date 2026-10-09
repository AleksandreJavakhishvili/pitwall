//! Client ↔ server over a temp socket, with an engine on fake providers
//! (no real agents, agw or `~/.pitwall`): handshake and versions, the
//! methods the CLI uses, and the approval flow with caller identity.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pitwall_client::{Client, Error};
use pitwall_core::exec::LocalExec;
use pitwall_core::paths::Paths;
use pitwall_core::provider::ProviderCaps;
use pitwall_core::testing::{FakeProvider, ManualClock, MemStore, RecordingSink, TempDir};
use pitwall_core::{Deps, Engine, Shared};
use pitwall_daemon::{serve, Approvals, Caller, Config, Handle, Identify, ProcessIdentity};
use pitwall_proto::frame::{self, Frame};
use pitwall_proto::{code, AgentCreate, ApprovalAnswer, FormRequest, RequesterKind, Risk, Role, SessionAdd, SessionFilter};
use serde_json::{json, Value};

/// Callers in the order connections arrive (then: outside Pitwall).
#[derive(Default)]
struct Callers(Mutex<VecDeque<Caller>>);

impl Identify for Callers {
    fn identify(&self, _pid: Option<u32>) -> Caller {
        self.0.lock().unwrap().pop_front().unwrap_or_else(Caller::outside)
    }
}

fn agent_caller(id: &str, name: &str) -> Caller {
    Caller { pid: Some(4242), agent: Some((id.into(), name.into())), process: Some("pitwall-cli".into()), ui: false }
}

fn ui_caller() -> Caller {
    Caller { pid: Some(std::process::id()), agent: None, process: Some("pitwall".into()), ui: true }
}

struct World {
    engine: Shared,
    vm: Arc<FakeProvider>,
    approvals: Arc<Approvals>,
    callers: Arc<Callers>,
    server: Option<Handle>,
    socket: PathBuf,
    _dir: TempDir,
}

/// A short socket path in its own temp folder (Unix socket paths are
/// limited to ~100 bytes).
fn short_socket() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pwd-{}", &uuid_like()[..12]));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("d.sock")
}

fn uuid_like() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(std::process::id() as u64);
    format!("{:016x}", h.finish())
}

fn world(approval_timeout: Duration) -> World {
    let dir = TempDir::new("daemon");
    let local = FakeProvider::named("local", "this-mac", Arc::new(LocalExec));
    let vm = FakeProvider::named("vmhost", "box", Arc::new(LocalExec));
    vm.set_caps(ProviderCaps { start: true, attach_existing: true, survives_detach: true, ..Default::default() });
    vm.set_remote(true);
    vm.add_session("work", "claude-code", "/srv/work");
    vm.add_session("idle", "codex", "/srv/idle").exit(0);
    let engine = Engine::open(Deps {
        paths: Paths::new(dir.path().join("pitwall")),
        events: RecordingSink::new(),
        clock: ManualClock::new(1),
        store: MemStore::with(vec![]),
        providers: vec![local, vm.clone()],
    });
    let approvals = Approvals::new(approval_timeout);
    let callers = Arc::new(Callers::default());
    let socket = short_socket();
    let server = serve(engine.clone(), approvals.clone(), callers.clone(), Config { socket: socket.clone(), version: "0.1.0-test".into(), settings: None, workspace: None })
        .expect("listen on a temp socket");
    World { engine, vm, approvals, callers, server: Some(server), socket, _dir: dir }
}

impl World {
    fn client(&self) -> Client {
        Client::connect(&self.socket).expect("connect")
    }

    fn client_as(&self, caller: Caller) -> Client {
        self.callers.0.lock().unwrap().push_back(caller);
        self.client()
    }

    /// Answer the first approval that shows up, the way the app's dialog does.
    fn answer_when_asked(&self, allow: bool) -> std::thread::JoinHandle<pitwall_proto::ApprovalView> {
        let a = self.approvals.clone();
        std::thread::spawn(move || loop {
            if let Some(p) = a.pending().into_iter().next() {
                a.answer(&ApprovalAnswer { id: p.id.clone(), allow, remember: false }).unwrap();
                return p;
            }
            std::thread::sleep(Duration::from_millis(5));
        })
    }
}

impl Drop for World {
    fn drop(&mut self) {
        if let Some(s) = self.server.take() {
            s.stop();
        }
        if let Some(dir) = self.socket.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn server_error(e: Error) -> pitwall_proto::ErrorBody {
    match e {
        Error::Server(b) => b,
        other => panic!("expected a server error, got {other}"),
    }
}

#[test]
fn handshake_versions_and_unknown_methods() {
    let w = world(Duration::from_secs(5));
    let mut c = w.client();
    assert_eq!(c.welcome().protocol, pitwall_proto::PROTOCOL);
    assert!(c.has("sessions") && c.has("approvals"));
    assert_eq!(server_error(c.call("agent.prompt", json!({})).unwrap_err()).code, code::UNKNOWN_METHOD);
    assert_eq!(server_error(c.call("agent.create", json!({"kind": 3})).unwrap_err()).code, code::BAD_PARAMS);

    // A client that only speaks a future version is rejected, not served.
    let mut raw = pitwall_client::connect_raw(&w.socket).unwrap();
    frame::write_json(&mut raw, &json!({"hello": {"protocol": {"min": 2, "max": 2}, "client": "future", "role": "cli"}})).unwrap();
    let Some(Frame::Json(body)) = frame::read(&mut raw).unwrap() else { panic!("an answer") };
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["reject"]["code"], "incompatible");
    assert_eq!(v["reject"]["protocol"], json!({"min": 1, "max": 1}));
    assert_eq!(frame::read(&mut raw).unwrap(), None, "then the server hangs up");
    // Garbage instead of a hello.
    let mut raw = pitwall_client::connect_raw(&w.socket).unwrap();
    raw.write_all(&frame::json(&json!({"hi": 1}))).unwrap();
    let Some(Frame::Json(body)) = frame::read(&mut raw).unwrap() else { panic!("an answer") };
    assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["reject"]["code"], "bad_hello");
}

#[test]
fn the_socket_is_private_and_removed_on_stop() {
    let mut w = world(Duration::from_secs(5));
    // Windows: a named pipe whose security descriptor admits only this user.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&w.socket).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    // A second server on the same socket refuses to start.
    let again = serve(w.engine.clone(), w.approvals.clone(), w.callers.clone(), Config { socket: w.socket.clone(), version: "x".into(), settings: None, workspace: None });
    assert_eq!(again.err().map(|e| e.kind()), Some(std::io::ErrorKind::AddrInUse));
    w.server.take().unwrap().stop();
    assert!(!w.socket.exists());
    assert!(matches!(Client::connect(&w.socket), Err(Error::Connect { .. })));
}

#[test]
fn agents_machines_and_sessions() {
    let w = world(Duration::from_secs(5));
    let mut c = w.client();
    assert!(c.agents().unwrap().is_empty());

    let project = TempDir::new("proj");
    let made = c
        .create_agent(&AgentCreate {
            kind: "shell".into(),
            project: project.path().to_string_lossy().into(),
            name: Some("scratch".into()),
            cols: Some(100),
            rows: Some(30),
            ..Default::default()
        })
        .unwrap();
    assert_eq!((made.name.as_str(), made.kind.as_str(), made.running), ("scratch", "shell", true));
    assert_eq!(c.agents().unwrap().iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), [made.id.as_str()]);
    let e = server_error(c.create_agent(&AgentCreate { kind: "nope".into(), project: "/".into(), ..Default::default() }).unwrap_err());
    assert!(e.message.contains("unknown agent kind"), "{e}");

    let machines = c.machines().unwrap();
    let summary: Vec<_> = machines.iter().map(|p| (p.provider.as_str(), p.can_create, p.can_add_sessions, p.machines[0].id.as_str())).collect();
    assert_eq!(summary, [("local", true, false, "this-mac"), ("vmhost", false, true, "box")]);

    let places = c.sessions(&SessionFilter { provider: Some("vmhost".into()), machine: None }).unwrap();
    let sessions = &places[0].machines.as_ref().unwrap()[0].sessions;
    let rows: Vec<_> = sessions.iter().map(|s| (s.native.as_str(), s.kind.as_str(), s.status.as_str(), s.in_pitwall)).collect();
    assert_eq!(rows, [("work", "claude", "running", false), ("idle", "codex", "stopped", false)]);
    assert!(c.sessions(&SessionFilter { provider: Some("other".into()), machine: None }).unwrap().is_empty());
}

#[test]
fn adding_a_session_needs_no_approval() {
    let w = world(Duration::from_millis(200));
    let mut c = w.client_as(agent_caller("a1", "api"));
    let add = SessionAdd { provider: "vmhost".into(), machine: "box".into(), native: "work".into(), ..Default::default() };
    let first = c.add_session(&add).unwrap();
    assert!(first.agent.running && !first.already && !first.started);
    assert!(first.agent.caps.remove_keeps_session);
    let again = c.add_session(&add).unwrap();
    assert_eq!((again.agent.id, again.already), (first.agent.id, true), "adding twice is one agent");
    // A running session with --start: nothing to start, nothing to ask.
    assert!(!c.add_session(&SessionAdd { start: true, ..add }).unwrap().started);
    // A stopped one without --start: tracked, still stopped.
    let idle = c.add_session(&SessionAdd { provider: "vmhost".into(), machine: "box".into(), native: "idle".into(), ..Default::default() }).unwrap();
    assert!(!idle.agent.running && !idle.started);
    assert!(w.vm.launched().is_empty(), "nothing was started");
    let e = server_error(c.add_session(&SessionAdd { provider: "vmhost".into(), machine: "box".into(), native: "ghost".into(), ..Default::default() }).unwrap_err());
    assert!(e.message.contains("no session"), "{e}");
    let places = c.sessions(&SessionFilter::default()).unwrap();
    assert!(places[0].machines.as_ref().unwrap()[0].sessions.iter().all(|s| s.in_pitwall));
}

fn start_idle() -> SessionAdd {
    SessionAdd { provider: "vmhost".into(), machine: "box".into(), native: "idle".into(), start: true, cols: Some(90), rows: Some(30) }
}

#[test]
fn starting_a_stopped_session_waits_for_the_users_approval() {
    let w = world(Duration::from_secs(10));
    let mut c = w.client_as(agent_caller("a1", "api"));
    let asked = w.answer_when_asked(true);
    let added = c.add_session(&start_idle()).unwrap();
    let view = asked.join().unwrap();
    assert!(added.started && added.agent.running);
    assert_eq!(w.vm.launched().len(), 1, "started on its machine once");
    // The dialog names who asked as the server established it.
    assert_eq!(view.action, "session.add");
    assert_eq!(view.summary, "start the session \"idle\" on Fake box (vmhost)");
    assert_eq!((view.requester.kind, view.requester.name.as_str(), view.requester.agent_id.as_deref()), (RequesterKind::Agent, "api", Some("a1")));
    assert!(view.rememberable && view.expires_at > view.created_at);
    assert!(w.approvals.pending().is_empty());
}

#[test]
fn a_denied_start_leaves_the_session_stopped() {
    let w = world(Duration::from_secs(10));
    let mut c = w.client();
    let asked = w.answer_when_asked(false);
    let e = server_error(c.add_session(&start_idle()).unwrap_err());
    assert_eq!(asked.join().unwrap().requester.kind, RequesterKind::Outside);
    assert_eq!(e.code, code::DENIED);
    assert!(e.message.contains("still stopped"), "{e}");
    assert!(w.vm.launched().is_empty());
    let agents = c.agents().unwrap();
    assert_eq!(agents.len(), 1, "it was added");
    assert!(!agents[0].running);
}

#[test]
fn an_unanswered_start_times_out_as_a_denial() {
    let w = world(Duration::from_millis(150));
    let mut c = w.client();
    let e = server_error(c.add_session(&start_idle()).unwrap_err());
    assert_eq!(e.code, code::APPROVAL_TIMEOUT);
    assert!(w.vm.launched().is_empty());
    assert!(w.approvals.pending().is_empty());
}

#[test]
fn only_the_ui_answers_approvals() {
    let w = world(Duration::from_secs(10));
    // An agent asks to start a session…
    let socket = w.socket.clone();
    w.callers.0.lock().unwrap().push_back(agent_caller("a1", "api"));
    let asking = std::thread::spawn(move || Client::connect(&socket).unwrap().add_session(&start_idle()));
    while w.approvals.pending().is_empty() {
        std::thread::sleep(Duration::from_millis(5));
    }
    let id = w.approvals.pending()[0].id.clone();
    // …then tries to approve it itself (or list approvals): refused.
    let mut sneaky = w.client_as(agent_caller("a1", "api"));
    assert_eq!(server_error(sneaky.approvals().unwrap_err()).code, code::DENIED);
    let yes = ApprovalAnswer { id: id.clone(), allow: true, remember: true };
    assert_eq!(server_error(sneaky.answer_approval(&yes).unwrap_err()).code, code::DENIED);
    // A process outside Pitwall can't either, whatever role it claims.
    let mut claims_ui = {
        w.callers.0.lock().unwrap().push_back(Caller::outside());
        Client::connect_as(&w.socket, "pitwall-app/9", Role::Ui).unwrap()
    };
    assert_eq!(server_error(claims_ui.answer_approval(&yes).unwrap_err()).code, code::DENIED);
    assert_eq!(w.approvals.pending().len(), 1, "still waiting");
    // Pitwall's own window can.
    let mut ui = w.client_as(ui_caller());
    assert_eq!(ui.approvals().unwrap()[0].id, id);
    ui.answer_approval(&yes).unwrap();
    assert!(asking.join().unwrap().unwrap().started);
    assert_eq!(server_error(ui.answer_approval(&yes).unwrap_err()).code, code::NOT_FOUND, "answered once");
    // Remembered for that caller: the next start isn't asked.
    w.vm.ctl("idle").unwrap().exit(0);
    std::thread::sleep(Duration::from_millis(50));
    let again = w.client_as(agent_caller("a1", "api")).add_session(&start_idle()).unwrap();
    assert!(again.already && again.started);
}

fn vm_form() -> pitwall_proto::CreateForm {
    use pitwall_proto::{CreateChoice, CreateField, CreateForm, CREATE_NEW};
    let pick = |v: &str, phrase: &str, creates: Option<&str>| CreateChoice {
        value: v.into(),
        label: v.into(),
        detail: None,
        phrase: Some(phrase.into()),
        creates: creates.map(Into::into),
    };
    CreateForm {
        folder: false,
        fields: vec![CreateField::select(
            "workspace",
            "Workspace",
            vec![pick("work", "in workspace work", None), pick(CREATE_NEW, "in a new workspace {name}", Some("workspace {name}"))],
            Some("work".into()),
        )],
        summary: Some("Creates session {name} on box {workspace}.".into()),
        submit: "Create".into(),
        ..CreateForm::folder("", "", "")
    }
}

fn create_on_vm(workspace: &str) -> AgentCreate {
    AgentCreate {
        machine: Some("box".into()),
        name: Some("api-fix".into()),
        options: [("workspace".to_string(), workspace.to_string())].into(),
        cols: Some(90),
        rows: Some(30),
        ..Default::default()
    }
}

#[test]
fn creating_on_a_platform_machine_waits_for_the_users_approval() {
    let w = world(Duration::from_secs(10));
    w.vm.set_caps(ProviderCaps { create: true, start: true, attach_existing: true, survives_detach: true, ..Default::default() });
    w.vm.set_form(vm_form());
    let mut c = w.client_as(agent_caller("a1", "api"));
    // The form is read-only: no approval.
    let form = c.create_form(&FormRequest { provider: "vmhost".into(), machine: "box".into() }).unwrap();
    assert_eq!((form.folder, form.machine_label.as_str(), form.submit.as_str()), (false, "Fake box", "Create"));
    assert!(w.approvals.pending().is_empty());
    // Bad input is refused before anyone is asked.
    let e = server_error(c.create_agent(&AgentCreate { name: None, ..create_on_vm("work") }).unwrap_err());
    assert_eq!(e.code, code::BAD_PARAMS);
    let e = server_error(c.create_agent(&create_on_vm("nope")).unwrap_err());
    assert!(e.code == code::BAD_PARAMS && e.message.contains("isn't a choice"), "{e}");
    assert!(w.approvals.pending().is_empty() && w.vm.created().is_empty());

    // Denied: nothing is created.
    let asked = w.answer_when_asked(false);
    let e = server_error(c.create_agent(&create_on_vm("work")).unwrap_err());
    let view = asked.join().unwrap();
    assert_eq!(e.code, code::DENIED);
    assert!(e.message.contains("Nothing was created"), "{e}");
    assert!(w.vm.created().is_empty() && c.agents().unwrap().is_empty());
    assert_eq!(view.action, "agent.create");
    assert_eq!(view.summary, "create the session \"api-fix\" on Fake box (vmhost)");
    assert_eq!(view.details[0], "Creates session api-fix on box in workspace work.");
    assert_eq!((view.risk, view.requester.name.as_str()), (Risk::Low, "api"));

    // Allowed: made there, then attached and in Pitwall.
    let asked = w.answer_when_asked(true);
    let made = c.create_agent(&create_on_vm("work")).unwrap();
    asked.join().unwrap();
    assert_eq!((made.name.as_str(), made.cwd.as_str(), made.running), ("api-fix", "/srv/work", true));
    assert!(made.caps.remove_keeps_session);
    assert_eq!(w.vm.created().len(), 1);

    // Also making a workspace there: asked every time (high risk).
    let asked = w.answer_when_asked(false);
    let _ = c.create_agent(&AgentCreate { name: Some("scratch".into()), ..create_on_vm(pitwall_proto::CREATE_NEW) });
    let view = asked.join().unwrap();
    assert_eq!((view.risk, view.rememberable), (Risk::High, false));
    assert!(view.details.iter().any(|d| d == "Also creates workspace scratch on Fake box."), "{:?}", view.details);
    // On this Mac nothing is asked.
    let project = TempDir::new("proj");
    let local = AgentCreate { kind: "shell".into(), project: project.path().to_string_lossy().into(), ..Default::default() };
    assert!(c.create_agent(&local).is_ok());
    assert!(w.approvals.pending().is_empty());
}

/// The real identity: a client in this very process is Pitwall itself (the
/// in-process server's UI); no agent's process is its ancestor.
#[test]
fn process_identity_of_a_local_peer() {
    let w = world(Duration::from_secs(5));
    let caller = ProcessIdentity::new(w.engine.clone()).identify(Some(std::process::id()));
    assert!(caller.ui && caller.agent.is_none());
    assert!(caller.process.is_some());
    #[cfg(unix)]
    {
        let parent = ProcessIdentity::new(w.engine.clone()).identify(Some(std::os::unix::process::parent_id()));
        assert!(!parent.ui, "another process is not the UI");
    }
    assert_eq!(ProcessIdentity::new(w.engine.clone()).identify(None), Caller::outside());
}

// ------------------------------------------------------------ managing agents

/// A shell agent in a temp folder (this Mac), started over the socket.
fn local_agent(c: &mut Client, name: &str, dir: &TempDir) -> pitwall_proto::AgentView {
    c.create_agent(&AgentCreate { kind: "shell".into(), project: dir.path().to_string_lossy().into(), name: Some(name.into()), ..Default::default() }).unwrap()
}

fn agent_ref(id: &str) -> Value {
    json!({ "agentId": id })
}

#[test]
fn the_server_offers_managing_but_not_spaces_without_windows() {
    let w = world(Duration::from_secs(5));
    let mut c = w.client();
    assert!(c.has("manage") && !c.has("spaces"));
    assert_eq!(server_error(c.call("space.list", Value::Null).unwrap_err()).code, code::UNSUPPORTED);
    let e = server_error(c.call_cap::<Value>("spaces", "space.list", Value::Null).unwrap_err());
    assert!(e.code == code::UNSUPPORTED && e.message.contains("update Pitwall"), "{e}");
}

#[test]
fn stopping_restarting_and_removing_ask_the_user() {
    let w = world(Duration::from_secs(10));
    let dir = TempDir::new("proj");
    let mut c = w.client_as(agent_caller("eng", "Race Engineer"));
    let a = local_agent(&mut c, "worker", &dir);

    // Denied: still running, the dialog named who asked.
    let asked = w.answer_when_asked(false);
    let e = server_error(c.call("agent.stop", agent_ref(&a.id)).unwrap_err());
    let view = asked.join().unwrap();
    assert_eq!((e.code.as_str(), view.action.as_str(), view.summary.as_str()), (code::DENIED, "agent.stop", "stop \"worker\""));
    assert_eq!((view.requester.kind, view.requester.name.as_str(), view.risk), (RequesterKind::Agent, "Race Engineer", Risk::Low));
    assert!(e.message.contains("still running"), "{e}");
    assert!(c.agents().unwrap()[0].running);

    // Allowed: stopped.
    let asked = w.answer_when_asked(true);
    let v: pitwall_proto::AgentView = serde_json::from_value(c.call("agent.stop", agent_ref(&a.id)).unwrap()).unwrap();
    asked.join().unwrap();
    assert!(!v.running);
    // Stopping what isn't running asks nothing.
    assert_eq!(server_error(c.call("agent.stop", agent_ref(&a.id)).unwrap_err()).code, code::NOT_RUNNING);
    assert!(w.approvals.pending().is_empty());

    let asked = w.answer_when_asked(true);
    let v: pitwall_proto::AgentView = serde_json::from_value(c.call("agent.restart", agent_ref(&a.id)).unwrap()).unwrap();
    assert_eq!(asked.join().unwrap().summary, "restart \"worker\"");
    assert!(v.running);

    // Remove: asked every time (high risk); a worktree it doesn't have is refused first.
    let e = server_error(c.call("agent.remove", json!({"agentId": a.id, "deleteWorktree": true})).unwrap_err());
    assert_eq!(e.code, code::BAD_PARAMS);
    assert!(w.approvals.pending().is_empty());
    let asked = w.answer_when_asked(false);
    assert_eq!(server_error(c.call("agent.remove", agent_ref(&a.id)).unwrap_err()).code, code::DENIED);
    let view = asked.join().unwrap();
    assert_eq!((view.risk, view.rememberable, view.summary.as_str()), (Risk::High, false, "remove \"worker\" from Pitwall"));
    assert_eq!(c.agents().unwrap().len(), 1);
    let asked = w.answer_when_asked(true);
    assert_eq!(c.call("agent.remove", agent_ref(&a.id)).unwrap(), Value::Null);
    asked.join().unwrap();
    assert!(c.agents().unwrap().is_empty());
    assert_eq!(server_error(c.call("agent.remove", agent_ref(&a.id)).unwrap_err()).code, code::NOT_FOUND);
}

#[test]
fn an_unanswered_remove_times_out_and_the_ui_is_never_asked() {
    let w = world(Duration::from_millis(150));
    let dir = TempDir::new("proj");
    let mut c = w.client();
    let a = local_agent(&mut c, "worker", &dir);
    let e = server_error(c.call("agent.remove", agent_ref(&a.id)).unwrap_err());
    assert_eq!(e.code, code::APPROVAL_TIMEOUT);
    assert!(w.approvals.pending().is_empty() && c.agents().unwrap().len() == 1);
    // Pitwall's own window acts directly (its button is the approval).
    let mut ui = w.client_as(ui_caller());
    ui.call("agent.stop", agent_ref(&a.id)).unwrap();
    ui.call("agent.remove", agent_ref(&a.id)).unwrap();
    assert!(ui.agents().unwrap().is_empty());
}

#[test]
fn renaming_queueing_and_reading_never_ask() {
    let w = world(Duration::from_millis(100));
    let dir = TempDir::new("proj");
    let mut c = w.client_as(agent_caller("eng", "Race Engineer"));
    let a = local_agent(&mut c, "worker", &dir);

    let v = c.call("agent.rename", json!({"agentId": a.id, "name": "api"})).unwrap();
    assert_eq!(v["name"], "api");
    assert_eq!(server_error(c.call("agent.rename", json!({"agentId": a.id, "name": " "})).unwrap_err()).code, code::BAD_PARAMS);

    c.call("queue.add", json!({"agentId": a.id, "text": "run the tests"})).unwrap();
    let v = c.call("queue.add", json!({"agentId": a.id, "text": "then lint"})).unwrap();
    let items = v["queue"].as_array().unwrap().clone();
    assert_eq!(items.len(), 2);
    assert_eq!(server_error(c.call("queue.add", json!({"agentId": a.id, "text": "  "})).unwrap_err()).code, code::BAD_PARAMS);
    let all = c.call("queue.list", json!({})).unwrap();
    assert_eq!((all[0]["name"].as_str(), all[0]["items"][1]["text"].as_str()), (Some("api"), Some("then lint")));
    let v = c.call("queue.remove", json!({"agentId": a.id, "itemId": items[1]["id"]})).unwrap();
    assert_eq!(v["queue"].as_array().unwrap().len(), 1);
    assert_eq!(server_error(c.call("queue.remove", json!({"agentId": a.id, "itemId": "nope"})).unwrap_err()).code, code::NOT_FOUND);
    // Send now: the first item goes to the agent's terminal.
    let v = c.call("queue.send", json!({"agentId": a.id})).unwrap();
    assert!(v["queue"].as_array().unwrap().is_empty());
    assert_eq!(v["lastSent"], "run the tests");
    assert_eq!(server_error(c.call("queue.send", json!({"agentId": a.id})).unwrap_err()).code, code::NOT_FOUND, "nothing queued");

    let d = c.call("agent.status", agent_ref(&a.id)).unwrap();
    assert_eq!((d["status"].as_str(), d["source"].as_str(), d["running"].as_bool()), (Some("unknown"), Some("activity"), Some(true)));
    assert!(!d["explanation"].as_array().unwrap().is_empty());

    // Projects: the list only.
    let p = dir.path().to_string_lossy().into_owned();
    let list = c.call("project.add", json!({"path": p})).unwrap();
    assert!(list.as_array().unwrap().iter().any(|x| x["path"].as_str() == Some(std::fs::canonicalize(&p).unwrap().to_str().unwrap()) || x["path"].as_str() == Some(p.as_str())));
    assert_eq!(server_error(c.call("project.add", json!({"path": "/no/such/folder"})).unwrap_err()).code, code::NOT_FOUND);
    c.call("project.remove", json!({"path": p})).unwrap();
    assert!(dir.path().is_dir(), "never deletes the folder");
    assert_eq!(server_error(c.call("project.remove", json!({"path": p})).unwrap_err()).code, code::NOT_FOUND);

    // Rules: no sets yet; a set that doesn't exist is refused.
    let sets = c.call("rules.sets", Value::Null).unwrap();
    assert_eq!(sets["sets"], json!([]));
    assert_eq!(server_error(c.call("rules.default", json!({"project": p, "set": "strict"})).unwrap_err()).code, code::NOT_FOUND);
    c.call("rules.default", json!({"project": p, "set": null})).unwrap();
    let r = c.call("rules.list", Value::Null).unwrap();
    assert_eq!(r["agents"][0]["agentId"], a.id.as_str());
    // A terminal has no rules.
    assert_eq!(server_error(c.call("rules.apply", agent_ref(&a.id)).unwrap_err()).code, code::UNSUPPORTED);
    // Writing into the main checkout needs --main-checkout, and then the user.
    let claude = c
        .create_agent(&AgentCreate { kind: "claude".into(), project: dir.path().to_string_lossy().into(), name: Some("writer".into()), ..Default::default() })
        .unwrap();
    assert!(claude.caps.rules, "{:?}", claude.caps);
    let a = claude;
    let e = server_error(c.call("rules.apply", agent_ref(&a.id)).unwrap_err());
    assert!(e.code == code::CONFLICT && e.message.contains("--main-checkout"), "{e}");
    let asked = w.answer_when_asked(false);
    let e = server_error(c.call("rules.apply", json!({"agentId": a.id, "mainCheckout": true})).unwrap_err());
    let view = asked.join().unwrap();
    assert_eq!((e.code.as_str(), view.action.as_str(), view.risk), (code::DENIED, "rules.apply", Risk::High));
    assert!(e.message.contains("Nothing was written"), "{e}");

    // Review: read-only, outside a repository it says so.
    let e = server_error(c.call("review.changes", agent_ref(&a.id)).unwrap_err());
    assert!(e.code == code::OTHER || e.code == code::UNSUPPORTED, "{e}");
    assert!(w.approvals.pending().is_empty());
}

#[test]
fn waiting_for_a_status() {
    let w = world(Duration::from_secs(5));
    let dir = TempDir::new("proj");
    let mut c = w.client();
    let a = local_agent(&mut c, "worker", &dir);
    // Already there: at once.
    let r = c.call("agent.wait", json!({"agentId": a.id, "until": ["unknown"]})).unwrap();
    assert_eq!(r["agent"]["status"], "unknown");
    // Never there: times out.
    let started = std::time::Instant::now();
    let e = server_error(c.call("agent.wait", json!({"agentId": a.id, "until": ["idle"], "timeoutMs": 300})).unwrap_err());
    assert!(e.code == code::TIMEOUT && e.message.contains("still unknown"), "{e}");
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert_eq!(server_error(c.call("agent.wait", json!({"agentId": a.id, "until": []})).unwrap_err()).code, code::BAD_PARAMS);
    // --fresh: the current status doesn't count; it ends when it changes to one wanted.
    let socket = w.socket.clone();
    let id = a.id.clone();
    let waiting = std::thread::spawn(move || Client::connect(&socket).unwrap().call("agent.wait", json!({"agentId": id, "until": ["unknown", "stopped"], "fresh": true})));
    std::thread::sleep(Duration::from_millis(250));
    assert!(!waiting.is_finished(), "unknown now doesn't count");
    w.client_as(ui_caller()).call("agent.stop", agent_ref(&a.id)).unwrap();
    let r = waiting.join().unwrap().unwrap();
    assert_eq!(r["agent"]["running"], false, "not running counts as stopped");
    // A stopped agent won't become idle.
    let e = server_error(c.call("agent.wait", json!({"agentId": a.id, "until": ["idle"]})).unwrap_err());
    assert_eq!(e.code, code::NOT_RUNNING);
}
