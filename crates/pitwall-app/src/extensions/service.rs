//! Extensions' services (docs/spec/extensions.md, "A service"): the one
//! long-running program an extension may declare
//! (`pitwall_core::extensions::service`), such as the server its agents
//! talk to. [`Services`] keeps each one running while its extension is on
//! and the user confirmed its exact commands (`ui.json`
//! `extensions.<id>.service.confirmed`, a digest of them): started at
//! start, when the extension is turned on or after an update; stopped
//! (asked with `SIGTERM`, its whole group killed after [`GRACE`]) when it
//! is turned off, updated, removed, its commands change, or Pitwall quits.
//!
//! Before each start Pitwall looks, off the UI thread: a setting it uses
//! not set, its address not on this computer, its address already taken
//! (something else runs it: that one is used, nothing is started and
//! nothing stopped), its program not installed, its version below the one
//! it needs. Then it starts it in its own process group with the minimal
//! environment (the login PATH, home, user, language) and the variables
//! the manifest names, its output going to the extension's Log with
//! secrets taken out. One that stops by itself is started again after 1,
//! 5 and 30 seconds; a fourth stop within ten minutes gives up and shows
//! its last lines, with Start.
//!
//! Its first run (a first sign-in) runs only on a press. With `opens`, the
//! first https address it prints is opened in the browser and kept
//! nowhere: not in the Log, the activity log or the card.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use gpui::{App, AppContext, Context, Entity, Global};
use pitwall_core::extensions::log::{self, Dir, Line};
use pitwall_core::extensions::process::{self, Process};
use pitwall_core::extensions::service::{self as core, EnvFrom, Probe, Ready, Service, Verdict};
use pitwall_core::extensions::Manifest;

/// How long a service has to stop when asked, before its group is killed.
pub const GRACE: Duration = Duration::from_secs(3);
/// How long Pitwall waits before starting one that stopped by itself
/// again: the first, second and third time.
pub const BACKOFF: [Duration; 3] = [Duration::from_secs(1), Duration::from_secs(5), Duration::from_secs(30)];
/// Stops by itself within this long count together; one more than
/// [`BACKOFF`] has gives up.
pub const WINDOW: Duration = Duration::from_secs(600);
/// How long its version command and its first run may take.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
pub const FIRST_RUN_TIMEOUT: Duration = Duration::from_secs(60);
/// How often Pitwall looks whether the one running elsewhere still is.
pub const ELSEWHERE_EVERY: Duration = Duration::from_secs(30);
/// The last lines of its output kept for "stopped: …".
const TAIL: usize = 6;

/// Where a service is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum State {
    /// Its extension is off (or it has none).
    #[default]
    Off,
    /// Its commands weren't confirmed as they are now (`changed`: other
    /// ones were, before an update).
    NeedsConfirm { changed: bool },
    /// The user stopped it.
    UserStopped,
    /// Looking before it starts.
    Checking,
    /// A setting it uses isn't set (its label).
    NeedsSetting(String),
    /// Its address isn't on this computer (why).
    BadAddress(String),
    /// Its program isn't installed (its name).
    NotInstalled(String),
    /// Its version is below the one it needs.
    TooOld { found: String, need: String },
    Starting,
    Running { since: SystemTime },
    /// Something else listens on its address (it): used, never stopped.
    Elsewhere(String),
    /// It stopped by itself and starts again (the how-many-th time).
    Restarting { attempt: usize },
    /// It couldn't start, or kept stopping and Pitwall gave up: why, with
    /// its last lines.
    Stopped(String),
    /// Stopped while its extension is updated
    /// (`crate::extensions::safe_update`): started again by the update.
    Updating,
}

impl State {
    /// It is up, Pitwall's or another.
    pub fn up(&self) -> bool {
        matches!(self, State::Running { .. } | State::Elsewhere(_))
    }
}

/// How a first run went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstRun {
    Running,
    /// Done: what it did, in plain words (never its link).
    Done(String),
    Failed(String),
}

/// A started service's process, shared with the threads that watch it
/// (taken out when it is stopped).
type Child = Arc<Mutex<Option<Process>>>;

/// One extension's service.
#[derive(Default)]
pub struct Svc {
    pub state: State,
    /// Its version, as its version command said.
    pub version: Option<String>,
    /// Its program was found on the PATH when last looked.
    pub installed: Option<bool>,
    pub first_run: Option<FirstRun>,
    child: Option<Child>,
    /// A process being stopped: the next start waits for it.
    stopping: Option<Child>,
    gen: u64,
    fingerprint: Option<u64>,
    /// When it stopped by itself lately.
    crashes: VecDeque<Instant>,
}

/// Where a start's threads leave what they tell, for the UI thread to
/// pick up (a thread never wakes it).
#[derive(Clone, Default)]
struct Tx(Arc<Mutex<VecDeque<Ev>>>);

impl Tx {
    fn send(&self, ev: Ev) -> Result<(), ()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push_back(ev);
        Ok(())
    }
}

/// How often what a service's threads told is looked at.
const POLL: Duration = Duration::from_millis(100);

/// What a start's thread tells.
enum Ev {
    /// What was found, and whether it was started (its process) or why
    /// not.
    Checked { verdict: Verdict, installed: bool, fresh: Option<bool>, started: Result<Option<Child>, String> },
    Ready,
    /// It exited (code, `None`: by a signal), with its last lines.
    Exited { code: Option<i32>, tail: String },
}

/// Every extension's service.
#[derive(Default)]
pub struct Services {
    svcs: HashMap<String, Svc>,
    logs: HashMap<String, log::Shared>,
    next_gen: u64,
    /// Extensions being updated: their services stay stopped until the
    /// update lets them go ([`Services::hold`]).
    held: std::collections::HashSet<String>,
}

#[derive(Clone)]
struct Handle(Entity<Services>);

impl Global for Handle {}

/// The app's services (made when there are none).
pub fn services(cx: &mut App) -> Entity<Services> {
    if let Some(h) = cx.try_global::<Handle>() {
        return h.0.clone();
    }
    let e = cx.new(|_| Services { next_gen: 1, ..Default::default() });
    cx.set_global(Handle(e.clone()));
    e
}

/// The services, if any (drawing code).
pub fn try_services(cx: &App) -> Option<Entity<Services>> {
    cx.try_global::<Handle>().map(|h| h.0.clone())
}

/// Tests: what opens a first run's link instead of the browser.
#[derive(Clone)]
pub struct OpenLink(pub Opener);

/// What opens a link.
pub type Opener = std::rc::Rc<dyn Fn(&str, &mut App)>;

impl Global for OpenLink {}

/// Extension `id`'s service's private data folder (its `{data}`).
pub fn data_dir(id: &str, cx: &App) -> Option<PathBuf> {
    let root = super::dir(cx)?.parent()?.to_path_buf();
    Some(pitwall_core::paths::Paths::new(root).extension_data_dir(id))
}

/// `data` as the user reads it.
pub fn shown(data: &Path) -> String {
    pitwall_core::paths::tildify(&data.to_string_lossy())
}

/// Whether `m`'s service's commands as they are now were confirmed;
/// `Err(changed)`: not (`changed`: others were).
pub fn confirmed(m: &Manifest, cx: &App) -> Result<(), bool> {
    let Some(s) = &m.service else { return Ok(()) };
    match crate::ui_state::get(cx).extension(&m.id).service.confirmed {
        Some(c) if c.digest == core::digest(s) => Ok(()),
        Some(_) => Err(true),
        None => Err(false),
    }
}

/// The user confirmed `m`'s service's commands as they are: it starts.
pub fn confirm(m: &Manifest, cx: &mut App) {
    let Some(s) = &m.service else { return };
    let digest = core::digest(s);
    let at = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    crate::ui_state::update(cx, |u| {
        *u = std::mem::take(u).with_extension(&m.id, |e| {
            e.service.confirmed = Some(crate::main_screen::workspace::ServiceConfirmed { digest, at });
            e.service.stopped = false;
        })
    });
    activity(&m.id, "service: confirmed", None, None, cx);
    let s = services(cx);
    s.update(cx, |s, cx| s.reconcile(cx));
}

/// Start (`true`) or stop the service of extension `id` by the user's
/// press; kept across starts.
pub fn set_stopped(id: &str, stopped: bool, cx: &mut App) {
    crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension(id, |e| e.service.stopped = stopped));
    let s = services(cx);
    s.update(cx, |s, cx| {
        if let Some(v) = s.svcs.get_mut(id) {
            v.fingerprint = None;
            v.crashes.clear();
        }
        s.reconcile(cx)
    });
}

/// Look again and start it again (after its program was installed, or
/// Restart).
pub fn recheck(id: &str, cx: &mut App) {
    let s = services(cx);
    s.update(cx, |s, cx| s.restart(id, cx));
}

/// A line in extension `ext`'s activity log about its service.
fn activity(ext: &str, what: &str, failed: Option<String>, detail: Option<String>, cx: &mut App) {
    use pitwall_core::extensions::activity::{Entry, Outcome, Via};
    let outcome = match failed {
        None => Outcome::Ok,
        Some(message) => Outcome::Failed { code: "failed".into(), message },
    };
    let log = super::calls::activity(ext, cx);
    log.lock().unwrap_or_else(|e| e.into_inner()).push(Entry::now(Via::Pitwall, what, outcome).with_detail(detail));
}

/// What one start runs with, gathered on the UI thread.
struct Plan {
    m: Manifest,
    s: Service,
    values: std::collections::BTreeMap<String, String>,
    /// Secret variables: name, and where the secret is kept.
    secrets: Vec<(String, String)>,
    store: Arc<dyn crate::platform::secrets::SecretStore>,
    folder: PathBuf,
    data: PathBuf,
    path: Option<String>,
}

impl Services {
    /// Extension `id`'s service.
    pub fn get(&self, id: &str) -> Option<&Svc> {
        self.svcs.get(id)
    }

    /// Its log's lines, oldest first.
    pub fn log(&self, id: &str) -> Vec<Line> {
        self.logs.get(id).map(|l| l.lock().unwrap_or_else(|e| e.into_inner()).lines()).unwrap_or_default()
    }

    fn note(&mut self, id: &str, text: &str) {
        self.logs.entry(id.to_string()).or_default().lock().unwrap_or_else(|e| e.into_inner()).push(Dir::Note, text);
    }

    /// Start or stop each service to match its extension's switch, its
    /// confirmation, the user's Stop and its settings.
    pub fn reconcile(&mut self, cx: &mut Context<Self>) {
        let found = super::found(cx);
        let ui = crate::ui_state::get(cx);
        let path = super::actions::steps_path(cx);
        let mut seen = Vec::new();
        for m in found.iter().filter_map(|f| f.manifest.as_ref().ok()) {
            let Some(s) = &m.service else { continue };
            seen.push(m.id.clone());
            let on = ui.extension_on(&m.id);
            let prefs = ui.extension(&m.id).service;
            let installed = pitwall_core::extensions::plan::resolve(Path::new("."), &s.run[0], path.as_deref()).is_ok();
            let want = if !on {
                Err(State::Off)
            } else if self.held.contains(&m.id) {
                Err(State::Updating)
            } else if let Err(changed) = confirmed(m, cx) {
                Err(State::NeedsConfirm { changed })
            } else if prefs.stopped {
                Err(State::UserStopped)
            } else {
                Ok(())
            };
            let st = self.svcs.entry(m.id.clone()).or_default();
            st.installed = Some(installed);
            match want {
                Err(state) => {
                    if st.child.is_some() {
                        let why = match &state {
                            State::Off => "Stopped: the extension was turned off.",
                            State::Updating => "Stopped for the extension's update.",
                            State::NeedsConfirm { .. } => "Stopped: its commands changed; confirm them to start it again.",
                            _ => "Stopped by you.",
                        };
                        Self::stop_child(st);
                        self.note(&m.id, why);
                        activity(&m.id, "service: stopped", None, Some(why.to_string()), cx);
                    }
                    let st = self.svcs.entry(m.id.clone()).or_default();
                    st.gen = 0;
                    st.fingerprint = None;
                    st.state = state;
                    if !on {
                        st.first_run = None;
                        st.crashes.clear();
                        // Off: its log is forgotten, as an adapter's.
                        self.logs.remove(&m.id);
                    }
                }
                Ok(()) => {
                    let Some(plan) = Self::plan(m, s, cx) else { continue };
                    let fp = fingerprint(&plan);
                    if st.fingerprint != Some(fp) {
                        st.fingerprint = Some(fp);
                        st.crashes.clear();
                        self.start(plan, cx);
                    }
                }
            }
        }
        // An extension removed, or one without a service any more.
        let gone: Vec<String> = self.svcs.keys().filter(|k| !seen.contains(k)).cloned().collect();
        for id in gone {
            if let Some(mut st) = self.svcs.remove(&id) {
                Self::stop_child(&mut st);
            }
            self.logs.remove(&id);
        }
        cx.notify();
    }

    /// Stop it now and start it again (Restart, an install that ended).
    pub fn restart(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(st) = self.svcs.get_mut(id) {
            if st.child.is_some() {
                Self::stop_child(st);
                self.note(id, "Restarting.");
            }
            let st = self.svcs.entry(id.to_string()).or_default();
            st.fingerprint = None;
            st.crashes.clear();
            st.gen = 0;
        }
        self.reconcile(cx);
    }

    /// Keep extension `id`'s service stopped (`true`: stopped now, as
    /// while it is updated) or let it start again (`false`, looked at
    /// afresh). Returns the process being stopped, to wait for.
    pub fn hold(&mut self, id: &str, held: bool, cx: &mut Context<Self>) -> Option<Arc<Mutex<Option<Process>>>> {
        if held {
            self.held.insert(id.to_string());
        } else if self.held.remove(id) {
            if let Some(st) = self.svcs.get_mut(id) {
                st.fingerprint = None;
                st.crashes.clear();
            }
        }
        self.reconcile(cx);
        self.stopping(id)
    }

    /// The process being stopped for extension `id` (removing it waits for
    /// it before deleting its data).
    pub fn stopping(&self, id: &str) -> Option<Arc<Mutex<Option<Process>>>> {
        self.svcs.get(id).and_then(|s| s.stopping.clone().or_else(|| s.child.clone()))
    }

    /// What a start of `m`'s service runs with.
    fn plan(m: &Manifest, s: &Service, cx: &mut Context<Self>) -> Option<Plan> {
        let folder = super::folder_of(&m.id, cx)?;
        let data = data_dir(&m.id, cx)?;
        let target = super::connections::settings_target(&m.id, cx);
        let values = target.as_ref().map(|c| c.values.clone()).unwrap_or_default();
        let mut secrets = Vec::new();
        for e in &s.env {
            if let EnvFrom::Secret(key) = &e.from {
                if let Some(c) = target.as_ref().filter(|c| c.secrets_set.contains(key)) {
                    secrets.push((e.name.clone(), super::account(c.secrets_name(), key, cx)));
                }
            }
        }
        Some(Plan { m: m.clone(), s: s.clone(), values, secrets, store: super::secret_store(cx), folder, data, path: super::actions::steps_path(cx) })
    }

    /// Look, then start it when nothing stands in the way; off the UI
    /// thread.
    fn start(&mut self, plan: Plan, cx: &mut Context<Self>) {
        let id = plan.m.id.clone();
        let gen = self.next_gen;
        self.next_gen += 1;
        let log = self.logs.entry(id.clone()).or_default().clone();
        let st = self.svcs.entry(id.clone()).or_default();
        if st.child.is_some() {
            Self::stop_child(st);
        }
        st.gen = gen;
        st.state = State::Checking;
        let prev = st.stopping.take();
        let tx = Tx::default();
        let rx = tx.0.clone();
        std::thread::spawn(move || check_and_start(plan, prev, log, tx));
        // What its threads tell, looked at every [`POLL`] until they all
        // ended (a process started late is stopped by `event`).
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(POLL).await;
            let (evs, over) = {
                let mut q = rx.lock().unwrap_or_else(|e| e.into_inner());
                (q.drain(..).collect::<Vec<_>>(), Arc::strong_count(&rx) == 1)
            };
            for ev in evs {
                if this.update(cx, |s, cx| s.event(&id, gen, ev, cx)).is_err() {
                    return;
                }
            }
            if over {
                return;
            }
        })
        .detach();
        cx.notify();
    }

    fn event(&mut self, id: &str, gen: u64, ev: Ev, cx: &mut Context<Self>) {
        let Some(st) = self.svcs.get_mut(id) else { return };
        if st.gen != gen {
            // A start that was stopped meanwhile: its process goes too.
            if let Ev::Checked { started: Ok(Some(child)), .. } = ev {
                stop_in_background(child);
            }
            return;
        }
        match ev {
            Ev::Checked { verdict, installed, fresh, started } => {
                st.installed = Some(installed);
                match (&verdict, started) {
                    (_, Err(why)) => {
                        st.state = State::Stopped(why.clone());
                        self.note(id, &why);
                        activity(id, "service: couldn't start", Some(why), None, cx);
                    }
                    (Verdict::Start { version }, Ok(Some(child))) => {
                        st.version = version.clone();
                        st.child = Some(child);
                        if st.state == State::Checking || matches!(st.state, State::Restarting { .. }) {
                            st.state = State::Starting;
                        }
                        if fresh == Some(true) && !crate::ui_state::get(cx).extension(id).service.fresh {
                            crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension(id, |e| e.service.fresh = true));
                        }
                    }
                    (Verdict::Start { .. }, Ok(None)) => st.state = State::Stopped("It couldn't start.".into()),
                    (v, _) => {
                        st.state = match v.clone() {
                            Verdict::NeedsSetting(l) => State::NeedsSetting(l),
                            Verdict::BadAddress(why) => State::BadAddress(why),
                            Verdict::NotInstalled(p) => State::NotInstalled(p),
                            Verdict::TooOld { found, need } => {
                                st.version = Some(found.clone());
                                State::TooOld { found, need }
                            }
                            Verdict::Elsewhere(a) => State::Elsewhere(a),
                            Verdict::Start { .. } => unreachable!(),
                        };
                        if let State::Elsewhere(a) = &st.state {
                            let a = a.clone();
                            self.note(id, &format!("Something else is already listening on {a}: Pitwall uses it and starts nothing."));
                            self.watch_elsewhere(id, gen, a, cx);
                        }
                    }
                }
            }
            Ev::Ready => {
                if matches!(st.state, State::Starting) {
                    st.state = State::Running { since: SystemTime::now() };
                    let v = st.version.clone();
                    self.note(id, "Running.");
                    activity(id, "service: running", None, v.map(|v| format!("version {v}")), cx);
                }
            }
            Ev::Exited { code, tail } => {
                st.child = None;
                let why = match code {
                    Some(0) => "It exited.".to_string(),
                    Some(n) => format!("It exited with code {n}."),
                    None => "It was stopped by a signal.".to_string(),
                };
                let now = Instant::now();
                st.crashes.push_back(now);
                while st.crashes.front().is_some_and(|t| now.duration_since(*t) > WINDOW) {
                    st.crashes.pop_front();
                }
                let n = st.crashes.len();
                if n > BACKOFF.len() {
                    let last = if tail.is_empty() { String::new() } else { format!(" Its last lines:\n{tail}") };
                    let text = format!("{why} It stopped {n} times in a few minutes, so Pitwall stopped starting it.{last}");
                    st.state = State::Stopped(text);
                    st.gen = 0;
                    self.note(id, &format!("{why} Gave up after {n} stops."));
                    activity(id, "service: gave up", Some(why), None, cx);
                } else {
                    let wait = BACKOFF[n - 1];
                    st.state = State::Restarting { attempt: n };
                    self.note(id, &format!("{why} Starting it again in {} s.", wait.as_secs()));
                    let id = id.to_string();
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(wait).await;
                        let _ = this.update(cx, |s, cx| s.again(&id, gen, cx));
                    })
                    .detach();
                }
            }
        }
        cx.notify();
    }

    /// Start it again after it stopped by itself (when nothing changed
    /// meanwhile).
    fn again(&mut self, id: &str, gen: u64, cx: &mut Context<Self>) {
        let Some(st) = self.svcs.get(id) else { return };
        if st.gen != gen || !matches!(st.state, State::Restarting { .. }) {
            return;
        }
        let Some(m) = super::manifest_of(id, cx) else { return };
        let Some(s) = m.service.clone() else { return };
        if let Some(plan) = Self::plan(&m, &s, cx) {
            self.start(plan, cx);
            if let Some(st) = self.svcs.get_mut(id) {
                // It stays "starting again" until it is up.
                st.state = State::Restarting { attempt: st.crashes.len() };
            }
        }
    }

    /// While another program listens on its address: looked at again
    /// every [`ELSEWHERE_EVERY`] (bound, never connected to); once nothing
    /// listens, it shows as stopped, with Start.
    fn watch_elsewhere(&mut self, id: &str, gen: u64, addr: String, cx: &mut Context<Self>) {
        let id = id.to_string();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(ELSEWHERE_EVERY).await;
            let a = addr.clone();
            let taken = cx.background_executor().spawn(async move { a.parse().map(core::taken).unwrap_or(false) }).await;
            let more = this
                .update(cx, |s, cx| {
                    let Some(st) = s.svcs.get_mut(&id) else { return false };
                    if st.gen != gen || !matches!(st.state, State::Elsewhere(_)) {
                        return false;
                    }
                    if taken {
                        return true;
                    }
                    st.state = State::Stopped(format!("Nothing is listening on {addr} any more."));
                    st.fingerprint = None;
                    s.note(&id, &format!("Nothing is listening on {addr} any more."));
                    cx.notify();
                    false
                })
                .unwrap_or(false);
            if !more {
                return;
            }
        })
        .detach();
    }

    /// Ask its process to stop, its group killed after [`GRACE`]; kept as
    /// `stopping` until then.
    fn stop_child(st: &mut Svc) {
        st.gen = 0;
        if let Some(child) = st.child.take() {
            st.stopping = Some(child.clone());
            stop_in_background(child);
        }
        if matches!(st.state, State::Running { .. } | State::Starting | State::Restarting { .. } | State::Checking) {
            st.state = State::Off;
        }
    }

    /// Pitwall quits: each is asked to stop, and what is left after
    /// [`GRACE`] is killed with its group.
    pub fn quit(&mut self) {
        let children: Vec<Child> = self.svcs.values_mut().filter_map(|s| s.child.take()).collect();
        for c in &children {
            if let Some(p) = c.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                p.terminate();
            }
        }
        let end = Instant::now() + GRACE;
        while Instant::now() < end && children.iter().any(|c| c.lock().unwrap_or_else(|e| e.into_inner()).as_mut().is_some_and(|p| p.exited().is_none())) {
            std::thread::sleep(Duration::from_millis(50));
        }
        for c in children {
            if let Some(mut p) = c.lock().unwrap_or_else(|e| e.into_inner()).take() {
                p.stop();
            }
        }
    }

    /// Run `m`'s first run (on a press), off the UI thread. With `opens`,
    /// the first https address it prints is opened in the browser; it is
    /// never logged, shown or kept.
    pub fn first_run(&mut self, m: &Manifest, cx: &mut Context<Self>) {
        let Some(s) = m.service.clone() else { return };
        let Some(f) = s.first_run.clone() else { return };
        let id = m.id.clone();
        if matches!(self.svcs.get(&id).and_then(|s| s.first_run.as_ref()), Some(FirstRun::Running)) {
            return;
        }
        let Some(plan) = Self::plan(m, &s, cx) else { return };
        self.svcs.entry(id.clone()).or_default().first_run = Some(FirstRun::Running);
        self.note(&id, &format!("Running “{}”.", f.label));
        let log = self.logs.entry(id.clone()).or_default().clone();
        let out: Arc<Mutex<Option<FirstResult>>> = Arc::default();
        let worker = out.clone();
        std::thread::spawn(move || {
            let res = run_first(&plan, &f, &log);
            *worker.lock().unwrap_or_else(|e| e.into_inner()) = Some(res);
        });
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(POLL).await;
            let res = out.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(res) = res {
                let _ = this.update(cx, |s, cx| s.first_run_done(&id, res, cx));
                return;
            }
        })
        .detach();
        cx.notify();
    }

    fn first_run_done(&mut self, id: &str, res: Result<Option<String>, String>, cx: &mut Context<Self>) {
        let opens = super::manifest_of(id, cx).and_then(|m| m.service).and_then(|s| s.first_run).is_some_and(|f| f.opens);
        let state = match res {
            Ok(Some(url)) => {
                // The link goes to the browser only; what is kept says
                // that it was opened.
                match cx.try_global::<OpenLink>().cloned() {
                    Some(hook) => (hook.0)(&url, cx),
                    None => crate::platform::files::open_url(cx, &url),
                }
                FirstRun::Done("Opened a one-time sign-in link in your browser.".into())
            }
            Ok(None) if opens => FirstRun::Failed("It printed no https link to open.".into()),
            Ok(None) => FirstRun::Done("Done.".into()),
            Err(why) => FirstRun::Failed(why),
        };
        match &state {
            FirstRun::Done(what) => {
                self.note(id, what);
                activity(id, "service: first run", None, Some(what.clone()), cx);
                crate::ui_state::update(cx, |u| *u = std::mem::take(u).with_extension(id, |e| e.service.fresh = false));
            }
            FirstRun::Failed(why) => {
                self.note(id, &format!("The first run failed: {why}"));
                activity(id, "service: first run", Some(why.clone()), None, cx);
            }
            FirstRun::Running => {}
        }
        self.svcs.entry(id.to_string()).or_default().first_run = Some(state);
        cx.notify();
    }
}

/// Variables, by name.
type Env = Vec<(String, String)>;

/// How a first run ended: the link it printed (with `opens`), or why
/// it failed.
type FirstResult = Result<Option<String>, String>;

/// What a start depends on: its commands with the settings put in, its
/// variables and secrets' places, its folders and the PATH.
fn fingerprint(p: &Plan) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let c = core::commands(&p.m, &p.s, &p.values, &p.data.to_string_lossy());
    (&p.m.version, core::digest(&p.s), &c.run, &c.env, &p.secrets, &p.folder, &p.data, &p.path).hash(&mut h);
    h.finish()
}

/// Ask `child`'s group to stop, and kill it after [`GRACE`]; on a thread
/// of its own.
fn stop_in_background(child: Child) {
    std::thread::spawn(move || {
        let asked = child.lock().unwrap_or_else(|e| e.into_inner()).as_mut().is_some_and(Process::terminate);
        if asked {
            let end = Instant::now() + GRACE;
            while Instant::now() < end {
                if child.lock().unwrap_or_else(|e| e.into_inner()).as_mut().is_none_or(|p| p.exited().is_some()) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        if let Some(mut p) = child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            p.stop();
        }
    });
}

/// The environment a service (and its first run) gets: the minimal one,
/// and its variables (secrets read from the store); the secrets' values,
/// to take out of its output.
fn environment(p: &Plan) -> Result<(Env, Vec<String>), String> {
    let mut env = core::minimal_env(std::env::vars(), p.path.as_deref());
    let mut secrets = Vec::new();
    for e in &p.s.env {
        match &e.from {
            EnvFrom::Value(v) => {
                if let Ok(v) = core::expand_one(v, &p.m, &p.values, &p.data.to_string_lossy()) {
                    env.push((e.name.clone(), v));
                }
            }
            EnvFrom::Secret(_) => {
                let Some((_, account)) = p.secrets.iter().find(|(n, _)| *n == e.name) else { continue };
                match p.store.get(account) {
                    Ok(Some(v)) => {
                        secrets.push(v.clone());
                        env.push((e.name.clone(), v));
                    }
                    Ok(None) => return Err("A secret it needs is gone from the system's secure store: set it again in Settings.".into()),
                    Err(e) => return Err(e),
                }
            }
        }
    }
    Ok((env, secrets))
}

/// Run `argv` (found already as `program`) with `env` until it ends or
/// `timeout`: its exit code and output, both streams together.
fn run_once(program: &Path, argv: &[String], dir: &Path, env: &[(String, String)], timeout: Duration) -> Result<(Option<i32>, String), String> {
    let (mut p, out, err) = process::start_service(program, &argv[1..], dir, env).map_err(|e| format!("It couldn't start: {e}"))?;
    let text: Arc<Mutex<Vec<u8>>> = Arc::default();
    let readers: Vec<_> = [out, err]
        .into_iter()
        .map(|mut r| {
            let text = text.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 2048];
                while let Ok(n) = r.read(&mut buf) {
                    if n == 0 {
                        return;
                    }
                    let mut t = text.lock().unwrap_or_else(|e| e.into_inner());
                    if t.len() < 256 * 1024 {
                        t.extend_from_slice(&buf[..n]);
                    }
                }
            })
        })
        .collect();
    let end = Instant::now() + timeout;
    let code = loop {
        if let Some(c) = p.exited() {
            break Some(c);
        }
        if Instant::now() > end {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    p.stop();
    let Some(code) = code else { return Err(format!("It took longer than {} s and was stopped.", timeout.as_secs())) };
    for r in readers {
        let _ = r.join();
    }
    let text = String::from_utf8_lossy(&text.lock().unwrap_or_else(|e| e.into_inner())).into_owned();
    Ok((code, text))
}

/// The last `n` lines of `text`, cleaned.
fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
    pitwall_core::extensions::protocol::clean(&lines[lines.len().saturating_sub(n)..].join("\n"), 1200, true)
}

/// The first run, on a thread: the link it printed (with `opens`), or why
/// it failed (its output with every link taken out).
fn run_first(p: &Plan, f: &core::FirstRun, log: &log::Shared) -> Result<Option<String>, String> {
    let data = p.data.to_string_lossy().into_owned();
    let argv = core::expand(&f.run, &p.m, &p.values, &data).map_err(|label| format!("Set {label} first."))?;
    let program = pitwall_core::extensions::plan::resolve(&p.folder, &argv[0], p.path.as_deref())?;
    let (env, secrets) = environment(p)?;
    core::prepare_data(&p.data).map_err(|e| format!("Its data folder can't be made: {e}"))?;
    log.lock().unwrap_or_else(|e| e.into_inner()).set_secrets(secrets.clone());
    let (code, text) = run_once(&program, &argv, &p.folder, &env, FIRST_RUN_TIMEOUT)?;
    // Nothing it printed is kept with a link in it.
    let scrubbed = log::redact(&core::scrub_links(&text), &secrets);
    match code {
        Some(0) => Ok(if f.opens { core::first_https_url(&text) } else { None }),
        Some(n) => Err(format!("It exited with code {n}.{}", tail_of(&scrubbed))),
        None => Err(format!("It was stopped by a signal.{}", tail_of(&scrubbed))),
    }
}

fn tail_of(text: &str) -> String {
    let t = last_lines(text, TAIL);
    if t.is_empty() {
        String::new()
    } else {
        format!(" It said:\n{t}")
    }
}

/// Before a start, on a thread: wait for the one being stopped, look
/// (settings, address, program, version), then start it and watch it.
fn check_and_start(p: Plan, prev: Option<Child>, log: log::Shared, tx: Tx) {
    if let Some(prev) = prev {
        let end = Instant::now() + GRACE + Duration::from_secs(2);
        while Instant::now() < end && prev.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let note = |text: &str| log.lock().unwrap_or_else(|e| e.into_inner()).push(Dir::Note, text);
    let data = p.data.to_string_lossy().into_owned();
    let program = pitwall_core::extensions::plan::resolve(&p.folder, &p.s.run[0], p.path.as_deref());
    let addr = core::listen_of(&p.m, &p.s, &p.values).and_then(Result::ok);
    let port_taken = addr.is_some_and(core::taken);
    let mut probe = Probe { installed: program.is_ok(), version_output: None, port_taken };
    if let (Ok(_), Some(v), false) = (&program, &p.s.version, port_taken) {
        if let Ok(argv) = core::expand(&v.run, &p.m, &p.values, &data) {
            if let Ok(prog) = pitwall_core::extensions::plan::resolve(&p.folder, &argv[0], p.path.as_deref()) {
                let env = core::minimal_env(std::env::vars(), p.path.as_deref());
                match run_once(&prog, &argv, &p.folder, &env, PROBE_TIMEOUT) {
                    Ok((_, out)) => probe.version_output = Some(out),
                    Err(why) => note(&format!("Its version couldn't be told: {why}")),
                }
            }
        }
    }
    let verdict = core::verdict(&p.m, &p.s, &p.values, &probe);
    let installed = probe.installed;
    let Verdict::Start { .. } = &verdict else {
        let _ = tx.send(Ev::Checked { verdict, installed, fresh: None, started: Ok(None) });
        return;
    };
    let fail = |why: String| {
        let _ = tx.send(Ev::Checked { verdict: verdict.clone(), installed, fresh: None, started: Err(why) });
    };
    let (env, secrets) = match environment(&p) {
        Ok(e) => e,
        Err(why) => return fail(why),
    };
    let fresh = match core::prepare_data(&p.data) {
        Ok(f) => f,
        Err(e) => return fail(format!("Its data folder {} can't be made: {e}", shown(&p.data))),
    };
    let argv = match core::expand(&p.s.run, &p.m, &p.values, &data) {
        Ok(a) => a,
        Err(label) => return fail(format!("Set {label} first.")),
    };
    let program = match program {
        Ok(p) => p,
        Err(why) => return fail(why),
    };
    log.lock().unwrap_or_else(|e| e.into_inner()).set_secrets(secrets.clone());
    note(&format!("Starting {}", pitwall_core::extensions::plan::line(&argv)));
    let (proc_, out, err) = match process::start_service(&program, &argv[1..], &p.folder, &env) {
        Ok(x) => x,
        Err(e) => return fail(format!("It couldn't start: {e}")),
    };
    let child: Child = Arc::new(Mutex::new(Some(proc_)));
    let _ = tx.send(Ev::Checked { verdict: verdict.clone(), installed, fresh: Some(fresh), started: Ok(Some(child.clone())) });
    // Its output: into the log (secrets out), its last lines kept, and a
    // `ready.line` looked for.
    let tail: Arc<Mutex<VecDeque<String>>> = Arc::default();
    let ready_sent = Arc::new(AtomicBool::new(false));
    let line_re = match &p.s.ready {
        Ready::Line(pat) => regex::Regex::new(pat).ok(),
        _ => None,
    };
    let mut readers = Vec::new();
    for (r, dir) in [(out, Dir::FromAdapter), (err, Dir::Stderr)] {
        let (log, tail, tx, ready_sent, re, secrets) = (log.clone(), tail.clone(), tx.clone(), ready_sent.clone(), line_re.clone(), secrets.clone());
        readers.push(std::thread::spawn(move || {
            let mut r = BufReader::new(r);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match r.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
                let text = String::from_utf8_lossy(&buf).trim_end().to_string();
                log.lock().unwrap_or_else(|e| e.into_inner()).push(dir, &text);
                {
                    let mut t = tail.lock().unwrap_or_else(|e| e.into_inner());
                    t.push_back(log::redact(&text, &secrets));
                    while t.len() > TAIL {
                        t.pop_front();
                    }
                }
                if re.as_ref().is_some_and(|re| re.is_match(&text)) && !ready_sent.swap(true, Ordering::SeqCst) {
                    let _ = tx.send(Ev::Ready);
                }
            }
        }));
    }
    // Up once its address answers (it is Pitwall's own: the address was
    // free before it started), or once it has run a moment.
    if !matches!(p.s.ready, Ready::Line(_)) {
        let (child, tx, ready_sent) = (child.clone(), tx.clone(), ready_sent.clone());
        std::thread::spawn(move || {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(120) {
                std::thread::sleep(Duration::from_millis(200));
                let alive = child.lock().unwrap_or_else(|e| e.into_inner()).as_mut().is_some_and(|p| p.exited().is_none());
                if !alive {
                    return;
                }
                let up = match addr {
                    Some(a) => std::net::TcpStream::connect_timeout(&a, Duration::from_millis(300)).is_ok(),
                    None => start.elapsed() >= Duration::from_secs(1),
                };
                if up {
                    if !ready_sent.swap(true, Ordering::SeqCst) {
                        let _ = tx.send(Ev::Ready);
                    }
                    return;
                }
            }
        });
    }
    // Its end: told with its last lines, unless Pitwall stopped it (then
    // it is taken out first).
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let mut c = child.lock().unwrap_or_else(|e| e.into_inner());
        let Some(proc_) = c.as_mut() else { return };
        if let Some(code) = proc_.exited() {
            proc_.stop();
            *c = None;
            drop(c);
            // What it printed last, a moment to arrive.
            let end = Instant::now() + Duration::from_millis(500);
            while Instant::now() < end && readers.iter().any(|r| !r.is_finished()) {
                std::thread::sleep(Duration::from_millis(20));
            }
            let tail = tail.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect::<Vec<_>>().join("\n");
            let _ = tx.send(Ev::Exited { code, tail });
            return;
        }
    }
}

