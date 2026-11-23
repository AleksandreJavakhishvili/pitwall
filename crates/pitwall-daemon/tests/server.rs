//! Client ↔ server over a temp socket, with an engine on fake providers
//! (no real agents, agw or `~/.pitwall`): handshake and versions, the
//! methods the CLI uses, the approval flow with caller identity, and events.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pitwall_client::{Client, Error};
use pitwall_core::events::{Event, EventSink};
use pitwall_core::exec::LocalExec;
use pitwall_core::paths::Paths;
use pitwall_core::provider::ProviderCaps;
use pitwall_core::testing::{FakeProvider, ManualClock, MemStore, RecordingSink, TempDir};
use pitwall_core::{Deps, Engine, Shared};
use pitwall_daemon::{serve, Approvals, Caller, Config, Handle, Hub, Identify, ProcessIdentity, Registry};
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
    Caller { pid: Some(4242), agent: Some((id.into(), name.into())), process: Some("pitwall-cli".into()), ui: false, extension: None }
}

fn ui_caller() -> Caller {
    Caller { pid: Some(std::process::id()), agent: None, process: Some("pitwall".into()), ui: true, extension: None }
}

struct World {
    engine: Shared,
    vm: Arc<FakeProvider>,
    approvals: Arc<Approvals>,
    callers: Arc<Callers>,
    /// The engine's event sink (tests emit through it as the engine would).
    hub: Arc<Hub>,
    /// The extensions that are on, as callers.
    extensions: Arc<Registry>,
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
    let hub = Hub::new(RecordingSink::new());
    let engine = Engine::open(Deps {
        paths: Paths::new(dir.path().join("pitwall")),
        events: hub.clone(),
        clock: ManualClock::new(1),
        store: MemStore::with(vec![]),
        providers: vec![local, vm.clone()],
    });
    let approvals = Approvals::new(approval_timeout);
    let callers = Arc::new(Callers::default());
    let extensions = Registry::new();
    let socket = short_socket();
    let cfg = Config { socket: socket.clone(), version: "0.1.0-test".into(), settings: None, workspace: None, viewer: None, events: Some(hub.clone()), extensions: Some(extensions.clone()), ext_backend: None };
    let server = serve(engine.clone(), approvals.clone(), callers.clone(), cfg).expect("listen on a temp socket");
    World { engine, vm, approvals, callers, hub, extensions, server: Some(server), socket, _dir: dir }
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
    let again = serve(w.engine.clone(), w.approvals.clone(), w.callers.clone(), Config { socket: w.socket.clone(), version: "x".into(), settings: None, workspace: None, viewer: None, events: None, extensions: None, ext_backend: None });
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

    assert!(!a.name_auto, "a name typed in New agent is kept");
    let v = c.call("agent.rename", json!({"agentId": a.id, "name": "api"})).unwrap();
    assert_eq!((&v["name"], &v["nameAuto"]), (&json!("api"), &json!(false)));
    let v = c.call("agent.rename", json!({"agentId": a.id, "auto": true})).unwrap();
    assert_eq!((&v["name"], &v["nameAuto"]), (&json!("api"), &json!(true)), "no title yet: the name stays");
    let v = c.call("agent.rename", json!({"agentId": a.id, "name": "api"})).unwrap();
    assert_eq!(v["nameAuto"], false, "renaming pins it again");
    assert_eq!(server_error(c.call("agent.rename", json!({"agentId": a.id, "name": " "})).unwrap_err()).code, code::BAD_PARAMS);

    let v = c.call("agent.track", json!({"agentId": a.id, "path": "/tmp/wt-a"})).unwrap();
    assert_eq!(v["tracked"], json!(["/tmp/wt-a"]));
    assert_eq!(c.call("agent.track", json!({"agentId": a.id})).unwrap()["tracked"], json!(["/tmp/wt-a"]));
    let v = c.call("agent.track", json!({"agentId": a.id, "path": "/tmp/wt-a", "untrack": true})).unwrap();
    assert_eq!(v["tracked"], json!([]));
    assert_eq!(server_error(c.call("agent.track", json!({"agentId": a.id, "path": "wt"})).unwrap_err()).code, code::BAD_PARAMS);

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

// ------------------------------------------------------------ events

fn next(c: &mut Client) -> pitwall_proto::Event {
    c.next_event().unwrap().expect("an event")
}

/// Wait (up to 5 s) for `f`.
fn eventually(what: &str, f: impl Fn() -> bool) {
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(std::time::Instant::now() < until, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_subscriber_gets_the_state_then_every_change() {
    let w = world(Duration::from_secs(5));
    let mut c = w.client();
    assert!(c.has("events"));
    let got = c.subscribe(&[]).unwrap();
    assert_eq!(got.topics, ["agents", "attention", "blocked"]);
    // The state now comes first, so a viewer needs no agent.list.
    let first = next(&mut c);
    assert_eq!((first.event.as_str(), first.data.clone()), ("agents.changed", json!([])));
    let count = next(&mut c);
    assert_eq!((count.event.as_str(), count.data.clone()), ("blocked.changed", json!({"count": 0})));

    // Another client adds an agent; the engine reports it.
    let mut other = w.client();
    let add = SessionAdd { provider: "vmhost".into(), machine: "box".into(), native: "work".into(), ..Default::default() };
    let added = other.add_session(&add).unwrap().agent;
    w.hub.emit(Event::AgentsChanged(w.engine.views()));
    let changed = next(&mut c);
    assert_eq!(changed.event, "agents.changed");
    let views: Vec<pitwall_proto::AgentView> = serde_json::from_value(changed.data).unwrap();
    assert_eq!(views.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), [added.id.as_str()], "as agent.list shows them");

    w.hub.emit(Event::Attention(pitwall_core::model::Attention { agent_id: added.id.clone(), name: added.name.clone(), reason: "blocked", detail: Some("Allow Bash?".into()) }));
    w.hub.emit(Event::BlockedCount(1));
    w.hub.emit(Event::ProjectsChanged(vec![]));
    let att = next(&mut c);
    assert_eq!((att.event.as_str(), att.data.clone()), ("attention", json!({"agentId": added.id, "name": added.name, "reason": "blocked", "detail": "Allow Bash?"})));
    assert_eq!(next(&mut c).data, json!({"count": 1}));

    // Calls still work on a subscribed connection; events meanwhile are kept.
    w.hub.emit(Event::BlockedCount(0));
    assert_eq!(c.agents().unwrap().len(), 1);
    let kept = next(&mut c);
    assert_eq!((kept.event.as_str(), kept.data.clone()), ("blocked.changed", json!({"count": 0})));

    // Subscribing again changes the topics.
    assert_eq!(c.subscribe(&["attention".into()]).unwrap().topics, ["attention"]);
    w.hub.emit(Event::BlockedCount(5));
    w.hub.emit(Event::AgentsChanged(vec![]));
    w.hub.emit(Event::Attention(pitwall_core::model::Attention { agent_id: added.id.clone(), name: added.name, reason: "done", detail: None }));
    let only = next(&mut c);
    assert_eq!((only.event.as_str(), only.data["reason"].clone()), ("attention", json!("done")));
}

#[test]
fn agents_may_follow_agents_but_not_approvals() {
    let w = world(Duration::from_secs(5));
    let mut agent = w.client_as(agent_caller("a1", "api"));
    let e = server_error(agent.subscribe(&["agents".into(), "approvals".into()]).unwrap_err());
    assert_eq!(e.code, code::DENIED);
    assert_eq!(w.hub.subscribers(), 0, "a denied subscribe subscribes nothing");
    assert_eq!(agent.subscribe(&["agents".into(), "later".into()]).unwrap().topics, ["agents"], "unknown topics are left out");
    assert_eq!(next(&mut agent).event, "agents.changed");
    // The UI may ask for approvals; they aren't sent over the socket yet.
    let mut ui = w.client_as(ui_caller());
    assert_eq!(ui.subscribe(&["approvals".into(), "blocked".into()]).unwrap().topics, ["blocked"]);
}

#[test]
fn a_closed_connection_unsubscribes() {
    let w = world(Duration::from_secs(5));
    let mut c = w.client();
    c.subscribe(&[]).unwrap();
    let mut c2 = w.client();
    c2.subscribe(&["attention".into()]).unwrap();
    assert_eq!(w.hub.subscribers(), 2);
    drop(c);
    eventually("the closed connection's queue is gone", || w.hub.subscribers() == 1);
    // The other one still gets events.
    w.hub.emit(Event::BlockedCount(1));
    w.hub.emit(Event::Attention(pitwall_core::model::Attention { agent_id: "x".into(), name: "x".into(), reason: "done", detail: None }));
    assert_eq!(next(&mut c2).event, "attention");
    drop(c2);
    eventually("no subscribers", || w.hub.subscribers() == 0);
}

#[test]
fn a_client_that_stops_reading_never_holds_up_the_engine() {
    let w = world(Duration::from_secs(5));
    let mut raw = pitwall_client::connect_raw(&w.socket).unwrap();
    frame::write_json(&mut raw, &json!({"hello": {"protocol": {"min": 1, "max": 1}, "client": "slow", "role": "cli"}})).unwrap();
    frame::read(&mut raw).unwrap();
    frame::write_json(&mut raw, &json!({"id": 1, "method": "events.subscribe"})).unwrap();
    // Agent lists adding up to far more than the socket or pipe buffers
    // (~60 MB), while nobody reads. Emitting has to finish: if it waited on
    // the client it would never return. No time bound: serializing this
    // much in a debug build takes seconds, more on slow runners.
    let add = SessionAdd { provider: "vmhost".into(), machine: "box".into(), native: "work".into(), ..Default::default() };
    let one = w.client().add_session(&add).unwrap().agent;
    let big: Vec<_> = (0..20).map(|i| pitwall_proto::AgentView { id: format!("a{i}"), ..one.clone() }).collect();
    const N: usize = 2000;
    let (done, emitted) = std::sync::mpsc::channel();
    let hub = w.hub.clone();
    std::thread::spawn(move || {
        for i in 1..=N {
            hub.emit(Event::AgentsChanged(big.clone()));
            hub.emit(Event::Attention(pitwall_core::model::Attention { agent_id: format!("a{i}"), name: "n".into(), reason: "done", detail: None }));
            hub.emit(Event::BlockedCount(i));
        }
        let _ = done.send(());
    });
    emitted.recv_timeout(Duration::from_secs(300)).expect("emitting never waits on the client");
    // Reading again: the latest state arrives; what piled up was trimmed.
    let mut seen = 0;
    let mut last_alert = 0;
    loop {
        let Some(Frame::Json(body)) = frame::read(&mut raw).unwrap() else { panic!("the connection stays open") };
        let v: Value = serde_json::from_slice(&body).unwrap();
        if v.get("id").is_some() {
            continue;
        }
        seen += 1;
        if v["event"] == "attention" {
            last_alert = v["data"]["agentId"].as_str().unwrap()[1..].parse().unwrap();
        }
        if v["event"] == "blocked.changed" && v["data"]["count"] == N {
            break;
        }
    }
    assert_eq!(last_alert, N, "the newest alerts are kept");
    assert!(seen < N, "most of the {} events were coalesced or dropped ({seen} arrived)", 3 * N);
}

/// Profiles over the socket: reading never asks; making one doesn't;
/// what decides an agent's launch asks (denied: nothing written); a
/// project's answers say where they came from; deleting a profile keeps
/// what its projects run under.
#[test]
fn profiles_and_picks_ask_before_deciding_what_agents_launch_with() {
    let w = world(Duration::from_secs(10));
    let paths = w.engine.paths().clone();
    let kind = w.engine.kinds().find("claude").expect("the claude kind");
    let mut a = pitwall_core::accounts::Accounts::load(&paths);
    let work = a.add(&paths, &kind, "Work", None).unwrap();
    a.add_connection("inbox", "Home").unwrap();
    a.save(&paths).unwrap();
    let mut c = w.client_as(agent_caller("eng", "Race Engineer"));

    let p: pitwall_proto::ProfileView = serde_json::from_value(c.call("profile.save", json!({"name": "Work"})).unwrap()).unwrap();
    assert_eq!(p.id, "profile-work");
    // No project uses it yet: setting its picks asks nothing.
    let p: pitwall_proto::ProfileView = serde_json::from_value(c.call("profile.set", json!({"profile": "work", "slot": "claude", "value": "Work"})).unwrap()).unwrap();
    assert_eq!((p.picks[0].value.as_str(), p.picks[0].name.as_str()), (work.id.as_str(), "Work"));
    assert!(w.approvals.pending().is_empty());
    let e = server_error(c.call("profile.set", json!({"profile": "Work", "slot": "claude", "value": "none"})).unwrap_err());
    assert_eq!(e.code, code::BAD_PARAMS, "none is for extensions only");

    // Assigning asks; denied leaves it as it was.
    let asked = w.answer_when_asked(false);
    let e = server_error(c.call("project.profile", json!({"path": "/code/api", "profile": "Work"})).unwrap_err());
    let view = asked.join().unwrap();
    assert_eq!((e.code.as_str(), view.summary.as_str()), (code::DENIED, "/code/api uses the profile \"Work\""));
    assert!(view.details.iter().any(|d| d.contains("Claude")), "{:?}", view.details);
    assert!(pitwall_core::accounts::Accounts::load(&paths).profile_of("/code/api").is_none());
    let asked = w.answer_when_asked(true);
    let v: pitwall_proto::ProjectAccounts = serde_json::from_value(c.call("project.profile", json!({"path": "/code/api", "profile": "Work"})).unwrap()).unwrap();
    asked.join().unwrap();
    assert_eq!(v.profile.as_deref(), Some("Work"));
    let claude = v.slots.iter().find(|s| s.slot == "claude").unwrap();
    assert_eq!((claude.name.as_str(), claude.from.as_str()), ("Work", "profile"));

    // A project's own pick (an override), and the extension's.
    let asked = w.answer_when_asked(true);
    let v: pitwall_proto::ProjectAccounts = serde_json::from_value(c.call("project.pick", json!({"path": "/code/api", "slot": "ext:inbox", "value": "none"})).unwrap()).unwrap();
    asked.join().unwrap();
    let inbox = v.slots.iter().find(|s| s.slot == "ext:inbox").unwrap();
    assert_eq!((inbox.id.as_deref(), inbox.name.as_str(), inbox.from.as_str(), inbox.own.as_deref()), (None, "None", "project", Some("none")));
    let e = server_error(c.call("project.pick", json!({"path": "/code/api", "slot": "nonsense", "value": "x"})).unwrap_err());
    assert_eq!(e.code, code::BAD_PARAMS);

    // Reading never asks, and lists it all.
    let all: pitwall_proto::AccountsView = serde_json::from_value(c.call("accounts.list", Value::Null).unwrap()).unwrap();
    assert_eq!(all.profiles[0].projects, ["/code/api"]);
    assert!(all.accounts.iter().any(|x| x.slot == "ext:inbox" && x.default && x.name == "Home"));

    // Profile now in use: setting it asks.
    let asked = w.answer_when_asked(true);
    c.call("profile.set", json!({"profile": "Work", "slot": "claude", "value": "default"})).unwrap();
    assert!(asked.join().unwrap().details[0].contains("/code/api"));
    let asked = w.answer_when_asked(true);
    c.call("profile.set", json!({"profile": "Work", "slot": "claude", "value": "Work"})).unwrap();
    asked.join().unwrap();

    // Deleted: the project keeps running under Work, as its own pick.
    let asked = w.answer_when_asked(true);
    c.call("profile.delete", json!({"profile": "Work"})).unwrap();
    asked.join().unwrap();
    let v: pitwall_proto::ProjectAccounts = serde_json::from_value(c.call("project.accounts", json!({"path": "/code/api"})).unwrap()).unwrap();
    let claude = v.slots.iter().find(|s| s.slot == "claude").unwrap();
    assert_eq!((v.profile, claude.name.as_str(), claude.from.as_str()), (None, "Work", "project"));
}

// ------------------------------------------------------------ extensions

fn ext_caller(id: &str, name: &str) -> Caller {
    Caller { pid: Some(5150), agent: None, process: Some("node".into()), ui: false, extension: Some((id.into(), name.into())) }
}

/// A process an extension's adapter started, on the socket: it reads
/// what it is granted, is refused the rest in plain words, a revoke
/// applies at its next call, and every call is in its activity log.
#[test]
fn an_extensions_process_gets_only_what_it_is_granted() {
    let w = world(Duration::from_secs(5));
    let log: pitwall_core::extensions::activity::Shared = Arc::default();
    w.extensions.set("ci", "CI", vec!["agents.read".into()], Some(log.clone()));
    let mut c = w.client_as(ext_caller("ci", "CI"));
    assert_eq!(c.call("agent.list", json!({})).unwrap(), json!([]));
    let e = server_error(c.call("project.list", json!({})).unwrap_err());
    assert_eq!(e.code, code::DENIED);
    assert!(e.message.contains("CI isn't granted projects.read"), "{}", e.message);
    let e = server_error(c.call("queue.add", json!({"agentId": "a1", "text": "hi"})).unwrap_err());
    assert_eq!(e.code, code::DENIED);
    assert!(e.message.contains("CI isn't granted agents.queue"), "{}", e.message);
    let e = server_error(c.call("agent.stop", json!({"agentId": "a1"})).unwrap_err());
    assert_eq!(e.code, code::UNSUPPORTED, "no approval is even asked");
    assert!(w.approvals.pending().is_empty());
    // Revoked in Settings: the next call is refused.
    w.extensions.set("ci", "CI", vec![], Some(log.clone()));
    assert_eq!(server_error(c.call("agent.list", json!({})).unwrap_err()).code, code::DENIED);
    let entries = log.lock().unwrap().entries();
    let seen: Vec<(&str, bool)> = entries.iter().map(|e| (e.method.as_str(), e.refused())).collect();
    assert_eq!(seen, [("agent.list", false), ("project.list", true), ("queue.add", true), ("agent.stop", true), ("agent.list", true)]);
    assert!(entries.iter().all(|e| e.via == pitwall_core::extensions::activity::Via::Socket));
    // The user's own terminal is as it was.
    let mut me = w.client();
    assert!(me.call("project.list", json!({})).is_ok());
}

/// Its calls are counted: past the budget they are `busy`.
#[test]
fn an_extension_past_its_call_budget_is_busy() {
    let w = world(Duration::from_secs(5));
    w.extensions.set("ci", "CI", vec!["agents.read".into()], None);
    let mut c = w.client_as(ext_caller("ci", "CI"));
    let mut busy = 0;
    for _ in 0..pitwall_daemon::extensions::MAX_CALLS + 5 {
        if let Err(e) = c.call("agent.list", json!({})) {
            assert_eq!(server_error(e).code, "busy");
            busy += 1;
        }
    }
    assert!(busy >= 5, "{busy}");
}

/// Its events need the permission, and stop when it is revoked.
#[test]
fn an_extensions_events_follow_its_grant() {
    let w = world(Duration::from_secs(5));
    w.extensions.set("ci", "CI", vec![], None);
    let mut c = w.client_as(ext_caller("ci", "CI"));
    assert_eq!(server_error(c.subscribe(&["agents".into()]).unwrap_err()).code, code::DENIED);
    assert_eq!(w.hub.subscribers(), 0);
    w.extensions.set("ci", "CI", vec!["agents.read".into()], None);
    assert_eq!(c.subscribe(&["agents".into(), "blocked".into()]).unwrap().topics, ["agents", "blocked"]);
    assert_eq!(next(&mut c).event, "agents.changed");
    assert_eq!(next(&mut c).event, "blocked.changed");
    w.extensions.set("ci", "CI", vec![], None);
    w.hub.emit(Event::BlockedCount(3));
    // Granted again, the next one arrives; the one while revoked didn't.
    std::thread::sleep(Duration::from_millis(100));
    w.extensions.set("ci", "CI", vec!["agents.read".into()], None);
    w.hub.emit(Event::BlockedCount(4));
    assert_eq!(next(&mut c).data, json!({"count": 4}));
}

/// The same server as the socket's, for an adapter's stdio calls and the
/// buttons Pitwall runs for a view (`Server::call_as`).
fn in_process(w: &World) -> Arc<pitwall_daemon::Server> {
    let cfg = Config { socket: w.socket.clone(), version: "0.1.0-test".into(), settings: None, workspace: None, viewer: None, events: None, extensions: Some(w.extensions.clone()), ext_backend: None };
    pitwall_daemon::build(w.engine.clone(), w.approvals.clone(), w.callers.clone(), cfg)
}

/// Every action is refused without its grant, the same on the socket, on
/// the adapter's stdio and from a view's button Pitwall runs; nothing is
/// queued, started, shown or asked. Granting one grants only that one.
#[test]
fn each_action_is_refused_without_its_grant_on_every_path() {
    use pitwall_core::extensions::activity::Via;
    let w = world(Duration::from_secs(5));
    let dir = TempDir::new("proj");
    let mine = local_agent(&mut w.client(), "worker", &dir);
    pitwall_core::onboarding::add_project(&w.engine, dir.path().to_string_lossy().into()).unwrap();
    let log: pitwall_core::extensions::activity::Shared = Arc::default();
    // Everything but the four actions.
    let reads: Vec<String> = ["agents.read", "projects.read", "worktrees.read", "profiles.read", "notify"].map(String::from).to_vec();
    w.extensions.set("ci", "CI", reads, Some(log.clone()));
    let calls = [
        ("queue.add", json!({"agentId": mine.id, "text": "rm -rf the repo"}), "agents.queue"),
        ("agent.create", json!({"kind": "shell", "project": dir.path(), "prompt": "go"}), "agents.start"),
        ("agent.focus", json!({"agentId": mine.id}), "agents.focus"),
        ("viewer.open", json!({"agentId": mine.id, "files": ["a.txt"]}), "show"),
        ("web.open", json!({"agentId": mine.id, "url": "http://localhost:9/"}), "show"),
    ];
    let mut sock = w.client_as(ext_caller("ci", "CI"));
    let server = in_process(&w);
    let me = server.extension_caller("ci").unwrap();
    for (m, p, perm) in &calls {
        let e = server_error(sock.call(m, p.clone()).unwrap_err());
        assert_eq!(e.code, code::DENIED, "{m} on the socket");
        assert!(e.message.contains(&format!("isn't granted {perm}")), "{m}: {}", e.message);
        for via in [Via::Adapter, Via::View] {
            assert_eq!(server.call_as(&me, m, p.clone(), via).unwrap_err().code, code::DENIED, "{m} via {via:?}");
        }
    }
    assert!(w.approvals.pending().is_empty(), "nothing was asked");
    assert!(w.engine.views().iter().all(|v| v.queue.is_empty()), "nothing was queued");
    assert_eq!(w.engine.views().len(), 1, "nothing was started");
    // Logged, each refused, with what it asked for.
    let entries = log.lock().unwrap().entries();
    assert_eq!(entries.len(), calls.len() * 3);
    assert!(entries.iter().all(|e| e.refused()));
    let queued = entries.iter().find(|e| e.method == "queue.add").unwrap();
    assert_eq!(queued.detail.as_deref(), Some("to “worker”: “rm -rf the repo”"));
    // `show` alone: the agent actions stay refused.
    w.extensions.set("ci", "CI", vec!["show".into()], Some(log.clone()));
    assert_eq!(server.call_as(&me, "queue.add", calls[0].1.clone(), Via::Adapter).unwrap_err().code, code::DENIED);
    assert_eq!(server.call_as(&me, "agent.focus", calls[2].1.clone(), Via::View).unwrap_err().code, code::DENIED);
    // Past the grant, `viewer.open` reaches the host (none here).
    assert_eq!(server.call_as(&me, "viewer.open", calls[3].1.clone(), Via::View).unwrap_err().code, code::UNSUPPORTED);
}

/// Never an extension's, whatever it is granted: sending Next up,
/// answering or listing approvals, changing settings (its own grants and
/// switches live there), removing queued items, stopping agents.
#[test]
fn an_extension_never_sends_approves_or_grants_itself() {
    let w = world(Duration::from_secs(5));
    let dir = TempDir::new("proj");
    let a = local_agent(&mut w.client(), "worker", &dir);
    let all: Vec<String> = pitwall_core::extensions::permissions::ALL.iter().map(|p| p.name.to_string()).collect();
    w.extensions.set("ci", "CI", all, None);
    let mut c = w.client_as(ext_caller("ci", "CI"));
    for (m, p) in [
        ("queue.send", json!({"agentId": a.id})),
        ("queue.remove", json!({"agentId": a.id, "itemId": "x"})),
        ("approval.list", json!({})),
        ("approval.answer", json!({"id": "x", "allow": true})),
        ("settings.set", json!({"key": "extensions.ci.grants", "value": ["agents.queue"]})),
        ("settings.reset", json!({"key": "extensions"})),
        ("agent.stop", json!({"agentId": a.id})),
        ("agent.restart", json!({"agentId": a.id})),
        ("agent.remove", json!({"agentId": a.id})),
        ("session.add", json!({"provider": "vmhost", "machine": "box", "native": "idle", "start": true})),
    ] {
        let e = server_error(c.call(m, p).unwrap_err());
        // The UI's own are refused as such; the rest as not an extension's.
        assert!(e.code == code::UNSUPPORTED || e.code == code::DENIED, "{m}: {} {}", e.code, e.message);
    }
    assert!(w.approvals.pending().is_empty());
    // agent.focus is the UI's and extensions': an agent or a terminal is
    // refused it.
    let mut agent = w.client_as(agent_caller("eng", "Race Engineer"));
    assert_eq!(server_error(agent.call("agent.focus", json!({"agentId": a.id})).unwrap_err()).code, code::DENIED);
    assert_eq!(server_error(w.client().call("agent.focus", json!({"agentId": a.id})).unwrap_err()).code, code::DENIED);
    // `prompt` on agent.create is an extension's alone.
    let e = server_error(w.client().call("agent.create", json!({"kind": "shell", "project": dir.path(), "prompt": "go"})).unwrap_err());
    assert_eq!(e.code, code::BAD_PARAMS);
}

/// `agents.queue`: the item is the extension's (its name on it), bounded
/// in length, paced per agent and capped per agent; the user removes it
/// like any other; turned off, its items go.
#[test]
fn queued_text_is_marked_paced_and_bounded() {
    let w = world(Duration::from_secs(5));
    let dir = TempDir::new("proj");
    let a = local_agent(&mut w.client(), "worker", &dir);
    let b = local_agent(&mut w.client(), "other", &dir);
    w.extensions.set("ci", "CI", vec!["agents.queue".into()], None);
    let mut c = w.client_as(ext_caller("ci", "CI"));
    let long = "x".repeat(pitwall_daemon::actions::MAX_QUEUE_TEXT + 1);
    assert_eq!(server_error(c.call("queue.add", json!({"agentId": a.id, "text": long})).unwrap_err()).code, code::BAD_PARAMS);
    assert_eq!(server_error(c.call("queue.add", json!({"agentId": a.id, "text": " \n "})).unwrap_err()).code, code::BAD_PARAMS);
    assert_eq!(server_error(c.call("queue.add", json!({"agentId": "nope", "text": "hi"})).unwrap_err()).code, code::NOT_FOUND);
    let v = c.call("queue.add", json!({"agentId": a.id, "text": "fix the failing test"})).unwrap();
    assert_eq!(v["queue"][0]["origin"], json!({"extension": "ci", "name": "CI"}));
    let e = server_error(c.call("queue.add", json!({"agentId": a.id, "text": "again"})).unwrap_err());
    assert_eq!(e.code, "busy", "one per agent every 10 s: {}", e.message);
    assert!(c.call("queue.add", json!({"agentId": b.id, "text": "another agent"})).is_ok());
    // The user's own items are as they were: unmarked.
    let mine = w.client().call("queue.add", json!({"agentId": a.id, "text": "mine"})).unwrap();
    assert!(mine["queue"][1].get("origin").is_none());
    // Three of its items waiting is the most.
    for t in ["2", "3"] {
        w.engine.queue_add_from(&a.id, t.into(), Some(pitwall_proto::QueueOrigin { extension: "ci".into(), name: "CI".into() })).unwrap();
    }
    assert_eq!(w.engine.queued_by(&a.id, "ci"), 3);
    // The user removes its item like any other.
    let first = w.engine.views().into_iter().find(|v| v.id == a.id).unwrap().queue[0].id.clone();
    w.client().call("queue.remove", json!({"agentId": a.id, "itemId": first})).unwrap();
    assert_eq!(w.engine.queued_by(&a.id, "ci"), 2);
    // Turned off: its items go, the user's stay.
    assert_eq!(w.engine.queue_remove_origin("ci"), 3);
    let left: Vec<String> = w.engine.views().into_iter().find(|v| v.id == a.id).unwrap().queue.into_iter().map(|q| q.text).collect();
    assert_eq!(left, ["mine"]);
}

/// `agents.start`: asked every time (never remembered), the dialog names
/// the extension, the project, the tool and the prompt in full; a no or
/// no answer starts nothing; a yes starts it with the prompt in Next up as
/// the extension's item. Only Pitwall's projects, this computer, no
/// account; one request waiting at a time.
#[test]
fn starting_an_agent_asks_the_user_every_time() {
    let w = world(Duration::from_millis(300));
    let dir = TempDir::new("proj");
    let project = dir.path().to_string_lossy().to_string();
    pitwall_core::onboarding::add_project(&w.engine, project.clone()).unwrap();
    w.extensions.set("ci", "CI", vec!["agents.start".into()], None);
    let mut c = w.client_as(ext_caller("ci", "CI"));
    let ask = |extra: Value| {
        let mut p = json!({"kind": "shell", "project": project});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        p
    };
    for (extra, code) in [
        (json!({"project": "/not/a/project"}), code::NOT_FOUND),
        (json!({"machine": "box", "provider": "vmhost"}), code::BAD_PARAMS),
        (json!({"account": "work"}), code::BAD_PARAMS),
        (json!({"resume": "abc"}), code::BAD_PARAMS),
        (json!({"prompt": "y".repeat(pitwall_daemon::actions::MAX_QUEUE_TEXT + 1)}), code::BAD_PARAMS),
    ] {
        assert_eq!(server_error(c.call("agent.create", ask(extra.clone())).unwrap_err()).code, code, "{extra}");
    }
    assert!(w.approvals.pending().is_empty(), "refused before asking");
    // No answer in time: a no.
    let e = server_error(c.call("agent.create", ask(json!({"prompt": "look at CI"}))).unwrap_err());
    assert_eq!(e.code, code::APPROVAL_TIMEOUT);
    assert!(w.engine.views().is_empty());
    // Denied.
    let asked = w.answer_when_asked(false);
    assert_eq!(server_error(c.call("agent.create", ask(json!({"prompt": "look at CI"}))).unwrap_err()).code, code::DENIED);
    let view = asked.join().unwrap();
    assert_eq!(view.risk, Risk::High);
    assert!(!view.rememberable, "asked every time");
    assert_eq!(view.requester.kind, RequesterKind::Extension);
    assert!(view.summary.starts_with("start a "), "{}", view.summary);
    let details = view.details.join("\n");
    for want in ["Asked by the extension “CI”, not by you.", &format!("Project: {project}"), "Tool: ", "look at CI", "sent only when you press Send"] {
        assert!(details.contains(want), "{want} in {details}");
    }
    assert!(w.engine.views().is_empty());
    // Allowed (the third request in ten minutes): started, its prompt
    // waiting in Next up as CI's.
    let asked = w.answer_when_asked(true);
    let v = c.call("agent.create", ask(json!({"prompt": "look at CI\nline two"}))).unwrap();
    asked.join().unwrap();
    assert_eq!(v["queue"][0]["text"], "look at CI\nline two");
    assert_eq!(v["queue"][0]["origin"]["name"], "CI");
    assert_eq!(w.engine.views().len(), 1);
    // A fourth in ten minutes is refused without asking.
    let e = server_error(c.call("agent.create", ask(json!({}))).unwrap_err());
    assert_eq!(e.code, "busy", "{}", e.message);
    assert!(w.approvals.pending().is_empty());
}

/// The real identity: a process whose parent is an adapter Pitwall
/// started is the extension's; this test's own process (no adapter in its
/// chain) stays outside.
#[test]
fn the_parent_chain_names_the_extension() {
    let w = world(Duration::from_secs(5));
    let id = ProcessIdentity::with_extensions(w.engine.clone(), w.extensions.clone());
    let me = std::process::id();
    assert_eq!(id.identify(Some(me)).extension, None);
    w.extensions.set("ci", "CI", vec![], None);
    // This process stands in for an adapter: a child of it is the
    // extension's.
    w.extensions.adapter_started("ci", me);
    let (program, args): (&str, &[&str]) = if cfg!(windows) { ("cmd", &["/C", "ping", "-n", "3", "127.0.0.1"]) } else { ("sleep", &["2"]) };
    let mut child = std::process::Command::new(program).args(args).spawn().unwrap();
    let c = id.identify(Some(child.id()));
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(c.extension, Some(("ci".into(), "CI".into())));
    assert_eq!(c.requester().kind, RequesterKind::Extension);
    assert!(!c.ui);
}
