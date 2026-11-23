//! An extension's setup actions (docs/spec/extensions.md, "Setup
//! actions"): [`plan`] says exactly what a button will run and what it
//! will skip, for the confirmation; [`run_live`] runs it after the user
//! confirms, off the UI thread, each step with a timeout and its output
//! kept short, telling a [`Live`] state as it goes; [`Runs`] keeps the
//! runs (so a run goes on, and shows, after Settings is closed and opened
//! again) and lets one be cancelled. Nothing runs at any other time.

use std::collections::{BTreeMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};


/// How long one step may take.
pub const STEP_TIMEOUT: Duration = Duration::from_secs(180);
/// How much of a step's output is kept.
const KEEP: usize = 8 * 1024;
/// The lines of output a failure shows.
const TAIL_LINES: usize = 6;

// What a step runs, worked out without running it, is the core's: the
// same plan `pitwall extension check` prints.
pub use pitwall_core::extensions::plan::{kind_name, plan, Planned};

/// How a step went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Failed: why, and the last lines of its output.
    Failed(String),
    Skipped(String),
}

/// Where a step is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepState {
    Waiting,
    Running(Instant),
    Ended(Outcome),
}

/// A run as it goes: each step's line and state, the running step's
/// output so far (its last bytes), whether it is over, and whether the
/// user cancelled it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunState {
    pub steps: Vec<(String, StepState)>,
    pub output: VecDeque<u8>,
    pub over: bool,
    pub cancelled: bool,
    /// Bumped on every change (the UI redraws when it moves).
    pub changes: u64,
}

impl RunState {
    pub fn new(plan: &[Planned]) -> RunState {
        RunState { steps: plan.iter().map(|p| (p.line(), StepState::Waiting)).collect(), output: VecDeque::new(), over: false, cancelled: false, changes: 0 }
    }

    /// The running step's last lines of output, as plain text.
    pub fn output_tail(&self) -> String {
        let text: Vec<u8> = self.output.iter().copied().collect();
        let text = String::from_utf8_lossy(&text);
        let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
        let tail = lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n");
        pitwall_core::extensions::protocol::clean(&tail, 1200, true)
    }

    /// How it ended, in a few words (None while it runs).
    pub fn summary(&self) -> Option<String> {
        if !self.over {
            return None;
        }
        if self.cancelled {
            return Some("Cancelled".into());
        }
        let failed = self.steps.iter().filter(|(_, s)| matches!(s, StepState::Ended(Outcome::Failed(_)))).count();
        let ran = self.steps.iter().filter(|(_, s)| matches!(s, StepState::Ended(Outcome::Done | Outcome::Failed(_)))).count();
        Some(match (failed, ran) {
            (0, 0) => "Nothing ran".into(),
            (0, _) => "All set".into(),
            (f, _) => format!("{f} of {} steps failed", self.steps.len()),
        })
    }

    pub fn failed(&self) -> bool {
        self.steps.iter().any(|(_, s)| matches!(s, StepState::Ended(Outcome::Failed(_))))
    }
}

/// A run's state, shared with the thread running it.
pub type Live = Arc<Mutex<RunState>>;

fn change(live: &Live, f: impl FnOnce(&mut RunState)) {
    let mut st = live.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut st);
    st.changes += 1;
}

/// Run `plan` in `cwd`, in order (blocking: call it off the UI thread). A
/// failed step that isn't optional skips the later steps for the same
/// kind (all later steps when it is for none).
pub fn run(plan: &[Planned], cwd: &Path, timeout: Duration) -> Vec<Outcome> {
    let live: Live = Arc::new(Mutex::new(RunState::new(plan)));
    run_live(plan, cwd, timeout, &live);
    let st = live.lock().unwrap_or_else(|e| e.into_inner());
    st.steps.iter().map(|(_, s)| match s {
        StepState::Ended(o) => o.clone(),
        _ => Outcome::Skipped("it didn't run".into()),
    }).collect()
}

/// [`run`], telling `live` each step's state as it changes and the running
/// step's output as it comes; `live.cancelled` set stops the running
/// step's program and skips the rest.
pub fn run_live(plan: &[Planned], cwd: &Path, timeout: Duration, live: &Live) {
    let mut failed_kinds: Vec<Option<String>> = Vec::new();
    let cancelled = || live.lock().unwrap_or_else(|e| e.into_inner()).cancelled;
    for (i, p) in plan.iter().enumerate() {
        let skip = if cancelled() {
            Some("cancelled".to_string())
        } else if let Some(why) = &p.skip {
            Some(why.clone())
        } else if failed_kinds.iter().any(|k| k.is_none() || *k == p.for_kind) {
            Some(match &p.for_kind {
                Some(k) => format!("an earlier step for {} failed", kind_name(k)),
                None => "an earlier step failed".into(),
            })
        } else {
            None
        };
        if let Some(why) = skip {
            change(live, |st| st.steps[i].1 = StepState::Ended(Outcome::Skipped(why)));
            continue;
        }
        change(live, |st| {
            st.output.clear();
            st.steps[i].1 = StepState::Running(Instant::now());
        });
        let o = run_one(p, cwd, timeout, live);
        if matches!(o, Outcome::Failed(_)) && !p.optional {
            failed_kinds.push(p.for_kind.clone());
        }
        change(live, |st| st.steps[i].1 = StepState::Ended(o));
    }
    change(live, |st| st.over = true);
}

fn run_one(p: &Planned, cwd: &Path, timeout: Duration, live: &Live) -> Outcome {
    let Some(program) = &p.program else { return Outcome::Skipped("it isn't installed".into()) };
    let mut cmd = crate::platform::background_command(program);
    cmd.args(&p.argv[1..]).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(path) = pitwall_core::shell::spawn_path() {
        cmd.env("PATH", path);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Outcome::Failed(format!("it couldn't start: {e}")),
    };
    let readers: Vec<_> = [child.stdout.take().map(|r| Box::new(r) as Box<dyn Read + Send>), child.stderr.take().map(|r| Box::new(r) as Box<dyn Read + Send>)]
        .into_iter()
        .flatten()
        .map(|mut r| {
            let live = live.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 2048];
                while let Ok(n) = r.read(&mut buf) {
                    if n == 0 {
                        return;
                    }
                    change(&live, |st| {
                        st.output.extend(&buf[..n]);
                        while st.output.len() > KEEP {
                            st.output.pop_front();
                        }
                    });
                }
            })
        })
        .collect();
    let end = Instant::now() + timeout;
    let mut stopped = false;
    let status = loop {
        let cancelled = live.lock().unwrap_or_else(|e| e.into_inner()).cancelled;
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if cancelled => {
                stopped = true;
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    // Its output, once it has ended (a program it started may still hold
    // the pipes after a timeout: those readers are left to finish).
    if status.is_some() {
        for r in readers {
            let _ = r.join();
        }
    }
    if stopped {
        return Outcome::Skipped("cancelled while it ran".into());
    }
    let tail = live.lock().unwrap_or_else(|e| e.into_inner()).output_tail();
    match status {
        Some(s) if s.success() => Outcome::Done,
        Some(s) => {
            let why = s.code().map_or("it was stopped".to_string(), |c| format!("it exited with code {c}"));
            Outcome::Failed(if tail.is_empty() { why } else { format!("{why}:\n{tail}") })
        }
        None => Outcome::Failed(format!("it took longer than {} s and was stopped", timeout.as_secs())),
    }
}

/// Each recorded run without a digest of its steps (from a Pitwall before
/// digests) gets the digest of the steps its action has now, so an update
/// that changes them later unticks it (`connections::steps_current`).
/// Called at start, before anything can be updated.
pub fn seal_steps(cx: &mut gpui::App) {
    let ui = crate::ui_state::get(cx);
    let found = super::found(cx);
    let accounts = crate::accounts::list(cx).cloned().unwrap_or_default();
    for m in found.iter().filter_map(|f| f.manifest.as_ref().ok()) {
        let prefs = ui.extension(&m.id);
        let mut add: BTreeMap<String, String> = BTreeMap::new();
        for a in &m.actions {
            let keys: Vec<String> = if super::connections::uses_setting(a) {
                accounts.connections(&m.id).filter(|c| c.actions_done.contains(&a.id)).map(|c| format!("{}@{}", a.id, c.id)).collect()
            } else if prefs.actions_done.contains(&a.id) {
                vec![a.id.clone()]
            } else {
                vec![]
            };
            for k in keys.into_iter().filter(|k| !prefs.action_steps.contains_key(k)) {
                add.insert(k, pitwall_core::extensions::steps_digest(a));
            }
        }
        if !add.is_empty() {
            crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension(&m.id, |e| e.action_steps.extend(add)));
        }
    }
}

/// The runs since Pitwall started, by (extension, action): the last of
/// each, running or over.
#[derive(Default)]
pub struct Runs {
    runs: std::collections::HashMap<(String, String), Live>,
}

struct RunsHandle(gpui::Entity<Runs>);

impl gpui::Global for RunsHandle {}

/// The runs (made on first use).
pub fn runs(cx: &mut gpui::App) -> gpui::Entity<Runs> {
    if let Some(h) = cx.try_global::<RunsHandle>() {
        return h.0.clone();
    }
    let e = gpui::AppContext::new(cx, |_| Runs::default());
    cx.set_global(RunsHandle(e.clone()));
    e
}

impl Runs {
    /// The last run of `ext`'s `action`, as it is now.
    pub fn get(&self, ext: &str, action: &str) -> Option<RunState> {
        self.runs.get(&(ext.to_string(), action.to_string())).map(|l| l.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    /// Whether `ext`'s `action` is running.
    pub fn running(&self, ext: &str, action: &str) -> bool {
        self.get(ext, action).is_some_and(|s| !s.over)
    }

    /// Stop `ext`'s `action`: its running step's program, and the rest.
    pub fn cancel(&self, ext: &str, action: &str) {
        if let Some(l) = self.runs.get(&(ext.to_string(), action.to_string())) {
            change(l, |st| st.cancelled = true);
        }
    }

    /// Run `plan` (the user confirmed it) for `ext`'s `action` in `cwd`, off
    /// the UI thread, telling whoever observes this as it goes; `done` gets
    /// how it ended.
    pub fn start(&mut self, (ext, action): (&str, &str), plan: Vec<Planned>, cwd: PathBuf, timeout: Duration, done: impl FnOnce(&RunState, &mut gpui::App) + 'static, cx: &mut gpui::Context<Self>) {
        let key = (ext.to_string(), action.to_string());
        if self.running(ext, action) {
            return;
        }
        let live: Live = Arc::new(Mutex::new(RunState::new(&plan)));
        self.runs.insert(key, live.clone());
        cx.notify();
        let worker = live.clone();
        std::thread::spawn(move || run_live(&plan, &cwd, timeout, &worker));
        // Told while it runs: each change, and the running step's time.
        cx.spawn(async move |this, cx| {
            let mut seen = u64::MAX;
            let mut ticks = 0u32;
            let mut done = Some(done);
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                ticks += 1;
                let (changes, over) = {
                    let st = live.lock().unwrap_or_else(|e| e.into_inner());
                    (st.changes, st.over)
                };
                if changes != seen || ticks.is_multiple_of(10) {
                    seen = changes;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        return;
                    }
                }
                if over {
                    let st = live.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    if let Some(done) = done.take() {
                        let _ = this.update(cx, |_, cx| done(&st, cx));
                    }
                    return;
                }
            }
        })
        .detach();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use pitwall_core::extensions::Manifest;
    use std::collections::BTreeMap;

    fn manifest() -> Manifest {
        let text = r#"{"id": "x", "name": "X", "settings": [{"key": "server", "type": "url", "label": "Server"}],
            "actions": [{"id": "setup", "label": "Set up", "description": "d", "steps": [
                {"for": "fake-a", "run": ["fake-ok", "add", "--url", "{setting:server}"]},
                {"for": "fake-a", "run": ["fake-fail", "it's"]},
                {"for": "fake-a", "run": ["fake-ok", "after"]},
                {"for": "fake-b", "run": ["fake-missing", "x"]},
                {"for": "fake-c", "run": ["fake-fail"], "optional": true},
                {"for": "fake-c", "run": ["fake-slow"]},
                {"run": ["fake-ok", "last"]}
            ]}]}"#;
        pitwall_core::extensions::parse(text).unwrap()
    }

    fn fakes(dir: &Path) {
        use std::os::unix::fs::PermissionsExt;
        for (name, body) in [
            ("fake-ok", "#!/bin/sh\necho \"ok $*\"\n"),
            ("fake-fail", "#!/bin/sh\necho line one\necho 'not allowed' >&2\nexit 2\n"),
            ("fake-slow", "#!/bin/sh\nsleep 5\n"),
        ] {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// The confirmation lists exactly what will run and what won't.
    #[test]
    fn the_plan_says_exactly_what_will_run() {
        let t = pitwall_core::testing::TempDir::new("actions-plan");
        fakes(t.path());
        let m = manifest();
        let path = t.path().to_string_lossy().into_owned();
        let none = plan(&m, &m.actions[0], &Default::default(), Some(&path));
        assert_eq!(none[0].skip.as_deref(), Some("set Server first"));
        let values = BTreeMap::from([("server".to_string(), "https://s.example".to_string())]);
        let p = plan(&m, &m.actions[0], &values, Some(&path));
        let lines: Vec<String> = p.iter().map(Planned::line).collect();
        assert_eq!(lines[0], "fake-ok add --url https://s.example");
        assert_eq!(lines[1], "fake-fail 'it'\\''s'");
        assert_eq!(p[3].skip.as_deref(), Some("fake-b isn't installed"));
        assert!(p.iter().enumerate().all(|(i, s)| (i == 3) == s.skip.is_some()));
        assert_eq!(kind_name("claude"), "Claude Code");
    }

    /// A plan of fake programs in `dir`: (program, args).
    fn steps(dir: &Path, list: &[(&str, &[&str])]) -> Vec<Planned> {
        list.iter()
            .map(|(p, args)| {
                let mut argv = vec![p.to_string()];
                argv.extend(args.iter().map(|a| a.to_string()));
                Planned { argv, program: Some(dir.join(p)), skip: None, for_kind: None, optional: false, account: None }
            })
            .collect()
    }

    /// The step states seen while `plan` runs: each change once, in order.
    fn watch(plan: Vec<Planned>, cwd: &Path, cancel_when_running: Option<usize>) -> (Vec<Vec<char>>, RunState) {
        let live: Live = Arc::new(Mutex::new(RunState::new(&plan)));
        let worker = live.clone();
        let cwd2 = cwd.to_path_buf();
        let t = std::thread::spawn(move || run_live(&plan, &cwd2, Duration::from_secs(10), &worker));
        let mut seen: Vec<Vec<char>> = Vec::new();
        loop {
            let st = live.lock().unwrap().clone();
            let now: Vec<char> = st
                .steps
                .iter()
                .map(|(_, s)| match s {
                    StepState::Waiting => 'w',
                    StepState::Running(_) => 'r',
                    StepState::Ended(Outcome::Done) => 'd',
                    StepState::Ended(Outcome::Failed(_)) => 'f',
                    StepState::Ended(Outcome::Skipped(_)) => 's',
                })
                .collect();
            if seen.last() != Some(&now) {
                seen.push(now.clone());
            }
            if cancel_when_running.is_some_and(|i| now[i] == 'r') {
                change(&live, |st| st.cancelled = true);
            }
            if st.over {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        t.join().unwrap();
        let st = live.lock().unwrap().clone();
        (seen, st)
    }

    /// Steps change state in order, one running at a time, with the
    /// running step's output there while it runs.
    #[test]
    fn a_run_streams_each_steps_state_in_order() {
        use std::os::unix::fs::PermissionsExt;
        let t = pitwall_core::testing::TempDir::new("actions-live");
        fakes(t.path());
        let talk = t.path().join("fake-talk");
        std::fs::write(&talk, "#!/bin/sh\necho first line\nsleep 0.4\necho second line\n").unwrap();
        std::fs::set_permissions(&talk, std::fs::Permissions::from_mode(0o755)).unwrap();
        let plan = steps(t.path(), &[("fake-talk", &[]), ("fake-fail", &[]), ("fake-ok", &["x"])]);
        // The output while the first step runs.
        let live: Live = Arc::new(Mutex::new(RunState::new(&plan)));
        let (w, p2, cwd) = (live.clone(), plan.clone(), t.path().to_path_buf());
        let th = std::thread::spawn(move || run_live(&p2, &cwd, Duration::from_secs(10), &w));
        let start = Instant::now();
        let mut saw_output = false;
        while start.elapsed() < Duration::from_secs(5) && !live.lock().unwrap().over {
            let st = live.lock().unwrap().clone();
            if matches!(st.steps[0].1, StepState::Running(_)) && st.output_tail() == "first line" {
                saw_output = true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        th.join().unwrap();
        assert!(saw_output, "the running step's output shows while it runs");

        let (seen, st) = watch(plan, t.path(), None);
        let as_text: Vec<String> = seen.iter().map(|v| v.iter().collect()).collect();
        // Polled, so a quick change may be missed; what is seen is in
        // this order, the slow first step seen running.
        let order = ["www", "rww", "dww", "drw", "dfw", "dfs"];
        let mut at = 0;
        for s in &as_text {
            at += order[at..].iter().position(|o| o == s).unwrap_or_else(|| panic!("{as_text:?} isn't in order"));
        }
        assert!(as_text.contains(&"rww".to_string()) && as_text.last().map(String::as_str) == Some("dfs"), "{as_text:?}");
        assert_eq!(st.summary().as_deref(), Some("1 of 3 steps failed"));
    }

    /// Cancel stops the running step's program at once and skips the rest.
    #[test]
    fn a_run_can_be_cancelled() {
        let t = pitwall_core::testing::TempDir::new("actions-cancel");
        fakes(t.path());
        let plan = steps(t.path(), &[("fake-ok", &["a"]), ("fake-slow", &[]), ("fake-ok", &["b"])]);
        let start = Instant::now();
        let (_, st) = watch(plan, t.path(), Some(1));
        assert!(start.elapsed() < Duration::from_secs(3), "the slow step was stopped");
        assert_eq!(st.steps[0].1, StepState::Ended(Outcome::Done));
        assert_eq!(st.steps[1].1, StepState::Ended(Outcome::Skipped("cancelled while it ran".into())));
        assert_eq!(st.steps[2].1, StepState::Ended(Outcome::Skipped("cancelled".into())));
        assert_eq!(st.summary().as_deref(), Some("Cancelled"));
    }

    /// A run kept by the app, not by Settings: its steps are there from
    /// the moment it starts, Settings closed and opened again mid-run
    /// reads it as it is, and how it ended is recorded (the setup step
    /// ticked).
    #[gpui::test]
    fn a_run_outlives_settings_and_is_recorded(cx: &mut gpui::TestAppContext) {
        let t = pitwall_core::testing::TempDir::new("actions-runs");
        fakes(t.path());
        use std::os::unix::fs::PermissionsExt;
        let wait = t.path().join("fake-wait");
        std::fs::write(&wait, format!("#!/bin/sh\nwhile [ ! -f '{}' ]; do sleep 0.05; done\necho went on\n", t.path().join("go").display())).unwrap();
        std::fs::set_permissions(&wait, std::fs::Permissions::from_mode(0o755)).unwrap();
        let plan = steps(t.path(), &[("fake-ok", &["a"]), ("fake-wait", &[])]);
        let cwd = t.path().to_path_buf();
        cx.update(|cx| {
            crate::ui_state::store(cx);
            let r = runs(cx);
            r.update(cx, |r, cx| r.start(("x", "setup"), plan, cwd, Duration::from_secs(10), |st, cx| crate::settings::extensions::record_run(&pitwall_core::extensions::Manifest::named("x", "X"), "setup", None, st, cx), cx));
            let st = runs(cx).read(cx).get("x", "setup").expect("listed at once");
            assert_eq!(st.steps.len(), 2);
            assert!(runs(cx).read(cx).running("x", "setup"));
        });
        // Mid-run, read again as a Settings opened anew would.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            std::thread::sleep(Duration::from_millis(20));
            cx.executor().advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
            let st = cx.update(|cx| runs(cx).read(cx).get("x", "setup").unwrap());
            if matches!(st.steps[1].1, StepState::Running(_)) {
                assert_eq!(st.steps[0].1, StepState::Ended(Outcome::Done));
                break;
            }
            assert!(Instant::now() < deadline, "the second step never ran");
        }
        cx.update(|cx| assert!(crate::ui_state::get(cx).extension("x").actions_done.is_empty(), "not recorded while it runs"));
        std::fs::write(t.path().join("go"), "").unwrap();
        loop {
            std::thread::sleep(Duration::from_millis(20));
            cx.executor().advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
            if cx.update(|cx| !runs(cx).read(cx).running("x", "setup")) {
                break;
            }
            assert!(Instant::now() < deadline + Duration::from_secs(5), "never ended");
        }
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        cx.update(|cx| {
            let st = runs(cx).read(cx).get("x", "setup").unwrap();
            assert_eq!(st.summary().as_deref(), Some("All set"));
            assert_eq!(crate::ui_state::get(cx).extension("x").actions_done, ["setup"]);
        });
    }

    /// A setup run recorded before digests keeps its tick, sealed with the
    /// steps its action has at start; steps changed after that (an update
    /// moved a pin) untick it, until it runs again.
    #[gpui::test]
    fn a_recorded_run_counts_only_for_the_steps_that_ran(cx: &mut gpui::TestAppContext) {
        let with = |pin: &str| pitwall_core::extensions::parse(&format!(r#"{{"id": "x", "name": "X", "actions": [{{"id": "setup", "setup": true, "label": "Set up", "description": "d", "steps": [{{"run": ["tool", "add", "x#{pin}"]}}]}}]}}"#)).unwrap();
        let found = |m: &Manifest| vec![pitwall_core::extensions::Found { folder: "x".into(), manifest: Ok(m.clone()) }];
        let (old, new) = (with("v1"), with("v2"));
        cx.update(|cx| {
            crate::ui_state::store(cx);
            crate::extensions::set_found(found(&old), cx);
            // As an older Pitwall left it: ticked, no digest.
            crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension("x", |e| e.actions_done.push("setup".into())));
            seal_steps(cx);
            let prefs = crate::ui_state::get(cx).extension("x");
            assert_eq!(prefs.action_steps.get("setup"), Some(&pitwall_core::extensions::steps_digest(&old.actions[0])));
            let done = |m: &Manifest, cx: &gpui::App| crate::extensions::connections::prefs_for(m, None, cx).actions_done;
            assert_eq!(done(&old, cx), ["setup"]);
            assert!(done(&new, cx).is_empty(), "the pin moved: asked again");
            // Run again with the new steps: ticked again.
            crate::extensions::set_found(found(&new), cx);
            crate::extensions::connections::record_action(&new, "setup", None, true, cx);
            assert_eq!(done(&new, cx), ["setup"]);
            // A failed run forgets the digest with the tick.
            crate::extensions::connections::record_action(&new, "setup", None, false, cx);
            assert!(crate::ui_state::get(cx).extension("x").action_steps.is_empty());
        });
    }

    /// Done, failed with its last lines, skipped after a failure for the
    /// same kind, skipped as missing, optional failure carrying on, and a
    /// timeout; only fake programs in a temporary folder run.
    #[test]
    fn a_run_reports_each_step_in_plain_words() {
        let t = pitwall_core::testing::TempDir::new("actions-run");
        fakes(t.path());
        let m = manifest();
        let values = BTreeMap::from([("server".to_string(), "https://s.example".to_string())]);
        let p = plan(&m, &m.actions[0], &values, Some(&t.path().to_string_lossy()));
        let out = run(&p, t.path(), Duration::from_millis(1500));
        assert_eq!(out[0], Outcome::Done);
        match &out[1] {
            Outcome::Failed(why) => assert!(why.starts_with("it exited with code 2") && why.contains("not allowed") && why.contains("line one"), "{why}"),
            o => panic!("{o:?}"),
        }
        assert_eq!(out[2], Outcome::Skipped("an earlier step for fake-a failed".into()));
        assert_eq!(out[3], Outcome::Skipped("fake-b isn't installed".into()));
        assert!(matches!(out[4], Outcome::Failed(_)), "optional: it fails ...");
        assert_eq!(out[5], Outcome::Failed("it took longer than 1 s and was stopped".into()), "... and the next one still runs");
        assert_eq!(out[6], Outcome::Done, "steps for no kind don't stop for another kind's failure");
    }
}
