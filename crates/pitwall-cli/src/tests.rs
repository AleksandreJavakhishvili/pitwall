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
    let server = serve(engine, approvals.clone(), Arc::new(Outside), pitwall_daemon::Config { socket: socket.clone(), version: "t".into() }).unwrap();
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
