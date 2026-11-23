//! What extensions read from Pitwall (docs/spec/extension-api.md,
//! "Protocol 2"): the daemon's method table answers a protocol 2 adapter's
//! `call`s with the extension as the caller ([`pitwall_daemon::Server`]),
//! the same table and the same permissions as a process its adapter
//! started gets on the socket. This module keeps what both need: the
//! server ([`set_api`]), the registry of extensions that are on with what
//! the user granted each ([`sync`]), each extension's activity log
//! ([`activity`]), and the limits.
//!
//! Limits (each exceeded one is logged in the extension's activity; one
//! exceeded again and again stops its adapter, saying why):
//! - calls: [`pitwall_daemon::extensions::MAX_CALLS`] a second per
//!   extension (more are answered `busy`), at most [`MAX_IN_FLIGHT`]
//!   unanswered at once;
//! - a call's params: [`protocol::MAX_PARAMS`]; a reply or an event:
//!   [`protocol::MAX_OUT`] (a bigger reply is an error, a bigger event is
//!   dropped);
//! - events: at most [`EVENTS_PER_SEC`] a second to an adapter (the
//!   daemon's queue keeps the latest agent list and the newest alerts
//!   meanwhile).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, Global};
use pitwall_core::extensions::activity::{self, Activity, Entry, Outcome, Via};
use pitwall_core::extensions::permissions::{self, Decisions};
use pitwall_core::extensions::protocol::{self, CallError, ToAdapter};
use pitwall_core::extensions::Manifest;
use pitwall_daemon::{Registry, Server};

use super::adapter::Sender;

/// The most calls of one adapter waiting for their answer.
pub const MAX_IN_FLIGHT: usize = 8;
/// The most events an adapter gets a second.
pub const EVENTS_PER_SEC: usize = 10;
/// Calls refused for its limits in a row before its adapter is stopped.
pub const MAX_STRIKES: u32 = 100;

/// The server that answers extensions' calls, and the extensions that are
/// on.
#[derive(Clone)]
pub struct Api {
    pub server: Option<Arc<Server>>,
    pub registry: Arc<Registry>,
}

impl Global for Api {}

/// Answer extensions' calls with `server` (its registry is theirs).
pub fn set_api(server: Arc<Server>, cx: &mut App) {
    let registry = server.extensions.clone();
    cx.set_global(Api { server: Some(server), registry });
}

/// The extensions' registry (made, without a server, when there is none:
/// tests and a refused data folder).
pub fn api(cx: &mut App) -> Api {
    if let Some(a) = cx.try_global::<Api>() {
        return a.clone();
    }
    let a = Api { server: None, registry: Registry::new() };
    cx.set_global(a.clone());
    a
}

/// Each extension's activity log, read from its file once.
#[derive(Default)]
struct Logs(HashMap<String, activity::Shared>);

impl Global for Logs {}

/// Extension `id`'s activity log (kept under `<extensions>/.log/`).
pub fn activity(id: &str, cx: &mut App) -> activity::Shared {
    if let Some(l) = cx.try_global::<Logs>().and_then(|l| l.0.get(id)) {
        return l.clone();
    }
    let file = super::dir(cx).map(|d| activity::file_of(&d.join(".log"), id));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let log: activity::Shared = Arc::new(std::sync::Mutex::new(Activity::open(file, now)));
    cx.default_global::<Logs>().0.insert(id.to_string(), log.clone());
    log
}

/// Extension `id`'s activity, oldest first.
pub fn entries(id: &str, cx: &mut App) -> Vec<Entry> {
    activity(id, cx).lock().unwrap_or_else(|e| e.into_inner()).entries()
}

/// What `m` may use now, with `decisions` kept for it.
pub fn effective(m: &Manifest, decisions: &Decisions) -> Vec<String> {
    permissions::effective(m, decisions)
}

/// The registry follows the switches and grants: each extension that is
/// on with what it may use now (a revoke applies to its next call), the
/// others removed.
pub fn sync(cx: &mut App) {
    let reg = api(cx).registry;
    let ui = crate::ui_state::get(cx);
    let on = super::enabled(&super::found(cx), &ui);
    for id in reg.ids() {
        if !on.iter().any(|m| m.id == id) {
            reg.remove(&id);
        }
    }
    for m in on {
        let grants = effective(&m, &ui.extension(&m.id).grants);
        let log = activity(&m.id, cx);
        reg.set(&m.id, &m.name, grants, Some(log));
    }
}

/// The extensions that may have text waiting in Next up: on, and granted
/// `agents.queue` now.
pub fn queuers(cx: &App) -> Vec<String> {
    let ui = crate::ui_state::get(cx);
    super::enabled(&super::found(cx), &ui).into_iter().filter(|m| effective(m, &ui.extension(&m.id).grants).iter().any(|g| g == permissions::AGENTS_QUEUE)).map(|m| m.id).collect()
}

/// The extensions whose text Auto-send may send: those of [`queuers`] the
/// user lets send without asking (off unless turned on, per extension).
pub fn auto_senders(cx: &App) -> Vec<String> {
    let ui = crate::ui_state::get(cx);
    queuers(cx).into_iter().filter(|id| ui.extension(id).send_without_asking.as_ref().is_some_and(|d| d.granted)).collect()
}

/// The engine follows: Auto-send sends only [`auto_senders`]' text, and
/// the text of an extension turned off or no longer granted
/// `agents.queue` leaves Next up (never sent).
pub fn sync_queue(engine: &pitwall_core::Engine, cx: &App) {
    engine.set_auto_senders(auto_senders(cx));
    engine.queue_keep_origins(&queuers(cx));
}

/// Let extension `id` send what it queues without asking (`on`), or not:
/// logged in its activity.
pub fn set_send_without_asking(id: &str, on: bool, cx: &mut App) {
    let key = id.to_string();
    crate::ui_state::update(cx, |s| {
        *s = s.clone().with_extension(&key, |e| e.send_without_asking = on.then(|| permissions::Decision::now(true)));
    });
    let word = if on { "send without asking: turned on by you" } else { "send without asking: turned off by you" };
    activity(id, cx).lock().unwrap_or_else(|e| e.into_inner()).push(Entry::now(Via::Pitwall, word, Outcome::Ok));
}

/// Grant or refuse permission `name` of extension `id` now, dated today
/// (a revoke applies to its next call).
pub fn decide(id: &str, name: &str, granted: bool, cx: &mut App) {
    let (id, name) = (id.to_string(), name.to_string());
    crate::ui_state::update(cx, |s| {
        *s = s.clone().with_extension(&id, |e| {
            e.grants.insert(name.clone(), permissions::Decision::now(granted));
        })
    });
}

/// Answer call `method` of extension `ext` (off the UI thread): the
/// result, an error over [`protocol::MAX_OUT`] for one too big.
pub fn answer(server: Option<&Server>, ext: &str, method: &str, params: serde_json::Value) -> Result<serde_json::Value, CallError> {
    let Some(server) = server else {
        return Err(CallError::new("unsupported", "This Pitwall can't answer calls."));
    };
    let caller = server.extension_caller(ext).ok_or_else(|| CallError::new("denied", "This extension is off."))?;
    let v = server.call_as(&caller, method, params, Via::Adapter).map_err(|e| CallError::new(&e.code, e.message))?;
    let size = serde_json::to_string(&v).map(|s| s.len()).unwrap_or(0);
    if size > protocol::MAX_OUT {
        return Err(CallError::new("too_large", format!("The answer is over {} KiB.", protocol::MAX_OUT / 1024)));
    }
    Ok(v)
}

/// Log, in extension `ext`'s activity, what Pitwall refused before asking
/// the daemon (a limit, protocol 1).
pub fn refused(reg: &Registry, ext: &str, method: &str, e: &CallError) {
    reg.record(ext, Entry::now(Via::Adapter, method, Outcome::Refused { code: e.code.clone(), message: e.message.clone() }));
}

/// Start sending extension `ext`'s events from `q` to its adapter
/// (`to`), at most [`EVENTS_PER_SEC`] a second, each one it may still get
/// and within [`protocol::MAX_OUT`]; until `q` is closed or the adapter
/// is gone.
pub fn pump(server: Arc<Server>, ext: String, q: Arc<pitwall_daemon::events::Queue>, to: Sender) {
    let _ = std::thread::Builder::new().name("pitwall-ext-events".into()).spawn(move || {
        let caller = server.extension_caller(&ext);
        let mut sent: std::collections::VecDeque<Instant> = Default::default();
        while let Some(ev) = q.pop() {
            let Some(caller) = &caller else { return };
            if !server.may_get(caller, &ev.event) {
                continue;
            }
            let msg = ToAdapter::Event { event: ev.event.clone(), data: ev.data };
            if msg.line().len() > protocol::MAX_OUT {
                server.extensions.record(&ext, Entry::now(Via::Pitwall, &ev.event, Outcome::Refused { code: "too_large".into(), message: format!("an event over {} KiB was not sent", protocol::MAX_OUT / 1024) }));
                continue;
            }
            // Paced: the queue keeps the latest state meanwhile.
            let now = Instant::now();
            while sent.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1)) {
                sent.pop_front();
            }
            if sent.len() >= EVENTS_PER_SEC {
                if let Some(first) = sent.front() {
                    std::thread::sleep(Duration::from_secs(1).saturating_sub(now.duration_since(*first)));
                }
                sent.pop_front();
            }
            if !to.send(&msg) {
                q.close();
                return;
            }
            sent.push_back(Instant::now());
        }
    });
}
