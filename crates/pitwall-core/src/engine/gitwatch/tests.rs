use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::engine::ticker::run_git_due_for_test as run_due;
use crate::error::{PwError, Result as PwResult};
use crate::exec::watch::Hub;
use crate::exec::{Cmd, LocalExec, OnChange, Out, Stat, MAX_DIRS};
use crate::model::Status;
use crate::testing::{record, FakeExec, Harness};
use crate::vcs::snapshot::TempRepo;

/// This machine, counting git processes; watching can be turned off (an OS
/// that refuses) or made to break later (a folder it couldn't add).
#[derive(Default)]
pub(crate) struct CountingExec {
    pub runs: AtomicU64,
    pub micros: AtomicU64,
    pub refuse_watch: AtomicBool,
    pub broken: Arc<AtomicBool>,
    /// Its own OS watcher, so tests running at once don't see each other
    /// (an FSEvents restart tells every other checkout "changed").
    hub: std::sync::OnceLock<Arc<Hub>>,
}

struct Breakable {
    inner: Box<dyn Watching>,
    broken: Arc<AtomicBool>,
}

impl Watching for Breakable {
    fn healthy(&self) -> bool {
        !self.broken.load(Ordering::SeqCst) && self.inner.healthy()
    }
    fn complete(&self) -> bool {
        self.inner.complete()
    }
    fn set_ignored(&self, ignored: &[String]) {
        self.inner.set_ignored(ignored);
    }
}

impl CountingExec {
    pub fn git_runs(&self) -> u64 {
        self.runs.load(Ordering::SeqCst)
    }
}

impl Exec for CountingExec {
    fn run(&self, cmd: &Cmd) -> PwResult<Out> {
        let t = Instant::now();
        let out = LocalExec.run(cmd);
        if cmd.argv.first() == Some(&"git") {
            self.runs.fetch_add(1, Ordering::SeqCst);
            self.micros
                .fetch_add(t.elapsed().as_micros() as u64, Ordering::SeqCst);
        }
        out
    }
    fn read_file(&self, path: &str, max: u64) -> PwResult<Option<Vec<u8>>> {
        LocalExec.read_file(path, max)
    }
    fn write_file(&self, path: &str, bytes: &[u8]) -> PwResult<()> {
        LocalExec.write_file(path, bytes)
    }
    fn remove_file(&self, path: &str) -> PwResult<()> {
        LocalExec.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> PwResult<()> {
        LocalExec.remove_dir(path)
    }
    fn copy_file(&self, from: &str, to: &str) -> PwResult<()> {
        LocalExec.copy_file(from, to)
    }
    fn stat(&self, path: &str) -> PwResult<Option<Stat>> {
        LocalExec.stat(path)
    }
    fn real_path(&self, path: &str) -> PwResult<String> {
        LocalExec.real_path(path)
    }
    fn temp_dir(&self) -> PwResult<String> {
        LocalExec.temp_dir()
    }
    fn home(&self) -> PwResult<String> {
        LocalExec.home()
    }
    fn watch(&self, spec: &WatchSpec, on_change: OnChange) -> PwResult<Box<dyn Watching>> {
        if self.refuse_watch.load(Ordering::SeqCst) {
            return Err(PwError::other("inotify watch limit reached"));
        }
        let hub = self
            .hub
            .get_or_init(|| Hub::new(None, MAX_DIRS).expect("file watcher"));
        let inner = hub.add(spec, on_change)?;
        Ok(Box::new(Breakable {
            inner,
            broken: self.broken.clone(),
        }))
    }
}

fn repo() -> TempRepo {
    let r = TempRepo::new();
    r.write("a.txt", "1\n");
    r.write(".gitignore", "build/\nnode_modules/\ntarget/\n");
    r.commit_all("init");
    // Dependencies and build output that are already there (and ignored).
    r.write("node_modules/p/package.json", "{}");
    r.write("target/debug/.keep", "");
    r
}

/// An engine whose agents (`ids`, all in `dir`) run on this machine, which
/// can watch folders.
fn harness(dir: &str, ids: &[&str]) -> (Harness, Arc<CountingExec>) {
    let x = Arc::new(CountingExec::default());
    let h = Harness::with_exec(ids.iter().map(|id| record(id, dir)).collect(), x.clone());
    for id in ids {
        h.engine
            .with(id, |a| {
                a.facts.provider.fs_events = true;
                a.status = Status::Idle;
            })
            .unwrap();
    }
    (h, x)
}

/// Wait (real time) until agent `id`'s watch saw a change.
fn wait_changed(h: &Harness, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if h.engine.with(id, |a| a.fs.changed(&a.rec.cwd)).unwrap() == Some(true) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("the watch never saw the change");
}

/// Let the backend deliver anything still on its way, then call the agent
/// up to date (what a refresh at this moment would see).
fn settle(h: &Harness, id: &str) {
    std::thread::sleep(Duration::from_millis(700));
    h.engine
        .with(id, |a| {
            if let FsWatch::On { tree, seen, .. } = &mut a.fs {
                *seen = tree.gen();
            }
        })
        .unwrap();
}

fn numbers(h: &Harness) -> (u32, u32, u32, Option<String>) {
    let v = &h.engine.views()[0];
    (v.added, v.removed, v.files_changed, v.branch.clone())
}

#[test]
fn a_change_refreshes_and_no_change_runs_no_git() {
    let r = repo();
    let (h, x) = harness(r.path(), &["a"]);
    assert_eq!(run_due(&h.engine), ["a"], "the first look");
    assert!(
        h.engine
            .with("a", |a| a.fs.changed(r.path()))
            .unwrap()
            .is_some(),
        "watching"
    );
    settle(&h, "a");

    // Nothing changes: no git at all, idle or working (until the safety poll).
    let before = x.git_runs();
    for _ in 0..20 {
        h.clock.advance(1_000);
        assert!(run_due(&h.engine).is_empty());
    }
    h.engine.with("a", |a| a.status = Status::Working).unwrap();
    // The safety poll: 60 s where the watch sees everything (FSEvents,
    // Windows), 30 s where it skips folders by name (inotify).
    let safety = if h.engine.with("a", |a| a.fs.complete()).unwrap() {
        60
    } else {
        30
    };
    for _ in 20..safety - 1 {
        h.clock.advance(1_000);
        assert!(
            run_due(&h.engine).is_empty(),
            "working, but nothing changed"
        );
    }
    assert_eq!(x.git_runs(), before, "no git while nothing changes");
    h.clock.advance(1_000);
    assert_eq!(
        run_due(&h.engine),
        ["a"],
        "the safety poll after the last look"
    );
    settle(&h, "a");
    h.engine.with("a", |a| a.status = Status::Idle).unwrap();

    // An edit, even while idle: refreshed on the next tick.
    r.write("a.txt", "1\n2\n3\n");
    r.write("new.txt", "x\n");
    wait_changed(&h, "a");
    h.clock.advance(3_000);
    assert_eq!(run_due(&h.engine), ["a"]);
    assert_eq!(numbers(&h).0, 3);
    assert_eq!(numbers(&h).2, 2);

    // Dependencies and build output don't count.
    settle(&h, "a");
    r.write("node_modules/p/i.js", "x");
    r.write("target/debug/x", "x");
    std::thread::sleep(Duration::from_millis(700));
    h.clock.advance(5_000);
    assert!(run_due(&h.engine).is_empty(), "noise folders changed only");

    // An ignored folder that appears later counts once: the refresh finds
    // nothing, so git's ignore list is read again.
    h.clock.advance(RELIST_MS);
    r.write("build/out.o", "x");
    wait_changed(&h, "a");
    h.clock.advance(3_000);
    assert_eq!(run_due(&h.engine), ["a"]);
    settle(&h, "a");
    r.write("build/out2.o", "x");
    r.write("build/sub/out3.o", "x");
    std::thread::sleep(Duration::from_millis(700));
    h.clock.advance(5_000);
    assert!(
        run_due(&h.engine).is_empty(),
        "git-ignored paths changed only"
    );
}

#[test]
fn changes_are_paced_like_polling() {
    let r = repo();
    let (h, _x) = harness(r.path(), &["a"]);
    run_due(&h.engine);
    settle(&h, "a");
    h.clock.advance(1_000);
    r.write("a.txt", "2\n");
    wait_changed(&h, "a");
    assert!(
        run_due(&h.engine).is_empty(),
        "not sooner than 3 s after the last look"
    );
    h.clock.advance(2_000);
    assert_eq!(run_due(&h.engine), ["a"]);
}

#[test]
fn a_branch_switch_refreshes_the_branch() {
    let r = repo();
    let (h, _x) = harness(r.path(), &["a"]);
    run_due(&h.engine);
    assert_eq!(numbers(&h).3.as_deref(), Some("main"));
    settle(&h, "a");
    r.git(&["checkout", "-q", "-b", "feature"]);
    wait_changed(&h, "a");
    h.clock.advance(3_000);
    assert_eq!(run_due(&h.engine), ["a"]);
    assert_eq!(numbers(&h).3.as_deref(), Some("feature"));
}

#[test]
fn a_linked_worktree_is_watched_with_its_git_folders() {
    let r = repo();
    let wt = format!("{}-wt", r.path());
    r.git(&["worktree", "add", "-q", "-b", "wt", &wt]);
    let (h, _x) = harness(&wt, &["a"]);
    run_due(&h.engine);
    assert_eq!(numbers(&h).3.as_deref(), Some("wt"));
    settle(&h, "a");
    // Its HEAD lives in the main repository's .git/worktrees/<name>.
    Git::new(&LocalExec, &wt)
        .run(&["checkout", "-q", "-b", "wt2"])
        .unwrap();
    wait_changed(&h, "a");
    h.clock.advance(3_000);
    run_due(&h.engine);
    assert_eq!(numbers(&h).3.as_deref(), Some("wt2"));
    let _ = std::fs::remove_dir_all(&wt);
}

#[test]
fn agents_in_one_checkout_share_a_watch() {
    let r = repo();
    let (h, _x) = harness(r.path(), &["a", "b"]);
    run_due(&h.engine);
    assert_eq!(h.engine.trees.len(), 1);
    let same = {
        let agents = h.engine.agents();
        match (&agents[0].fs, &agents[1].fs) {
            (FsWatch::On { tree: t1, .. }, FsWatch::On { tree: t2, .. }) => Arc::ptr_eq(t1, t2),
            _ => false,
        }
    };
    assert!(same);
    // Stopped agents keep no watch.
    for id in ["a", "b"] {
        h.engine.with(id, |a| a.status = Status::Stopped).unwrap();
    }
    run_due(&h.engine);
    assert_eq!(h.engine.trees.len(), 0, "released with its last agent");
}

#[test]
fn where_the_os_refuses_a_watch_agents_are_polled() {
    let r = repo();
    let (h, x) = harness(r.path(), &["a"]);
    x.refuse_watch.store(true, Ordering::SeqCst);
    run_due(&h.engine);
    assert!(matches!(
        h.engine.with("a", |a| matches!(a.fs, FsWatch::Off { .. })),
        Ok(true)
    ));
    // Polled as before: while working, every 3 s (then backing off).
    h.engine.with("a", |a| a.status = Status::Working).unwrap();
    h.clock.advance(3_000);
    assert_eq!(run_due(&h.engine), ["a"]);
    // Tried again only after a while.
    let before = x.git_runs();
    x.refuse_watch.store(false, Ordering::SeqCst);
    h.clock.advance(6_000);
    run_due(&h.engine);
    assert_eq!(
        x.git_runs() - before,
        3,
        "a plain refresh: no new attempt yet"
    );
    h.clock.advance(RETRY_MS);
    run_due(&h.engine);
    assert!(
        h.engine
            .with("a", |a| a.fs.changed(r.path()))
            .unwrap()
            .is_some(),
        "watching now"
    );
}

#[test]
fn a_watch_that_breaks_falls_back_to_polling() {
    let r = repo();
    let (h, x) = harness(r.path(), &["a"]);
    run_due(&h.engine);
    settle(&h, "a");
    h.engine.with("a", |a| a.status = Status::Working).unwrap();
    h.clock.advance(3_000);
    assert!(run_due(&h.engine).is_empty(), "watched: nothing changed");
    x.broken.store(true, Ordering::SeqCst);
    assert_eq!(run_due(&h.engine), ["a"], "polled again");
}

#[test]
fn remote_agents_are_still_polled() {
    // A machine without file watching (agw): the provider says so.
    let x = FakeExec::new();
    x.on(&["git", "-C", "/w", "diff", ".."], "1\t0\ta.txt\0")
        .on(
            &["git", "-C", "/w", "rev-parse", ".."],
            "/w\n/w/.git\n/w/.git\n",
        )
        .on(&["git", "-C", "/w", "ls-files", ".."], "")
        .on(&["git", "-C", "/w", "symbolic-ref", ".."], "main\n");
    let h = Harness::with_exec(vec![record("a", "/w")], x.clone());
    h.engine
        .with("a", |a| {
            a.facts.provider.fs_events = false;
            a.status = Status::Working;
        })
        .unwrap();
    run_due(&h.engine);
    h.clock.advance(3_000);
    assert_eq!(run_due(&h.engine), ["a"], "polled every 3 s while working");
    assert_eq!(
        x.ran(&["git", "-C", "/w", "rev-parse", ".."]),
        0,
        "no watch attempted"
    );
    assert!(matches!(
        h.engine.with("a", |a| matches!(a.fs, FsWatch::None)),
        Ok(true)
    ));

    // Even when the provider says yes, a machine that can't watch is polled.
    h.engine
        .with("a", |a| a.facts.provider.fs_events = true)
        .unwrap();
    h.clock.advance(6_000);
    assert_eq!(run_due(&h.engine), ["a"]);
    assert!(matches!(
        h.engine.with("a", |a| matches!(a.fs, FsWatch::Off { .. })),
        Ok(true)
    ));
}
