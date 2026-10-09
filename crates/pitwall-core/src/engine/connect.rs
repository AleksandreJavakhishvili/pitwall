//! Reconnecting saved agents to their sessions, in the background.
//!
//! [`Engine::open`](super::Engine::open) never waits on a provider: every
//! saved agent comes back at once, "connecting…" (status `Unknown`, no
//! terminal), and this module attaches them afterwards, a few at a time
//! ([`PARALLEL`]), each attempt on its own thread. Local holders answer in
//! milliseconds, so those agents are live almost at once; a remote machine
//! that is slow, asleep or unreachable only delays its own agents.
//!
//! An attempt that takes longer than the timeout is given up on for display
//! ("unreachable — retrying") but left to finish: a late success still
//! attaches. Unreachable sessions are tried again with a growing pause
//! ([`RETRY_FIRST`] .. [`RETRY_MAX`]); a session that isn't running any more
//! leaves the agent stopped, as before. A dropped remote attachment whose
//! machine can't be asked comes here too (`lifecycle::relink`).
//!
//! Facts that need the machine (its label, the home folder) are looked up on
//! the same background thread after the attach, and cached per machine.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use super::{Engine, Shared};
use crate::error::{ErrorCode, PwError};
use crate::model::Status;
use crate::provider::{Locator, TermSize};

/// Attempts running at once (attempts past their timeout don't count).
pub(crate) const PARALLEL: usize = 6;
/// An attempt that hasn't answered by then shows as unreachable.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(20);
/// First pause before trying an unreachable session again; doubles up to
/// [`RETRY_MAX`].
const RETRY_FIRST: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(60);

/// `status_detail` while the first attempt runs.
pub const CONNECTING: &str = "connecting…";
/// `status_detail` while its machine can't be reached.
pub const UNREACHABLE: &str = "unreachable — retrying";

/// An agent waiting to be (re)attached to its session.
#[derive(Debug, Clone)]
pub struct Connect {
    /// Which attempt is current; a result for another one is dropped.
    gen: u64,
    /// When the running attempt began (`None`: none runs).
    started: Option<Instant>,
    /// The running attempt is past its timeout (shown as unreachable).
    late: bool,
    /// Failed attempts so far (the pause grows with them).
    failures: u32,
    /// When to try again (no attempt running).
    next: Instant,
}

impl Connect {
    pub(crate) fn now() -> Connect {
        Connect { gen: 0, started: None, late: false, failures: 0, next: Instant::now() }
    }

    /// After `failures` failed attempts: try again after a pause.
    fn later(failures: u32) -> Connect {
        let pause = RETRY_FIRST.saturating_mul(1 << failures.saturating_sub(1).min(8)).min(RETRY_MAX);
        Connect { gen: 0, started: None, late: false, failures, next: Instant::now() + pause }
    }

    /// An attempt is running now.
    pub fn in_flight(&self) -> bool {
        self.started.is_some()
    }

    /// It may still attach to its session any moment: an attempt runs, or
    /// the first one hasn't started yet. Restarting meanwhile could leave
    /// two of it.
    pub fn busy(&self) -> bool {
        self.in_flight() || self.failures == 0
    }
}

/// Engine-wide state of the reconnector.
pub(crate) struct Connector {
    running: AtomicBool,
    gen: AtomicU64,
    timeout_ms: AtomicU64,
    retry_ms: AtomicU64,
    /// The loop's thread (woken when an attempt ends).
    thread: std::sync::Mutex<Option<std::thread::Thread>>,
}

impl Default for Connector {
    fn default() -> Self {
        Connector {
            running: AtomicBool::new(false),
            gen: AtomicU64::new(0),
            timeout_ms: AtomicU64::new(ATTEMPT_TIMEOUT.as_millis() as u64),
            retry_ms: AtomicU64::new(0),
            thread: Default::default(),
        }
    }
}

impl Connector {
    fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms.load(Ordering::Relaxed))
    }

    /// Tests: a shorter attempt timeout and (non-zero) a fixed retry pause.
    #[cfg(test)]
    pub(crate) fn set_timing(&self, timeout: Duration, retry: Duration) {
        self.timeout_ms.store(timeout.as_millis() as u64, Ordering::Relaxed);
        self.retry_ms.store(retry.as_millis() as u64, Ordering::Relaxed);
    }

    /// An attempt ended (a slot is free): look again now.
    fn poke(&self) {
        if let Some(t) = &*self.thread.lock().unwrap_or_else(|e| e.into_inner()) {
            t.unpark();
        }
    }

    fn retry_after(&self, failures: u32) -> Connect {
        let mut c = Connect::later(failures);
        let fixed = self.retry_ms.load(Ordering::Relaxed);
        if fixed > 0 {
            c.next = Instant::now() + Duration::from_millis(fixed);
        }
        c
    }
}

/// Make sure the reconnect loop runs (it ends when nothing is waiting).
pub(crate) fn kick(core: &Shared) {
    if core.connector.running.swap(true, Ordering::AcqRel) {
        return;
    }
    let weak = Arc::downgrade(core);
    let spawned = std::thread::Builder::new().name("reconnect".into()).spawn(move || run(weak));
    if spawned.is_err() {
        core.connector.running.store(false, Ordering::Release);
    }
}

/// Agent `id` can't reach its session right now: show it and try again later.
pub(crate) fn retry_later(core: &Shared, id: &str) {
    let failures = core.with(id, |a| a.connect.as_ref().map_or(0, |c| c.failures)).unwrap_or(0) + 1;
    let next = core.connector.retry_after(failures);
    let _ = core.with(id, |a| {
        a.host = None;
        a.connect = Some(next);
        a.status = Status::Unknown;
        a.detail = Some(UNREACHABLE.into());
    });
    core.changed(false);
    kick(core);
}

/// How long the loop sleeps at least / at most between looks.
const NAP_MIN: Duration = Duration::from_millis(20);
const NAP_MAX: Duration = Duration::from_secs(1);

fn run(weak: Weak<Engine>) {
    if let Some(core) = weak.upgrade() {
        *core.connector.thread.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::thread::current());
    }
    loop {
        let Some(core) = weak.upgrade() else { return };
        let now = Instant::now();
        let timeout = core.connector.timeout();
        let mut start = Vec::new();
        let mut waiting = false;
        let mut changed = false;
        let mut wake = now + NAP_MAX;
        {
            let mut agents = core.agents();
            let mut busy = agents
                .iter()
                .filter_map(|a| a.connect.as_ref())
                .filter(|c| c.in_flight() && !c.late)
                .count();
            for a in agents.iter_mut() {
                let Some(c) = a.connect.as_mut() else { continue };
                waiting = true;
                match c.started {
                    Some(t) if !c.late && now.duration_since(t) >= timeout => {
                        c.late = true;
                        busy -= 1;
                        a.detail = Some(UNREACHABLE.into());
                        changed = true;
                    }
                    Some(t) if !c.late => wake = wake.min(t + timeout),
                    Some(_) => {}
                    None if c.next <= now => start.push(a.rec.id.clone()),
                    None => wake = wake.min(c.next),
                }
            }
            // This Mac's agents first: they answer at once.
            start.sort_by_key(|id| {
                agents.iter().find(|a| &a.rec.id == id).is_none_or(|a| !a.facts.provider.local_process)
            });
            start.truncate(PARALLEL.saturating_sub(busy));
            for id in &start {
                if let Some(c) = agents.iter_mut().find(|a| &a.rec.id == id).and_then(|a| a.connect.as_mut()) {
                    c.gen = core.connector.gen.fetch_add(1, Ordering::Relaxed) + 1;
                    c.started = Some(now);
                    c.late = false;
                }
            }
        }
        if changed {
            core.changed(false);
        }
        for id in start {
            let core = core.clone();
            let gen = core.with(&id, |a| a.connect.as_ref().map_or(0, |c| c.gen)).unwrap_or(0);
            let _ = std::thread::Builder::new().name("reconnect-one".into()).spawn(move || attempt(&core, &id, gen));
        }
        if !waiting {
            core.connector.running.store(false, Ordering::Release);
            // Something may have started waiting since we looked.
            let again = core.agents().iter().any(|a| a.connect.is_some());
            if !again || core.connector.running.swap(true, Ordering::AcqRel) {
                return;
            }
        }
        drop(core);
        // Until a retry or timeout is due, or an attempt ends (`poke`).
        std::thread::park_timeout(wake.saturating_duration_since(Instant::now()).clamp(NAP_MIN, NAP_MAX));
    }
}

/// Whether a failed attach means "try again later" rather than "not running".
fn retryable(e: &PwError, local: bool, late: bool) -> bool {
    match e.code {
        ErrorCode::Unreachable => true,
        ErrorCode::NotRunning | ErrorCode::NotFound | ErrorCode::Unsupported | ErrorCode::Denied | ErrorCode::Conflict => false,
        // Timeouts and broken connections to another machine.
        ErrorCode::Other => late || !local,
    }
}

/// One attempt for agent `id` (attempt `gen`). Blocking: its own thread.
fn attempt(core: &Shared, id: &str, gen: u64) {
    let Ok((loc, size, local)) =
        core.with(id, |a| (a.rec.locator(), TermSize::from_pair(a.rec.term_size()), a.facts.provider.local_process))
    else {
        return;
    };
    let current = |a: &super::Agent| a.connect.as_ref().is_some_and(|c| c.gen == gen);
    let res = core.providers().for_locator(&loc).and_then(|p| p.attach(&loc, size));
    match res {
        Ok(io) => {
            if core.with(id, |a| current(a)).unwrap_or(false) {
                let host = core.host_for(io, size);
                if let Some((cols, rows)) = core.with(id, |a| a.rec.term_size()).ok().flatten() {
                    host.resize(cols, rows);
                }
                let now = core.now();
                let kept = core.with(id, |a| {
                    if !current(a) {
                        return false;
                    }
                    a.connect = None;
                    a.attach_host(host.clone(), now);
                    true
                });
                // Removed meanwhile (`remove` ended the session): just let go.
                let _ = kept;
            }
        }
        Err(e) => {
            let late = core.with(id, |a| a.connect.as_ref().is_some_and(|c| c.late)).unwrap_or(false);
            if !core.with(id, |a| current(a)).unwrap_or(false) {
                return;
            }
            if retryable(&e, local, late) {
                retry_later(core, id);
                core.connector.poke();
                return;
            }
            let now = core.now();
            let _ = core.with(id, |a| {
                if current(a) {
                    a.connect = None;
                    a.status = Status::Stopped;
                    a.detail = None;
                    a.status_since = now;
                }
            });
        }
    }
    core.changed(false);
    core.connector.poke();
    refresh_facts(core, id, &loc);
}

/// Look up what needs the agent's machine (label, home) and show it.
pub(crate) fn refresh_facts(core: &Shared, id: &str, loc: &Locator) {
    let Ok(rec) = core.with(id, |a| a.rec.clone()) else { return };
    let facts = core.facts(loc, &rec);
    if core
        .with(id, |a| {
            a.facts = facts;
            a.facts_pending = false;
        })
        .is_ok()
    {
        core.changed(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Deps, Engine};
    use crate::exec::LocalExec;
    use crate::model::AgentView;
    use crate::paths::Paths;
    use crate::provider::{MachineId, ProviderId};
    use crate::testing::{record, FakeProvider, ManualClock, MemStore, RecordingSink, TempDir};

    fn wait_until(secs: u64, f: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        f()
    }

    struct World {
        engine: Shared,
        vm: Arc<FakeProvider>,
        _dir: TempDir,
    }

    /// `locals` agents on this Mac whose sessions survived (one more whose
    /// session ended), and one agent on a remote machine `vm`.
    fn world(locals: usize, vm_slow: Duration) -> World {
        let dir = TempDir::new("connect");
        let local = FakeProvider::named(ProviderId::LOCAL, MachineId::THIS_MAC, Arc::new(LocalExec));
        let vm = FakeProvider::named("vmhost", "box", Arc::new(LocalExec));
        vm.set_remote(true);
        vm.set_slow(vm_slow);
        let cwd = dir.path().to_string_lossy().into_owned();
        let mut recs = Vec::new();
        for i in 0..locals {
            let id = format!("l{i}");
            local.add_session(&id, "shell", &cwd);
            recs.push(record(&id, &cwd));
        }
        recs.push(record("gone", &cwd));
        vm.add_session("far", "shell", &cwd);
        let mut far = record("far", &cwd);
        far.locator = Some(Locator::new(&ProviderId::new("vmhost"), &MachineId::new("box"), "far"));
        recs.push(far);
        let engine = Engine::open(Deps {
            paths: Paths::new(dir.path().join("pitwall")),
            events: RecordingSink::new(),
            clock: ManualClock::new(1),
            store: MemStore::with(recs),
            providers: vec![local, vm.clone()],
        });
        World { engine, vm, _dir: dir }
    }

    fn view(e: &Engine, id: &str) -> AgentView {
        e.views().into_iter().find(|v| v.id == id).unwrap()
    }

    /// A machine that takes 10 s to answer doesn't hold up opening, nor the
    /// agents on this Mac; it shows as connecting, then unreachable.
    #[test]
    fn open_never_waits_on_a_slow_machine() {
        let t = Instant::now();
        let w = world(8, Duration::from_secs(10));
        let opened = t.elapsed();
        assert!(opened < Duration::from_millis(100), "open took {opened:?}");
        w.engine.connector.set_timing(Duration::from_millis(400), Duration::from_secs(30));
        let far = view(&w.engine, "far");
        assert_eq!((far.status, far.running, far.status_detail.as_deref()), (Status::Unknown, false, Some(CONNECTING)));
        assert!(!far.caps.restart && !far.caps.input, "nothing to restart or type into while it may attach");

        let live = |e: &Engine| (0..8).all(|i| view(e, &format!("l{i}")).running);
        assert!(wait_until(2, || live(&w.engine)), "this Mac's agents are live at once");
        assert!(t.elapsed() < Duration::from_secs(2));
        assert_eq!(view(&w.engine, "gone").status, Status::Stopped, "an ended session: stopped, as before");
        assert_eq!(view(&w.engine, "far").status_detail.as_deref(), Some(CONNECTING), "the slow one is still connecting");
        assert!(w.engine.host("far").is_err());

        assert!(wait_until(3, || view(&w.engine, "far").status_detail.as_deref() == Some(UNREACHABLE)), "then shows unreachable");
        let far = view(&w.engine, "far");
        assert_eq!((far.status, far.running), (Status::Unknown, false));
        assert!(crate::engine::lifecycle::restart(&w.engine, "far", None).unwrap_err().contains("still connecting"));
    }

    /// A slow attach that answers within the timeout attaches; its facts
    /// (machine label, home) arrive in the background.
    #[test]
    fn a_slow_session_attaches_when_it_answers() {
        let w = world(1, Duration::from_millis(300));
        assert_eq!(view(&w.engine, "far").machine.label, "box", "no listing asked while opening");
        assert!(wait_until(5, || view(&w.engine, "far").running));
        assert!(wait_until(5, || view(&w.engine, "far").machine.label == "Fake box"));
        assert_eq!(w.engine.connecting(), 0);
        assert!(w.engine.wait_connected(Duration::ZERO));
    }

    /// An unreachable machine is tried again until it answers.
    #[test]
    fn an_unreachable_machine_is_retried() {
        let dir_world = {
            let w = world(1, Duration::ZERO);
            w.vm.set_attach_error(Some(PwError::unreachable("machine is asleep")));
            w.engine.connector.set_timing(Duration::from_secs(5), Duration::from_millis(50));
            w
        };
        let w = dir_world;
        // The first attempt may have raced the error being set.
        if view(&w.engine, "far").running {
            return;
        }
        assert!(wait_until(3, || view(&w.engine, "far").status_detail.as_deref() == Some(UNREACHABLE)));
        assert!(!view(&w.engine, "far").running);
        w.vm.set_attach_error(None);
        assert!(wait_until(3, || view(&w.engine, "far").running), "attached once it answers");
        assert_eq!(view(&w.engine, "far").status_detail, None);
    }

    #[test]
    fn the_retry_pause_grows_to_a_cap() {
        let pause = |n| Connect::later(n).next.saturating_duration_since(Instant::now());
        assert!(pause(1) <= RETRY_FIRST && pause(1) > RETRY_FIRST - Duration::from_secs(1));
        assert!(pause(2) > RETRY_FIRST);
        assert!(pause(30) <= RETRY_MAX && pause(30) > RETRY_MAX - Duration::from_secs(1));
    }

    #[test]
    fn what_is_worth_retrying() {
        assert!(retryable(&PwError::unreachable("asleep"), true, false));
        assert!(!retryable(&PwError::not_running(), false, true));
        assert!(retryable(&PwError::other("timed out"), false, false), "remote: broken connection");
        assert!(!retryable(&PwError::other("bad socket"), true, false), "this Mac: answered, but no");
        assert!(retryable(&PwError::other("timed out"), true, true));
    }
}
