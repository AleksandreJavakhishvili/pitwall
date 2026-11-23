//! A setup action and the tools' accounts (docs/spec/extensions.md,
//! "Setup actions"): its steps for a tool run for the tool's default
//! folder and for each of Pitwall's accounts of it, each with that
//! account's environment; the step is ticked only once every account is
//! covered; an account added later gets the confirmed steps by itself.
//! Only fake programs in a temporary folder run.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use pitwall_core::accounts::Account;
use pitwall_core::paths::Paths;
use pitwall_core::testing::TempDir;
use pitwall_proto::settings::Page;

use super::super::view::SettingsView;
use crate::extensions::{actions, connections};

const KIT: &str = r#"{"id": "kit", "name": "Kit", "actions": [
    {"id": "setup", "setup": true, "label": "Set up", "description": "d", "steps": [
        {"for": "claude", "run": ["fake-rm", "x"], "optional": true},
        {"for": "claude", "run": ["fake-add", "x"]}
    ]},
    {"id": "remove", "label": "Remove", "description": "d", "steps": [{"for": "claude", "run": ["fake-rm", "x"]}]}
]}"#;

pub(super) fn frame(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// Click control `id`, the page scrolled first so it is in view (the
/// dialog's page area is shorter than the window).
pub(super) fn click(view: &Entity<SettingsView>, cx: &mut VisualTestContext, id: &str) {
    frame(cx);
    let sel: &'static str = Box::leak(id.to_string().into_boxed_str());
    let b = cx.debug_bounds(sel).unwrap_or_else(|| panic!("{id} isn't drawn"));
    let (top, bottom) = (gpui::px(120.), gpui::px(super::super::view::HEIGHT - 40.));
    let dy = if b.bottom() > bottom { b.bottom() - bottom } else if b.origin.y < top { b.origin.y - top } else { gpui::px(0.) };
    if dy != gpui::px(0.) {
        view.update(cx, |v, cx| {
            v.scroll_by(dy);
            cx.notify();
        });
        frame(cx);
    }
    let b = cx.debug_bounds(sel).unwrap_or_else(|| panic!("{id} isn't drawn"));
    assert!(b.origin.y >= top && b.bottom() <= bottom, "{id} out of view: {b:?}");
    cx.simulate_mouse_move(b.center(), None, Modifiers::none());
    cx.run_until_parked();
    cx.simulate_click(b.center(), Modifiers::none());
    cx.run_until_parked();
    frame(cx);
}

pub(super) fn drawn(cx: &mut VisualTestContext, id: &str) -> bool {
    frame(cx);
    cx.debug_bounds(Box::leak(id.to_string().into_boxed_str())).is_some()
}

/// The fake tools in `bin`: each writes what it ran, and for which
/// config folder, to `bin/log`; `fake-add` fails for a folder named in a
/// `fail-<folder>` file.
fn fakes(bin: &Path) {
    std::fs::create_dir_all(bin).unwrap();
    let log = bin.join("log");
    for (name, body) in [
        ("fake-rm", format!("#!/bin/sh\necho \"rm ${{CLAUDE_CONFIG_DIR:-default}}\" >> '{}'\nexit 1\n", log.display())),
        ("fake-add", format!("#!/bin/sh\nd=$(basename \"${{CLAUDE_CONFIG_DIR:-default}}\")\nif [ -f '{}/fail-'\"$d\" ]; then echo 'no luck' >&2; exit 3; fi\necho \"add ${{CLAUDE_CONFIG_DIR:-default}}\" >> '{}'\n", bin.display(), log.display())),
    ] {
        let p = bin.join(name);
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// What the fakes ran for accounts (their folders under `t`), in order:
/// `add <account folder name>`; runs for the default folder left out
/// (their folder is the test's own environment's).
fn ran(t: &TempDir) -> Vec<String> {
    let text = std::fs::read_to_string(t.path().join("bin/log")).unwrap_or_default();
    let accounts = t.path().join("accounts");
    text.lines()
        .filter_map(|l| {
            let (what, dir) = l.split_once(' ')?;
            let dir = Path::new(dir);
            let name = dir.file_name()?.to_string_lossy();
            (dir.parent() == Some(accounts.as_path())).then(|| format!("{what} {name}"))
        })
        .collect()
}

/// Runs for the default folder in the log.
fn default_runs(t: &TempDir) -> usize {
    let text = std::fs::read_to_string(t.path().join("bin/log")).unwrap_or_default();
    let accounts = t.path().join("accounts").to_string_lossy().into_owned();
    text.lines().filter(|l| !l.contains(&accounts)).count()
}

fn account(t: &TempDir, name: &str) -> Account {
    let id = format!("claude-{}", name.to_lowercase());
    let dir: PathBuf = t.path().join("accounts").join(&id);
    std::fs::create_dir_all(&dir).unwrap();
    Account { id, kind: "claude".into(), name: name.into(), dir: dir.to_string_lossy().into_owned(), ..Default::default() }
}

fn add_account(cx: &mut VisualTestContext, a: Account) {
    cx.update(|_, cx| {
        let m = crate::accounts::model(cx);
        m.update(cx, |m, cx| m.change(|l| { l.accounts.push(a); Ok(()) }, cx)).unwrap();
    });
    cx.run_until_parked();
}

/// Settings → Extensions with extension "kit" (`manifest`) installed and
/// on, its programs the fakes, with `accounts`; `follow`: Pitwall follows
/// the accounts and `ui.json` as at start (automatic setup with them).
fn open<'a>(cx: &'a mut TestAppContext, t: &TempDir, manifest: &str, accounts: &[&str], follow: bool) -> (Entity<SettingsView>, &'a mut VisualTestContext) {
    let dir = t.path().join("pw/extensions/kit");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.json"), manifest).unwrap();
    fakes(&t.path().join("bin"));
    let paths = Paths::new(t.path().join("pw"));
    let list: Vec<Account> = accounts.iter().map(|n| account(t, n)).collect();
    let bin = t.path().join("bin").to_string_lossy().into_owned();
    cx.update(|cx| {
        cx.set_global(crate::theme::Theme::dark());
        crate::kit::init(cx);
        crate::ui_state::init(None, cx);
        cx.set_global(actions::StepsPath(bin));
        let model = crate::accounts::model(cx);
        model.update(cx, |a, _| a.set_paths(paths));
        model.update(cx, |a, cx| a.change(|l| { l.accounts.extend(list); Ok(()) }, cx)).unwrap();
        crate::extensions::init(Some(t.path().join("pw")), cx);
        crate::extensions::catalog::set_bundle(None, cx);
        crate::ui_state::update(cx, |s| *s = s.clone().with_extension("kit", |e| e.enabled = true));
        if follow {
            crate::extensions::follow(cx);
        }
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

/// Until `kit`'s `setup` has run (it started before).
fn wait_run(cx: &mut VisualTestContext) -> actions::RunState {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        if let Some(st) = cx.update(|_, cx| actions::runs(cx).read(cx).get("kit", "setup")).filter(|s| s.over) {
            // Its end recorded.
            cx.executor().advance_clock(Duration::from_millis(200));
            cx.run_until_parked();
            frame(cx);
            return st;
        }
        assert!(Instant::now() < deadline, "the run never ended");
    }
}

fn manifest(cx: &mut VisualTestContext) -> pitwall_core::extensions::Manifest {
    cx.update(|_, cx| crate::extensions::manifest_of("kit", cx)).expect("found")
}

fn done(cx: &mut VisualTestContext) -> bool {
    let m = manifest(cx);
    cx.update(|_, cx| connections::prefs_for(&m, None, cx).actions_done.contains(&"setup".to_string()))
}

fn not_yet(cx: &mut VisualTestContext) -> Vec<String> {
    let m = manifest(cx);
    cx.update(|_, cx| connections::not_yet_for(&m, &m.actions[0], None, cx).into_iter().map(|a| a.name).collect())
}

/// The confirmation lists every command for each place, the run shows
/// each account's steps, a failure for one account leaves the other's
/// result, and the step is ticked only once both are covered; Run again
/// then covers the missing account only.
#[gpui::test]
fn setup_runs_for_every_account_and_ticks_once_all_are_covered(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-setup-accounts");
    let (view, cx) = open(cx, &t, KIT, &["Work", "Home"], false);
    std::fs::write(t.path().join("bin/fail-claude-home"), "").unwrap();
    click(&view, cx, "extension-action-kit-setup");
    let plan = view.read_with(cx, |v, _| v.exts.confirm.clone()).expect("asked to confirm").2;
    let rows = super::plan_rows(&plan);
    let heads: Vec<String> = rows.iter().filter_map(|r| r.0.clone()).collect();
    assert_eq!(heads, ["Claude Code, default", "Claude Code, account Work", "Claude Code, account Home"]);
    assert_eq!(rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(), ["fake-rm x", "fake-add x", "fake-rm x", "fake-add x", "fake-rm x", "fake-add x"]);
    for i in [0, 2, 4] {
        assert!(drawn(cx, &format!("extension-confirm-for-kit-setup-{i}")), "each place's line is shown");
    }
    assert!(ran(&t).is_empty(), "nothing runs before Run");
    // The real Run button: the run's end reaches the view afterwards.
    click(&view, cx, "extension-run-kit-setup");
    let st = wait_run(cx);
    assert_eq!(ran(&t), ["rm claude-work", "add claude-work", "rm claude-home"], "each account's own folder; Home's add failed");
    assert_eq!(default_runs(&t), 2, "the default folder too");
    // Optional per account: each remove failed (nothing to remove), and
    // the add after it still ran; Work is done, Home's add failed.
    assert!(matches!(st.steps[3].1, actions::StepState::Ended(actions::Outcome::Done)));
    assert_eq!((st.group_word(&st.group(0)), st.group_word(&st.group(2)), st.group_word(&st.group(4))), ("done", "done", "failed"));
    assert!(drawn(cx, "extension-run-for-kit-setup-2") && drawn(cx, "extension-run-for-kit-setup-4"), "each account's steps under its name");
    assert!(!done(cx), "not ticked: Home failed");
    assert_eq!(not_yet(cx), ["Home"]);
    assert!(drawn(cx, "extension-not-yet-kit-setup"), "said at the step");
    // Run again: Home only.
    std::fs::remove_file(t.path().join("bin/fail-claude-home")).unwrap();
    let before = ran(&t).len();
    click(&view, cx, "extension-action-kit-setup");
    let plan = view.read_with(cx, |v, _| v.exts.confirm.clone()).expect("asked").2;
    assert!(plan.iter().all(|p| p.account.as_ref().is_some_and(|a| a.name == "Home")), "only what's missing");
    click(&view, cx, "extension-run-kit-setup");
    wait_run(cx);
    assert_eq!(ran(&t)[before..], ["rm claude-home", "add claude-home"]);
    assert!(done(cx), "ticked: every account covered");
    assert!(!drawn(cx, "extension-not-yet-kit-setup"));
}

/// A run recorded before accounts were kept isn't done while there are
/// accounts: the step says which still need it.
#[gpui::test]
fn a_setup_from_before_accounts_asks_for_them(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-setup-legacy");
    let (_view, cx) = open(cx, &t, KIT, &["Work"], false);
    let m = manifest(cx);
    cx.update(|_, cx| {
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension("kit", |e| e.actions_done.push("setup".into())));
        actions::seal_steps(cx);
    });
    assert!(!done(cx));
    assert_eq!(not_yet(cx), ["Work"]);
    assert!(drawn(cx, "extension-not-yet-kit-setup"));
    // Without accounts it is done as before.
    cx.update(|_, cx| {
        let model = crate::accounts::model(cx);
        model.update(cx, |a, cx| a.change(|l| { l.accounts.clear(); Ok(()) }, cx)).unwrap();
        assert!(connections::prefs_for(&m, None, cx).actions_done.contains(&"setup".to_string()));
    });
}

/// That same record, its steps unchanged, is set up for the accounts by
/// itself at start (only for them), and ticked.
#[gpui::test]
fn a_setup_from_before_accounts_covers_them_at_start(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-setup-legacy-auto");
    let (_view, cx) = open(cx, &t, KIT, &["Work", "Home"], false);
    cx.update(|_, cx| {
        crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension("kit", |e| e.actions_done.push("setup".into())));
        actions::seal_steps(cx);
        actions::auto_setup(cx);
    });
    let st = wait_run(cx);
    assert_eq!(ran(&t), ["rm claude-work", "add claude-work", "rm claude-home", "add claude-home"]);
    assert_eq!(default_runs(&t), 0, "the default folder was done");
    assert!(st.note.as_deref().is_some_and(|n| n.starts_with("Set up for Claude Code, account Work; Claude Code, account Home")), "{:?}", st.note);
    assert!(done(cx));
}

/// After a confirmed setup, an account added later gets exactly those
/// steps by itself, with its environment, shown and logged; once the
/// steps change (an update) nothing runs; with the extension off nothing
/// runs.
#[gpui::test]
fn an_account_added_later_gets_the_confirmed_setup(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-setup-later");
    let (view, cx) = open(cx, &t, KIT, &["Work"], true);
    click(&view, cx, "extension-action-kit-setup");
    click(&view, cx, "extension-run-kit-setup");
    wait_run(cx);
    assert_eq!(ran(&t), ["rm claude-work", "add claude-work"]);
    assert!(done(cx));

    add_account(cx, account(&t, "Home"));
    let st = wait_run(cx);
    assert_eq!(ran(&t)[2..], ["rm claude-home", "add claude-home"], "Home, in its own folder, without a press");
    assert!(st.note.as_deref().is_some_and(|n| n.starts_with("Set up for Claude Code, account Home, as you confirmed on")), "{:?}", st.note);
    assert!(done(cx), "Home covered");
    let said: Vec<String> = cx.update(|_, cx| crate::extensions::calls::entries("kit", cx).into_iter().map(|e| e.method).collect());
    assert!(said.iter().any(|s| s == "setup setup: started by Pitwall") && said.iter().any(|s| s == "setup setup: done"), "{said:?}");

    // Off: nothing runs for a new account.
    cx.update(|_, cx| crate::ui_state::update(cx, |s| *s = s.clone().set_extension("kit", false)));
    add_account(cx, account(&t, "Spare"));
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(ran(&t).len(), 4, "off: nothing ran");

    // On again with changed steps: nothing runs, and the step asks.
    let changed = KIT.replace(r#"["fake-add", "x"]"#, r#"["fake-add", "y"]"#);
    std::fs::write(t.path().join("pw/extensions/kit/extension.json"), changed).unwrap();
    cx.update(|_, cx| {
        crate::extensions::rescan(cx);
        crate::ui_state::update(cx, |s| *s = s.clone().set_extension("kit", true));
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(300));
    cx.run_until_parked();
    assert_eq!(ran(&t).len(), 4, "changed steps weren't confirmed: nothing ran");
    assert!(!done(cx));
}

/// An automatic run that fails shows its failure and isn't tried again
/// by itself.
#[gpui::test]
fn a_failed_automatic_setup_shows_and_does_not_loop(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-setup-fail");
    let (_view, cx) = open(cx, &t, KIT, &["Work"], false);
    std::fs::write(t.path().join("bin/fail-claude-work"), "").unwrap();
    let m = manifest(cx);
    cx.update(|_, cx| {
        connections::record_action(&m, "setup", None, true, cx);
        connections::record_confirmed(&m, &m.actions[0], None, cx);
        actions::auto_setup(cx);
    });
    let st = wait_run(cx);
    assert!(st.failed());
    assert!(drawn(cx, "extension-run-for-kit-setup-0"), "the failed run on the card");
    let said = cx.update(|_, cx| crate::extensions::calls::entries("kit", cx));
    assert!(said.iter().any(|e| e.method == "setup setup: failed"), "{said:?}");
    let before = std::fs::read_to_string(t.path().join("bin/log")).unwrap_or_default();
    for _ in 0..3 {
        cx.update(|_, cx| actions::auto_setup(cx));
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(std::fs::read_to_string(t.path().join("bin/log")).unwrap_or_default(), before, "not tried again");
    assert!(!done(cx));
    assert!(drawn(cx, "extension-not-yet-kit-setup"));
}

/// A development folder's extension runs nothing by itself without a
/// confirmation of its steps as they are (a record from before
/// confirmations doesn't stand for one there).
#[gpui::test]
fn a_development_folder_needs_a_confirmation(cx: &mut TestAppContext) {
    let t = TempDir::new("ext-setup-dev");
    let (_view, cx) = open(cx, &t, r#"{"id": "other", "name": "Other"}"#, &["Work"], false);
    let dev = t.path().join("dev/kit");
    std::fs::create_dir_all(&dev).unwrap();
    std::fs::write(dev.join("extension.json"), KIT).unwrap();
    std::fs::remove_dir_all(t.path().join("pw/extensions/kit")).unwrap();
    cx.update(|_, cx| {
        crate::ui_state::update(cx, |u| {
            *u = std::mem::take(u).with_extension("kit", |e| {
                e.dev_path = Some(dev.to_string_lossy().into_owned());
                e.actions_done.push("setup".into());
            })
        });
        crate::extensions::rescan(cx);
        actions::seal_steps(cx);
        assert!(crate::extensions::dev_folder("kit", cx).is_some());
        actions::auto_setup(cx);
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(cx.update(|_, cx| actions::runs(cx).read(cx).get("kit", "setup")).is_none(), "nothing ran");
    assert!(ran(&t).is_empty());
}
