//! Test doubles for the host-facing traits, so engine behaviour can be tested
//! without a UI, real time, real processes or `~/.pitwall`. Built for this
//! crate's tests and, with the `testing` feature, for hosts' tests.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub mod contract;
mod fake;

pub use fake::{FakeProvider, FakeTerm, FakeTermCtl, Launched};

use crate::clock::Clock;
use crate::engine::{Deps, Engine, Shared};
use crate::events::{Event, EventSink};
use crate::error::{PwError, Result};
use crate::exec::{Cmd, DirEntry, Exec, FileKind, LocalExec, Out, Stat};
use crate::model::AgentRecord;
use crate::paths::Paths;
use crate::provider::{MachineId, ProviderId};
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

    fn save(&self, snapshot: &Snapshot) -> std::result::Result<(), String> {
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
/// background threads run (saved agents are re-attached before `new`
/// returns); tests call the pieces they exercise.
pub struct Harness {
    pub engine: Shared,
    /// Where the agents run: `local:this-mac` (what records without a
    /// locator mean), with fake shells for terminals.
    pub provider: Arc<FakeProvider>,
    pub events: Arc<RecordingSink>,
    pub clock: Arc<ManualClock>,
    pub store: Arc<MemStore>,
    pub dir: TempDir,
}

impl Harness {
    /// Git and files on this machine (tests use temp repos).
    pub fn new(agents: Vec<AgentRecord>) -> Harness {
        Harness::with_exec(agents, Arc::new(LocalExec))
    }

    /// Agents' machine is `exec` (e.g. a [`FakeExec`]).
    pub fn with_exec(agents: Vec<AgentRecord>, exec: Arc<dyn Exec>) -> Harness {
        let dir = TempDir::new("engine");
        let (events, clock, store) = (RecordingSink::new(), ManualClock::new(1_000), MemStore::with(agents));
        let provider = FakeProvider::named(ProviderId::LOCAL, MachineId::THIS_MAC, exec);
        let engine = Engine::open(Deps {
            paths: Paths::new(dir.path().join("pitwall")),
            events: events.clone(),
            clock: clock.clone(),
            store: store.clone(),
            providers: vec![provider.clone()],
        });
        // Saved agents are re-attached in the background: settle first.
        assert!(engine.wait_connected(std::time::Duration::from_secs(10)), "saved agents settle");
        Harness { engine, provider, events, clock, store, dir }
    }
}

// ---------------------------------------------------------------- FakeExec

/// One command a [`FakeExec`] was asked to run.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub argv: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: std::time::Duration,
}

impl Call {
    pub fn env_has(&self, key: &str, value: &str) -> bool {
        self.env.iter().any(|(k, v)| k == key && v == value)
    }

    pub fn env_get(&self, key: &str) -> Option<&str> {
        self.env.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn matches(&self, pattern: &[&str]) -> bool {
        matches(pattern, &self.argv)
    }
}

/// `*` matches any one argument; a trailing `..` matches whatever follows.
fn matches(pattern: &[&str], argv: &[String]) -> bool {
    if let Some((&"..", head)) = pattern.split_last() {
        return argv.len() >= head.len() && matches(head, &argv[..head.len()]);
    }
    pattern.len() == argv.len() && pattern.iter().zip(argv).all(|(p, a)| *p == "*" || p == a)
}

enum Entry {
    File(Vec<u8>),
    Dir,
    /// A symlink to this (absolute) path.
    Link(String),
}

/// An argv pattern and what running a matching command returns.
type Script = (Vec<String>, Result<Out>);

/// A scripted machine: commands answer with canned output (the newest
/// matching script wins; an unscripted command fails to run), files live in
/// memory, and every call is recorded.
pub struct FakeExec {
    scripts: Mutex<Vec<Script>>,
    calls: Mutex<Vec<Call>>,
    files: Mutex<BTreeMap<String, Entry>>,
}

fn guard<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub const FAKE_TEMP: &str = "/fake/tmp";
pub const FAKE_HOME: &str = "/fake/home";

impl FakeExec {
    pub fn new() -> Arc<FakeExec> {
        let x = FakeExec { scripts: Mutex::default(), calls: Mutex::default(), files: Mutex::default() };
        x.dir(FAKE_TEMP).dir(FAKE_HOME);
        Arc::new(x)
    }

    /// `pattern` exits 0 printing `stdout`.
    pub fn on(&self, pattern: &[&str], stdout: &str) -> &Self {
        self.on_exit(pattern, 0, stdout, "")
    }

    pub fn on_exit(&self, pattern: &[&str], status: i32, stdout: &str, stderr: &str) -> &Self {
        let out = Out { status, stdout: stdout.as_bytes().to_vec(), stderr: stderr.as_bytes().to_vec() };
        self.script(pattern, Ok(out))
    }

    /// `pattern` can't be run (missing program, timeout).
    pub fn on_error(&self, pattern: &[&str], error: &str) -> &Self {
        self.script(pattern, Err(PwError::other(error)))
    }

    fn script(&self, pattern: &[&str], reply: Result<Out>) -> &Self {
        guard(&self.scripts).push((pattern.iter().map(|s| s.to_string()).collect(), reply));
        self
    }

    pub fn file(&self, path: &str, bytes: &[u8]) -> &Self {
        guard(&self.files).insert(path.to_string(), Entry::File(bytes.to_vec()));
        self
    }

    pub fn dir(&self, path: &str) -> &Self {
        guard(&self.files).insert(path.trim_end_matches('/').to_string(), Entry::Dir);
        self
    }

    /// A symlink at `path` pointing to the absolute path `target`.
    pub fn link(&self, path: &str, target: &str) -> &Self {
        guard(&self.files).insert(path.trim_end_matches('/').to_string(), Entry::Link(target.to_string()));
        self
    }

    /// `path` with every symlink in it resolved (`None`: a dangling or
    /// looping link).
    fn resolve(&self, path: &str) -> Option<String> {
        let mut path = path.trim_end_matches('/').to_string();
        for _ in 0..40 {
            let files = guard(&self.files);
            let parts: Vec<&str> = path.split('/').collect();
            let hit = (1..=parts.len()).find_map(|n| match files.get(&parts[..n].join("/")) {
                Some(Entry::Link(t)) => Some((n, t.clone())),
                _ => None,
            });
            let Some((n, target)) = hit else { return Some(path) };
            path = std::iter::once(target.trim_end_matches('/')).chain(parts[n..].iter().copied()).collect::<Vec<_>>().join("/");
        }
        None
    }

    /// A file's contents, if it exists.
    pub fn contents(&self, path: &str) -> Option<Vec<u8>> {
        match guard(&self.files).get(path) {
            Some(Entry::File(b)) => Some(b.clone()),
            _ => None,
        }
    }

    /// Every path that exists (files and folders).
    pub fn paths(&self) -> Vec<String> {
        guard(&self.files).keys().cloned().collect()
    }

    pub fn calls(&self) -> Vec<Call> {
        guard(&self.calls).clone()
    }

    /// How many calls matched `pattern`.
    pub fn ran(&self, pattern: &[&str]) -> usize {
        guard(&self.calls).iter().filter(|c| c.matches(pattern)).count()
    }

    fn kind(&self, path: &str) -> Option<FileKind> {
        let files = guard(&self.files);
        match files.get(path) {
            Some(Entry::File(_)) => Some(FileKind::File),
            Some(Entry::Dir) => Some(FileKind::Dir),
            Some(Entry::Link(_)) => Some(FileKind::Symlink),
            // A folder that holds something exists implicitly.
            None => files.keys().any(|k| k.starts_with(&format!("{path}/"))).then_some(FileKind::Dir),
        }
    }
}

impl Exec for FakeExec {
    fn run(&self, cmd: &Cmd) -> Result<Out> {
        let argv: Vec<String> = cmd.argv.iter().map(|s| s.to_string()).collect();
        guard(&self.calls).push(Call {
            argv: argv.clone(),
            cwd: cmd.cwd.map(String::from),
            env: cmd.env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            stdin: cmd.stdin.map(<[u8]>::to_vec),
            timeout: cmd.timeout,
        });
        if cmd.cancelled() {
            return Err(PwError::other(format!("{}: cancelled", argv[0])));
        }
        let scripts = guard(&self.scripts);
        let hit = scripts.iter().rev().find(|(p, _)| matches(&p.iter().map(String::as_str).collect::<Vec<_>>(), &argv));
        match hit {
            Some((_, reply)) => reply.clone(),
            None => Err(PwError::other(format!("FakeExec: no script for {argv:?}"))),
        }
    }

    fn read_file(&self, path: &str, max: u64) -> Result<Option<Vec<u8>>> {
        let Some(path) = self.resolve(path) else { return Ok(None) };
        let path = path.as_str();
        match guard(&self.files).get(path) {
            Some(Entry::File(b)) => Ok(Some(b[..b.len().min(max as usize)].to_vec())),
            Some(Entry::Dir) => Err(PwError::other(format!("{path}: is a directory"))),
            Some(Entry::Link(_)) | None => Ok(None),
        }
    }

    fn write_file(&self, path: &str, bytes: &[u8]) -> Result<()> {
        if self.kind(path) == Some(FileKind::Dir) {
            return Err(PwError::other(format!("{path}: is a directory")));
        }
        self.file(path, bytes);
        Ok(())
    }

    fn remove_file(&self, path: &str) -> Result<()> {
        let mut files = guard(&self.files);
        match files.get(path) {
            Some(Entry::File(_)) => {
                files.remove(path);
                Ok(())
            }
            Some(Entry::Dir) => Err(PwError::other(format!("{path}: is a directory"))),
            Some(Entry::Link(_)) => {
                files.remove(path);
                Ok(())
            }
            None => Err(PwError::other(format!("{path}: not found"))),
        }
    }

    fn remove_dir(&self, path: &str) -> Result<()> {
        if self.kind(path) != Some(FileKind::Dir) {
            return Err(PwError::other(format!("{path}: not a directory")));
        }
        let mut files = guard(&self.files);
        if files.keys().any(|k| k.starts_with(&format!("{path}/"))) {
            return Err(PwError::other(format!("{path}: directory not empty")));
        }
        files.remove(path);
        Ok(())
    }

    fn copy_file(&self, from: &str, to: &str) -> Result<()> {
        let bytes = self.contents(from).ok_or_else(|| format!("{from}: not found"))?;
        self.write_file(to, &bytes)
    }

    fn stat(&self, path: &str) -> Result<Option<Stat>> {
        let len = self.contents(path).map_or(0, |b| b.len() as u64);
        Ok(self.kind(path).map(|kind| Stat { kind, len }))
    }

    fn real_path(&self, path: &str) -> Result<String> {
        let p = self.resolve(path).filter(|p| self.kind(p).is_some());
        p.ok_or_else(|| PwError::not_found(format!("{path}: not found")))
    }

    fn temp_dir(&self) -> Result<String> {
        Ok(FAKE_TEMP.into())
    }

    fn home(&self) -> Result<String> {
        Ok(FAKE_HOME.into())
    }

    fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>> {
        let dir = self.resolve(path).ok_or_else(|| PwError::not_found(format!("{path}: not found")))?;
        if self.kind(&dir) != Some(FileKind::Dir) {
            return Err(PwError::other(format!("{path}: not a directory")));
        }
        let prefix = format!("{dir}/");
        let names: std::collections::BTreeSet<String> = guard(&self.files)
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .filter_map(|rest| rest.split('/').next())
            .filter(|n| !n.is_empty())
            .map(String::from)
            .collect();
        Ok(names.into_iter().map(|name| DirEntry { kind: self.kind(&format!("{prefix}{name}")).unwrap_or(FileKind::Other), name }).collect())
    }
}
