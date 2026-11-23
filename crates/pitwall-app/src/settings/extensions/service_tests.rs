//! An extension's service on its card (docs/extensions/service.md), with
//! the real buttons: installed by its install action, confirmed, started
//! and running; stopped with its whole group when it is turned off or
//! removed, or when an update changes its commands (then asked again);
//! started again after it stops by itself, until Pitwall gives up with
//! its last lines; left alone when something else listens on its address;
//! updated when its version is too old; and its first run's link opened
//! through the test hook, kept nowhere. Only a fake shell server in a
//! temporary folder runs, on ephemeral ports.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::rc::Rc;
use std::cell::RefCell;
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use pitwall_core::paths::Paths;
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;

use super::super::view::SettingsView;
use super::accounts_tests::{click, drawn, frame};
use crate::extensions::actions;
use crate::extensions::service::{self, FirstRun, OpenLink, State};

/// A first run's one-time link: it must never be kept anywhere.
pub(super) const SECRET: &str = "ONE-TIME-7f3a9c";

/// Extension "kit": an install action and a service; `ready` its readiness
/// (`{"line": "listening"}`, or an address setting's), `tag` an argument
/// that changes its command.
pub(super) fn kit(listen: Option<&str>, tag: &str) -> String {
    let (settings, ready) = match listen {
        Some(a) => (format!(r#""settings": [{{"key": "listen", "type": "text", "label": "Address", "default": "{a}"}}],"#), r#"{"listen": "{setting:listen}"}"#.to_string()),
        None => (String::new(), r#"{"line": "listening"}"#.to_string()),
    };
    format!(
        r#"{{"v": 2, "pitwall": ">=0.8.0", "id": "kit", "name": "Kit", "version": "1.0.0", {settings}
  "actions": [{{"id": "install", "label": "Install the server", "description": "d", "steps": [{{"run": ["fake-install"]}}]}}],
  "service": {{"label": "Kit's server", "run": ["fake-srv", "--data", "{{data}}", "--tag", "{tag}"], "install": "install",
    "version": {{"run": ["fake-srv", "--version"], "match": "fake-srv (\\d+\\.\\d+\\.\\d+)", "requires": ">=0.6.0"}},
    "ready": {ready},
    "firstRun": {{"label": "Sign in", "run": ["fake-srv", "link", "--data", "{{data}}"], "opens": true, "again": "A new link replaces the last one."}}}}}}"#
    )
}

/// The fake server, kept in `bin/server` until `fake-install` puts it on
/// the PATH (`bin/path`) as `fake-srv`. Its version is `bin/version`'s
/// (else 0.6.0); with `bin/crash` it prints two lines and exits 3; else it
/// starts a child in its group, notes both pids and says "listening".
fn fakes(bin: &Path, installed: bool) {
    let path = bin.join("path");
    std::fs::create_dir_all(&path).unwrap();
    let b = bin.display();
    let server = format!(
        r#"#!/bin/sh
v=0.6.0
[ -f '{b}/version' ] && read v < '{b}/version'
case "$1" in
  --version) echo "fake-srv $v"; exit 0;;
  link) echo "Your link: https://example.test/first/{SECRET}."; exit 0;;
esac
echo "start $*" >> '{b}/started'
if [ -f '{b}/crash' ]; then echo "warming up"; echo "boom: no database" >&2; exit 3; fi
/bin/sleep 300 &
echo $! > '{b}/child'
echo $$ > '{b}/pid'
trap 'echo bye; exit 0' TERM
echo "listening"
while :; do /bin/sleep 1; done
"#
    );
    let write = |p: &Path, body: &str| {
        std::fs::write(p, body).unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    write(&bin.join("server"), &server);
    write(&path.join("fake-install"), &format!("#!/bin/sh\n/bin/cp '{b}/server' '{b}/path/fake-srv'\n/bin/rm -f '{b}/version'\necho installed\n"));
    if installed {
        write(&path.join("fake-srv"), &server);
    }
}

pub(super) fn open<'a>(cx: &'a mut TestAppContext, t: &TempDir, manifest: &str, installed: bool) -> (Entity<SettingsView>, &'a mut VisualTestContext) {
    let dir = t.path().join("pw/extensions/kit");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.json"), manifest).unwrap();
    let bin = t.path().join("bin");
    fakes(&bin, installed);
    let paths = Paths::new(t.path().join("pw"));
    let path = bin.join("path").to_string_lossy().into_owned();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        cx.set_global(actions::StepsPath(path));
        let model = crate::accounts::model(cx);
        model.update(cx, |a, _| a.set_paths(paths));
        crate::extensions::init(Some(t.path().join("pw")), cx);
        crate::extensions::catalog::set_bundle(None, cx);
        crate::ui_state::update(cx, |s| *s = s.clone().with_extension("kit", |e| e.enabled = true));
        crate::extensions::follow(cx);
        crate::settings::data::set_page(cx, Page::Extensions);
    });
    let (view, cx) = cx.add_window_view(|_, cx| SettingsView::new(true, cx));
    cx.simulate_resize(gpui::size(gpui::px(1400.), gpui::px(2400.)));
    view.update(cx, |v, cx| {
        v.exts_open("kit");
        cx.notify();
    });
    frame(cx);
    (view, cx)
}

pub(super) fn state(cx: &mut VisualTestContext) -> State {
    cx.update(|_, cx| service::try_services(cx).and_then(|s| s.read(cx).get("kit").map(|v| v.state.clone())).unwrap_or_default())
}

/// Until `want` holds of its state (real threads run meanwhile, Pitwall's
/// clock is moved on by `step` each time).
pub(super) fn wait(cx: &mut VisualTestContext, step: Duration, what: &str, want: impl Fn(&State) -> bool) -> State {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(step);
        cx.run_until_parked();
        let s = state(cx);
        if want(&s) {
            frame(cx);
            return s;
        }
        assert!(Instant::now() < deadline, "{what}: still {s:?}");
    }
}

pub(super) fn running(s: &State) -> bool {
    matches!(s, State::Running { .. })
}

/// The pid noted in `bin/<file>`, and whether it is still alive.
pub(super) fn alive(t: &TempDir, file: &str) -> bool {
    let pid = std::fs::read_to_string(t.path().join("bin").join(file)).unwrap();
    let out = std::process::Command::new("/bin/ps").args(["-o", "stat=", "-p", pid.trim()]).output().unwrap();
    let stat = String::from_utf8_lossy(&out.stdout).trim().to_string();
    !stat.is_empty() && !stat.starts_with('Z')
}

/// Until the server and its child are gone.
pub(super) fn gone(t: &TempDir, cx: &mut VisualTestContext) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(t, "pid") || alive(t, "child") {
        assert!(Instant::now() < deadline, "the service's group is still running");
        std::thread::sleep(Duration::from_millis(50));
        cx.run_until_parked();
    }
}

pub(super) fn starts(t: &TempDir) -> usize {
    std::fs::read_to_string(t.path().join("bin/started")).map(|s| s.lines().count()).unwrap_or(0)
}

/// Not installed: its install action (confirmed as any), then the card
/// to confirm its exact commands, then running; turned off, its whole
/// group stops.
#[gpui::test]
fn installed_confirmed_started_and_stopped_with_its_group(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-svc-install");
    let (view, cx) = open(cx, &t, &kit(None, "a"), false);
    assert!(drawn(cx, "extension-service-install-kit"), "Install is offered");
    click(&view, cx, "extension-service-install-kit");
    click(&view, cx, "extension-run-kit-install");
    wait(cx, Duration::from_millis(100), "installed", |_| t.path().join("bin/path/fake-srv").exists());
    let s = wait(cx, Duration::from_millis(100), "asks to confirm", |s| matches!(s, State::NeedsConfirm { .. }));
    assert_eq!(s, State::NeedsConfirm { changed: false });
    assert_eq!(starts(&t), 0, "nothing starts before it is confirmed");
    // The card lists exactly what runs.
    assert!(drawn(cx, "extension-service-confirm-kit"));
    let lines = cx.update(|_, cx| {
        let m = crate::extensions::manifest_of("kit", cx).unwrap();
        let data = service::data_dir("kit", cx).unwrap();
        pitwall_core::extensions::service::lines(&m, m.service.as_ref().unwrap(), &Default::default(), &service::shown(&data))
    });
    assert!(lines[0].1[0].starts_with("fake-srv --data ") && lines[0].1[0].ends_with("extension-data/kit --tag a"), "{lines:?}");
    click(&view, cx, "extension-service-confirm-yes-kit");
    wait(cx, Duration::from_millis(100), "running", running);
    assert_eq!(starts(&t), 1);
    assert!(drawn(cx, "extension-service-stop-kit") && drawn(cx, "extension-service-restart-kit"));
    assert!(alive(&t, "pid") && alive(&t, "child"));
    let data = t.path().join("pw/extension-data/kit");
    assert_eq!(std::fs::metadata(&data).unwrap().permissions().mode() & 0o777, 0o700, "its data folder is private");
    assert!(cx.update(|_, cx| crate::ui_state::get(cx).extension("kit").service.fresh), "a fresh data folder: its first run is offered");
    // Off: its group is stopped (asked first: it said bye).
    cx.update(|_, cx| crate::ui_state::update(cx, |s| *s = s.clone().set_extension("kit", false)));
    cx.run_until_parked();
    gone(&t, cx);
    assert_eq!(state(cx), State::Off);
}

/// An update that changes its command stops it and asks again; remove
/// stops it too, and keeps its data unless asked.
#[gpui::test]
fn changed_commands_stop_it_and_ask_again(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-svc-changed");
    let (view, cx) = open(cx, &t, &kit(None, "a"), true);
    click(&view, cx, "extension-service-confirm-yes-kit");
    wait(cx, Duration::from_millis(100), "running", running);
    std::fs::write(t.path().join("pw/extensions/kit/extension.json"), kit(None, "b")).unwrap();
    cx.update(|_, cx| crate::extensions::rescan(cx));
    let s = wait(cx, Duration::from_millis(100), "asks again", |s| matches!(s, State::NeedsConfirm { .. }));
    assert_eq!(s, State::NeedsConfirm { changed: true });
    gone(&t, cx);
    assert!(drawn(cx, "extension-service-confirm-kit"), "the card asks again");
    click(&view, cx, "extension-service-confirm-yes-kit");
    wait(cx, Duration::from_millis(100), "running again", running);
    assert!(std::fs::read_to_string(t.path().join("bin/started")).unwrap().lines().last().unwrap().ends_with("--tag b"));
    // Remove (its data kept: the default).
    click(&view, cx, "extension-remove-kit");
    assert!(drawn(cx, "extension-remove-data-kit"), "the remove card offers its data");
    click(&view, cx, "extension-ask-yes-kit");
    gone(&t, cx);
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }
    assert!(!t.path().join("pw/extensions/kit").exists(), "removed");
    assert!(t.path().join("pw/extension-data/kit").exists(), "its data is kept");
}

/// One that stops by itself is started again after 1, 5 and 30 seconds;
/// a fourth stop gives up, with its last lines and Start.
#[gpui::test]
fn a_crashing_service_is_retried_then_given_up(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-svc-crash");
    std::fs::create_dir_all(t.path().join("bin")).unwrap();
    std::fs::write(t.path().join("bin/crash"), "").unwrap();
    let (view, cx) = open(cx, &t, &kit(None, "a"), true);
    click(&view, cx, "extension-service-confirm-yes-kit");
    let s = wait(cx, Duration::from_secs(1), "gave up", |s| matches!(s, State::Stopped(_)));
    assert_eq!(starts(&t), 4, "started, then again three times");
    let State::Stopped(why) = s else { unreachable!() };
    assert!(why.contains("exited with code 3") && why.contains("boom: no database") && why.contains("warming up"), "{why}");
    assert!(drawn(cx, "extension-service-why-kit") && drawn(cx, "extension-service-start-kit"));
    // Start, once fixed.
    std::fs::remove_file(t.path().join("bin/crash")).unwrap();
    click(&view, cx, "extension-service-start-kit");
    wait(cx, Duration::from_millis(100), "running", running);
    cx.update(|_, cx| crate::ui_state::update(cx, |s| *s = s.clone().set_extension("kit", false)));
    cx.run_until_parked();
    gone(&t, cx);
}

/// Something else listening on its address: used, never started, never
/// stopped.
#[gpui::test]
fn a_taken_address_is_running_elsewhere(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-svc-elsewhere");
    let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = other.local_addr().unwrap().to_string();
    let (view, cx) = open(cx, &t, &kit(Some(&addr), "a"), true);
    click(&view, cx, "extension-service-confirm-yes-kit");
    let s = wait(cx, Duration::from_millis(100), "elsewhere", |s| matches!(s, State::Elsewhere(_)));
    assert_eq!(s, State::Elsewhere(addr.clone()));
    assert_eq!(starts(&t), 0, "nothing is started");
    assert!(drawn(cx, "extension-service-why-kit"));
    // Off: the other one is left as it is.
    cx.update(|_, cx| crate::ui_state::update(cx, |s| *s = s.clone().set_extension("kit", false)));
    cx.run_until_parked();
    let _ = other.set_nonblocking(true);
    assert!(std::net::TcpStream::connect(&addr).is_ok(), "the other one still listens");
}

/// Its version below the one it needs: said, not started, and its install
/// action offered as Update; once it ran, it starts. Quitting stops it.
#[gpui::test]
fn a_version_too_old_is_updated(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-svc-old");
    std::fs::create_dir_all(t.path().join("bin")).unwrap();
    std::fs::write(t.path().join("bin/version"), "0.5.0\n").unwrap();
    let (view, cx) = open(cx, &t, &kit(None, "a"), true);
    click(&view, cx, "extension-service-confirm-yes-kit");
    let s = wait(cx, Duration::from_millis(100), "too old", |s| matches!(s, State::TooOld { .. }));
    assert_eq!(s, State::TooOld { found: "0.5.0".into(), need: "0.6.0".into() });
    assert_eq!(starts(&t), 0);
    assert!(drawn(cx, "extension-service-install-text-kit"));
    click(&view, cx, "extension-service-install-kit");
    click(&view, cx, "extension-run-kit-install");
    wait(cx, Duration::from_millis(100), "updated and running", running);
    assert_eq!(cx.update(|_, cx| service::try_services(cx).unwrap().read(cx).get("kit").unwrap().version.clone()), Some("0.6.0".into()));
    // Pitwall quits: its group is stopped before it returns.
    cx.update(|_, cx| crate::extensions::quit(cx));
    assert!(!alive(&t, "pid") && !alive(&t, "child"), "stopped at quit");
}

/// The first run opens the link it printed through the hook; the link is
/// in no log, no activity, no file and not on the card.
#[gpui::test]
fn the_first_run_opens_its_link_and_keeps_it_nowhere(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-svc-first");
    let opened: Rc<RefCell<Vec<String>>> = Rc::default();
    let (view, cx) = open(cx, &t, &kit(None, "a"), true);
    let o = opened.clone();
    cx.update(|_, cx| cx.set_global(OpenLink(Rc::new(move |url: &str, _| o.borrow_mut().push(url.to_string())))));
    click(&view, cx, "extension-service-confirm-yes-kit");
    wait(cx, Duration::from_millis(100), "running", running);
    click(&view, cx, "extension-service-first-kit");
    let deadline = Instant::now() + Duration::from_secs(10);
    while cx.update(|_, cx| service::try_services(cx).unwrap().read(cx).get("kit").unwrap().first_run.clone()) == Some(FirstRun::Running) {
        assert!(Instant::now() < deadline, "the first run never ended");
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }
    frame(cx);
    assert_eq!(*opened.borrow(), [format!("https://example.test/first/{SECRET}")], "the link, without the full stop");
    let done = cx.update(|_, cx| service::try_services(cx).unwrap().read(cx).get("kit").unwrap().first_run.clone());
    assert_eq!(done, Some(FirstRun::Done("Opened a one-time sign-in link in your browser.".into())));
    assert!(!cx.update(|_, cx| crate::ui_state::get(cx).extension("kit").service.fresh), "not fresh any more");
    // Kept nowhere: its log, its activity, any file Pitwall wrote.
    let log = cx.update(|_, cx| service::try_services(cx).unwrap().read(cx).log("kit"));
    assert!(log.iter().all(|l| !l.text.contains(SECRET) && !l.text.contains("example.test")), "{log:?}");
    let activity = cx.update(|_, cx| crate::extensions::calls::activity("kit", cx).lock().unwrap().entries());
    assert!(!format!("{activity:?}").contains(SECRET) && format!("{activity:?}").contains("first run"));
    let ui = cx.update(|_, cx| serde_json::to_string(&crate::ui_state::get(cx)).unwrap());
    assert!(!ui.contains(SECRET));
    fn walk(dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if let Ok(text) = std::fs::read_to_string(&p) {
                if text.contains(SECRET) {
                    out.push(p.display().to_string());
                }
            }
        }
    }
    let mut found = Vec::new();
    walk(&t.path().join("pw"), &mut found);
    assert!(found.is_empty(), "the link was written to {found:?}");
    assert!(drawn(cx, "extension-service-first-text-kit"));
    // Under More now, asked before it runs again.
    click(&view, cx, "extension-service-again-kit");
    assert!(drawn(cx, "extension-service-again-yes-kit"));
    click(&view, cx, "extension-service-again-cancel-kit");
    cx.update(|_, cx| crate::ui_state::update(cx, |s| *s = s.clone().set_extension("kit", false)));
    cx.run_until_parked();
    gone(&t, cx);
}
