//! The app's side of `extension.*` (docs/spec/extensions.md, "Setup by an
//! agent"): an agent sees each extension's card as data and moves its
//! setup forward, each step approved by the user in Pitwall.
//!
//! The socket server calls [`AppExtensions`] on its own threads; each
//! request runs on the main thread ([`attach`]) with exactly the code
//! Settings' buttons use: the checklist (`settings::extensions::checklist`),
//! an action's plan and run (`plan_action`, `start_action`), the service's
//! confirm, start, stop and first run (`extensions::service`), install,
//! update and remove (`extensions::catalog`), turning on with the
//! permission screen's picks (`turn_on_with`), connections and plain
//! settings (`extensions::connections`).
//!
//! [`ExtensionsBackend::prepare`] works out what will run (the lines the
//! approval shows) and keeps it under a token; [`ExtensionsBackend::run`]
//! runs that, not a plan made again, and waits for how it went. Nothing
//! secret is gathered here: a connection's secrets are only "set" or not,
//! and a first run's link goes from the service to the browser only.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::StreamExt;
use gpui::{App, Global};

use pitwall_core::accounts::Account;
use pitwall_core::extensions::install::{self, Origin, Status};
use pitwall_core::extensions::service as core;
use pitwall_core::extensions::{Action, Manifest, SettingKind};
use pitwall_daemon::extension_setup::{Prepared, BUSY};
use pitwall_daemon::ExtensionsBackend;
use pitwall_proto::extensions::*;
use pitwall_proto::{code, ErrorBody, Risk};

use super::actions::{self, Planned, RunState};
use super::catalog;
use super::connections;
use super::hub::{self, Run};
use super::service::{self, FirstRun, State};
use crate::settings::extensions::{card_status, checklist, plan_action, plan_rows, run_key, start_action, url_value, ServiceFacts, Step};

/// How long a request waits for the main thread.
const ANSWER_WITHIN: Duration = Duration::from_secs(10);
/// How often a wait looks again.
const POLL: Duration = Duration::from_millis(150);
/// The longest a setup run is waited for before the answer says it still
/// runs.
const RUN_WITHIN: Duration = Duration::from_secs(9 * 60);
/// The longest a service is waited for to settle.
const SERVICE_WITHIN: Duration = Duration::from_secs(60);
/// The longest an install, update, remove or source check is waited for.
const FETCH_WITHIN: Duration = Duration::from_secs(150);

type Job = Box<dyn FnOnce(&mut App) + Send>;

/// `extension.*` for the server's threads.
pub struct AppExtensions {
    tx: UnboundedSender<Job>,
    rx: Mutex<Option<UnboundedReceiver<Job>>>,
    next: AtomicU64,
}

/// What a token stands for: prepared, approved, then run.
enum Task {
    Install(String),
    On(Manifest, BTreeMap<String, bool>),
    Off(String),
    Update(Box<catalog::Prepared>),
    Remove(Manifest),
    Setup(Box<Setup>),
    Service { m: Manifest, what: ServiceOp, digest: String },
    FirstRun(Manifest),
    Set { ext: String, conn: Option<String>, key: String, value: Option<String> },
    ConnAdd { ext: String, name: String },
    ConnRename { id: String, name: String },
    ConnRemove { id: String, default: Option<String> },
}

/// A setup action's run, as the approval showed it.
struct Setup {
    m: Manifest,
    a: Action,
    conn: Option<Account>,
    key: String,
    plan: Vec<Planned>,
}

/// The tasks prepared and not yet run or dropped, by token.
#[derive(Default)]
struct Tasks(HashMap<u64, Task>);

impl Global for Tasks {}

/// How a run's end is waited for.
enum Watch {
    /// Over already.
    Now(Result<String, String>),
    /// Told on this channel.
    Told(mpsc::Receiver<Result<String, String>>, Duration),
    /// Its service settles (`first`: its first run ends).
    Service { id: String, first: bool },
}

fn err(code: &str, text: impl Into<String>) -> ErrorBody {
    ErrorBody::new(code, text)
}

fn not_found(id: &str) -> ErrorBody {
    err(code::NOT_FOUND, format!("no extension “{id}” is installed (see `pitwall extension list`)"))
}

impl AppExtensions {
    pub fn new() -> Arc<AppExtensions> {
        let (tx, rx) = unbounded();
        Arc::new(AppExtensions { tx, rx: Mutex::new(Some(rx)), next: AtomicU64::new(1) })
    }

    /// Run `f` on the main thread and wait for its answer.
    fn on_main<T: Send + 'static>(&self, f: impl FnOnce(&mut App) -> Result<T, ErrorBody> + Send + 'static) -> Result<T, ErrorBody> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .unbounded_send(Box::new(move |cx: &mut App| {
                let _ = tx.send(f(cx));
            }))
            .map_err(|_| err(code::NOT_RUNNING, "Pitwall is quitting"))?;
        rx.recv_timeout(ANSWER_WITHIN).map_err(|_| err(code::OTHER, "Pitwall didn't answer in time"))?
    }

    /// Look on the main thread every [`POLL`] until `f` says it is over,
    /// or `within`: `None` then.
    fn until<T: Send + 'static>(&self, within: Duration, f: impl Fn(&mut App) -> Option<T> + Send + Sync + 'static) -> Option<T> {
        let f = Arc::new(f);
        let end = Instant::now() + within;
        loop {
            let g = f.clone();
            if let Ok(Some(v)) = self.on_main(move |cx| Ok(g(cx))) {
                return Some(v);
            }
            if Instant::now() >= end {
                return None;
            }
            std::thread::sleep(POLL);
        }
    }

    /// The answer to an act: what it did, and how the extension stands.
    fn answer(&self, id: &str, op: &str, text: String, running: bool) -> Result<ExtActed, ErrorBody> {
        let i = id.to_string();
        let status = self.on_main(move |cx| status_of(&i, None, cx)).ok();
        Ok(ExtActed { proto: EXT_PROTO, id: id.to_string(), op: op.to_string(), approved: true, text, running, status })
    }
}

/// On the main thread: run what the server asks. Once, at start.
pub fn attach(backend: Arc<AppExtensions>, cx: &mut App) {
    let Some(mut rx) = backend.rx.lock().ok().and_then(|mut r| r.take()) else { return };
    cx.spawn(async move |cx| {
        while let Some(job) = rx.next().await {
            cx.update(job);
        }
    })
    .detach();
}

/// The usable installed extension `id`.
fn manifest(id: &str, cx: &App) -> Result<Manifest, ErrorBody> {
    let found = super::found(cx);
    match found.iter().find(|f| f.key() == id) {
        Some(f) => f.manifest.clone().map_err(|why| err(code::CONFLICT, format!("“{id}” can't be used: {why}"))),
        None => Err(not_found(id)),
    }
}

/// Connection `key` (name or id) of `m`, or the one Settings shows.
fn connection(m: &Manifest, key: Option<&str>, cx: &App) -> Result<Option<Account>, ErrorBody> {
    if m.settings.is_empty() {
        return match key {
            Some(_) => Err(err(code::BAD_PARAMS, format!("{} has no settings, so no connections", m.name))),
            None => Ok(None),
        };
    }
    let list = connections::list(&m.id, cx);
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        None => Ok(connections::chosen(&m.id, None, cx)),
        Some(k) => {
            let named: Vec<&Account> = list.iter().filter(|c| c.name.eq_ignore_ascii_case(k)).collect();
            list.iter()
                .find(|c| c.id == k)
                .or((named.len() == 1).then(|| named[0]))
                .cloned()
                .map(Some)
                .ok_or_else(|| err(code::NOT_FOUND, format!("{} has no connection “{k}” (its connections: {})", m.name, list.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", "))))
        }
    }
}

/// Where installed extension `id` came from, in words.
fn source_of(id: &str, cx: &App) -> String {
    if let Some(d) = super::dev_folder(id, cx) {
        return format!("development folder {}", pitwall_core::paths::tildify(&d.to_string_lossy()));
    }
    let Some(dir) = super::dir(cx) else { return String::new() };
    match install::local(&dir.join(id)).origin {
        Some(Origin::Bundled) => "comes with Pitwall".into(),
        Some(Origin::Git { url, tag, .. }) => format!("git {url} {tag}"),
        Some(Origin::Npm { package, version, .. }) => format!("npm {package}@{version}"),
        None => "put there by hand".into(),
    }
}

/// A newer version of installed `m` known now (no fetch).
fn update_of(m: &Manifest, cx: &App) -> Option<String> {
    let dir = super::dir(cx)?;
    let origin = install::local(&dir.join(&m.id)).origin;
    let shipped = catalog::shipped_one(&m.id, cx);
    catalog::update_offer(m, origin.as_ref(), shipped.as_ref(), cx).and_then(|o| o.manifest.version)
}

fn kind_name(k: SettingKind) -> String {
    k.name().to_string()
}

/// `extension.list`.
fn list(cx: &mut App) -> ExtList {
    let found = super::found(cx);
    let ui = crate::ui_state::get(cx);
    let mut installed = Vec::new();
    for f in found.iter() {
        let id = f.key().to_string();
        let item = match &f.manifest {
            Ok(m) => {
                let c = connections::chosen(&m.id, None, cx).filter(|_| !m.settings.is_empty());
                let (state, ..) = card_status(m, c.as_ref(), cx);
                ExtListItem { id: id.clone(), name: m.name.clone(), version: m.version.clone(), description: m.description.clone(), installed: true, on: ui.extension_on(&m.id), source: source_of(&id, cx), state: state.into(), update: update_of(m, cx), demo: m.demo, problem: None }
            }
            Err(why) => ExtListItem { id: id.clone(), name: f.folder.clone(), installed: true, source: source_of(&id, cx), state: "Can't be used".into(), problem: Some(why.clone()), ..Default::default() },
        };
        installed.push(item);
    }
    let available = catalog::shipped(cx)
        .into_iter()
        .filter(|s| s.status == Status::Available && !found.iter().any(|f| f.key() == s.offer.manifest.id))
        .map(|s| {
            let m = s.offer.manifest;
            ExtListItem { id: m.id.clone(), name: m.name.clone(), version: m.version.clone(), description: m.description.clone(), installed: false, on: false, source: "comes with Pitwall".into(), state: "Available".into(), update: None, demo: m.demo, problem: None }
        })
        .collect();
    ExtList { proto: EXT_PROTO, installed, available }
}

fn service_view(m: &Manifest, cx: &App) -> Option<ExtService> {
    let s = m.service.as_ref()?;
    let svc = service::try_services(cx);
    let st = svc.as_ref().and_then(|x| x.read(cx).get(&m.id));
    let state = st.map(|v| v.state.clone()).unwrap_or_default();
    let mut v = ExtService { label: s.label.clone(), version: st.and_then(|x| x.version.clone()), needs: s.version.as_ref().and_then(|v| v.requires.clone()).filter(|r| !r.is_empty()), install_action: s.install.clone(), ..Default::default() };
    let (state, detail) = match state {
        State::Off => (ServiceState::Off, String::new()),
        State::NeedsConfirm { changed } => {
            v.changed = changed;
            (ServiceState::NeedsConfirmation, if changed { "An update changed what it runs, so it was stopped: its commands wait for the user's confirmation again.".into() } else { "Its commands wait for the user's confirmation.".into() })
        }
        State::UserStopped => (ServiceState::StoppedByUser, "Stopped by the user: it isn't started until Start.".into()),
        State::Checking => (ServiceState::Checking, "Looking before it starts.".into()),
        State::NeedsSetting(l) => (ServiceState::NeedsSetting, format!("Set {l} first.")),
        State::BadAddress(why) => (ServiceState::BadAddress, why),
        State::NotInstalled(p) => (ServiceState::NotInstalled, format!("“{p}” isn't installed for the user's login shell.")),
        State::TooOld { found, need } => {
            v.version = Some(found.clone());
            v.needs = Some(need.clone());
            (ServiceState::TooOld, format!("{} {found}; this extension needs {need} or later.", s.label))
        }
        State::Starting => (ServiceState::Starting, "Starting.".into()),
        State::Running { since } => {
            v.since = since.duration_since(SystemTime::UNIX_EPOCH).ok().map(|d| d.as_secs());
            (ServiceState::Running, String::new())
        }
        State::Elsewhere(a) => {
            v.address = Some(a.clone());
            (ServiceState::RunningElsewhere, format!("Something else is already listening on {a}: the user runs it. Pitwall uses it and never starts or stops it."))
        }
        State::Restarting { attempt } => (ServiceState::Restarting, format!("It stopped by itself and starts again ({attempt} of {} tries).", service::BACKOFF.len())),
        State::Stopped(why) => (ServiceState::Stopped, why),
    };
    v.state = state;
    v.detail = detail;
    if let Some(f) = &s.first_run {
        let (last, text) = match st.and_then(|x| x.first_run.clone()) {
            Some(FirstRun::Running) => (Some("running"), Some("Running.".to_string())),
            Some(FirstRun::Done(w)) => (Some("done"), Some(w)),
            Some(FirstRun::Failed(w)) => (Some("failed"), Some(w)),
            None => (None, None),
        };
        let pending = crate::ui_state::get(cx).extension(&m.id).service.fresh;
        v.first_run = Some(ExtFirstRun { label: f.label.clone(), pending, opens: f.opens, last: last.map(String::from), text });
    }
    Some(v)
}

fn adapter_view(m: &Manifest, c: Option<&Account>, cx: &App) -> Option<ExtAdapter> {
    m.adapter.as_ref()?;
    let h = hub::try_hub(cx);
    let st = h.as_ref().and_then(|h| match c {
        Some(c) => h.read(cx).connection(&m.id, &c.id).map(|s| (s.run.clone(), s.conn.clone())),
        None => h.read(cx).get(&m.id).map(|s| (s.run.clone(), s.conn.clone())),
    });
    let (run, conn) = st.unwrap_or_default();
    let (word, _, why) = crate::settings::extensions::run_text(&run, conn.as_ref());
    let state = match (&run, word) {
        (Run::Off, _) => AdapterState::Off,
        (Run::Setup(_), _) => AdapterState::NeedsSetup,
        (Run::Stopped(_), _) => AdapterState::Stopped,
        (_, "connected") => AdapterState::Connected,
        (_, "signed out") => AdapterState::SignedOut,
        (_, "error") => AdapterState::Error,
        _ => AdapterState::Connecting,
    };
    Some(ExtAdapter { state, reason: why })
}

/// `extension.status`: the card as data, and the next step.
fn status_of(id: &str, conn: Option<&str>, cx: &mut App) -> Result<ExtStatus, ErrorBody> {
    let found = super::found(cx);
    let Some(f) = found.iter().find(|f| f.key() == id) else {
        // One Pitwall ships and isn't installed: only that.
        if let Some(s) = catalog::shipped_one(id, cx).filter(|s| s.status == Status::Available) {
            let m = s.offer.manifest;
            let mut st = ExtStatus { proto: EXT_PROTO, id: m.id.clone(), name: m.name.clone(), version: m.version.clone(), installed: false, source: "comes with Pitwall".into(), state: "Available".into(), ..Default::default() };
            st.next = next_of(&st);
            return Ok(st);
        }
        return Err(not_found(id));
    };
    let m = match &f.manifest {
        Ok(m) => m.clone(),
        Err(why) => {
            let mut st = ExtStatus { proto: EXT_PROTO, id: id.to_string(), name: f.folder.clone(), installed: true, source: source_of(id, cx), state: "Can't be used".into(), problem: Some(why.clone()), ..Default::default() };
            st.next = next_of(&st);
            return Ok(st);
        }
    };
    let c = connection(&m, conn, cx)?;
    let ui = crate::ui_state::get(cx);
    let on = ui.extension_on(&m.id);
    let (state, _, why, connected) = card_status(&m, c.as_ref(), cx);
    let prefs = connections::prefs_for(&m, c.as_ref(), cx);
    let facts = ServiceFacts::of(&m, cx);
    let service = service_view(&m, cx);
    let svc_state = service.as_ref().map(|s| s.state);
    let label_of = |key: &str| m.settings.iter().find(|s| s.key == key).map(|s| s.label.clone()).unwrap_or_else(|| key.to_string());
    let path = actions::steps_path(cx);
    let mut steps = Vec::new();
    let list = if on { checklist(&m, &prefs, connected, facts) } else { vec![] };
    for (step, done) in &list {
        let mut s = ExtStep { state: if *done { StepState::Done } else { StepState::Todo }, ..Default::default() };
        match step {
            Step::Setting(key) => {
                let def = m.settings.iter().find(|x| x.key == *key);
                s.kind = StepKind::Setting;
                s.key = Some(key.clone());
                s.label = label_of(key);
                s.secret = def.is_some_and(|d| d.kind == SettingKind::Secret);
                s.detail = def.and_then(|d| d.hint.clone());
            }
            Step::Action(aid) => {
                let a = m.actions.iter().find(|a| a.id == *aid);
                s.kind = StepKind::Action;
                s.key = Some(aid.clone());
                s.label = a.map(|a| a.label.clone()).unwrap_or_else(|| aid.clone());
                if let Some(a) = a {
                    if !*done {
                        s.not_yet_for = connections::not_yet_for(&m, a, c.as_ref(), cx).iter().map(actions::ToolAccount::words).collect();
                        if !s.not_yet_for.is_empty() {
                            s.detail = Some(format!("Not yet set up for: {}", s.not_yet_for.join("; ")));
                        }
                        // Nothing of it can run: what stops it.
                        let values = c.as_ref().map(|c| c.values.clone()).unwrap_or_default();
                        let plan = actions::plan_for(&m, a, &values, path.as_deref(), &crate::accounts::tool_accounts(cx), true);
                        if !plan.is_empty() && plan.iter().all(|p| p.skip.is_some()) {
                            let mut why: Vec<String> = plan.iter().filter_map(|p| p.skip.clone()).collect();
                            why.dedup();
                            s.state = StepState::Blocked;
                            s.blocked_by = Some(why.join("; "));
                        }
                    }
                }
            }
            Step::ServiceInstall => {
                let v = service.as_ref();
                s.kind = StepKind::ServiceInstall;
                s.label = match svc_state {
                    Some(ServiceState::TooOld) => format!("Update {}", v.map(|v| v.label.as_str()).unwrap_or_default()),
                    _ if *done => format!("{} is installed", v.map(|v| v.label.as_str()).unwrap_or_default()),
                    _ => format!("Install {}", v.map(|v| v.label.as_str()).unwrap_or_default()),
                };
                if !*done {
                    s.detail = v.map(|v| v.detail.clone()).filter(|d| !d.is_empty());
                } else {
                    s.detail = v.and_then(|v| v.version.clone()).map(|x| format!("Version {x}."));
                }
            }
            Step::ServiceStart => {
                let v = service.as_ref();
                let label = v.map(|v| v.label.clone()).unwrap_or_default();
                s.kind = StepKind::ServiceStart;
                s.label = format!("Confirm and start {label}");
                s.detail = v.map(|v| v.detail.clone()).filter(|d| !d.is_empty());
                if !*done && matches!(svc_state, Some(ServiceState::NotInstalled | ServiceState::TooOld)) {
                    s.state = StepState::Blocked;
                    s.blocked_by = Some(format!("{label} has to be installed (and new enough) first."));
                }
            }
            Step::ServiceFirstRun => {
                let f = service.as_ref().and_then(|v| v.first_run.clone()).unwrap_or_default();
                s.kind = StepKind::ServiceFirstRun;
                s.label = f.label.clone();
                s.detail = f.text.clone();
                if !*done && svc_state != Some(ServiceState::Running) {
                    s.state = StepState::Blocked;
                    s.blocked_by = Some(match svc_state {
                        Some(ServiceState::RunningElsewhere) => "The server runs elsewhere: its first sign-in is done where it runs.".to_string(),
                        _ => format!("{} has to be running first.", service.as_ref().map(|v| v.label.as_str()).unwrap_or_default()),
                    });
                }
            }
            Step::Connected => {
                s.kind = StepKind::Connected;
                s.label = if connected { "Connected".into() } else { "Connect".into() };
                s.detail = Some(why.clone()).filter(|w| !w.is_empty());
                let run = hub::try_hub(cx).and_then(|h| match c.as_ref() {
                    Some(c) => h.read(cx).connection(&m.id, &c.id).map(|s| s.run.clone()),
                    None => h.read(cx).get(&m.id).map(|s| s.run.clone()),
                });
                if let (false, Some(Run::Setup(w))) = (connected, run) {
                    s.state = StepState::Blocked;
                    s.blocked_by = Some(w);
                }
            }
        }
        steps.push(s);
    }
    let settings = m
        .settings
        .iter()
        .map(|d| ExtSettingInfo { key: d.key.clone(), label: d.label.clone(), kind: kind_name(d.kind), required: d.required, hint: d.hint.clone(), default: d.default.clone(), options: d.options.iter().map(|o| o.value.clone()).collect() })
        .collect();
    let default = connections::default_id(&m.id, cx);
    let mut conns = Vec::new();
    for a in if m.settings.is_empty() { vec![] } else { connections::list(&m.id, cx) } {
        let (word, ..) = card_status(&m, Some(&a), cx);
        let secrets = m.settings.iter().filter(|d| d.kind == SettingKind::Secret).map(|d| (d.key.clone(), a.secrets_set.contains(&d.key))).collect();
        let values = a.values.iter().filter(|(k, v)| !v.trim().is_empty() && m.settings.iter().any(|d| d.key == **k && d.kind != SettingKind::Secret)).map(|(k, v)| (k.clone(), v.clone())).collect();
        conns.push(ExtConnection { id: a.id.clone(), name: a.name.clone(), default: default.as_deref() == Some(&a.id), values, secrets, state: word.into() });
    }
    let grants = ui.extension(&m.id).grants;
    let permissions = m
        .permissions
        .iter()
        .map(|p| ExtPermission { name: p.name.clone(), title: pitwall_core::extensions::permissions::find(&p.name).map_or(p.name.clone(), |x| x.title.to_string()), reason: p.reason.clone(), granted: grants.get(&p.name).map(|d| d.granted) })
        .collect();
    let actions_info = m
        .actions
        .iter()
        .map(|a| ExtActionInfo { id: a.id.clone(), label: a.label.clone(), description: a.description.clone(), setup: a.setup, done: prefs.actions_done.contains(&a.id), not_yet_for: connections::not_yet_for(&m, a, c.as_ref(), cx).iter().map(actions::ToolAccount::words).collect() })
        .collect();
    let mut st = ExtStatus {
        proto: EXT_PROTO,
        id: m.id.clone(),
        name: m.name.clone(),
        version: m.version.clone(),
        installed: true,
        on,
        source: source_of(&m.id, cx),
        state: state.into(),
        problem: None,
        update: update_of(&m, cx),
        connection: c.as_ref().map(|c| c.name.clone()),
        steps,
        settings,
        connections: conns,
        adapter: adapter_view(&m, c.as_ref(), cx),
        service,
        permissions,
        actions: actions_info,
        next: ExtNext::default(),
    };
    st.next = next_of(&st);
    Ok(st)
}

/// `extension.log`: the card's Log, newest last.
fn log_of(id: &str, lines: usize, cx: &App) -> Result<ExtLog, ErrorBody> {
    if !super::found(cx).iter().any(|f| f.key() == id) {
        return Err(not_found(id));
    }
    use pitwall_core::extensions::log::Dir;
    let all = crate::settings::extensions_dev::log_lines(id, cx);
    let n = lines.clamp(1, MAX_LOG_LINES);
    let lines = all[all.len().saturating_sub(n)..]
        .iter()
        .map(|(src, l)| ExtLogLine {
            at: l.at.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
            dir: match l.dir {
                Dir::ToAdapter => "to",
                Dir::FromAdapter => "from",
                Dir::Stderr => "stderr",
                Dir::Note => "pitwall",
            }
            .into(),
            source: src.clone(),
            text: l.text.clone(),
        })
        .collect();
    Ok(ExtLog { proto: EXT_PROTO, id: id.to_string(), lines })
}

/// Each line of a plan, as the confirmation lists it: where it runs when
/// that changes, then each command (and why one won't run).
fn plan_details(plan: &[Planned]) -> Vec<String> {
    let mut out = vec!["These commands run, in this order, in your home folder (no shell; each for at most 3 minutes):".to_string()];
    if !plan.is_empty() && plan.iter().all(|p| p.account.is_some()) {
        out.push("Only for the accounts it hasn't been set up for yet.".into());
    }
    for (head, line, skip) in plan_rows(plan) {
        if let Some(h) = head {
            out.push(format!("{h}:"));
        }
        out.push(match skip {
            None => format!("$ {line}"),
            Some(why) => format!("$ {line}   (skipped: {why})"),
        });
    }
    if plan.iter().any(|p| p.optional && p.skip.is_none()) {
        out.push("A failing remove step is fine: there was nothing to remove.".into());
    }
    out
}

/// What `m` does on this computer while on (its "What this extension does"
/// and permission screen, in lines).
fn on_details(m: &Manifest, picks: &BTreeMap<String, bool>, cx: &App) -> Vec<String> {
    use pitwall_core::extensions::permissions;
    let mut out = Vec::new();
    if let Some(a) = &m.adapter {
        out.push(format!("While it is on, it runs this in its folder: $ {}", a.command.join(" ")));
    } else if m.service.is_none() {
        out.push("It runs no program of its own.".into());
    }
    if let Some(s) = &m.service {
        out.push(format!("It keeps a local program running continuously while it is on: {} (its commands are confirmed separately before it starts).", s.label));
    }
    let values = connections::settings_values(&m.id, cx);
    let env = pitwall_core::extensions::plan::env_preview(m, &values);
    if !env.is_empty() {
        out.push(format!("It sets these on agents you start while it is on: {}", env.join(", ")));
    }
    for a in &m.actions {
        let cmds: Vec<String> = a.steps.iter().map(|s| s.run.join(" ")).collect();
        out.push(format!("“{}” runs only when asked and confirmed: {}", a.label, cmds.join("; ")));
    }
    for line in m.contributes.lines() {
        out.push(format!("Adds to Pitwall: {line}"));
    }
    for p in &m.permissions {
        let known = permissions::find(&p.name);
        let title = known.map_or(p.name.clone(), |k| k.title.to_string());
        let state = match permissions::unavailable_why(&p.name) {
            Some(why) => format!("not granted ({why})"),
            None if picks.get(&p.name).copied().unwrap_or(false) => "granted".into(),
            None => "not granted (the user can grant it later in Settings)".into(),
        };
        let detail = known.map(|k| format!(" {}", k.detail)).unwrap_or_default();
        out.push(format!("Permission “{title}” ({}): {state}. Its reason: “{}”.{detail}", p.name, p.reason));
    }
    if !m.permissions.is_empty() {
        out.push("Permissions bound what Pitwall gives it; its program runs as you either way. You change them in Settings → Extensions.".into());
    }
    out
}

/// The setup action to run: `want`, else the first setup action not done.
fn pick_action(m: &Manifest, want: Option<&str>, c: Option<&Account>, cx: &App) -> Result<Action, ErrorBody> {
    if let Some(w) = want.map(str::trim).filter(|w| !w.is_empty()) {
        return m.actions.iter().find(|a| a.id == w).cloned().ok_or_else(|| {
            let ids: Vec<&str> = m.actions.iter().map(|a| a.id.as_str()).collect();
            err(code::NOT_FOUND, format!("{} has no action “{w}” (its actions: {})", m.name, if ids.is_empty() { "none".into() } else { ids.join(", ") }))
        });
    }
    let done = connections::prefs_for(m, c, cx).actions_done;
    m.actions.iter().find(|a| a.setup && !done.contains(&a.id)).cloned().ok_or_else(|| err(code::BAD_PARAMS, format!("{} has no setup action left to run: name one with --action (see `pitwall extension status {}`)", m.name, m.id)))
}

/// A setup run's task and what the approval says.
fn setup_task(m: &Manifest, a: Action, c: Option<Account>, cx: &mut App) -> Result<(Task, String, Vec<String>), ErrorBody> {
    let key = run_key(&a, c.as_ref());
    if actions::runs(cx).read(cx).running(&m.id, &key) {
        return Err(err(BUSY, format!("“{}” is running now: wait for it to end (`pitwall extension status {}`).", a.label, m.id)));
    }
    let plan = plan_action(m, &a, c.as_ref(), cx);
    if plan.iter().all(|p| p.skip.is_some()) {
        let why: Vec<String> = plan.iter().filter_map(|p| p.skip.clone()).collect();
        return Err(err(code::CONFLICT, format!("Nothing of “{}” can run: {}.", a.label, if why.is_empty() { "it has no steps".into() } else { why.join("; ") })));
    }
    let mut details = plan_details(&plan);
    if let Some(c) = c.as_ref().filter(|_| connections::uses_setting(&a)) {
        details.insert(0, format!("For the connection {}.", c.name));
    }
    let summary = format!("run “{}” of {}", a.label, m.name);
    Ok((Task::Setup(Box::new(Setup { m: m.clone(), a, conn: c, key, plan })), summary, details))
}

/// Work out `act` on the main thread: the task, the summary and the lines
/// the approval shows, and its risk.
fn prepare_on_main(act: &ExtAct, cx: &mut App) -> Result<(Task, String, Vec<String>, Risk), ErrorBody> {
    let id = act.id.trim();
    let high = |(t, s, d): (Task, String, Vec<String>)| (t, s, d, Risk::High);
    if let ExtOp::Install = act.op {
        if super::found(cx).iter().any(|f| f.key() == id) {
            return Err(err(code::CONFLICT, format!("“{id}” is installed already (see `pitwall extension status {id}`)")));
        }
        let s = catalog::shipped_one(id, cx).filter(|s| s.status == Status::Available).ok_or_else(|| err(code::NOT_FOUND, format!("Pitwall doesn't ship an extension “{id}” to install (see `pitwall extension list`)")))?;
        let m = &s.offer.manifest;
        let mut details = vec![format!("{} {} is copied into Pitwall's extensions folder. It stays off: nothing of it runs until it is turned on.", m.name, m.version.clone().unwrap_or_default())];
        for c in install::changes(None, &s.offer) {
            details.push(c.title);
            details.extend(c.lines.into_iter().map(|l| format!("  {l}")));
        }
        return Ok((Task::Install(id.to_string()), format!("install the extension {}", m.name), details, Risk::High));
    }
    let m = manifest(id, cx)?;
    let ui = crate::ui_state::get(cx);
    let on = ui.extension_on(&m.id);
    let need_on = |what: &str| if on { Ok(()) } else { Err(err(code::CONFLICT, format!("{} is off: turn it on first (`pitwall extension on {}`), then {what}.", m.name, m.id))) };
    let svc_state = service::try_services(cx).and_then(|s| s.read(cx).get(&m.id).map(|v| v.state.clone())).unwrap_or_default();
    Ok(match &act.op {
        ExtOp::Install => unreachable!("handled above"),
        ExtOp::On => {
            if on {
                return Err(err(code::CONFLICT, format!("{} is on already.", m.name)));
            }
            let kept = ui.extension(&m.id).grants;
            let picks = crate::settings::extensions::initial_picks(&m, &kept);
            let details = on_details(&m, &picks, cx);
            high((Task::On(m.clone(), picks), format!("turn on the extension {}{}", m.name, m.version.as_ref().map(|v| format!(" {v}")).unwrap_or_default()), details))
        }
        ExtOp::Off => {
            if !on {
                return Err(err(code::CONFLICT, format!("{} is off already.", m.name)));
            }
            let mut d = vec!["Its program stops, and its inbox and what it adds to Pitwall go until it is on again.".to_string()];
            if let Some(s) = &m.service {
                d.push(format!("{} stops (unless it runs elsewhere: that one is never stopped).", s.label));
            }
            d.push("Nothing its setup installed is removed; its settings and secrets stay.".into());
            high((Task::Off(m.id.clone()), format!("turn off the extension {}", m.name), d))
        }
        ExtOp::Update => unreachable!("prepared off the main thread first"),
        ExtOp::Remove => {
            if let Some(why) = catalog::dev_refusal(&m.id, cx) {
                return Err(err(code::CONFLICT, format!("{why} Removing it from the list is the user's, in Settings.")));
            }
            let n = connections::list(&m.id, cx).len();
            let mut d = vec![if n > 1 { format!("It is turned off, and its {n} connections with their saved secrets, and its folder, are deleted.") } else { "It is turned off, and its settings, its saved secrets and its folder are deleted.".to_string() }];
            if let Some(data) = m.service.as_ref().and_then(|_| service::data_dir(&m.id, cx)) {
                d.push(format!("Its service's data in {} is kept (only the user can delete it, in Settings).", service::shown(&data)));
            }
            d.push("Nothing its setup actions installed in your agents is removed (run its remove action first if it has one).".into());
            high((Task::Remove(m.clone()), format!("remove the extension {}", m.name), d))
        }
        ExtOp::Setup { action, connection: conn } => {
            need_on("run its setup")?;
            let c = connection(&m, conn.as_deref(), cx)?.filter(|_| !m.settings.is_empty());
            let a = pick_action(&m, action.as_deref(), c.as_ref(), cx)?;
            high(setup_task(&m, a, c, cx)?)
        }
        ExtOp::Service { what } => {
            need_on("use its service")?;
            let s = m.service.clone().ok_or_else(|| err(code::BAD_PARAMS, format!("{} has no service", m.name)))?;
            let digest = core::digest(&s);
            let values = connections::settings_values(&m.id, cx);
            let data = service::data_dir(&m.id, cx).map(|d| service::shown(&d)).unwrap_or_default();
            let cmds = core::commands(&m, &s, &values, &data);
            let run_line = format!("$ {}", pitwall_core::extensions::plan::line(&cmds.run));
            let elsewhere = |what: &str| err(code::CONFLICT, format!("{} runs elsewhere: something else listens on its address, so the user runs it. Pitwall never starts or stops that one; {what} is theirs.", s.label));
            let task = Task::Service { m: m.clone(), what: *what, digest };
            match what {
                ServiceOp::Confirm => {
                    let State::NeedsConfirm { changed } = svc_state else {
                        return Err(err(code::CONFLICT, format!("{}'s commands are confirmed already (its state: {}).", s.label, state_word(&svc_state))));
                    };
                    let mut d = Vec::new();
                    if changed {
                        d.push(format!("An update changed what it runs, so it was stopped. {}", install::SERVICE_RISK));
                    }
                    d.push("Pitwall will keep this running while the extension is on:".into());
                    d.push(run_line);
                    for (title, lines) in core::lines(&m, &s, &values, &data).into_iter().skip(1) {
                        d.push(title);
                        d.extend(lines.into_iter().map(|l| format!("  {l}")));
                    }
                    if let Some(why) = &cmds.missing {
                        d.push(format!("It starts once {why} is set."));
                    }
                    d.push("It runs as you, with your files, and stops when you turn the extension off or quit Pitwall.".into());
                    high((task, format!("confirm and start {}, {}'s service", s.label, m.name), d))
                }
                ServiceOp::Start => match svc_state {
                    State::Stopped(_) | State::UserStopped | State::BadAddress(_) => high((task, format!("start {}", s.label), vec![format!("Pitwall starts it, as you, and keeps it running while {} is on:", m.name), run_line])),
                    State::Elsewhere(_) => return Err(elsewhere("starting it")),
                    State::NeedsConfirm { .. } => return Err(err(code::CONFLICT, format!("Its commands aren't confirmed: `pitwall extension service {} confirm` first.", m.id))),
                    other => return Err(err(code::CONFLICT, format!("{} isn't stopped (it is {}): nothing to start.", s.label, state_word(&other)))),
                },
                ServiceOp::Stop => match svc_state {
                    State::Running { .. } | State::Starting | State::Restarting { .. } | State::Checking => high((task, format!("stop {}", s.label), vec!["Pitwall asks it to stop, then ends what is left of its group after 3 seconds.".into(), "It stays stopped (also after Pitwall starts again) until it is started.".into()])),
                    State::Elsewhere(_) => return Err(elsewhere("stopping it")),
                    other => return Err(err(code::CONFLICT, format!("{} isn't running (it is {}).", s.label, state_word(&other)))),
                },
                ServiceOp::Restart => match svc_state {
                    State::Off | State::UserStopped => return Err(err(code::CONFLICT, format!("{} isn't running: `pitwall extension service {} start`.", s.label, m.id))),
                    State::NeedsConfirm { .. } => return Err(err(code::CONFLICT, format!("Its commands aren't confirmed: `pitwall extension service {} confirm` first.", m.id))),
                    State::Running { .. } | State::Starting | State::Restarting { .. } => high((task, format!("restart {}", s.label), vec!["It is stopped, then started again with what is installed now:".into(), run_line])),
                    // Looked at again ("Check again"): started if it can be.
                    _ => high((task, format!("look again at {} and start it if it can", s.label), vec![format!("Pitwall looks again whether {} is installed, new enough and its address free; if so it starts it:", s.label), run_line, "Something else listening there is never started, stopped or signalled.".into()])),
                },
                ServiceOp::Update => {
                    let iid = s.install.clone().ok_or_else(|| err(code::BAD_PARAMS, format!("{} has no install action: the user updates {} themselves, then `pitwall extension service {} restart`.", m.name, s.label, m.id)))?;
                    let a = m.actions.iter().find(|a| a.id == iid).cloned().ok_or_else(|| err(code::OTHER, format!("its install action “{iid}” is missing")))?;
                    let c = connections::chosen(&m.id, None, cx).filter(|_| !m.settings.is_empty());
                    let (t, _, mut d) = setup_task(&m, a, c, cx)?;
                    d.push(format!("Then {} is started again with what is installed.", s.label));
                    (t, format!("update {} with “{}”, then start it again", s.label, m.actions.iter().find(|a| a.id == iid).map(|a| a.label.as_str()).unwrap_or_default()), d, Risk::High)
                }
            }
        }
        ExtOp::FirstRun => {
            need_on("run its first run")?;
            let s = m.service.clone().ok_or_else(|| err(code::BAD_PARAMS, format!("{} has no service", m.name)))?;
            let f = s.first_run.clone().ok_or_else(|| err(code::BAD_PARAMS, format!("{} has no first run", s.label)))?;
            match svc_state {
                State::Running { .. } => {}
                State::Elsewhere(_) => return Err(err(code::CONFLICT, format!("{} runs elsewhere (the user runs it): its first run is done where it runs, not by Pitwall.", s.label))),
                other => return Err(err(code::CONFLICT, format!("“{}” runs only while {} is running (it is {}).", f.label, s.label, state_word(&other)))),
            }
            if service::try_services(cx).and_then(|x| x.read(cx).get(&m.id).and_then(|v| v.first_run.clone())) == Some(FirstRun::Running) {
                return Err(err(BUSY, format!("“{}” is running now.", f.label)));
            }
            let values = connections::settings_values(&m.id, cx);
            let data = service::data_dir(&m.id, cx).map(|d| service::shown(&d)).unwrap_or_default();
            let line = core::commands(&m, &s, &values, &data).first_run.map(|a| pitwall_core::extensions::plan::line(&a)).unwrap_or_default();
            let fresh = ui.extension(&m.id).service.fresh;
            let mut d = Vec::new();
            if !fresh {
                d.push(format!("It ran before. {}", f.again.clone().unwrap_or_else(|| "It runs its first-run command again.".into())));
            }
            d.push(format!("$ {line}"));
            if f.opens {
                d.push("Pitwall opens the https link it prints in your browser. The link is never shown, kept or sent to whoever asked: they only hear that a link was opened.".into());
            }
            high((Task::FirstRun(m.clone()), format!("run “{}” of {}", f.label, s.label), d))
        }
        ExtOp::Set { key, value, connection: conn } => {
            let def = m.settings.iter().find(|d| d.key == *key).ok_or_else(|| err(code::NOT_FOUND, format!("{} has no setting “{key}” (its settings: {})", m.name, m.settings.iter().map(|d| d.key.as_str()).collect::<Vec<_>>().join(", "))))?;
            if def.kind == SettingKind::Secret {
                return Err(err(code::BAD_PARAMS, format!("“{key}” is a secret: only the user enters it, in Pitwall. Run `pitwall extension secret {} {key}` to show them the field; never ask for it in the chat.", m.id)));
            }
            let v = match def.kind {
                SettingKind::Url => url_value(value),
                _ => def.check_value(value),
            }
            .map_err(|why| err(code::BAD_PARAMS, format!("{}: {why}", def.label)))?;
            let c = connection(&m, conn.as_deref(), cx)?;
            let was = c.as_ref().and_then(|c| c.values.get(key).cloned()).filter(|x| !x.trim().is_empty());
            let to = v.clone().unwrap_or_else(|| "(empty)".into());
            let mut d = vec![format!("{}: {} → {to}", def.label, was.unwrap_or_else(|| "(not set)".into()))];
            match &c {
                Some(c) => d.push(format!("On the connection {}.", c.name)),
                None => d.push(format!("A connection named {} is made for it.", m.name)),
            }
            if on && m.adapter.is_some() {
                d.push("Its adapter starts again with it.".into());
            }
            (Task::Set { ext: m.id.clone(), conn: c.map(|c| c.id), key: key.clone(), value: v }, format!("set “{}” of {}", def.label, m.name), d, Risk::Low)
        }
        ExtOp::ConnectionAdd { name } => {
            if m.settings.is_empty() {
                return Err(err(code::BAD_PARAMS, format!("{} has no settings, so no connections", m.name)));
            }
            (Task::ConnAdd { ext: m.id.clone(), name: name.trim().to_string() }, format!("add the connection “{}” to {}", name.trim(), m.name), vec!["Another set of its settings and secrets (another server or key). Each project then picks which one its agents use.".into()], Risk::Low)
        }
        ExtOp::ConnectionRename { connection: key, name } => {
            let c = connection(&m, Some(key), cx)?.ok_or_else(|| err(code::NOT_FOUND, "no such connection"))?;
            (Task::ConnRename { id: c.id.clone(), name: name.trim().to_string() }, format!("rename {}'s connection {} to “{}”", m.name, c.name, name.trim()), vec![], Risk::Low)
        }
        ExtOp::ConnectionRemove { connection: key, default } => {
            let c = connection(&m, Some(key), cx)?.ok_or_else(|| err(code::NOT_FOUND, "no such connection"))?;
            let was_default = connections::default_id(&m.id, cx).as_deref() == Some(&c.id);
            let next = match default.as_deref() {
                Some(d) if was_default => Some(connection(&m, Some(d), cx)?.ok_or_else(|| err(code::NOT_FOUND, "no such connection"))?).filter(|n| n.id != c.id),
                _ => None,
            };
            let list = crate::accounts::list(cx).cloned().unwrap_or_default();
            let users: Vec<String> = list.profiles_of(&c.id).iter().map(|p| format!("profile {p}")).chain(list.projects_of(&c.id).iter().map(|p| pitwall_core::paths::tildify(p))).collect();
            let mut d = vec!["Its settings and saved secrets are deleted.".to_string()];
            if !users.is_empty() {
                d.push(format!("{} use it and go back to their profile's connection or the default.", users.join(", ")));
            }
            if was_default {
                d.push(match &next {
                    Some(n) => format!("It is the default: {} becomes the default.", n.name),
                    None => "It is the default: afterwards there is none (projects that don't pick one get none).".into(),
                });
            }
            (Task::ConnRemove { id: c.id.clone(), default: next.map(|n| n.id) }, format!("remove {}'s connection {}", m.name, c.name), d, Risk::High)
        }
    })
}

/// A service state in a few words.
fn state_word(s: &State) -> String {
    match s {
        State::Off => "off".into(),
        State::NeedsConfirm { .. } => "waiting for its commands to be confirmed".into(),
        State::UserStopped => "stopped by the user".into(),
        State::Checking => "being looked at".into(),
        State::NeedsSetting(l) => format!("waiting for {l}"),
        State::BadAddress(_) => "not started: a bad address".into(),
        State::NotInstalled(_) => "not installed".into(),
        State::TooOld { found, need } => format!("{found}, below {need}"),
        State::Starting => "starting".into(),
        State::Running { .. } => "running".into(),
        State::Elsewhere(a) => format!("running elsewhere ({a})"),
        State::Restarting { .. } => "starting again".into(),
        State::Stopped(_) => "stopped".into(),
    }
}

/// Start `task` on the main thread; how its end is waited for.
fn start(task: Task, cx: &mut App) -> Result<(String, String, Watch), ErrorBody> {
    let told = |within: Duration| {
        let (tx, rx) = mpsc::channel::<Result<String, String>>();
        (tx, Watch::Told(rx, within))
    };
    let outcome = |o: catalog::Outcome, done: &str| match o {
        catalog::Outcome::Done => Ok(done.to_string()),
        catalog::Outcome::ChangedLocally => Err("Its folder was changed since Pitwall installed it (or put there by hand): updating replaces those changes, which only the user decides, in Settings. Nothing changed.".to_string()),
        catalog::Outcome::Failed(why) => Err(why),
    };
    Ok(match task {
        Task::Install(id) => {
            let (tx, w) = told(FETCH_WITHIN);
            catalog::install_one(&id, cx, move |o, _| {
                let _ = tx.send(outcome(o, "Installed. It is off: turn it on to set it up."));
            });
            (id, "install".into(), w)
        }
        Task::On(m, picks) => {
            if m.permissions.is_empty() {
                crate::ui_state::update(cx, |s| *s = s.clone().set_extension(&m.id, true));
            } else {
                crate::settings::extensions::turn_on_with(&m, &picks, cx);
            }
            let granted: Vec<&str> = picks.iter().filter(|(_, on)| **on).map(|(n, _)| n.as_str()).collect();
            let text = if m.permissions.is_empty() { "It is on.".to_string() } else { format!("It is on, granted: {}.", if granted.is_empty() { "nothing".into() } else { granted.join(", ") }) };
            (m.id, "on".into(), Watch::Now(Ok(text)))
        }
        Task::Off(id) => {
            crate::ui_state::update(cx, |s| *s = s.clone().set_extension(&id, false));
            (id, "off".into(), Watch::Now(Ok("It is off.".into())))
        }
        Task::Update(p) => {
            let id = p.offer.manifest.id.clone();
            let v = p.offer.manifest.version.clone().unwrap_or_default();
            let (tx, w) = told(FETCH_WITHIN);
            catalog::apply(&p, false, cx, move |o, _| {
                let _ = tx.send(outcome(o, &format!("Updated to {v}.")));
            });
            (id, "update".into(), w)
        }
        Task::Remove(m) => {
            let (tx, w) = told(FETCH_WITHIN);
            let kept = m.service.as_ref().and_then(|_| service::data_dir(&m.id, cx)).map(|d| format!(" Its service's data in {} was kept.", service::shown(&d))).unwrap_or_default();
            catalog::remove_one(&m, false, cx, move |o, _| {
                let _ = tx.send(outcome(o, &format!("Removed.{kept}")));
            });
            (m.id, "remove".into(), w)
        }
        Task::Setup(b) => {
            let Setup { m, a, conn, key, plan } = *b;
            if actions::runs(cx).read(cx).running(&m.id, &key) {
                return Err(err(BUSY, format!("“{}” is running now.", a.label)));
            }
            let (tx, w) = told(RUN_WITHIN);
            let label = a.label.clone();
            start_action(&m, &a, conn, &key, plan, cx, move |st: &RunState, _| {
                let summary = st.summary().unwrap_or_default();
                let mut lines = vec![format!("“{label}”: {summary}")];
                for (line, state) in &st.steps {
                    let w = match state {
                        actions::StepState::Ended(actions::Outcome::Done) => "done".to_string(),
                        actions::StepState::Ended(actions::Outcome::Failed(why)) => format!("failed: {why}"),
                        actions::StepState::Ended(actions::Outcome::Skipped(why)) => format!("skipped: {why}"),
                        _ => "not run".into(),
                    };
                    lines.push(format!("$ {line} — {w}"));
                }
                let text = lines.join("\n");
                let _ = tx.send(if st.failed() || st.cancelled { Err(text) } else { Ok(text) });
            });
            (m.id, "setup".into(), w)
        }
        Task::Service { m, what, digest } => {
            // Exactly what was shown: changed since (an update), nothing
            // starts.
            let now = manifest(&m.id, cx)?;
            if now.service.as_ref().map(core::digest).as_deref() != Some(digest.as_str()) {
                return Err(err(code::CONFLICT, "What it runs changed since the user was asked (an update or a reload): nothing was started. Ask again."));
            }
            let id = now.id.clone();
            match what {
                ServiceOp::Confirm => service::confirm(&now, cx),
                ServiceOp::Start => {
                    service::set_stopped(&id, false, cx);
                    service::recheck(&id, cx);
                }
                ServiceOp::Stop => service::set_stopped(&id, true, cx),
                ServiceOp::Restart => service::recheck(&id, cx),
                ServiceOp::Update => unreachable!("an update is its install action (a setup task)"),
            }
            (id.clone(), format!("service {}", what.word()), Watch::Service { id, first: false })
        }
        Task::FirstRun(m) => {
            let s = service::services(cx);
            s.update(cx, |s, cx| s.first_run(&m, cx));
            (m.id.clone(), "first-run".into(), Watch::Service { id: m.id, first: true })
        }
        Task::Set { ext, conn, key, value } => {
            connections::set_value(&ext, conn.as_deref(), &key, value, cx).map_err(|e| err("failed", e))?;
            (ext, "set".into(), Watch::Now(Ok(format!("Set {key}."))))
        }
        Task::ConnAdd { ext, name } => {
            let model = crate::accounts::model(cx);
            let made = model.update(cx, |a, cx| a.add_connection(&ext, &name, cx)).map_err(|e| err(code::BAD_PARAMS, e.replace("an account", "a connection")))?;
            (ext, "connection add".into(), Watch::Now(Ok(format!("Added the connection {}.", made.name))))
        }
        Task::ConnRename { id, name } => {
            let ext = crate::accounts::list(cx).and_then(|l| l.get(&id)).and_then(|c| pitwall_core::accounts::ext_of(&c.kind).map(String::from)).unwrap_or_default();
            let model = crate::accounts::model(cx);
            model.update(cx, |a, cx| a.rename(&id, &name, cx)).map_err(|e| err(code::BAD_PARAMS, e.replace("an account", "a connection")))?;
            (ext, "connection rename".into(), Watch::Now(Ok(format!("Renamed it {name}."))))
        }
        Task::ConnRemove { id, default } => {
            let ext = crate::accounts::list(cx).and_then(|l| l.get(&id)).and_then(|c| pitwall_core::accounts::ext_of(&c.kind).map(String::from)).unwrap_or_default();
            let (tx, w) = told(FETCH_WITHIN);
            connections::remove_with(&id, default.as_deref(), cx, move |e, _| {
                let _ = tx.send(match e {
                    None => Ok("Removed the connection.".into()),
                    Some(e) => Err(e),
                });
            });
            (ext, "connection remove".into(), w)
        }
    })
}

/// Whether extension `id`'s service has settled (`first`: its first run
/// ended): how, in words; `None` while under way.
fn settled(id: &str, first: bool, cx: &App) -> Option<Result<String, String>> {
    let svc = service::try_services(cx)?;
    let v = svc.read(cx).get(id)?;
    if first {
        return match v.first_run.clone()? {
            FirstRun::Running => None,
            FirstRun::Done(w) => Some(Ok(w)),
            FirstRun::Failed(w) => Some(Err(w)),
        };
    }
    match &v.state {
        State::Checking | State::Starting | State::Restarting { .. } => None,
        State::Stopped(why) => Some(Err(format!("It stopped: {why}"))),
        other => Some(Ok(format!("It is {}.", state_word(other)))),
    }
}

impl ExtensionsBackend for AppExtensions {
    fn list(&self) -> Result<ExtList, ErrorBody> {
        self.on_main(|cx| Ok(list(cx)))
    }

    fn status(&self, r: &ExtRef) -> Result<ExtStatus, ErrorBody> {
        let r = r.clone();
        self.on_main(move |cx| status_of(r.id.trim(), r.connection.as_deref(), cx))
    }

    fn log(&self, r: &ExtLogRequest) -> Result<ExtLog, ErrorBody> {
        let (id, n) = (r.id.trim().to_string(), r.lines.unwrap_or(100));
        self.on_main(move |cx| log_of(&id, n, cx))
    }

    fn check_updates(&self, id: &str) -> Result<ExtActed, ErrorBody> {
        let i = id.to_string();
        let source = self.on_main(move |cx| {
            let m = manifest(&i, cx)?;
            if let Some(why) = catalog::dev_refusal(&m.id, cx) {
                return Err(err(code::CONFLICT, why));
            }
            if m.source.is_some() {
                catalog::check_one(&m.id, cx);
            }
            Ok(m.source.is_some())
        })?;
        if source {
            let i = id.to_string();
            let ended = self.until(FETCH_WITHIN, move |cx| (!catalog::updates(cx).read(cx).checking(&i)).then_some(()));
            if ended.is_none() {
                return Ok(ExtActed { proto: EXT_PROTO, id: id.to_string(), op: "check-updates".into(), approved: false, text: "Still checking: run `status` in a moment.".into(), running: true, status: None });
            }
        }
        let i = id.to_string();
        let text = self.on_main(move |cx| {
            let m = manifest(&i, cx)?;
            let check = catalog::updates(cx).read(cx).check(&m.id).cloned();
            let failed = check.as_ref().and_then(|c| c.newest.as_ref().err().cloned().or_else(|| c.moved.clone()));
            Ok(match (update_of(&m, cx), failed) {
                (Some(v), _) => format!("Version {v} is available (now {}): `pitwall extension update {}` asks the user, showing what it changes.", m.version.clone().unwrap_or_default(), m.id),
                (None, Some(why)) => format!("The check didn't work: {why}"),
                (None, None) if m.source.is_none() => format!("It comes with Pitwall and is up to date ({}); its updates come with Pitwall's.", m.version.clone().unwrap_or_default()),
                (None, None) => format!("It is up to date ({}).", m.version.clone().unwrap_or_default()),
            })
        })?;
        let mut a = self.answer(id, "check-updates", text, false)?;
        a.approved = false;
        Ok(a)
    }

    fn prepare(&self, act: &ExtAct) -> Result<Prepared, ErrorBody> {
        let token = self.next.fetch_add(1, Ordering::SeqCst);
        if let ExtOp::Update = act.op {
            return self.prepare_update(act.id.trim(), token);
        }
        let act = act.clone();
        self.on_main(move |cx| {
            let (task, summary, details, risk) = prepare_on_main(&act, cx)?;
            cx.default_global::<Tasks>().0.insert(token, task);
            Ok(Prepared { token, summary, details, risk })
        })
    }

    fn run(&self, token: u64) -> Result<ExtActed, ErrorBody> {
        let (id, op, watch) = self.on_main(move |cx| {
            let task = cx.default_global::<Tasks>().0.remove(&token).ok_or_else(|| err(code::OTHER, "that request is gone"))?;
            start(task, cx)
        })?;
        let (res, running) = match watch {
            Watch::Now(r) => (r, false),
            Watch::Told(rx, within) => match rx.recv_timeout(within) {
                Ok(r) => (r, false),
                Err(_) => (Ok("It still runs: run `status` to see how it went.".into()), true),
            },
            Watch::Service { id, first } => {
                let within = if first { service::FIRST_RUN_TIMEOUT + Duration::from_secs(15) } else { SERVICE_WITHIN };
                let i = id.clone();
                match self.until(within, move |cx| settled(&i, first, cx)) {
                    Some(r) => (r, false),
                    None => (Ok("It is still under way: run `status` in a few seconds.".into()), true),
                }
            }
        };
        match res {
            Ok(text) => self.answer(&id, &op, text, running),
            Err(why) => Err(err("failed", format!("{why}\nRun `pitwall extension status {id}` (and `pitwall extension log {id}`) to see where it stands."))),
        }
    }

    fn discard(&self, token: u64) {
        let _ = self.on_main(move |cx| {
            if let Some(Task::Update(p)) = cx.default_global::<Tasks>().0.remove(&token) {
                catalog::discard(&p, cx);
            }
            Ok(())
        });
    }
}

impl AppExtensions {
    /// An update: fetched (as Review… does, off the UI thread), then what
    /// it changes is what the approval lists.
    fn prepare_update(&self, id: &str, token: u64) -> Result<Prepared, ErrorBody> {
        let (tx, rx) = mpsc::channel::<Result<catalog::Prepared, String>>();
        let i = id.to_string();
        self.on_main(move |cx| {
            let m = manifest(&i, cx)?;
            if let Some(why) = catalog::dev_refusal(&m.id, cx) {
                return Err(err(code::CONFLICT, why));
            }
            let dir = super::dir(cx).ok_or_else(|| err(code::OTHER, "no extensions folder"))?;
            let origin = install::local(&dir.join(&m.id)).origin;
            let shipped = catalog::shipped_one(&m.id, cx);
            let offer = catalog::update_offer(&m, origin.as_ref(), shipped.as_ref(), cx).ok_or_else(|| err(code::CONFLICT, format!("No update of {} is known: `pitwall extension check-updates {}` first.", m.name, m.id)))?;
            catalog::prepare(&m.id, offer, cx, move |r, _| {
                let _ = tx.send(r);
            });
            Ok(())
        })?;
        let p = rx.recv_timeout(FETCH_WITHIN).map_err(|_| err(code::OTHER, "Fetching the update took too long."))?.map_err(|why| err("failed", format!("The update can't be had: {why}")))?;
        self.on_main(move |cx| {
            let m = manifest(&p.offer.manifest.id, cx)?;
            let to = p.offer.manifest.version.clone().unwrap_or_default();
            let mut d = vec![format!("{} {} is replaced with {to}. Its settings and secrets stay; while it is on, it is turned off for the swap and on again.", m.name, m.version.clone().unwrap_or_default())];
            for c in &p.changes {
                d.push(c.title.clone());
                d.extend(c.lines.iter().map(|l| format!("  {l}")));
            }
            let summary = format!("update the extension {} to {to}", m.name);
            cx.default_global::<Tasks>().0.insert(token, Task::Update(Box::new(p)));
            Ok(Prepared { token, summary, details: d, risk: Risk::High })
        })
    }
}

