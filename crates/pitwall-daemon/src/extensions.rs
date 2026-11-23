//! Extensions as callers (docs/spec/extension-api.md, "Permissions" and
//! "Decision 5"). The app keeps a [`Registry`] of the extensions that are
//! on: each one's name, the permissions the user granted it (changed at
//! once when the user revokes one), the process id of each adapter Pitwall
//! started for it, its call budget and its activity log.
//!
//! An extension calls Pitwall two ways, with the same permissions:
//! - its adapter's stdio (protocol 2 `call`), which the app answers through
//!   [`crate::Api`] with the extension as the caller;
//! - the socket, from a process whose parent chain reaches one of its
//!   adapters ([`crate::identity`]): such a caller is the extension's, not
//!   "outside".
//!
//! What this doesn't stop: a process that leaves its adapter's process tree
//! (a double fork that is reparented to init or launchd, `setsid` and
//! `nohup` under a new parent, a launch agent, a cron job) is no longer
//! the extension's descendant and connects as any program of the user's
//! does. The adapter runs as the user; the permissions bound what Pitwall
//! gives, not what the user's account can do. Signing and sandboxing are a
//! later phase. On Windows a pid can be reused once its process ends, so a
//! stale entry could name another process for a moment; entries are
//! removed when an adapter stops.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pitwall_core::extensions::activity::{self, Entry, Outcome, Via};
use pitwall_proto::{code, ErrorBody};

/// The most calls an extension makes in [`WINDOW`] (its adapter and the
/// processes it started together); more are answered `busy`.
pub const MAX_CALLS: usize = 20;
/// The window [`MAX_CALLS`] is counted over.
pub const WINDOW: Duration = Duration::from_secs(1);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One extension that is on.
struct Ext {
    name: String,
    grants: Vec<String>,
    activity: Option<activity::Shared>,
    calls: VecDeque<Instant>,
}

/// The extensions that are on, as callers.
#[derive(Default)]
pub struct Registry {
    exts: Mutex<HashMap<String, Ext>>,
    /// Adapter process id → its extension.
    pids: Mutex<HashMap<u32, String>>,
}

impl Registry {
    pub fn new() -> Arc<Registry> {
        Arc::new(Registry::default())
    }

    /// Extension `id` is on, named `name`, granted `grants` (replacing what
    /// it had: a revoke applies to the next call), its calls logged in
    /// `activity`.
    pub fn set(&self, id: &str, name: &str, grants: Vec<String>, activity: Option<activity::Shared>) {
        let mut exts = lock(&self.exts);
        match exts.get_mut(id) {
            Some(e) => {
                e.name = name.to_string();
                e.grants = grants;
                e.activity = activity;
            }
            None => {
                exts.insert(id.to_string(), Ext { name: name.to_string(), grants, activity, calls: VecDeque::new() });
            }
        }
    }

    /// Extension `id` is off: it is granted nothing, and its adapters are
    /// forgotten.
    pub fn remove(&self, id: &str) {
        lock(&self.exts).remove(id);
        lock(&self.pids).retain(|_, e| e != id);
    }

    /// The extensions on now.
    pub fn ids(&self) -> Vec<String> {
        lock(&self.exts).keys().cloned().collect()
    }

    /// Extension `id`'s name, when it is on.
    pub fn of_id(&self, id: &str) -> Option<String> {
        lock(&self.exts).get(id).map(|e| e.name.clone())
    }

    /// An adapter of extension `id` runs as process `pid`.
    pub fn adapter_started(&self, id: &str, pid: u32) {
        lock(&self.pids).insert(pid, id.to_string());
    }

    /// Process `pid`, an adapter, stopped.
    pub fn adapter_stopped(&self, pid: u32) {
        lock(&self.pids).remove(&pid);
    }

    /// The extension (id, name) whose adapter is the nearest in `chain` (a
    /// process and its parents, nearest first), with its position there.
    pub fn of_chain(&self, chain: &[u32]) -> Option<(usize, (String, String))> {
        let pids = lock(&self.pids);
        let exts = lock(&self.exts);
        chain.iter().enumerate().find_map(|(i, p)| {
            let id = pids.get(p)?;
            let e = exts.get(id)?;
            Some((i, (id.clone(), e.name.clone())))
        })
    }

    /// What extension `id` is granted now (nothing when it is off).
    pub fn grants(&self, id: &str) -> Vec<String> {
        lock(&self.exts).get(id).map(|e| e.grants.clone()).unwrap_or_default()
    }

    /// Extension `id` is granted `permission` now.
    pub fn allows(&self, id: &str, permission: &str) -> bool {
        lock(&self.exts).get(id).is_some_and(|e| e.grants.iter().any(|g| g == permission))
    }

    /// Count a call of extension `id`: `Err(busy)` past [`MAX_CALLS`] in
    /// [`WINDOW`], or when it is off.
    pub fn admit(&self, id: &str) -> Result<(), ErrorBody> {
        self.admit_at(id, Instant::now())
    }

    fn admit_at(&self, id: &str, now: Instant) -> Result<(), ErrorBody> {
        let mut exts = lock(&self.exts);
        let Some(e) = exts.get_mut(id) else {
            return Err(ErrorBody::new(code::DENIED, "This extension is off."));
        };
        while e.calls.front().is_some_and(|t| now.duration_since(*t) >= WINDOW) {
            e.calls.pop_front();
        }
        if e.calls.len() >= MAX_CALLS {
            return Err(ErrorBody::new(BUSY, format!("More than {MAX_CALLS} calls a second: slow down.")));
        }
        e.calls.push_back(now);
        Ok(())
    }

    /// Add `entry` to extension `id`'s activity log.
    pub fn record(&self, id: &str, entry: Entry) {
        let log = lock(&self.exts).get(id).and_then(|e| e.activity.clone());
        if let Some(l) = log {
            lock(&l).push(entry);
        }
    }

    /// Log how call `method` of extension `id` from `via` went.
    pub fn record_call(&self, id: &str, via: Via, method: &str, res: &Result<serde_json::Value, ErrorBody>) {
        let outcome = match res {
            Ok(_) => Outcome::Ok,
            Err(e) if refused(&e.code) => Outcome::Refused { code: e.code.clone(), message: e.message.clone() },
            Err(e) => Outcome::Failed { code: e.code.clone(), message: e.message.clone() },
        };
        self.record(id, Entry::now(via, method, outcome));
    }
}

/// An extension over its call budget.
pub const BUSY: &str = "busy";

/// The error codes that mean Pitwall refused (rather than that it failed).
fn refused(c: &str) -> bool {
    c == code::DENIED || c == code::UNSUPPORTED || c == BUSY || c == code::UNKNOWN_METHOD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_change_at_once_and_go_with_the_extension() {
        let r = Registry::new();
        assert!(!r.allows("ci", "agents.read"));
        r.set("ci", "CI", vec!["agents.read".into()], None);
        assert!(r.allows("ci", "agents.read") && !r.allows("ci", "projects.read"));
        r.set("ci", "CI", vec![], None);
        assert!(!r.allows("ci", "agents.read"), "revoked: the next call is refused");
        r.adapter_started("ci", 500);
        assert_eq!(r.of_chain(&[700, 600, 500, 1]), Some((2, ("ci".into(), "CI".into()))));
        assert_eq!(r.of_chain(&[700, 600]), None);
        r.adapter_stopped(500);
        assert_eq!(r.of_chain(&[500]), None);
        r.adapter_started("ci", 501);
        r.remove("ci");
        assert_eq!(r.of_chain(&[501]), None, "off: its adapters are forgotten");
        assert_eq!(r.admit("ci").unwrap_err().code, code::DENIED);
    }

    #[test]
    fn calls_past_the_budget_are_busy_then_allowed_again() {
        let r = Registry::new();
        r.set("ci", "CI", vec![], None);
        let t = Instant::now();
        for _ in 0..MAX_CALLS {
            r.admit_at("ci", t).unwrap();
        }
        assert_eq!(r.admit_at("ci", t + Duration::from_millis(500)).unwrap_err().code, BUSY);
        assert!(r.admit_at("ci", t + WINDOW).is_ok(), "a second later it may call again");
    }

    #[test]
    fn calls_are_logged_with_how_they_went() {
        let r = Registry::new();
        let log: activity::Shared = Arc::default();
        r.set("ci", "CI", vec![], Some(log.clone()));
        r.record_call("ci", Via::Socket, "queue.add", &Err(ErrorBody::new(code::UNSUPPORTED, "not yet")));
        r.record_call("ci", Via::Adapter, "agent.status", &Err(ErrorBody::new(code::NOT_FOUND, "no agent")));
        r.record_call("ci", Via::Adapter, "agent.list", &Ok(serde_json::json!([])));
        let e = lock(&log).entries();
        assert_eq!(e.len(), 3);
        assert!(e[0].refused() && !e[1].refused());
        assert_eq!((e[2].method.as_str(), &e[2].outcome), ("agent.list", &Outcome::Ok));
    }
}
