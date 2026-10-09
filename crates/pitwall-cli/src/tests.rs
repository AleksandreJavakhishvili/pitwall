//! Output snapshots (fixed fixtures), error JSON and exit codes, and the
//! commands end to end against a server on a temp socket with fake
//! providers (no real agents, agw or `~/.pitwall`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use super::*;
use pitwall_core::exec::LocalExec;
use pitwall_core::paths::Paths;
use pitwall_core::provider::ProviderCaps;
use pitwall_core::testing::{FakeProvider, ManualClock, MemStore, RecordingSink, TempDir};
use pitwall_core::{Deps, Engine};
use pitwall_daemon::{serve, Approvals, Caller, Identify};
use pitwall_proto::{AgentView, ApprovalAnswer, CreateForm, ProviderMachines, ScannedPlace, SessionAdded};

fn agent(id: &str, name: &str, running: bool) -> AgentView {
    serde_json::from_value(json!({
        "id": id, "name": name, "kind": "claude", "kindName": "Claude Code", "terminal": false, "sessionId": null,
        "cwd": "/Users/me/api", "cwdDisplay": "~/api", "project": "/Users/me/api", "projectDisplay": "~/api",
        "branch": null, "worktree": false, "worktreePending": false, "location": "agw",
        "machine": {"provider": "agw", "id": "vm-1", "label": "vm-1", "canCreate": false},
        "agentInTerminal": false, "restartAs": null, "status": if running { "idle" } else { "stopped" },
        "statusSource": "screen", "statusDetail": null, "running": running, "cols": 120, "rows": 40,
        "added": 0, "removed": 0, "filesChanged": 0, "queue": [], "autoSend": true, "lastSent": null,
        "lastSentAt": null, "createdAt": 1, "currentTaskId": null,
        "caps": {"input": running, "restart": true, "resume": false, "stop": running, "removeWorktree": false,
                 "diff": false, "review": false, "merge": false, "rules": false, "hooks": false, "removeKeepsSession": true}
    }))
    .unwrap()
}

#[test]
fn human_snapshots() {
    let list = [agent("a-1", "work", true), agent("a-22", "reviewer", false)];
    assert_eq!(
        render::agents(&list),
        "NAME      KIND         STATUS   MACHINE  FOLDER  ID\n\
         work      Claude Code  idle     vm-1     ~/api   a-1\n\
         reviewer  Claude Code  stopped  vm-1     ~/api   a-22\n"
    );
    assert_eq!(render::agents(&[]), "No agents in Pitwall.\n");
    assert_eq!(render::agent_new(&list[0]), "Started work (Claude Code) in ~/api — id a-1\n");

    let machines: Vec<ProviderMachines> = serde_json::from_value(json!([
        {"provider": "local", "label": "This Mac", "version": null, "canCreate": true, "canAddSessions": false,
         "machines": [{"id": "this-mac", "label": "This Mac", "detail": null}], "error": null},
        {"provider": "agw", "label": "agw", "version": "0.19.0", "canCreate": false, "canAddSessions": true,
         "machines": [{"id": "vm-1", "label": "vm-1", "detail": "home"}], "error": null}
    ]))
    .unwrap();
    assert_eq!(
        render::machines(&machines),
        "This Mac (local) — new agents\n  this-mac  This Mac\n\
         agw (agw 0.19.0) — add sessions\n  vm-1  vm-1  home\n"
    );

    let places: Vec<ScannedPlace> = serde_json::from_value(json!([{
        "provider": "agw", "label": "agw", "version": "0.19.0", "machines": [{"id": "vm-1", "label": "vm-1", "detail": null, "sessions": [
            {"provider": "agw", "machine": "vm-1", "native": "work", "name": "work", "kind": "claude", "kindName": "Claude Code",
             "program": "claude-code", "workspace": "pitwall", "user": null, "cwd": null, "status": "running", "inPitwall": true},
            {"provider": "agw", "machine": "vm-1", "native": "night-build", "name": "night-build", "kind": "codex", "kindName": "Codex",
             "program": "codex", "workspace": null, "user": null, "cwd": null, "status": "stopped", "inPitwall": false}
        ]}]
    }]))
    .unwrap();
    assert_eq!(
        render::sessions(&places),
        "agw / vm-1\n  work         Claude Code  running  in Pitwall  pitwall\n  night-build  Codex        stopped  -\n"
    );
    assert_eq!(render::sessions(&[]), "No other places with sessions (is agw set up?).\n");

    let added = |already, started| SessionAdded { agent: agent("a-1", "work", started || already), already, started };
    assert_eq!(render::session_added(&added(false, false)), "Added work on vm-1 (stopped) — id a-1\n");
    assert_eq!(render::session_added(&added(true, false)), "Already in Pitwall: work on vm-1 (running) — id a-1\n");
    assert_eq!(render::session_added(&added(false, true)), "Started and added work on vm-1 (running) — id a-1\n");
}

#[test]
fn errors_are_json_with_exit_codes() {
    let denied = Error::Server(ErrorBody::new(code::DENIED, "The user didn't allow it."));
    assert_eq!(failure(&denied), (json!({"error": {"code": "denied", "message": "The user didn't allow it."}}), 3));
    assert_eq!(failure(&Error::Server(ErrorBody::new(code::APPROVAL_TIMEOUT, "x"))).1, 3);
    assert_eq!(failure(&Error::Server(ErrorBody::new(code::NOT_FOUND, "x"))).1, 1);
    let gone = Error::Connect { path: PathBuf::from("/nowhere.sock"), source: std::io::Error::from(std::io::ErrorKind::NotFound) };
    let (body, status) = failure(&gone);
    assert_eq!((body["error"]["code"].as_str(), status), (Some("not_connected"), 1));
    assert!(body["error"]["message"].as_str().unwrap().contains("Is Pitwall running?"));
}

#[test]
fn project_folders_are_made_absolute() {
    let dir = TempDir::new("cli-abs");
    let real = std::fs::canonicalize(dir.path()).unwrap();
    std::fs::create_dir(real.join("sub")).unwrap();
    assert_eq!(absolute(None, &real), real.to_string_lossy());
    assert_eq!(absolute(Some("sub"), &real), real.join("sub").to_string_lossy());
    assert_eq!(absolute(Some("~/p"), &real), "~/p", "the server expands ~ on the target machine");
    assert_eq!(absolute(Some("/abs/missing"), &real), "/abs/missing");
}

fn vm_form() -> CreateForm {
    use pitwall_proto::{CreateChoice, CreateField};
    let pick = |v: &str| CreateChoice { value: v.into(), label: v.into(), detail: None, phrase: Some(format!("in {v}")), creates: None };
    CreateForm {
        folder: false,
        fields: vec![
            CreateField::select("workspace", "Workspace", vec![pick("work")], Some("work".into())),
            CreateField::select("runAs", "Runs as", vec![pick("admin"), pick("agent:bot")], Some("admin".into())),
        ],
        summary: Some("Creates {name} {workspace}".into()),
        submit: "Create".into(),
        ..CreateForm::folder("", "", "")
    }
}

#[test]
fn machine_flags_become_form_options() {
    let new = |args: &[&str]| match Cli::try_parse_from(["pitwall", "agent", "new", "--machine", "vm", "--name", "s"].iter().chain(args)).unwrap().command {
        Command::Agent(AgentCmd::New(n)) => machine_options(&n),
        _ => unreachable!(),
    };
    let map = |kv: &[(&str, &str)]| kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
    assert_eq!(new(&["--workspace", "work", "--as", "admin", "--template", "claude"]).unwrap(), map(&[("workspace", "work"), ("runAs", "admin"), ("template", "claude")]));
    assert_eq!(new(&["--workspace", "work", "--as", "agent:bot"]).unwrap(), map(&[("workspace", "work"), ("runAs", "agent:bot")]));
    assert_eq!(
        new(&["--new-workspace", "--workspace-template", "api-session", "--as", "new-agent:helper", "--agent-template", "claude"]).unwrap(),
        map(&[("workspace", "+new"), ("workspaceTemplate", "api-session"), ("runAs", "+new"), ("agentName", "helper"), ("agentTemplate", "claude")])
    );
    assert_eq!(new(&["--new-workspace", "scratch", "--as", "new-agent"]).unwrap(), map(&[("workspace", "+new"), ("workspaceName", "scratch"), ("runAs", "+new")]));
    assert_eq!(new(&["--option", "color=red"]).unwrap(), map(&[("color", "red")]));
    assert!(new(&["--as", "root"]).unwrap_err().contains("expected admin"));
    assert!(new(&["--option", "oops"]).unwrap_err().contains("FIELD=VALUE"));
}

struct Outside;
impl Identify for Outside {
    fn identify(&self, _pid: Option<u32>) -> Caller {
        Caller::outside()
    }
}

#[test]
fn commands_end_to_end() {
    let dir = TempDir::new("cli-e2e");
    let local = FakeProvider::named("local", "this-mac", Arc::new(LocalExec));
    let vm = FakeProvider::named("agw", "vm-1", Arc::new(LocalExec));
    vm.set_caps(ProviderCaps { start: true, attach_existing: true, survives_detach: true, ..Default::default() });
    vm.set_remote(true);
    vm.add_session("work", "claude-code", "/srv/work").exit(0);
    let engine = Engine::open(Deps {
        paths: Paths::new(dir.path().join("pitwall")),
        events: RecordingSink::new(),
        clock: ManualClock::new(1),
        store: MemStore::with(vec![]),
        providers: vec![local, vm.clone()],
    });
    let approvals = Approvals::new(Duration::from_secs(10));
    let sock_dir = std::env::temp_dir().join(format!("pwc-{}", std::process::id()));
    std::fs::create_dir_all(&sock_dir).unwrap();
    let socket = sock_dir.join("c.sock");
    let server = serve(engine, approvals.clone(), Arc::new(Outside), pitwall_daemon::Config { socket: socket.clone(), version: "t".into(), settings: None, workspace: None }).unwrap();
    let parse = |args: &[&str]| Cli::try_parse_from(std::iter::once("pitwall").chain(args.iter().copied())).unwrap().command;

    let o = run(&parse(&["agent", "list"]), &socket).unwrap();
    assert_eq!(o.json, json!([]));
    let o = run(&parse(&["machine", "list"]), &socket).unwrap();
    assert_eq!(o.json[1]["provider"], "agw");
    assert_eq!(o.json[1]["machines"][0]["id"], "vm-1");
    let o = run(&parse(&["session", "list", "--provider", "agw"]), &socket).unwrap();
    assert_eq!(o.json[0]["machines"][0]["sessions"][0]["native"], "work");
    assert_eq!(o.json[0]["machines"][0]["sessions"][0]["inPitwall"], false);

    // --start on a stopped session: denied in Pitwall → exit 3, still added.
    let answer = |allow: bool| {
        let a = approvals.clone();
        std::thread::spawn(move || loop {
            if let Some(p) = a.pending().first() {
                a.answer(&ApprovalAnswer { id: p.id.clone(), allow, remember: false }).unwrap();
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        })
    };
    let t = answer(false);
    let e = run(&parse(&["session", "add", "agw", "vm-1", "work", "--start"]), &socket).err().unwrap();
    t.join().unwrap();
    assert_eq!(failure(&e).1, 3);
    let t = answer(true);
    let o = run(&parse(&["session", "add", "agw", "vm-1", "work", "--start"]), &socket).unwrap();
    t.join().unwrap();
    assert_eq!((o.json["already"].as_bool(), o.json["started"].as_bool()), (Some(true), Some(true)));
    assert_eq!(o.json["agent"]["running"], true);
    assert_eq!(vm.launched().len(), 1);

    // A session on the VM: the user approves it in Pitwall first.
    vm.set_caps(ProviderCaps { create: true, start: true, attach_existing: true, survives_detach: true, ..Default::default() });
    vm.set_form(vm_form());
    let o = run(&parse(&["machine", "form", "vm-1"]), &socket).unwrap();
    assert_eq!((o.json["provider"].as_str(), o.json["folder"].as_bool()), (Some("agw"), Some(false)));
    assert!(o.human.contains("runAs — Runs as, default admin"), "{}", o.human);
    let new_on_vm = ["agent", "new", "--machine", "vm-1", "--name", "api-fix", "--workspace", "work", "--as", "admin"];
    let t = answer(false);
    let e = run(&parse(&new_on_vm), &socket).err().unwrap();
    t.join().unwrap();
    assert_eq!(failure(&e).1, 3, "denied: exit 3");
    assert!(vm.created().is_empty(), "nothing was created");
    let t = answer(true);
    let o = run(&parse(&new_on_vm), &socket).unwrap();
    t.join().unwrap();
    assert_eq!((o.json["name"].as_str(), o.json["running"].as_bool()), (Some("api-fix"), Some(true)));
    assert_eq!(vm.created()[0].1.get("runAs").map(String::as_str), Some("admin"));
    let e = run(&parse(&["agent", "new", "--machine", "vm-1", "--name", "x", "--as", "root"]), &socket).err().unwrap();
    assert!(failure(&e).0["error"]["message"].as_str().unwrap().contains("expected admin"));

    let project = TempDir::new("cli-proj");
    let p = project.path().to_string_lossy().into_owned();
    let o = run(&parse(&["agent", "new", "--kind", "shell", "--project", &p, "--name", "scratch"]), &socket).unwrap();
    assert_eq!((o.json["name"].as_str(), o.json["kind"].as_str()), (Some("scratch"), Some("shell")));
    let e = run(&parse(&["agent", "new", "--kind", "nope", "--project", &p]), &socket).err().unwrap();
    assert_eq!(failure(&e).1, 1);
    let o = run(&parse(&["agent", "list"]), &socket).unwrap();
    assert_eq!(o.json.as_array().unwrap().len(), 3);

    server.stop();
    let _ = std::fs::remove_dir_all(&sock_dir);
    assert_eq!(failure(&run(&parse(&["agent", "list"]), &socket).err().unwrap()).0["error"]["code"], "not_connected");
}

/// The app's side of `settings.*`, in memory.
#[derive(Default)]
struct MemSettings(std::sync::Mutex<BTreeMap<String, serde_json::Value>>);

impl pitwall_daemon::SettingsBackend for MemSettings {
    fn get(&self, s: &'static pitwall_proto::settings::Setting) -> Result<serde_json::Value, String> {
        Ok(self.0.lock().unwrap().get(s.key).cloned().unwrap_or_else(|| s.default_value()))
    }
    fn set(&self, s: &'static pitwall_proto::settings::Setting, v: serde_json::Value) -> Result<serde_json::Value, String> {
        self.0.lock().unwrap().insert(s.key.into(), v.clone());
        Ok(v)
    }
}

fn settings_cmd(args: &[&str]) -> args::SettingsCmd {
    match Cli::try_parse_from(["pitwall", "settings"].iter().chain(args)).unwrap().command {
        Command::Settings(s) => s,
        _ => unreachable!(),
    }
}

#[test]
fn settings_against_a_running_pitwall() {
    let dir = TempDir::new("cli-settings");
    let engine = Engine::open(Deps {
        paths: Paths::new(dir.path().join("pitwall")),
        events: RecordingSink::new(),
        clock: ManualClock::new(1),
        store: MemStore::with(vec![]),
        providers: vec![FakeProvider::named("local", "this-mac", Arc::new(LocalExec))],
    });
    let approvals = Approvals::new(Duration::from_secs(10));
    let backend = Arc::new(MemSettings::default());
    let sock_dir = std::env::temp_dir().join(format!("pwcs-{}", std::process::id()));
    std::fs::create_dir_all(&sock_dir).unwrap();
    let socket = sock_dir.join("s.sock");
    let cfg = pitwall_daemon::Config { socket: socket.clone(), version: "t".into(), settings: Some(backend.clone()), workspace: None };
    let server = serve(engine, approvals.clone(), Arc::new(Outside), cfg).unwrap();
    // The file is never touched while Pitwall answers.
    let root = dir.path().join("home");
    let run = |args: &[&str]| settings::run(&settings_cmd(args), &socket, &root);

    let o = run(&["list"]).unwrap();
    let list = o.json.as_array().unwrap();
    assert_eq!(list.len(), pitwall_proto::settings::SETTINGS.len());
    let theme = list.iter().find(|s| s["key"] == "appearance.theme").unwrap();
    assert_eq!((theme["value"].as_str(), theme["sensitivity"].as_str()), (Some("system"), Some("safe")));
    assert_eq!(theme["choices"], json!(["system", "dark", "light"]));
    assert!(o.human.contains("appearance.theme") && o.human.contains("system|dark|light"), "{}", o.human);
    assert!(o.human.contains("(approval)"));

    // Safe: applied directly, no approval.
    let o = run(&["set", "appearance.theme", "dark"]).unwrap();
    assert_eq!(o.json["value"], "dark");
    assert_eq!(o.human, "appearance.theme = dark\n");
    assert_eq!(run(&["get", "appearance.theme"]).unwrap().json["value"], "dark");
    assert_eq!(run(&["reset", "appearance.theme"]).unwrap().json["value"], "system");
    assert_eq!(run(&["set", "appearance.terminalFontSize", "15"]).unwrap().json["value"], 15);

    // Validation: clear errors, exit 2, nothing changed.
    let e = run(&["set", "appearance.theme", "purple"]).err().unwrap();
    let (body, status) = failure(&e);
    assert_eq!((body["error"]["code"].as_str(), status), (Some("bad_params"), 2), "bad arguments: exit 2");
    assert!(body["error"]["message"].as_str().unwrap().contains("one of system, dark, light"));
    let e = run(&["get", "appearance.nope"]).err().unwrap();
    assert_eq!(failure(&e).0["error"]["code"], "not_found");
    assert!(run(&["set", "agents.codexHooks", "false"]).is_err(), "can't be turned off");

    // Approval: denied → exit 3, unchanged; allowed → applied.
    let answer = |allow: bool| {
        let a = approvals.clone();
        std::thread::spawn(move || loop {
            if let Some(p) = a.pending().first() {
                assert!(p.summary.contains("agents.codexHooks"), "{}", p.summary);
                a.answer(&ApprovalAnswer { id: p.id.clone(), allow, remember: false }).unwrap();
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        })
    };
    let t = answer(false);
    let e = run(&["set", "agents.codexHooks", "true"]).err().unwrap();
    t.join().unwrap();
    assert_eq!(failure(&e).1, 3);
    assert_eq!(run(&["get", "agents.codexHooks"]).unwrap().json["value"], false);
    let t = answer(true);
    assert_eq!(run(&["set", "agents.codexHooks", "on"]).unwrap().json["value"], true);
    t.join().unwrap();
    // Already that value: nothing to approve.
    assert_eq!(run(&["set", "agents.codexHooks", "true"]).unwrap().json["value"], true);
    assert!(approvals.pending().is_empty());

    server.stop();
    assert!(!root.join("ui.json").exists());
    let _ = std::fs::remove_dir_all(&sock_dir);
}

#[test]
fn settings_without_pitwall_edit_ui_json() {
    let dir = TempDir::new("cli-settings-off");
    let root = dir.path().to_path_buf();
    let socket = root.join("nobody.sock");
    let run = |args: &[&str]| settings::run(&settings_cmd(args), &socket, &root);
    let file = || serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(root.join("ui.json")).unwrap()).unwrap();

    // No file yet: defaults; Pitwall-only settings read as unknown.
    let o = run(&["list"]).unwrap();
    assert_eq!(o.json.as_array().unwrap().iter().find(|s| s["key"] == "agents.codexHooks").unwrap()["value"], json!(null));
    assert!(o.human.contains("Pitwall isn't running"));
    assert_eq!(run(&["set", "appearance.density", "dense"]).unwrap().json["value"], "dense");
    assert_eq!(file(), json!({"v": 1, "density": "dense"}));

    // A v0.1 file keeps everything else, and its keys.
    std::fs::write(root.join("ui.json"), r#"{"v":1,"spaces":[{"id":"all"}],"hideElsewhere":true,"theme":"light","later":7}"#).unwrap();
    assert_eq!(run(&["get", "general.showElsewhere"]).unwrap().json["value"], false);
    run(&["set", "general.showElsewhere", "true"]).unwrap();
    run(&["reset", "appearance.theme"]).unwrap();
    let f = file();
    assert_eq!((f["hideElsewhere"].clone(), f["theme"].clone(), f["later"].clone()), (json!(false), json!("system"), json!(7)));
    assert_eq!(f["spaces"], json!([{"id": "all"}]));
    assert!(!root.join("ui.json.lock").exists(), "the lock is released");

    // Approval settings need the app.
    let e = run(&["set", "rules.allowNpx", "true"]).err().unwrap();
    assert_eq!(failure(&e).0["error"]["code"], "not_running");
    // A broken file is reported, never overwritten.
    std::fs::write(root.join("ui.json"), "{oops").unwrap();
    let e = run(&["set", "appearance.theme", "dark"]).err().unwrap();
    assert_eq!(failure(&e).0["error"]["code"], "conflict");
    assert_eq!(std::fs::read_to_string(root.join("ui.json")).unwrap(), "{oops");
    // Another writer holds the lock: wait, then give up.
    std::fs::write(root.join("ui.json"), r#"{"v":1}"#).unwrap();
    std::fs::write(root.join("ui.json.lock"), "1").unwrap();
    let e = run(&["set", "appearance.theme", "dark"]).err().unwrap();
    assert!(failure(&e).0["error"]["message"].as_str().unwrap().contains("locked"));
}

/// The app's spaces, in memory (the app's own is tested in pitwall-app).
#[derive(Default)]
struct MemSpaces(std::sync::Mutex<Vec<pitwall_proto::SpaceView>>);

impl pitwall_daemon::WorkspaceBackend for MemSpaces {
    fn spaces(&self) -> Result<Vec<pitwall_proto::SpaceView>, ErrorBody> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn create(&self, name: &str) -> Result<pitwall_proto::SpaceView, ErrorBody> {
        let mut all = self.0.lock().unwrap();
        let s = pitwall_proto::SpaceView {
            id: format!("space-{}", all.len()),
            name: name.into(),
            kind: "custom".into(),
            project: None,
            window: "main".into(),
            shown: vec![],
            members: vec![],
        };
        all.push(s.clone());
        Ok(s)
    }
    fn rename(&self, space: &str, name: &str) -> Result<pitwall_proto::SpaceView, ErrorBody> {
        let mut all = self.0.lock().unwrap();
        let id = pitwall_daemon::workspace::find(&all, space)?.id.clone();
        let s = all.iter_mut().find(|s| s.id == id).unwrap();
        s.name = name.into();
        Ok(s.clone())
    }
    fn move_to_window(&self, space: &str, window: &str) -> Result<pitwall_proto::SpaceView, ErrorBody> {
        let mut all = self.0.lock().unwrap();
        let id = pitwall_daemon::workspace::find(&all, space)?.id.clone();
        let s = all.iter_mut().find(|s| s.id == id).unwrap();
        s.window = if window == "new" { "pitwall-2".into() } else { window.into() };
        Ok(s.clone())
    }
    fn move_agent(&self, agent: &str, space: &str) -> Result<pitwall_proto::SpaceView, ErrorBody> {
        let mut all = self.0.lock().unwrap();
        let id = pitwall_daemon::workspace::find(&all, space)?.id.clone();
        for s in all.iter_mut() {
            s.shown.retain(|a| a != agent);
        }
        let s = all.iter_mut().find(|s| s.id == id).unwrap();
        s.shown.push(agent.into());
        Ok(s.clone())
    }
}

/// An in-process Pitwall: engine on fake providers, the socket server,
/// approvals answered (or not) by the test.
struct Host {
    server: Option<pitwall_daemon::Handle>,
    approvals: Arc<Approvals>,
    socket: PathBuf,
    sock_dir: PathBuf,
    _dir: TempDir,
}

impl Host {
    fn new(tag: &str, timeout: Duration, spaces: bool) -> Host {
        let dir = TempDir::new(tag);
        let engine = Engine::open(Deps {
            paths: Paths::new(dir.path().join("pitwall")),
            events: RecordingSink::new(),
            clock: ManualClock::new(1),
            store: MemStore::with(vec![]),
            providers: vec![FakeProvider::named("local", "this-mac", Arc::new(LocalExec))],
        });
        let approvals = Approvals::new(timeout);
        let sock_dir = std::env::temp_dir().join(format!("pw{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&sock_dir).unwrap();
        let socket = sock_dir.join("m.sock");
        let workspace: Option<Arc<dyn pitwall_daemon::WorkspaceBackend>> = if spaces {
            let mem = MemSpaces::default();
            pitwall_daemon::WorkspaceBackend::create(&mem, "All").unwrap();
            Some(Arc::new(mem))
        } else {
            None
        };
        let cfg = pitwall_daemon::Config { socket: socket.clone(), version: "t".into(), settings: None, workspace };
        let server = serve(engine, approvals.clone(), Arc::new(Outside), cfg).unwrap();
        Host { server: Some(server), approvals, socket, sock_dir, _dir: dir }
    }

    fn run(&self, args: &[&str]) -> Result<Output, Error> {
        run(&Cli::try_parse_from(std::iter::once("pitwall").chain(args.iter().copied())).unwrap().command, &self.socket)
    }

    /// Answer the next approval, as the user in Pitwall's dialog.
    fn answer(&self, allow: bool) -> std::thread::JoinHandle<pitwall_proto::ApprovalView> {
        let a = self.approvals.clone();
        std::thread::spawn(move || loop {
            if let Some(p) = a.pending().first().cloned() {
                a.answer(&ApprovalAnswer { id: p.id.clone(), allow, remember: false }).unwrap();
                return p;
            }
            std::thread::sleep(Duration::from_millis(5));
        })
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some(s) = self.server.take() {
            s.stop();
        }
        let _ = std::fs::remove_dir_all(&self.sock_dir);
    }
}

#[test]
fn managing_agents_queues_and_spaces() {
    let h = Host::new("m", Duration::from_secs(10), true);
    let project = TempDir::new("cli-m-proj");
    let p = project.path().to_string_lossy().into_owned();
    let a = h.run(&["agent", "new", "--kind", "shell", "--project", &p, "--name", "worker"]).unwrap().json;
    let id = a["id"].as_str().unwrap().to_string();
    h.run(&["agent", "new", "--kind", "shell", "--project", &p, "--name", "helper"]).unwrap();

    // Agents by name or id.
    let o = h.run(&["agent", "rename", "worker", "api"]).unwrap();
    assert_eq!((o.json["name"].as_str(), o.human.as_str()), (Some("api"), "Renamed worker to api\n"));
    assert_eq!(h.run(&["agent", "status", &id]).unwrap().json["agentId"], id.as_str());
    let e = h.run(&["agent", "status", "nobody"]).err().unwrap();
    assert_eq!(failure(&e), (json!({"error": {"code": "not_found", "message": "no agent \"nobody\" in Pitwall (see `pitwall agent list`)"}}), 1));

    // Queue: add, list (numbered), remove by number, send.
    h.run(&["queue", "add", "api", "run the tests"]).unwrap();
    let o = h.run(&["queue", "add", "api", "then lint"]).unwrap();
    assert_eq!(o.human, "Queued for api (2 in its queue)\n");
    let o = h.run(&["queue", "list", "--human"]).unwrap();
    assert_eq!(o.json[0]["items"].as_array().unwrap().len(), 2);
    assert!(o.human.contains("  1. run the tests\n  2. then lint\n"), "{}", o.human);
    assert_eq!(h.run(&["queue", "remove", "api", "2"]).unwrap().json["queue"].as_array().unwrap().len(), 1);
    let e = h.run(&["queue", "remove", "api", "5"]).err().unwrap();
    assert_eq!(failure(&e).0["error"]["code"], "not_found");
    let o = h.run(&["queue", "send", "api"]).unwrap();
    assert_eq!(o.json["lastSent"], "run the tests");
    // Batch: every agent with a status (both are "unknown" here).
    let o = h.run(&["queue", "add", "--status", "unknown", "hello"]).unwrap();
    assert_eq!(o.json.as_array().unwrap().len(), 2);
    assert!(o.human.starts_with("Queued for 2 agents"), "{}", o.human);
    assert_eq!(h.run(&["queue", "add", "--status", "blocked", "x"]).unwrap().human, "No agent has that status: nothing queued.\n");
    let e = h.run(&["queue", "add", "--status", "sleepy", "x"]).err().unwrap();
    assert_eq!(failure(&e).1, 2, "a bad status is a bad argument");

    // Spaces.
    let o = h.run(&["space", "create", "API work"]).unwrap();
    assert_eq!(o.json["name"], "API work");
    h.run(&["agent", "move", "api", "--space", "api work"]).unwrap();
    let o = h.run(&["space", "move", "API work", "--to-window", "new"]).unwrap();
    assert_eq!(o.json["window"], "pitwall-2");
    let o = h.run(&["space", "list"]).unwrap();
    assert_eq!(o.json[1]["shown"], json!([id]));
    assert!(o.human.contains("API work  custom  pitwall-2  api"), "{}", o.human);
    assert_eq!(h.run(&["space", "rename", "API work", "API"]).unwrap().json["name"], "API");
    assert_eq!(failure(&h.run(&["space", "rename", "nope", "x"]).err().unwrap()).0["error"]["code"], "not_found");

    // Wait: already there; a bad status is a bad argument.
    let o = h.run(&["wait", "api", "--for", "unknown"]).unwrap();
    assert_eq!(o.json["agent"]["id"], id.as_str());
    assert_eq!(failure(&h.run(&["wait", "api", "--for", "later"]).err().unwrap()).1, 2);
    let e = h.run(&["wait", "api", "--for", "idle", "--timeout", "0"]).err().unwrap();
    assert_eq!((failure(&e).0["error"]["code"].as_str(), failure(&e).1), (Some("timeout"), 1));

    // Projects.
    let o = h.run(&["project", "add", &p]).unwrap();
    assert_eq!(o.json.as_array().unwrap().len(), 1);
    assert_eq!(h.run(&["project", "list"]).unwrap().json.as_array().unwrap().len(), 1);
    assert!(h.run(&["project", "remove", &p]).unwrap().json.as_array().unwrap().is_empty());

    assert!(h.approvals.pending().is_empty(), "none of that asked the user");
}

#[test]
fn risky_commands_wait_for_the_user() {
    let h = Host::new("r", Duration::from_secs(10), false);
    let project = TempDir::new("cli-r-proj");
    let p = project.path().to_string_lossy().into_owned();
    h.run(&["agent", "new", "--kind", "shell", "--project", &p, "--name", "worker"]).unwrap();

    // Denied → exit 3, still running.
    let t = h.answer(false);
    let e = h.run(&["agent", "stop", "worker"]).err().unwrap();
    assert_eq!(t.join().unwrap().summary, "stop \"worker\"");
    assert_eq!((failure(&e).0["error"]["code"].as_str(), failure(&e).1), (Some("denied"), 3));
    assert_eq!(h.run(&["agent", "list"]).unwrap().json[0]["running"], true);
    // Allowed → done.
    let t = h.answer(true);
    let o = h.run(&["agent", "stop", "worker"]).unwrap();
    t.join().unwrap();
    assert_eq!((o.json["running"].as_bool(), o.human.starts_with("Stopped worker")), (Some(false), true));
    let t = h.answer(true);
    assert_eq!(h.run(&["agent", "restart", "worker"]).unwrap().json["running"], true);
    t.join().unwrap();
    // A worktree it doesn't have: refused before asking (bad argument).
    assert_eq!(failure(&h.run(&["agent", "remove", "worker", "--worktree"]).err().unwrap()).1, 2);
    let t = h.answer(true);
    let o = h.run(&["agent", "remove", "worker"]).unwrap();
    let asked = t.join().unwrap();
    assert_eq!((asked.risk, o.json["removed"].is_string()), (pitwall_proto::Risk::High, true));
    assert_eq!(h.run(&["agent", "list"]).unwrap().json, json!([]));
    // Without windows: spaces aren't offered.
    let e = h.run(&["space", "list"]).err().unwrap();
    assert_eq!(failure(&e).0["error"]["code"], "unsupported");
}

#[test]
fn an_unanswered_approval_is_exit_3() {
    let h = Host::new("t", Duration::from_millis(150), false);
    let project = TempDir::new("cli-t-proj");
    let p = project.path().to_string_lossy().into_owned();
    h.run(&["agent", "new", "--kind", "shell", "--project", &p, "--name", "worker"]).unwrap();
    let e = h.run(&["agent", "remove", "worker"]).err().unwrap();
    assert_eq!((failure(&e).0["error"]["code"].as_str(), failure(&e).1), (Some("approval_timeout"), 3));
    assert_eq!(h.run(&["agent", "list"]).unwrap().json.as_array().unwrap().len(), 1, "nothing removed");
}

#[test]
fn usage_errors_are_json_with_exit_2() {
    let e = Cli::try_parse_from(["pitwall", "agent", "frobnicate"]).unwrap_err();
    let v = usage_error(&e.render().to_string());
    assert_eq!(v["error"]["code"], "bad_args");
    assert!(v["error"]["message"].as_str().unwrap().contains("frobnicate"), "{v}");
    assert_eq!(exit_status("bad_args"), 2);
    assert_eq!(exit_status(code::BAD_PARAMS), 2);
    assert_eq!(exit_status(code::DENIED), 3);
    assert_eq!(exit_status(code::NOT_FOUND), 1);
}

#[test]
fn agents_are_found_by_id_or_unique_name() {
    let list = [agent("a-1", "api", true), agent("a-2", "Web", true), agent("a-3", "web", false)];
    assert_eq!(manage::resolve(&list, "a-2").unwrap().id, "a-2");
    assert_eq!(manage::resolve(&list, "API").unwrap().id, "a-1");
    let e = manage::resolve(&list, "web").err().unwrap();
    assert_eq!(failure(&e).0["error"]["code"], "conflict");
    assert!(failure(&e).0["error"]["message"].as_str().unwrap().contains("a-2, a-3"));
}

#[test]
fn human_snapshots_for_managing() {
    use pitwall_proto::{ChangedFile, FileStatus, ReviewChanges};
    let r = ReviewChanges {
        agent_id: "a-1".into(),
        since: "agent".into(),
        files: vec![
            ChangedFile { path: "src/lib.rs".into(), added: 12, removed: 3, binary: false, untracked: false, status: Some(FileStatus::M) },
            ChangedFile { path: "notes.md".into(), added: 4, removed: 0, binary: false, untracked: true, status: Some(FileStatus::U) },
        ],
        added: 16,
        removed: 3,
    };
    assert_eq!(manage::review("api", &r), "M  +12 -3  src/lib.rs\nU  +4 -0   notes.md\n   +16 -3  2 file(s)\n");
    assert_eq!(manage::review("api", &ReviewChanges { files: vec![], added: 0, removed: 0, ..r }), "api: no changes\n");
}
