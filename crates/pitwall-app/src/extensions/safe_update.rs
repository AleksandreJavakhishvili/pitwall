//! Updating an extension as one ordered step (docs/extensions/updates.md):
//! after the user confirms the review, Pitwall
//!
//! 1. stops its service and backs up its data (the service's own `backup`
//!    command, else a copy of its data folder up to
//!    [`core::COPY_CAP`]), when Pitwall runs it here and it has data;
//! 2. installs the new files, keeping the previous copy for Go back;
//! 3. runs its service's install action when the program is missing or
//!    older than the new version needs, then starts it and waits until it
//!    is up;
//! 4. checks that it is up and that its adapter connects as it did;
//! 5. runs again, for every account, each setup action whose steps changed
//!    (the agents' plugin; first instead with `update.order:
//!    "agentsFirst"`).
//!
//! A failure in 1–4 puts everything back by itself: the previous files,
//! its data from the backup and the previous service, and says what
//! failed and what it couldn't put back (a server program upgraded by its
//! install action stays). A failure in 5 leaves the update in place and
//! shows which accounts failed, with Retry.
//!
//! Go back offers to put back the data backup taken at that update
//! ([`go_back`]); the default keeps the data as it is.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use gpui::{App, AppContext, AsyncApp, Entity, Global};
use pitwall_core::extensions::install;
use pitwall_core::extensions::plan::ToolAccount;
use pitwall_core::extensions::protocol::ConnState;
use pitwall_core::extensions::update::{self as core, Note, PlanInput, Planned, Restart, Step};
use pitwall_core::extensions::Manifest;

use super::actions::{self, RunState};
use super::catalog::{self, Outcome, Prepared};
use super::service::{self, State};
use super::{connections, hub};

/// What an update calls once it ends.
type Done = Box<dyn FnOnce(Outcome, &mut App)>;

/// How often a step looks again.
const POLL: Duration = Duration::from_millis(100);
/// How long a service's own backup command may take.
pub const BACKUP_TIMEOUT: Duration = Duration::from_secs(600);

/// How long a service has to be up, and its adapter to connect, after an
/// update (tests make them short).
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub ready: Duration,
    pub connect: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts { ready: Duration::from_secs(core::READY_TIMEOUT_SECS), connect: Duration::from_secs(core::CONNECT_TIMEOUT_SECS) }
    }
}

impl Global for Timeouts {}

/// Where a step of a running update is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepState {
    Waiting,
    Running,
    /// Done, with what it did when worth saying.
    Done(Option<String>),
    Failed(String),
    /// Not run (why).
    Skipped(String),
}

/// A step as it goes.
#[derive(Debug, Clone)]
pub struct StepRun {
    pub planned: Planned,
    pub state: StepState,
}

/// How an update ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    Done,
    /// It stopped before anything changed (why).
    NothingChanged(String),
    /// A step failed after the new files were in: what failed, then what
    /// was put back and what couldn't be.
    RolledBack { why: String, notes: Vec<String> },
    /// The update stayed, but a setup action failed (each, with why):
    /// Retry runs those again.
    SetupFailed(Vec<String>),
}

/// One extension's update.
#[derive(Debug, Clone, Default)]
pub struct Run {
    pub from: Option<String>,
    pub to: Option<String>,
    pub steps: Vec<StepRun>,
    pub ending: Option<Ending>,
    /// The setup runs that failed, for Retry.
    retry: Vec<Job>,
    manifest: Option<Manifest>,
}

impl Run {
    pub fn over(&self) -> bool {
        self.ending.is_some()
    }
}

/// A setup action to run for one connection (`None`: the extension's).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Job {
    action: String,
    conn: Option<String>,
}

/// The updates since Pitwall started, and the data folders' sizes for the
/// reviews.
#[derive(Default)]
pub struct SafeUpdates {
    runs: HashMap<String, Run>,
    sizes: HashMap<String, Option<u64>>,
    measuring: HashSet<String>,
    /// Extensions whose data is too big to copy and the user said to
    /// update anyway (until the update ends).
    go_on: HashSet<String>,
    /// The extension whose Go back asks whether to put its data back too.
    pub asking_back: Option<String>,
}

impl SafeUpdates {
    /// The user said to update `id` without a backup.
    pub fn go_on(&self, id: &str) -> bool {
        self.go_on.contains(id)
    }

    pub fn set_go_on(&mut self, id: &str) {
        self.go_on.insert(id.to_string());
    }

    pub fn get(&self, id: &str) -> Option<&Run> {
        self.runs.get(id)
    }

    /// An update of `id` runs.
    pub fn running(&self, id: &str) -> bool {
        self.runs.get(id).is_some_and(|r| !r.over())
    }

    /// The size of `id`'s service data folder, once measured.
    pub fn size(&self, id: &str) -> Option<u64> {
        self.sizes.get(id).copied().flatten()
    }

    /// Forget how `id`'s last update went (Dismiss).
    pub fn dismiss(&mut self, id: &str) {
        if self.runs.get(id).is_some_and(Run::over) {
            self.runs.remove(id);
        }
    }

    fn set(&mut self, id: &str, step: &Step, state: StepState) {
        if let Some(r) = self.runs.get_mut(id) {
            for s in r.steps.iter_mut().filter(|s| s.planned.step == *step) {
                s.state = state.clone();
            }
        }
    }

    fn skip_rest(&mut self, id: &str, why: &str) {
        if let Some(r) = self.runs.get_mut(id) {
            for s in r.steps.iter_mut().filter(|s| matches!(s.state, StepState::Waiting | StepState::Running)) {
                s.state = StepState::Skipped(why.to_string());
            }
        }
    }
}

struct Handle(Entity<SafeUpdates>);

impl Global for Handle {}

/// The updates (made on first use).
pub fn model(cx: &mut App) -> Entity<SafeUpdates> {
    if let Some(h) = cx.try_global::<Handle>() {
        return h.0.clone();
    }
    let e = cx.new(|_| SafeUpdates::default());
    cx.set_global(Handle(e.clone()));
    e
}

pub fn try_model(cx: &App) -> Option<Entity<SafeUpdates>> {
    cx.try_global::<Handle>().map(|h| h.0.clone())
}

/// What an update of extension `id` to `new` depends on, as it is now.
struct Facts {
    id: String,
    old: Manifest,
    new: Manifest,
    on: bool,
    service_here: bool,
    data: Option<PathBuf>,
    has_data: bool,
    backups: Option<PathBuf>,
    values: BTreeMap<String, String>,
    path: Option<String>,
    accounts: Vec<ToolAccount>,
    set_up: Vec<String>,
    jobs: Vec<Job>,
    agents: usize,
    restart: Restart,
    adapter_ready: bool,
}

fn facts(id: &str, new: &Manifest, cx: &mut App) -> Option<Facts> {
    let old = super::manifest_of(id, cx)?;
    let ui = crate::ui_state::get(cx);
    let prefs = ui.extension(id);
    let on = ui.extension_on(id);
    let state = service::try_services(cx).and_then(|s| s.read(cx).get(id).map(|v| v.state.clone())).unwrap_or_default();
    let service_here = on && old.service.is_some() && service::confirmed(&old, cx).is_ok() && !prefs.service.stopped && !matches!(state, State::Elsewhere(_));
    let data = service::data_dir(id, cx);
    let has_data = data.as_ref().is_some_and(|d| std::fs::read_dir(d).is_ok_and(|mut r| r.next().is_some()));
    let backups = super::dir(cx).and_then(|d| d.parent().map(|r| pitwall_core::paths::Paths::new(r.to_path_buf()).extension_backups_dir(id)));
    // The setup actions set up before (for which connections).
    let keys: Vec<String> = prefs.action_steps.keys().cloned().collect();
    let set_up: Vec<String> = old.actions.iter().filter(|a| a.setup && keys.iter().any(|k| *k == a.id || k.starts_with(&format!("{}@", a.id)))).map(|a| a.id.clone()).collect();
    let jobs = keys
        .iter()
        .filter_map(|k| {
            let (a, c) = match k.split_once('@') {
                Some((a, c)) => (a.to_string(), Some(c.to_string())),
                None => (k.clone(), None),
            };
            set_up.contains(&a).then_some(Job { action: a, conn: c })
        })
        .collect();
    let adapter_ready = hub::hub(cx).read(cx).get(id).is_some_and(|s| matches!(s.conn, Some((ConnState::Ready, _))));
    Some(Facts {
        id: id.to_string(),
        new: new.clone(),
        on,
        service_here,
        data,
        has_data,
        backups,
        values: connections::settings_values(id, cx),
        path: actions::steps_path(cx),
        accounts: crate::accounts::tool_accounts(cx),
        set_up,
        jobs,
        agents: super::engine(cx).map(|e| e.extension_users(id).len()).unwrap_or(0),
        restart: super::stale::mode(id, cx),
        adapter_ready,
        old,
    })
}

impl Facts {
    fn plan(&self, size: Option<u64>) -> Vec<Planned> {
        let data = self.data.as_ref().map(|d| service::shown(d)).unwrap_or_default();
        let backup = pitwall_core::extensions::service::backup_shown(&data);
        let input = PlanInput {
            on: self.on,
            service_here: self.service_here,
            has_data: self.has_data,
            data_size: size,
            values: self.values.clone(),
            data: &data,
            backup: &backup,
            path: self.path.as_deref(),
            accounts: &self.accounts,
            set_up: &self.set_up,
            agents: self.agents,
            restart: self.restart,
        };
        core::plan(&self.old, &self.new, &input)
    }
}

/// What the review of an update says, before it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    /// The steps in order, with their commands.
    pub steps: Vec<Planned>,
    /// About the running agents that use its setup now, and what the
    /// restart setting does with them.
    pub agents: Vec<String>,
    /// Its data folder is bigger than Pitwall copies (its size), and it
    /// has no backup command of its own: going on needs a yes.
    pub too_big: Option<u64>,
}

/// The review of updating extension `id` to `new` (its data folder is
/// measured in the background the first time: looked at again then).
pub fn review(id: &str, new: &Manifest, cx: &mut App) -> Option<Review> {
    let f = facts(id, new, cx)?;
    let size = model(cx).read(cx).size(id);
    if f.has_data && core::backs_up(&f.old, &plan_input_stub(&f)) && core::backup_of(&f.old, &f.new).is_none() {
        measure(id, f.data.clone(), cx);
    }
    let too_big = size.filter(|n| *n > core::COPY_CAP && f.has_data && f.service_here && core::backup_of(&f.old, &f.new).is_none());
    Some(Review { steps: f.plan(size), agents: core::agents_lines(&f.new, f.agents, f.restart), too_big })
}

fn plan_input_stub(f: &Facts) -> PlanInput<'static> {
    PlanInput { on: f.on, service_here: f.service_here, has_data: f.has_data, ..Default::default() }
}

/// Measure `id`'s data folder off the UI thread (once).
fn measure(id: &str, data: Option<PathBuf>, cx: &mut App) {
    let Some(data) = data else { return };
    let m = model(cx);
    if !m.update(cx, |s, _| s.measuring.insert(id.to_string())) {
        return;
    }
    let id = id.to_string();
    cx.spawn(async move |cx| {
        let size = off(cx, move || core::dir_size(&data).ok()).await;
        m.update(cx, |s, cx| {
            s.sizes.insert(id, size);
            cx.notify();
        });
    })
    .detach();
}

/// Run `f` on a thread of its own; its result, looked at every [`POLL`].
async fn off<T: Send + 'static>(cx: &mut AsyncApp, f: impl FnOnce() -> T + Send + 'static) -> T {
    let out: Arc<Mutex<Option<T>>> = Arc::default();
    let w = out.clone();
    std::thread::spawn(move || {
        let r = f();
        *w.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
    });
    loop {
        if let Some(r) = out.lock().unwrap_or_else(|e| e.into_inner()).take() {
            return r;
        }
        cx.background_executor().timer(POLL).await;
    }
}

/// Call `f`, which answers through its callback (now or later).
async fn answer<T: 'static>(cx: &mut AsyncApp, f: impl FnOnce(Box<dyn FnOnce(T, &mut App)>, &mut App)) -> Option<T> {
    let (tx, rx) = futures::channel::oneshot::channel();
    cx.update(|cx| f(Box::new(move |t, _| drop(tx.send(t))), cx));
    rx.await.ok()
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Update extension `id` (`p`: the update looked at; `None`: Pitwall's own
/// copy) as one ordered step, live in [`model`]. `replace_changed`: a
/// folder changed by hand may be replaced; `without_backup`: the user
/// said to go on when its data is too big to copy. `done` hears
/// `ChangedLocally` (nothing ran: ask first), `Failed` when it couldn't
/// start, else `Done` (how it went is in the run).
pub fn start(id: &str, p: Option<Prepared>, replace_changed: bool, without_backup: bool, cx: &mut App, done: impl FnOnce(Outcome, &mut App) + 'static) {
    let fail = |why: String, cx: &mut App, done: Done| cx.defer(move |cx| done(Outcome::Failed(why), cx));
    let done: Done = Box::new(done);
    let p = match p {
        Some(p) => p,
        None => match catalog::shipped_one(id, cx) {
            Some(s) => Prepared { offer: s.offer, changes: vec![], replaces: Some(None) },
            None => return fail("It isn't shipped with this Pitwall.".into(), cx, done),
        },
    };
    if let Some(why) = catalog::dev_refusal(id, cx) {
        return fail(why, cx, done);
    }
    let Some(dir) = super::dir(cx) else { return fail("Extensions need Pitwall's data folder.".into(), cx, done) };
    if !replace_changed && !install::local(&dir.join(id)).as_installed {
        cx.defer(move |cx| done(Outcome::ChangedLocally, cx));
        return;
    }
    let Some(f) = facts(id, &p.offer.manifest, cx) else { return fail("It isn't installed.".into(), cx, done) };
    let m = model(cx);
    if m.read(cx).running(id) {
        return fail("It is being updated already.".into(), cx, done);
    }
    let size = m.read(cx).size(id);
    let steps = f.plan(size).into_iter().map(|planned| StepRun { planned, state: StepState::Waiting }).collect();
    m.update(cx, |s, cx| {
        s.runs.insert(id.to_string(), Run { from: f.old.version.clone(), to: f.new.version.clone(), steps, ending: None, retry: vec![], manifest: Some(f.new.clone()) });
        cx.notify();
    });
    activity(id, "update: started", None, Some(format!("{} → {}", f.old.version.clone().unwrap_or_default(), f.new.version.clone().unwrap_or_default())), cx);
    cx.spawn(async move |cx| {
        let ending = drive(cx, &m, f, p, replace_changed, without_backup).await;
        let outcome = match ending {
            Some(e) => {
                let id = e.0.clone();
                let words = match &e.1 {
                    Ending::Done => None,
                    Ending::NothingChanged(w) => Some(w.clone()),
                    Ending::RolledBack { why, .. } => Some(why.clone()),
                    Ending::SetupFailed(l) => Some(l.join("; ")),
                };
                cx.update(|cx| {
                    let what = match &e.1 {
                        Ending::Done => "update: done",
                        Ending::NothingChanged(_) => "update: stopped, nothing changed",
                        Ending::RolledBack { .. } => "update: failed and put back",
                        Ending::SetupFailed(_) => "update: done, a setup failed",
                    };
                    let failed = (!matches!(e.1, Ending::Done)).then(|| words.clone().unwrap_or_default());
                    activity(&id, what, failed, None, cx);
                    m.update(cx, |s, cx| {
                        s.go_on.remove(&id);
                        if let Some(r) = s.runs.get_mut(&id) {
                            r.ending = Some(e.1);
                        }
                        cx.notify();
                    });
                });
                Outcome::Done
            }
            None => Outcome::ChangedLocally,
        };
        cx.update(|cx| done(outcome, cx));
    })
    .detach();
}

/// The steps, in order. `None`: the folder was changed by hand after all
/// (nothing ran).
async fn drive(cx: &mut AsyncApp, m: &Entity<SafeUpdates>, f: Facts, p: Prepared, replace_changed: bool, without_backup: bool) -> Option<(String, Ending)> {
    let id = f.id.clone();
    let planned: Vec<Step> = m.read_with(cx, |u, _| u.get(&id).map(|r| r.steps.iter().map(|x| x.planned.step.clone()).collect()).unwrap_or_default());
    let has = |s: &Step| planned.contains(s);
    let set = |cx: &mut AsyncApp, s: Step, st: StepState| {
        m.update(cx, |u, cx| {
            u.set(&id, &s, st);
            cx.notify();
        })
    };
    let timeouts = cx.update(|cx| cx.try_global::<Timeouts>().copied().unwrap_or_default());
    // 1. Stop the service and back up its data.
    let mut backup: Option<PathBuf> = None;
    let mut held = false;
    if has(&Step::Backup) {
        set(cx, Step::Backup, StepState::Running);
        hold(cx, &id, true).await;
        held = true;
        if f.has_data {
            let job = backup_job(&f, without_backup);
            match off(cx, job).await {
                Ok((folder, words)) => {
                    backup = folder;
                    set(cx, Step::Backup, StepState::Done(Some(words)));
                }
                Err(why) => {
                    hold(cx, &id, false).await;
                    set(cx, Step::Backup, StepState::Failed(why.clone()));
                    m.update(cx, |u, _| u.skip_rest(&id, "Not run: nothing changed."));
                    return Some((id, Ending::NothingChanged(why)));
                }
            }
        } else {
            set(cx, Step::Backup, StepState::Done(Some("Its data folder is empty: nothing to back up.".into())));
        }
    }
    // 2. The new files.
    set(cx, Step::Install, StepState::Running);
    let old_confirm = cx.update(|cx| crate::ui_state::get(cx).extension(&id).service.confirmed);
    let installed = answer(cx, |done, cx| catalog::apply(&p, replace_changed, cx, done)).await.unwrap_or(Outcome::Failed("It was stopped.".into()));
    match installed {
        Outcome::Done => set(cx, Step::Install, StepState::Done(None)),
        Outcome::ChangedLocally => {
            if held {
                hold(cx, &id, false).await;
            }
            m.update(cx, |u, cx| {
                u.runs.remove(&id);
                cx.notify();
            });
            return None;
        }
        Outcome::Failed(why) => {
            if held {
                hold(cx, &id, false).await;
            }
            let why = format!("Its new files couldn't be installed: {why}");
            set(cx, Step::Install, StepState::Failed(why.clone()));
            m.update(cx, |u, _| u.skip_rest(&id, "Not run: nothing changed."));
            return Some((id, Ending::NothingChanged(why)));
        }
    }
    // Pressing Update confirmed the new service's commands (they were
    // listed): it runs them as it ran the old ones.
    if f.service_here {
        let new = f.new.clone();
        cx.update(|cx| confirm_quietly(&new, cx));
    }
    let record = crate::main_screen::workspace::LastUpdate { from: f.old.version.clone(), to: f.new.version.clone(), at: now_secs(), backup: backup.as_ref().map(|b| b.to_string_lossy().into_owned()), confirmed: old_confirm.as_ref().map(|c| c.digest.clone()) };
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension(&id, |e| e.last_update = Some(record))));
    let agents_first = f.new.update.order == core::Order::AgentsFirst;
    let mut failed_setup = Vec::new();
    if agents_first {
        failed_setup = setup_steps(cx, m, &f, &f.jobs).await;
    }
    // 3. The service's program, then the service.
    let mut upgraded = None;
    let service_failed: Option<String> = 'service: {
        if !has(&Step::ServiceStart) {
            if held {
                hold(cx, &id, false).await;
            }
            break 'service None;
        }
        hold(cx, &id, false).await;
        let s = f.new.service.clone().unwrap_or_else(|| unreachable!("a service step"));
        let mut st = settle(cx, &id, timeouts.ready).await;
        if has(&Step::ServiceInstall) {
            if matches!(st, State::NotInstalled(_) | State::TooOld { .. }) {
                set(cx, Step::ServiceInstall, StepState::Running);
                let a = s.install.as_ref().and_then(|i| f.new.actions.iter().find(|a| a.id == *i)).cloned();
                let Some(a) = a else { break 'service Some("Its install action is missing.".into()) };
                let plan = actions::plan(&f.new, &a, &f.values, f.path.as_deref());
                let (ext, key) = (id.clone(), a.id.clone());
                let res = answer(cx, move |done, cx| {
                    let runs = actions::runs(cx);
                    runs.update(cx, |r, cx| r.start((&ext, &key), plan, pitwall_core::paths::home(), actions::STEP_TIMEOUT, move |st: &RunState, cx| done(st.clone(), cx), cx));
                })
                .await;
                upgraded = Some(a.label.clone());
                match res {
                    Some(r) if !r.failed() && !r.cancelled => {
                        set(cx, Step::ServiceInstall, StepState::Done(r.summary()));
                        let i2 = id.clone();
                        cx.update(|cx| service::recheck(&i2, cx));
                        st = settle(cx, &id, timeouts.ready).await;
                    }
                    r => {
                        let why = format!("“{}” failed: {}", a.label, r.and_then(|r| r.summary()).unwrap_or_default());
                        set(cx, Step::ServiceInstall, StepState::Failed(why.clone()));
                        break 'service Some(why);
                    }
                }
            } else {
                set(cx, Step::ServiceInstall, StepState::Skipped("Its program is new enough.".into()));
            }
        }
        set(cx, Step::ServiceStart, StepState::Running);
        if !st.up() {
            let why = format!("{} didn't come up: {}", s.label, state_words(&st));
            set(cx, Step::ServiceStart, StepState::Failed(why.clone()));
            break 'service Some(why);
        }
        set(cx, Step::ServiceStart, StepState::Done(None));
        None
    };
    // 4. Up, and connected as before.
    let health_failed = match service_failed {
        Some(why) => Some(why),
        None if has(&Step::Health) => {
            set(cx, Step::Health, StepState::Running);
            match health(cx, &f, timeouts).await {
                Ok(()) => {
                    set(cx, Step::Health, StepState::Done(None));
                    None
                }
                Err(why) => {
                    set(cx, Step::Health, StepState::Failed(why.clone()));
                    Some(why)
                }
            }
        }
        None => None,
    };
    if let Some(why) = health_failed {
        m.update(cx, |u, _| u.skip_rest(&id, "Not run: the update was put back."));
        let notes = rollback(cx, &f, backup.as_deref(), old_confirm, upgraded, timeouts).await;
        return Some((id, Ending::RolledBack { why, notes }));
    }
    // 5. The agents' setup, last.
    if !agents_first {
        failed_setup = setup_steps(cx, m, &f, &f.jobs).await;
    }
    if failed_setup.is_empty() {
        Some((id, Ending::Done))
    } else {
        let words: Vec<String> = failed_setup.iter().map(|(_, w)| w.clone()).collect();
        let jobs: Vec<Job> = failed_setup.into_iter().map(|(j, _)| j).collect();
        m.update(cx, |u, _| {
            if let Some(r) = u.runs.get_mut(&id) {
                r.retry = jobs;
            }
        });
        Some((id, Ending::SetupFailed(words)))
    }
}

/// The work of step 1, for a thread: make the backup folder and back up
/// into it (the service's own command, else a copy). Its folder (`None`:
/// none was made, the user said to go on) and what it did.
fn backup_job(f: &Facts, without_backup: bool) -> impl FnOnce() -> Result<(Option<PathBuf>, String), String> + Send + 'static {
    let (old, new, values, path) = (f.old.clone(), f.new.clone(), f.values.clone(), f.path.clone());
    let (data, backups) = (f.data.clone(), f.backups.clone());
    move || {
        let (Some(data), Some(backups)) = (data, backups) else { return Err("Its data folder isn't known.".into()) };
        let folder = core::make_backup_folder(&backups, now_secs()).map_err(|e| format!("Its backup folder couldn't be made: {e}"))?;
        let mut note = Note { from: old.version.clone(), to: new.version.clone(), at: now_secs(), copy: true, replaces: vec![] };
        let mut said = String::new();
        if let Some((m, b)) = core::backup_of(&old, &new) {
            let cmd = pitwall_core::extensions::service::backup_command(m, b, &values, &data.to_string_lossy(), &folder.to_string_lossy());
            match cmd.map_err(|l| format!("set {l} first")).and_then(|cmd| run_backup(&cmd, &data, path.as_deref()).map(|()| cmd)) {
                Ok(cmd) => {
                    note.copy = false;
                    note.replaces = b.replaces.clone();
                    let _ = note.write(&folder);
                    core::prune_backups(&backups, core::KEEP_BACKUPS);
                    return Ok((Some(folder.clone()), format!("Backed up with {} to {}.", pitwall_core::extensions::plan::line(&cmd), service::shown(&folder))));
                }
                Err(why) => said = format!("Its backup command failed ({why}); Pitwall copied its data folder instead. "),
            }
        }
        match core::copy_data(&data, &folder, core::COPY_CAP) {
            Ok(_) => {
                let _ = note.write(&folder);
                core::prune_backups(&backups, core::KEEP_BACKUPS);
                Ok((Some(folder.clone()), format!("{said}Copied its data folder to {}.", service::shown(&folder))))
            }
            Err(core::CopyRefused::TooBig(n)) => {
                let _ = std::fs::remove_dir_all(&folder);
                if without_backup {
                    Ok((None, format!("Not backed up: its data is {}, more than Pitwall copies, and you said to go on.", core::size_words(n))))
                } else {
                    Err(format!("Its data is {}, more than Pitwall copies ({}): nothing changed. Review the update again to go on without a backup.", core::size_words(n), core::size_words(core::COPY_CAP)))
                }
            }
            Err(core::CopyRefused::Failed(why)) => {
                let _ = std::fs::remove_dir_all(&folder);
                Err(format!("{said}{why}: nothing changed."))
            }
        }
    }
}

/// Run a service's backup command `cmd` in its data folder with the
/// minimal environment; `Err(why)` with its last lines.
fn run_backup(cmd: &[String], data: &Path, path: Option<&str>) -> Result<(), String> {
    let program = pitwall_core::extensions::plan::resolve(data, &cmd[0], path)?;
    let env = pitwall_core::extensions::service::minimal_env(std::env::vars(), path);
    let (mut p, mut out, mut err) = pitwall_core::extensions::process::start_service(&program, &cmd[1..], data, &env).map_err(|e| format!("it couldn't start: {e}"))?;
    let read = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = r.read_to_string(&mut s);
            s
        })
    };
    let (o, e) = (read(std::mem::replace(&mut out, Box::new(std::io::empty()))), read(std::mem::replace(&mut err, Box::new(std::io::empty()))));
    let end = std::time::Instant::now() + BACKUP_TIMEOUT;
    let code = loop {
        if let Some(c) = p.exited() {
            break c;
        }
        if std::time::Instant::now() > end {
            p.stop();
            return Err(format!("it took more than {} minutes", BACKUP_TIMEOUT.as_secs() / 60));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let text = format!("{}{}", o.join().unwrap_or_default(), e.join().unwrap_or_default());
    let tail: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = tail[tail.len().saturating_sub(3)..].join(" / ");
    match code {
        Some(0) => Ok(()),
        Some(c) => Err(format!("it exited with code {c}{}", if tail.is_empty() { String::new() } else { format!(": {tail}") })),
        None => Err("it was stopped by a signal".into()),
    }
}

/// Keep extension `id`'s service stopped (`true`, waiting until its
/// process is gone) or let it start again.
async fn hold(cx: &mut AsyncApp, id: &str, held: bool) {
    let id2 = id.to_string();
    let stopping = cx.update(|cx| {
        let s = service::services(cx);
        s.update(cx, |s, cx| s.hold(&id2, held, cx))
    });
    if held {
        if let Some(c) = stopping {
            off(cx, move || {
                let end = std::time::Instant::now() + service::GRACE + Duration::from_secs(2);
                while std::time::Instant::now() < end && c.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
                    std::thread::sleep(Duration::from_millis(50));
                }
            })
            .await;
        }
    }
}

/// Wait until `id`'s service settles (up, or stopped for a reason), at
/// most `timeout`: its state then.
async fn settle(cx: &mut AsyncApp, id: &str, timeout: Duration) -> State {
    let start = cx.background_executor().now();
    loop {
        let st = cx.update(|cx| service::try_services(cx).and_then(|s| s.read(cx).get(id).map(|v| v.state.clone())).unwrap_or_default());
        let waiting = matches!(st, State::Checking | State::Starting | State::Restarting { .. } | State::Updating | State::Off);
        if !waiting || cx.background_executor().now().duration_since(start) >= timeout {
            return st;
        }
        cx.background_executor().timer(POLL).await;
    }
}

/// A service's state, as a reason.
fn state_words(st: &State) -> String {
    match st {
        State::Stopped(why) => why.clone(),
        State::NotInstalled(p) => format!("{p} isn't installed"),
        State::TooOld { found, need } => format!("it is {found}, older than the {need} it needs"),
        State::NeedsSetting(l) => format!("{l} isn't set"),
        State::BadAddress(why) => why.clone(),
        State::NeedsConfirm { .. } => "its commands weren't confirmed".into(),
        State::UserStopped => "you stopped it".into(),
        State::Checking | State::Starting | State::Restarting { .. } | State::Updating | State::Off => "it wasn't up in time".into(),
        State::Running { .. } | State::Elsewhere(_) => "it is up".into(),
    }
}

/// Step 4: its service is up and its adapter connects as it did before.
async fn health(cx: &mut AsyncApp, f: &Facts, t: Timeouts) -> Result<(), String> {
    let id = f.id.clone();
    let start = cx.background_executor().now();
    let svc_up = |cx: &mut AsyncApp| cx.update(|cx| service::try_services(cx).and_then(|s| s.read(cx).get(&id).map(|v| v.state.clone())).unwrap_or_default());
    loop {
        let st = svc_up(cx);
        if f.service_here && f.new.service.is_some() && !st.up() {
            return Err(format!("{} stopped after it started: {}", f.new.service.as_ref().map_or("Its service", |s| s.label.as_str()), state_words(&st)));
        }
        let (conn, run) = cx.update(|cx| hub::hub(cx).read(cx).get(&id).map(|e| (e.conn.clone(), e.run.clone())).unwrap_or_default());
        let ready = matches!(conn, Some((ConnState::Ready, _)));
        let crashed = match &run {
            hub::Run::Stopped(why) => Some(why.clone()),
            _ => None,
        };
        if let Some(why) = crashed.filter(|_| f.new.reports_status()) {
            return Err(format!("{} stopped: {why}", f.new.name));
        }
        if !f.adapter_ready || !f.new.reports_status() || ready {
            return Ok(());
        }
        if cx.background_executor().now().duration_since(start) >= t.connect {
            let said = conn.map(|(_, w)| w).filter(|w| !w.is_empty()).map(|w| format!(" (it says: {w})")).unwrap_or_default();
            return Err(format!("{} didn't connect within {} s as it did before the update{said}.", f.new.name, t.connect.as_secs()));
        }
        cx.background_executor().timer(POLL).await;
    }
}

/// Put back what the update changed: the previous files, the service's
/// commands as confirmed, its data from `backup`, and the previous
/// service running. What was put back and what couldn't be.
async fn rollback(cx: &mut AsyncApp, f: &Facts, backup: Option<&Path>, confirm: Option<crate::main_screen::workspace::ServiceConfirmed>, upgraded: Option<String>, t: Timeouts) -> Vec<String> {
    let id = f.id.clone();
    let mut notes = Vec::new();
    let v = |m: &Manifest| m.version.clone().unwrap_or_else(|| "the earlier copy".into());
    let service = f.service_here && f.old.service.is_some();
    if service {
        hold(cx, &id, true).await;
    }
    let i2 = id.clone();
    match answer(cx, move |done, cx| catalog::rollback_one(&i2, true, cx, done)).await {
        Some(Outcome::Done) => notes.push(format!("{} {} is back.", f.old.name, v(&f.old))),
        Some(Outcome::Failed(why)) => notes.push(format!("Its previous files couldn't be put back: {why}. Use Go back, or install {} again.", f.old.name)),
        _ => notes.push("Its previous files couldn't be put back. Use Go back, or install it again.".into()),
    }
    let i2 = id.clone();
    cx.update(|cx| crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension(&i2, |e| {
        e.service.confirmed = confirm;
        e.last_update = None;
    })));
    if let (Some(b), Some(data)) = (backup, f.data.clone()) {
        let b = b.to_path_buf();
        let shown = service::shown(&b);
        let res = off(cx, move || match Note::read(&b) {
            Some(n) if n.copy => core::restore_copy(&b, &data),
            Some(n) => core::restore_files(&b, &data, &n.replaces),
            None => Err("its backup has no note of what it holds".into()),
        })
        .await;
        match res {
            Ok(()) => notes.push(format!("Its data is as it was before the update (from the backup at {shown}; what was there is kept in it, under “{}”).", core::REPLACED_DIR)),
            Err(why) => notes.push(format!("Its data couldn't be put back by itself: {why} The backup is at {shown}.")),
        }
    } else if service && f.has_data {
        notes.push("Its data wasn't backed up, so it is as the new version left it.".into());
    }
    if let Some(action) = upgraded {
        let label = f.old.service.as_ref().map_or("Its server", |s| s.label.as_str());
        notes.push(format!(
            "{label}'s program was updated by “{action}”, and Pitwall can't put the older program back: if {} {} doesn't work with it, run that version's own install command by hand (its card lists it), or update again once a fixed release is out.",
            f.old.name,
            v(&f.old)
        ));
    }
    if service {
        hold(cx, &id, false).await;
        let st = settle(cx, &id, t.ready).await;
        let label = f.old.service.as_ref().map_or("Its service", |s| s.label.as_str());
        if st.up() {
            notes.push(format!("{label} runs again."));
        } else {
            notes.push(format!("{label} didn't start again: {}. Its card has Start.", state_words(&st)));
        }
    }
    notes
}

/// Step 5: each changed setup action, run again for every account, one
/// after another; the runs that failed, with why.
async fn setup_steps(cx: &mut AsyncApp, m: &Entity<SafeUpdates>, f: &Facts, jobs: &[Job]) -> Vec<(Job, String)> {
    let mut failed = Vec::new();
    let changed: Vec<String> = core::changed_setup(&f.old, &f.new, &f.set_up).iter().map(|a| a.id.clone()).collect();
    for action in changed {
        let step = Step::Setup(action.clone());
        let mine: Vec<Job> = jobs.iter().filter(|j| j.action == action).cloned().collect();
        let failed_here = run_jobs(cx, m, &f.id, &f.new, &step, &mine).await;
        failed.extend(failed_here);
    }
    failed
}

async fn run_jobs(cx: &mut AsyncApp, m: &Entity<SafeUpdates>, id: &str, new: &Manifest, step: &Step, jobs: &[Job]) -> Vec<(Job, String)> {
    m.update(cx, |u, cx| {
        u.set(id, step, StepState::Running);
        cx.notify();
    });
    let mut failed = Vec::new();
    for j in jobs {
        let Some(a) = new.actions.iter().find(|a| a.id == j.action).cloned() else { continue };
        // One already going (started by itself for an account) ends first.
        let key = connections::key_of(&a, j.conn.as_deref());
        loop {
            let (e, k) = (id.to_string(), key.clone());
            if !cx.update(|cx| actions::runs(cx).read(cx).running(&e, &k)) {
                break;
            }
            cx.background_executor().timer(POLL).await;
        }
        let (ext, new2, j2) = (id.to_string(), new.clone(), j.clone());
        let res = answer(cx, move |done, cx| {
            let conn = j2.conn.as_ref().and_then(|c| connections::list(&ext, cx).into_iter().find(|x| x.id == *c));
            let values = conn.as_ref().map(|c| c.values.clone()).unwrap_or_else(|| connections::settings_values(&ext, cx));
            let path = actions::steps_path(cx);
            let plan = actions::plan_for(&new2, &a, &values, path.as_deref(), &crate::accounts::tool_accounts(cx), true);
            let key = connections::key_of(&a, j2.conn.as_deref());
            let runs = actions::runs(cx);
            let (m2, action, conn) = (new2.clone(), a.id.clone(), j2.conn.clone());
            runs.update(cx, |r, cx| {
                r.start(
                    (&ext, &key),
                    plan,
                    pitwall_core::paths::home(),
                    actions::STEP_TIMEOUT,
                    move |st: &RunState, cx| {
                        crate::settings::extensions::record_run(&m2, &action, conn.as_deref(), st, cx);
                        done(st.clone(), cx);
                    },
                    cx,
                )
            });
            // The review listed these steps and Update confirmed them (an
            // account added later gets them by itself). After the start:
            // nothing starts them twice.
            connections::record_confirmed(&new2, &a, j2.conn.as_deref(), cx);
        })
        .await;
        let label = new.actions.iter().find(|a| a.id == j.action).map_or(j.action.clone(), |a| a.label.clone());
        let who = j.conn.as_ref().map(|c| format!(" ({c})")).unwrap_or_default();
        match res {
            Some(st) if !st.failed() && !st.cancelled => {}
            Some(st) => failed.push((j.clone(), format!("“{label}”{who}: {}", failed_places(&st)))),
            None => failed.push((j.clone(), format!("“{label}”{who}: it didn't finish"))),
        }
    }
    let state = if failed.is_empty() { StepState::Done(None) } else { StepState::Failed(failed.iter().map(|(_, w)| w.clone()).collect::<Vec<_>>().join("; ")) };
    m.update(cx, |u, cx| {
        u.set(id, step, state);
        cx.notify();
    });
    failed
}

/// Where a setup run failed: the accounts (or places) with a failed step,
/// and its summary.
fn failed_places(st: &RunState) -> String {
    let mut who: Vec<String> = Vec::new();
    for i in 0..st.steps.len() {
        if st.hard_failed(i) {
            let w = st.places.get(i).and_then(|p| p.0.clone()).unwrap_or_else(|| "once".into());
            if !who.contains(&w) {
                who.push(w);
            }
        }
    }
    let summary = st.summary().unwrap_or_default();
    if who.is_empty() {
        summary
    } else {
        format!("failed for {} — {summary}", who.join(", "))
    }
}

/// Retry: the setup runs that failed in `id`'s last update, again.
pub fn retry(id: &str, cx: &mut App) {
    let m = model(cx);
    let Some((jobs, new)) = m.read(cx).get(id).filter(|r| r.over()).and_then(|r| r.manifest.clone().map(|n| (r.retry.clone(), n))) else { return };
    if jobs.is_empty() {
        return;
    }
    m.update(cx, |u, cx| {
        if let Some(r) = u.runs.get_mut(id) {
            r.ending = None;
        }
        cx.notify();
    });
    let id = id.to_string();
    cx.spawn(async move |cx| {
        let mut failed = Vec::new();
        let mut actions_seen: Vec<String> = Vec::new();
        for j in &jobs {
            if actions_seen.contains(&j.action) {
                continue;
            }
            actions_seen.push(j.action.clone());
            let mine: Vec<Job> = jobs.iter().filter(|x| x.action == j.action).cloned().collect();
            failed.extend(run_jobs(cx, &m, &id, &new, &Step::Setup(j.action.clone()), &mine).await);
        }
        m.update(cx, |u, cx| {
            if let Some(r) = u.runs.get_mut(&id) {
                if failed.is_empty() {
                    r.retry.clear();
                    r.ending = Some(Ending::Done);
                } else {
                    r.ending = Some(Ending::SetupFailed(failed.iter().map(|(_, w)| w.clone()).collect()));
                    r.retry = failed.into_iter().map(|(j, _)| j).collect();
                }
            }
            cx.notify();
        });
    })
    .detach();
}

/// `m`'s service's commands, confirmed as they are without starting
/// anything (the update starts it when its turn comes).
fn confirm_quietly(m: &Manifest, cx: &mut App) {
    let Some(s) = &m.service else { return };
    let digest = pitwall_core::extensions::service::digest(s);
    let at = now_secs();
    crate::ui_state::update(cx, |u| {
        *u = std::mem::take(u).with_extension(&m.id, |e| e.service.confirmed = Some(crate::main_screen::workspace::ServiceConfirmed { digest, at }))
    });
}

/// The data backup taken at `id`'s last update, when Go back can offer
/// it: its folder and note (one Pitwall can put back by itself).
pub fn last_backup(id: &str, cx: &App) -> Option<(PathBuf, Note)> {
    let last = crate::ui_state::get(cx).extension(id).last_update?;
    let folder = PathBuf::from(last.backup?);
    let note = Note::read(&folder)?;
    note.restorable().then_some((folder, note))
}

/// Go back to `id`'s copy from before its last update; with
/// `restore_data`, its service's data is put back from the backup taken
/// then too (its service stopped meanwhile). `done` as
/// [`catalog::rollback_one`]'s.
pub fn go_back(id: &str, restore_data: bool, replace_changed: bool, cx: &mut App, done: impl FnOnce(Outcome, &mut App) + 'static) {
    let backup = if restore_data { last_backup(id, cx) } else { None };
    let data = service::data_dir(id, cx);
    let id = id.to_string();
    cx.spawn(async move |cx| {
        if backup.is_some() {
            hold(cx, &id, true).await;
        }
        let i2 = id.clone();
        let o = answer(cx, move |d, cx| catalog::rollback_one(&i2, replace_changed, cx, d)).await.unwrap_or(Outcome::Failed("It was stopped.".into()));
        let mut o = o;
        if o == Outcome::Done {
            // Its service's commands are the ones confirmed before the
            // update again: confirmed again.
            let i2 = id.clone();
            cx.update(|cx| {
                let prev = crate::ui_state::get(cx).extension(&i2).last_update.and_then(|l| l.confirmed);
                if let Some(m) = super::manifest_of(&i2, cx).filter(|m| m.service.as_ref().map(pitwall_core::extensions::service::digest) == prev) {
                    confirm_quietly(&m, cx);
                    let s = service::services(cx);
                    s.update(cx, |s, cx| s.reconcile(cx));
                }
            });
        }
        if let (Some((folder, note)), Some(data), Outcome::Done) = (backup.clone(), data, &o) {
            let res = off(cx, move || if note.copy { core::restore_copy(&folder, &data) } else { core::restore_files(&folder, &data, &note.replaces) }).await;
            if let Err(why) = res {
                o = Outcome::Failed(format!("Its previous files are back, but its data couldn't be put back: {why}"));
            }
        }
        if backup.is_some() {
            hold(cx, &id, false).await;
        }
        let i2 = id.clone();
        cx.update(|cx| {
            if o != Outcome::ChangedLocally {
                crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension(&i2, |e| e.last_update = None));
                let what = if restore_data { "go back: files and data put back" } else { "go back: files put back, data kept" };
                let failed = match &o {
                    Outcome::Failed(w) => Some(w.clone()),
                    _ => None,
                };
                activity(&i2, what, failed, None, cx);
            }
            done(o, cx)
        });
    })
    .detach();
}

/// A line in extension `ext`'s activity log about an update.
fn activity(ext: &str, what: &str, failed: Option<String>, detail: Option<String>, cx: &mut App) {
    use pitwall_core::extensions::activity::{Entry, Outcome as Out, Via};
    let outcome = match failed {
        None => Out::Ok,
        Some(message) => Out::Failed { code: "failed".into(), message },
    };
    let log = super::calls::activity(ext, cx);
    log.lock().unwrap_or_else(|e| e.into_inner()).push(Entry::now(Via::Pitwall, what, outcome).with_detail(detail));
}
