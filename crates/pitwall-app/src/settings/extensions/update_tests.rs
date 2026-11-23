//! Updating an extension as one ordered step (docs/extensions/updates.md),
//! with the real buttons: Review… lists the steps with their commands and
//! what becomes of the running agents; Update stops the service, backs up
//! its data with its own backup command, installs the new files, starts
//! the new service, checks it, then runs the changed setup for every
//! account; a service that doesn't come up puts the files, the data and
//! the previous service back; a setup that fails for one account leaves
//! the update with Retry; Go back offers the data backup. Agents started
//! before the update use the previous setup (`staleFor`) and get the
//! chip; what Pitwall does with them is the restart setting. Unix only:
//! the fake server is a shell script run in its own process group. Only
//! fake programs in a temporary folder run; agents are the engine's fake
//! provider's.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use pitwall_core::accounts::Account;
use pitwall_core::engine::lifecycle;
use pitwall_core::extensions::update::Restart;
use pitwall_core::model::CreateAgentRequest;
use pitwall_core::paths::Paths;
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;

use super::super::view::SettingsView;
use super::accounts_tests::{click, drawn, frame};
use crate::extensions::safe_update::{self, Ending, StepState};
use crate::extensions::service::{self, State};
use crate::extensions::{actions, stale};

/// Extension "kit" at `version`: a setup action for Claude Code that adds
/// plugin `pin`, a service started with `--tag <tag>` that backs up its
/// data with its own command, and `update.agents` `agents`.
fn kit(version: &str, tag: &str, pin: &str, agents: &str) -> String {
    format!(
        r#"{{"v": 2, "pitwall": ">=0.8.0", "id": "kit", "name": "Kit", "version": "{version}",
  "actions": [{{"id": "setup", "setup": true, "label": "Set up Kit", "description": "d", "steps": [{{"for": "claude", "run": ["fake-add", "{pin}"]}}]}}],
  "service": {{"label": "Kit's server", "run": ["fake-srv", "--data", "{{data}}", "--tag", "{tag}"], "ready": {{"line": "listening"}},
    "backup": {{"run": ["fake-srv", "backup", "--data", "{{data}}", "{{backup}}/db"], "replaces": ["db"]}}}},
  "update": {{"agents": "{agents}"}}}}"#
    )
}

/// The fakes in `bin/path`, each noting what it did in `bin/log`:
/// `fake-srv` (`start <tag>`, adding its tag to `<data>/db`, then
/// "listening" until asked to stop: `stop <tag>`; with `bin/crash-<tag>`
/// it exits at once; `backup` copies `db`), `fake-add` (`add <pin>
/// <account folder>`, failing for a folder with `bin/fail-<folder>`).
fn fakes(bin: &Path) {
    let path = bin.join("path");
    std::fs::create_dir_all(&path).unwrap();
    let b = bin.display();
    let server = format!(
        r#"#!/bin/sh
b='{b}'
case "$1" in
  --version) echo "fake-srv 1.0.0"; exit 0;;
  backup) echo backup >> "$b/log"; /bin/cp "$3/db" "$4"; exit 0;;
esac
tag="$4"
echo "start $tag" >> "$b/log"
echo "$tag" >> "$2/db"
if [ -f "$b/crash-$tag" ]; then echo "boom: can't read the database" >&2; exit 3; fi
trap 'echo "stop $tag" >> "$b/log"; exit 0' TERM
echo "listening"
while :; do /bin/sleep 1; done
"#
    );
    let add = format!("#!/bin/sh\nd=$(basename \"${{CLAUDE_CONFIG_DIR:-default}}\")\nif [ -f '{b}/fail-'\"$d\" ]; then echo 'no luck' >&2; exit 3; fi\necho \"add $1 $d\" >> '{b}/log'\n");
    for (name, body) in [("fake-srv", server), ("fake-add", add)] {
        let p = path.join(name);
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn log(t: &TempDir) -> Vec<String> {
    std::fs::read_to_string(t.path().join("bin/log")).unwrap_or_default().lines().map(str::to_string).collect()
}

fn ship(t: &TempDir, manifest: &str) {
    let d = t.path().join("bundle/kit");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("extension.json"), manifest).unwrap();
}

fn account(t: &TempDir, name: &str) -> Account {
    let id = format!("claude-{}", name.to_lowercase());
    let dir = t.path().join("accounts").join(&id);
    std::fs::create_dir_all(&dir).unwrap();
    Account { id, kind: "claude".into(), name: name.into(), dir: dir.to_string_lossy().into_owned(), ..Default::default() }
}

/// An engine with the fake provider, on Pitwall's data folder `root`.
fn engine(root: &Path) -> pitwall_core::engine::Shared {
    use pitwall_core::testing::{FakeProvider, ManualClock, MemStore, RecordingSink};
    pitwall_core::Engine::open(pitwall_core::Deps {
        paths: Paths::new(root.to_path_buf()),
        events: RecordingSink::new(),
        clock: ManualClock::new(1),
        store: MemStore::with(vec![]),
        providers: vec![FakeProvider::named("local", "this-mac", Arc::new(pitwall_core::exec::LocalExec))],
    })
}

/// Kit `v1` installed from Pitwall's copy, on, its service confirmed and
/// running, its setup done (with `accounts`), and Settings → Extensions
/// open on it.
fn open<'a>(cx: &'a mut TestAppContext, t: &TempDir, v1: &str, accounts: &[&str]) -> (Entity<SettingsView>, &'a mut VisualTestContext, pitwall_core::engine::Shared) {
    ship(t, v1);
    let bin = t.path().join("bin");
    fakes(&bin);
    let root = t.path().join("pw");
    let paths = Paths::new(root.clone());
    let list: Vec<Account> = accounts.iter().map(|n| account(t, n)).collect();
    let e = engine(&root);
    let path = bin.join("path").to_string_lossy().into_owned();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        cx.set_global(actions::StepsPath(path));
        cx.set_global(safe_update::Timeouts { ready: Duration::from_secs(4), connect: Duration::from_secs(4) });
        let model = crate::accounts::model(cx);
        model.update(cx, |a, _| a.set_paths(paths));
        model.update(cx, |a, cx| a.change(|l| { l.accounts.extend(list); Ok(()) }, cx)).unwrap();
        crate::extensions::init(Some(root.clone()), cx);
        crate::extensions::set_engine(e.clone(), cx);
        crate::extensions::catalog::set_bundle(Some(t.path().join("bundle")), cx);
        crate::extensions::catalog::install_one("kit", cx, |_, _| {});
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let m = crate::extensions::manifest_of("kit", cx).expect("installed");
        let digest = pitwall_core::extensions::steps_digest(&m.actions[0]);
        crate::ui_state::update(cx, |s| {
            *s = s.clone().with_extension("kit", |e| {
                e.enabled = true;
                e.actions_done.push("setup".into());
                e.action_steps.insert("setup".into(), digest);
            })
        });
        service::confirm(&m, cx);
        crate::extensions::follow(cx);
        crate::settings::data::set_page(cx, Page::Extensions);
    });
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(true, cx));
    cx.simulate_resize(gpui::size(gpui::px(1400.), gpui::px(3200.)));
    view.update(cx, |v, cx| {
        v.exts_open("kit");
        cx.notify();
    });
    wait(cx, "running", running);
    (view, cx, e)
}

fn state(cx: &mut VisualTestContext) -> State {
    cx.update(|_, cx| service::try_services(cx).and_then(|s| s.read(cx).get("kit").map(|v| v.state.clone())).unwrap_or_default())
}

fn running(cx: &mut VisualTestContext) -> bool {
    matches!(state(cx), State::Running { .. })
}

/// Until `want` (real programs run meanwhile; Pitwall's clock moves on).
fn wait(cx: &mut VisualTestContext, what: &str, want: impl Fn(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        if want(cx) {
            frame(cx);
            return;
        }
        assert!(Instant::now() < deadline, "{what}: still not");
    }
}

fn run(cx: &mut VisualTestContext) -> Option<safe_update::Run> {
    cx.update(|_, cx| safe_update::try_model(cx).and_then(|m| m.read(cx).get("kit").cloned()))
}

fn ended(cx: &mut VisualTestContext) -> Ending {
    wait(cx, "the update ended", |cx| run(cx).is_some_and(|r| r.over()));
    run(cx).unwrap().ending.unwrap()
}

/// Review… then Update.
fn update(view: &Entity<SettingsView>, cx: &mut VisualTestContext) {
    click(view, cx, "extension-update-kit");
    wait(cx, "reviewed", |cx| drawn(cx, "extension-review-update-kit"));
    click(view, cx, "extension-review-update-kit");
}

fn version(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| crate::extensions::manifest_of("kit", cx).and_then(|m| m.version))
}

fn new_agent(e: &pitwall_core::engine::Shared, project: &Path) -> String {
    let req = CreateAgentRequest { kind: "claude".into(), project_path: project.to_string_lossy().into_owned(), ..Default::default() };
    lifecycle::create(e, req).expect("started").id
}

fn stale_for(e: &pitwall_core::engine::Shared, id: &str) -> Vec<String> {
    e.views().into_iter().find(|v| v.id == id).map(|v| v.stale_for).unwrap_or_default()
}

/// The review lists every step in order with its commands; Update stops
/// the service and backs up its data before the new one starts, then
/// runs the changed setup last; agents started before use the previous
/// setup (with the chip), one started after doesn't.
#[gpui::test]
fn an_update_runs_its_steps_in_order_and_backs_up_first(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-safe-update");
    let (view, cx, e) = open(cx, &t, &kit("1.0.0", "a", "v1", "compatible"), &["Work"]);
    let project = t.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let before = new_agent(&e, &project);
    assert!(stale_for(&e, &before).is_empty());
    assert_eq!(e.extension_users("kit"), std::slice::from_ref(&before), "it started with Kit on");
    ship(&t, &kit("1.1.0", "b", "v2", "compatible"));
    cx.update(|_, cx| crate::extensions::rescan(cx));
    click(&view, cx, "extension-update-kit");
    wait(cx, "reviewed", |cx| drawn(cx, "extension-update-plan-kit"));
    let review = cx.update(|_, cx| {
        let new = pitwall_core::extensions::parse(&kit("1.1.0", "b", "v2", "compatible")).unwrap();
        safe_update::review("kit", &new, cx).unwrap()
    });
    let titles: Vec<&str> = review.steps.iter().map(|s| s.title.as_str()).collect();
    assert!(titles[0].starts_with("Stops Kit's server and backs up its data first"), "{titles:?}");
    assert!(review.steps[0].lines[0].starts_with("fake-srv backup --data ") && review.steps[0].lines[0].contains("extension-backups/kit/<time of the update>/db"), "{:?}", review.steps[0].lines);
    assert!(titles[1].starts_with("Installs Kit 1.1.0 (the installed 1.0.0 is kept for Go back)"));
    assert!(titles[2].starts_with("Starts Kit's server"));
    assert!(titles[3].starts_with("Checks that Kit's server is up"));
    assert!(titles[4].starts_with("Runs “Set up Kit” again for every account"), "{titles:?}");
    assert!(review.steps[4].lines.iter().any(|l| l.contains("account Work") && l.ends_with("fake-add v2")), "{:?}", review.steps[4].lines);
    assert_eq!(review.agents[0], "1 running agent keeps the previous setup until restarted.");
    assert!(review.agents[1].starts_with("After the update: Pitwall restarts nothing by itself"));

    let started = log(&t).len();
    click(&view, cx, "extension-review-update-kit");
    assert_eq!(ended(cx), Ending::Done);
    assert_eq!(version(cx).as_deref(), Some("1.1.0"));
    let after: Vec<String> = log(&t)[started..].to_vec();
    let at = |what: &str| after.iter().position(|l| l == what).unwrap_or_else(|| panic!("no {what} in {after:?}"));
    assert!(at("stop a") < at("backup") && at("backup") < at("start b"), "stopped and backed up before the new one: {after:?}");
    assert!(at("start b") < at("add v2 claude-work"), "the setup runs last: {after:?}");
    assert!(running(cx));
    // The backup: private, named by time, with what it stands for.
    let backups = t.path().join("pw/extension-backups/kit");
    let list = pitwall_core::extensions::update::backups(&backups);
    assert_eq!(list.len(), 1);
    assert_eq!(std::fs::metadata(&list[0]).unwrap().permissions().mode() & 0o777, 0o700);
    assert_eq!(std::fs::read_to_string(list[0].join("db")).unwrap(), "a\n", "the data as it was before");
    let note = pitwall_core::extensions::update::Note::read(&list[0]).unwrap();
    assert!(!note.copy && note.replaces == ["db"] && note.to.as_deref() == Some("1.1.0"));
    assert!(drawn(cx, "extension-update-run-kit"));
    // Every step is ticked.
    let r = run(cx).unwrap();
    assert!(r.steps.iter().all(|s| matches!(s.state, StepState::Done(_))), "{:?}", r.steps);

    // Agents: the one from before uses the previous setup, with the chip.
    cx.run_until_parked();
    assert_eq!(stale_for(&e, &before), ["kit"]);
    let view_of = e.views().into_iter().find(|v| v.id == before).unwrap();
    let chip = cx.update(|_, cx| stale::chip(&view_of, cx)).expect("a chip");
    assert_eq!(chip, ("old setup".to_string(), "Started before Kit was updated: it keeps the old version until restarted".to_string(), false));
    let after_update = new_agent(&e, &project);
    assert!(stale_for(&e, &after_update).is_empty(), "started after: the new setup");
    // Its card lists it, with Restart now and Restart all idle.
    cx.update(|_, cx| {
        let s = cx.new(|_| crate::agents::AgentStore::new(e.views()));
        cx.set_global(crate::agents::StoreHandle(s));
    });
    view.update(cx, |_, cx| cx.notify());
    assert!(drawn(cx, "extension-stale-kit") && drawn(cx, &format!("extension-stale-restart-kit-{before}")) && drawn(cx, "extension-stale-idle-kit"));
    cx.update(|_, cx| crate::extensions::quit(cx));
}

/// A new service that doesn't come up puts everything back by itself:
/// the previous files, its data from the backup, and the previous
/// service running again; the card says what failed.
#[gpui::test]
fn a_service_that_fails_to_come_up_is_rolled_back(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-safe-rollback");
    let (view, cx, _e) = open(cx, &t, &kit("1.0.0", "a", "v1", "compatible"), &[]);
    let db = t.path().join("pw/extension-data/kit/db");
    let data_before = std::fs::read_to_string(&db).unwrap();
    std::fs::write(t.path().join("bin/crash-b"), "").unwrap();
    ship(&t, &kit("1.1.0", "b", "v2", "compatible"));
    cx.update(|_, cx| crate::extensions::rescan(cx));
    update(&view, cx);
    let Ending::RolledBack { why, notes } = ended(cx) else { panic!("not rolled back: {:?}", run(cx)) };
    assert!(why.starts_with("Kit's server didn't come up"), "{why}");
    assert!(notes.iter().any(|n| n == "Kit 1.0.0 is back."), "{notes:?}");
    assert!(notes.iter().any(|n| n.starts_with("Its data is as it was before the update")), "{notes:?}");
    assert!(notes.iter().any(|n| n == "Kit's server runs again."), "{notes:?}");
    assert_eq!(version(cx).as_deref(), Some("1.0.0"), "the previous files");
    // The new one wrote to its data; the backup was put back (then the
    // old one started and added its line).
    assert_eq!(std::fs::read_to_string(&db).unwrap(), format!("{data_before}a\n"));
    assert!(running(cx));
    assert_eq!(log(&t).last().map(String::as_str), Some("start a"), "the old service runs again");
    assert!(cx.update(|_, cx| crate::ui_state::get(cx).extension("kit").last_update.is_none()));
    assert!(drawn(cx, "extension-update-why-kit") && drawn(cx, "extension-update-dismiss-kit"));
    // Its commands are the ones confirmed before: it runs them.
    assert_eq!(cx.update(|_, cx| service::confirmed(&crate::extensions::manifest_of("kit", cx).unwrap(), cx)), Ok(()));
    cx.update(|_, cx| crate::extensions::quit(cx));
}

/// A setup that fails for one account doesn't undo the update: the card
/// says which, with Retry; Retry runs it again.
#[gpui::test]
fn a_setup_failure_keeps_the_update_and_offers_retry(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-safe-setup");
    let (view, cx, _e) = open(cx, &t, &kit("1.0.0", "a", "v1", "compatible"), &["Work", "Home"]);
    std::fs::write(t.path().join("bin/fail-claude-home"), "").unwrap();
    ship(&t, &kit("1.1.0", "b", "v2", "compatible"));
    cx.update(|_, cx| crate::extensions::rescan(cx));
    update(&view, cx);
    let Ending::SetupFailed(list) = ended(cx) else { panic!("{:?}", run(cx)) };
    assert!(list[0].contains("Claude Code, account Home") && !list[0].contains("account Work"), "{list:?}");
    assert_eq!(version(cx).as_deref(), Some("1.1.0"), "the update stays");
    assert!(running(cx));
    assert!(log(&t).contains(&"add v2 claude-work".to_string()));
    assert!(drawn(cx, "extension-update-retry-kit"));
    std::fs::remove_file(t.path().join("bin/fail-claude-home")).unwrap();
    click(&view, cx, "extension-update-retry-kit");
    assert_eq!(ended(cx), Ending::Done);
    assert!(log(&t).contains(&"add v2 claude-home".to_string()));
    cx.update(|_, cx| crate::extensions::quit(cx));
}

/// With `update.agents: "restart"`, the review warns that running agents
/// stop working until restarted, and their chip is a warning.
#[gpui::test]
fn agents_that_must_restart_change_the_review(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-safe-restart");
    let (view, cx, e) = open(cx, &t, &kit("1.0.0", "a", "v1", "compatible"), &[]);
    let project = t.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let before = new_agent(&e, &project);
    ship(&t, &kit("1.1.0", "b", "v2", "restart"));
    cx.update(|_, cx| crate::extensions::rescan(cx));
    click(&view, cx, "extension-update-kit");
    wait(cx, "reviewed", |cx| drawn(cx, "extension-update-agents-kit"));
    let review = cx.update(|_, cx| safe_update::review("kit", &pitwall_core::extensions::parse(&kit("1.1.0", "b", "v2", "restart")).unwrap(), cx).unwrap());
    assert_eq!(review.agents[0], "Risk: 1 running agent will stop working with Kit until restarted.");
    click(&view, cx, "extension-review-update-kit");
    assert_eq!(ended(cx), Ending::Done);
    cx.run_until_parked();
    let v = e.views().into_iter().find(|v| v.id == before).unwrap();
    let (_, tip, warn) = cx.update(|_, cx| stale::chip(&v, cx)).unwrap();
    assert!(warn && tip == "Started before Kit was updated: it doesn't work with Kit until restarted", "{tip}");
    cx.update(|_, cx| crate::extensions::quit(cx));
}

/// Go back after an update that backed up its data offers to put the
/// backup back too (the default keeps the data); putting it back stops
/// the service for it and starts it again.
#[gpui::test]
fn go_back_offers_the_data_backup(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-safe-back");
    let (view, cx, _e) = open(cx, &t, &kit("1.0.0", "a", "v1", "compatible"), &[]);
    let db = t.path().join("pw/extension-data/kit/db");
    ship(&t, &kit("1.1.0", "b", "v2", "compatible"));
    cx.update(|_, cx| crate::extensions::rescan(cx));
    update(&view, cx);
    assert_eq!(ended(cx), Ending::Done);
    assert_eq!(std::fs::read_to_string(&db).unwrap(), "a\nb\n", "the new one added its line");
    click(&view, cx, "extension-update-dismiss-kit");
    click(&view, cx, "extension-back-kit");
    assert!(drawn(cx, "extension-back-data-kit"), "asked whether to put the data back");
    assert!(drawn(cx, "extension-back-keep-kit") && drawn(cx, "extension-back-restore-kit"));
    click(&view, cx, "extension-back-restore-kit");
    wait(cx, "gone back", |cx| version(cx).as_deref() == Some("1.0.0") && running(cx));
    // The backup's data, then the old service's line.
    assert_eq!(std::fs::read_to_string(&db).unwrap(), "a\na\n");
    assert!(cx.update(|_, cx| crate::ui_state::get(cx).extension("kit").last_update.is_none()));
    cx.update(|_, cx| crate::extensions::quit(cx));
}

/// Made-up running agents of extension "kit" for the restart setting:
/// `(id, status, can resume)`.
fn stale_store(cx: &mut TestAppContext, list: &[(&str, &str, bool)]) -> std::rc::Rc<std::cell::RefCell<Vec<String>>> {
    let restarted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let views: Vec<pitwall_proto::AgentView> = list
        .iter()
        .map(|(id, status, resume)| {
            let mut a = crate::agents::tests::agent(id, "/p", status, 1, true);
            a.running = true;
            a.caps.restart = true;
            a.caps.resume = *resume;
            a.stale_for = vec!["kit".into()];
            a
        })
        .collect();
    let seen = restarted.clone();
    cx.update(|cx| {
        crate::ui_state::init(None, cx);
        let s = cx.new(|_| crate::agents::AgentStore::new(views));
        cx.set_global(crate::agents::StoreHandle(s));
        cx.set_global(stale::RestartHook(std::rc::Rc::new(move |id: &str, _: &mut gpui::App| seen.borrow_mut().push(id.to_string()))));
    });
    restarted
}

/// Pitwall's clock moved on by `secs`, looking each [`stale::TICK`].
fn idle_for(cx: &mut TestAppContext, secs: u64) {
    for _ in 0..secs / stale::TICK.as_secs() {
        cx.executor().advance_clock(stale::TICK);
        cx.update(stale::tick);
        cx.run_until_parked();
    }
}

/// `ask` and `never` restart nothing; `whenIdle` restarts only an idle
/// agent that can continue its conversation, after it has waited for
/// [`stale::DWELL`], one at a time; never one that works or waits on an
/// approval, nor one that can't continue.
#[gpui::test]
fn the_restart_setting_decides_what_restarts(cx: &mut TestAppContext) {
    let restarted = stale_store(cx, &[("idle-1", "idle", true), ("busy", "working", true), ("asks", "blocked", true), ("fresh", "idle", false), ("idle-2", "done", true)]);
    assert_eq!(cx.update(|cx| stale::global_mode(cx)), Restart::Ask, "the default");
    idle_for(cx, 60);
    assert!(restarted.borrow().is_empty(), "ask: nothing by itself");
    cx.update(|cx| stale::set_mode("kit", Some(Restart::Never), cx));
    idle_for(cx, 60);
    assert!(restarted.borrow().is_empty(), "never: nothing");
    cx.update(|cx| stale::set_mode("kit", None, cx));
    cx.update(|cx| stale::set_global_mode(Restart::WhenIdle, cx));
    assert_eq!(cx.update(|cx| stale::mode("kit", cx)), Restart::WhenIdle);
    idle_for(cx, 20);
    assert!(restarted.borrow().is_empty(), "not before it has waited 30 s");
    idle_for(cx, 12);
    assert_eq!(*restarted.borrow(), ["idle-1"], "one at a time");
    // Restarted: the engine says each isn't stale any more.
    let restarted_now = |cx: &mut TestAppContext, id: &str| {
        let id = id.to_string();
        cx.update(|cx| {
            let s = cx.global::<crate::agents::StoreHandle>().0.clone();
            s.update(cx, |s, _| s.agents.retain(|a| a.id != id));
        });
    };
    restarted_now(cx, "idle-1");
    idle_for(cx, 4);
    assert_eq!(*restarted.borrow(), ["idle-1", "idle-2"]);
    restarted_now(cx, "idle-2");
    idle_for(cx, 120);
    assert_eq!(*restarted.borrow(), ["idle-1", "idle-2"], "never one that works, waits on an approval or can't continue");
    // Restart all idle on the card: each waiting now, whatever the setting.
    cx.update(|cx| stale::set_global_mode(Restart::Ask, cx));
    let n = cx.update(|cx| stale::restart_idle("kit", cx));
    cx.run_until_parked();
    assert_eq!((n, restarted.borrow().last().cloned()), (1, Some("fresh".to_string())), "only the idle one (it starts a new conversation)");
}
