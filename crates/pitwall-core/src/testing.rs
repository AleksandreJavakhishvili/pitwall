//! Test doubles for the host-facing traits, so engine behaviour can be tested
//! without a UI, real time or `~/.pitwall`. Built for this crate's tests and,
//! with the `testing` feature, for hosts' tests.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::clock::Clock;
use crate::engine::{Deps, Engine, Shared};
use crate::events::{Event, EventSink};
use crate::model::AgentRecord;
use crate::paths::Paths;
use crate::store::{Snapshot, Store};

/// Keeps every event, in order.
#[derive(Default)]
pub struct RecordingSink {
    events: Mutex<Vec<Event>>,
}

impl RecordingSink {
    pub fn new() -> Arc<RecordingSink> {
        Arc::new(RecordingSink::default())
    }

    /// Everything emitted so far (and forget it).
    pub fn take(&self) -> Vec<Event> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl EventSink for RecordingSink {
    fn emit(&self, event: Event) {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).push(event);
    }
}

/// A clock that only moves when told to.
pub struct ManualClock(AtomicU64);

impl ManualClock {
    pub fn new(start_ms: u64) -> Arc<ManualClock> {
        Arc::new(ManualClock(AtomicU64::new(start_ms.max(1))))
    }

    pub fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::Relaxed);
    }
}

impl Clock for ManualClock {
    fn mono_ms(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// An in-memory [`Store`] that counts saves.
#[derive(Default)]
pub struct MemStore {
    snapshot: Mutex<Snapshot>,
    saves: AtomicU64,
}

impl MemStore {
    pub fn with(agents: Vec<AgentRecord>) -> Arc<MemStore> {
        Arc::new(MemStore { snapshot: Mutex::new(Snapshot { agents }), saves: AtomicU64::new(0) })
    }

    pub fn saves(&self) -> u64 {
        self.saves.load(Ordering::Relaxed)
    }
}

impl Store for MemStore {
    fn load(&self) -> Snapshot {
        self.snapshot.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn save(&self, snapshot: &Snapshot) -> Result<(), String> {
        *self.snapshot.lock().unwrap_or_else(|e| e.into_inner()) = snapshot.clone();
        self.saves.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

/// A fresh directory under the system temp dir, deleted on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("pitwall-core-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A minimal stored agent record.
pub fn record(id: &str, cwd: &str) -> AgentRecord {
    serde_json::from_value(serde_json::json!({
        "id": id, "name": id, "kind": "shell", "kindName": "Shell",
        "cwd": cwd, "project": cwd, "createdAt": 1
    }))
    .expect("valid record")
}

/// An engine wired to fakes, with Pitwall's files in a temp dir. No
/// background threads run; tests call the pieces they exercise.
pub struct Harness {
    pub engine: Shared,
    pub events: Arc<RecordingSink>,
    pub clock: Arc<ManualClock>,
    pub store: Arc<MemStore>,
    pub dir: TempDir,
}

impl Harness {
    pub fn new(agents: Vec<AgentRecord>) -> Harness {
        let dir = TempDir::new("engine");
        let (events, clock, store) = (RecordingSink::new(), ManualClock::new(1_000), MemStore::with(agents));
        let engine = Engine::open(Deps {
            paths: Paths::new(dir.path().join("pitwall")),
            events: events.clone(),
            clock: clock.clone(),
            store: store.clone(),
            holder_bin: Err("no terminal holder in tests".into()),
        });
        Harness { engine, events, clock, store, dir }
    }
}
