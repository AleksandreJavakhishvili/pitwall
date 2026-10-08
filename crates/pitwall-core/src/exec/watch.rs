//! File-change notifications for checkouts on this machine
//! ([`LocalExec::watch`](super::LocalExec)), with the `notify` crate.
//!
//! One OS watcher serves every checkout (one inotify instance, one FSEvents
//! stream), and one thread filters its events and tells each checkout's owner.
//! Only changes that can change `git status` count: anything in the working
//! tree except git's internals (but its HEAD, index and refs) and what git
//! ignores (as the caller lists it: `WatchSpec::ignored`,
//! `Watching::set_ignored`). Such a watch is `complete`.
//!
//! Backends that watch one folder at a time (inotify, kqueue) get one watch
//! per folder, skipping those folders and also dependency and build folders
//! by name ([`NOISE_DIRS`], even where git doesn't ignore them: not
//! `complete`); new folders are added as they appear. Past [`MAX_DIRS`] (or when the OS refuses: Linux's
//! `max_user_watches`) watching fails and the caller polls instead.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, OnceLock, Weak};

use notify::event::{CreateKind, EventKind, MetadataKind, ModifyKind, RenameMode};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher, WatcherKind};

use super::{OnChange, PwError, Result, WatchSpec, Watching};
use crate::platform;

/// Dependency, build-output and cache folder names, not watched by
/// folder-by-folder backends (normally git-ignored anyway; skipped even
/// before git says so, so a fresh `node_modules` costs no watches).
pub const NOISE_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".parcel-cache",
    ".cache",
    ".gradle",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    ".tox",
];

/// Folders watched one by one (inotify, kqueue), all checkouts together.
pub const MAX_DIRS: usize = 32_768;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One watched checkout.
struct Root {
    tree: PathBuf,
    /// Git folders outside `tree`, longest first.
    git_dirs: Vec<PathBuf>,
    /// Absolute paths git ignores (as of the caller's last listing).
    ignored: Mutex<HashSet<PathBuf>>,
    on_change: OnChange,
    /// What this checkout asked the OS watcher for.
    paths: Mutex<Vec<PathBuf>>,
    /// A folder that appeared couldn't be watched: changes may be missed.
    broken: AtomicBool,
    /// Skip [`NOISE_DIRS`] even where git doesn't ignore them (folder-by-
    /// folder backends: watching them costs one OS watch per folder).
    skip_noise: bool,
}

/// Where a path stands for one checkout.
#[derive(Debug, PartialEq, Eq)]
enum Class {
    /// Not in this checkout.
    Outside,
    /// In it, but its changes don't matter.
    Ignored,
    Relevant,
}

/// A file inside a git folder that matters for `git status` and the branch.
fn git_file(rest: &[&OsStr]) -> bool {
    match rest {
        [name] => ["HEAD", "index", "packed-refs"]
            .iter()
            .any(|n| OsStr::new(n) == *name),
        [first, .., last] => *first == "refs" && !last.to_string_lossy().ends_with(".lock"),
        [] => false,
    }
}

impl Root {
    fn classify(&self, p: &Path) -> Class {
        for g in &self.git_dirs {
            if let Ok(rel) = p.strip_prefix(g) {
                let parts: Vec<&OsStr> = rel.iter().collect();
                return if git_file(&parts) {
                    Class::Relevant
                } else {
                    Class::Ignored
                };
            }
        }
        let Ok(rel) = p.strip_prefix(&self.tree) else {
            return Class::Outside;
        };
        let parts: Vec<&OsStr> = rel.iter().collect();
        for (i, part) in parts.iter().enumerate() {
            if *part == ".git" {
                return if git_file(&parts[i + 1..]) {
                    Class::Relevant
                } else {
                    Class::Ignored
                };
            }
            if self.skip_noise && NOISE_DIRS.iter().any(|n| OsStr::new(n) == *part) {
                return Class::Ignored;
            }
        }
        let ignored = lock(&self.ignored);
        let mut at = Some(p);
        while let Some(a) = at.filter(|a| *a != self.tree) {
            if ignored.contains(a) {
                return Class::Ignored;
            }
            at = a.parent();
        }
        Class::Relevant
    }

    /// Folders to watch one by one under `dir` (itself included): skips
    /// noise, ignored folders and symlinks; a `.git` folder contributes
    /// itself and its `refs`. `None` past `budget`.
    fn dirs_under(&self, dir: &Path, budget: usize) -> Option<Vec<PathBuf>> {
        let mut out = Vec::new();
        let mut todo = vec![dir.to_path_buf()];
        while let Some(d) = todo.pop() {
            out.push(d.clone());
            if out.len() > budget {
                return None;
            }
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in entries.flatten() {
                if !e.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                let path = e.path();
                if e.file_name() == ".git" {
                    out.push(path.clone());
                    todo.extend(all_dirs(&path.join("refs")));
                } else if self.classify(&path) == Class::Relevant {
                    todo.push(path);
                }
            }
        }
        Some(out)
    }
}

/// `dir` and every folder below it (git refs), none when it doesn't exist.
fn all_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        out.push(d);
        for e in entries.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                todo.push(e.path());
            }
        }
    }
    out
}

struct State {
    watcher: RecommendedWatcher,
    /// Paths given to the OS watcher, with how many checkouts asked.
    paths: HashMap<PathBuf, usize>,
}

impl State {
    fn add(&mut self, path: &Path, mode: RecursiveMode) -> Result<()> {
        if let Some(n) = self.paths.get_mut(path) {
            *n += 1;
            return Ok(());
        }
        self.watcher
            .watch(path, mode)
            .map_err(|e| PwError::other(format!("watch {}: {e}", path.display())))?;
        self.paths.insert(path.to_path_buf(), 1);
        Ok(())
    }

    fn remove(&mut self, path: &Path) {
        match self.paths.get_mut(path) {
            Some(n) if *n > 1 => *n -= 1,
            Some(_) => {
                self.paths.remove(path);
                let _ = self.watcher.unwatch(path);
            }
            None => {}
        }
    }
}

/// The one OS watcher and the checkouts it serves.
pub(crate) struct Hub {
    state: Mutex<State>,
    roots: Mutex<Vec<Arc<Root>>>,
    /// One watch per folder (inotify, kqueue) instead of one per tree.
    per_dir: bool,
    /// Adding a path restarts the OS stream (FSEvents): events in that
    /// moment may be lost, so every other checkout is told "changed".
    restarts: bool,
    max_dirs: usize,
}

impl Hub {
    /// `per_dir`: force one watch per folder (tests); `None` = what the
    /// backend needs.
    pub(crate) fn new(per_dir: Option<bool>, max_dirs: usize) -> Result<Arc<Hub>> {
        let kind = <RecommendedWatcher as Watcher>::kind();
        if !matches!(
            kind,
            WatcherKind::Inotify
                | WatcherKind::Fsevent
                | WatcherKind::Kqueue
                | WatcherKind::ReadDirectoryChangesWatcher
        ) {
            return Err(PwError::unsupported(
                "no file-change notifications on this system",
            ));
        }
        let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
        let watcher = notify::recommended_watcher(tx)
            .map_err(|e| PwError::other(format!("file watcher: {e}")))?;
        let hub = Arc::new(Hub {
            state: Mutex::new(State {
                watcher,
                paths: HashMap::new(),
            }),
            roots: Mutex::default(),
            per_dir: per_dir.unwrap_or(matches!(kind, WatcherKind::Inotify | WatcherKind::Kqueue)),
            restarts: kind == WatcherKind::Fsevent,
            max_dirs,
        });
        let weak = Arc::downgrade(&hub);
        std::thread::Builder::new()
            .name("fs-watch".into())
            .spawn(move || dispatch_loop(weak, rx))
            .map_err(|e| PwError::other(format!("file watcher: {e}")))?;
        Ok(hub)
    }

    pub(crate) fn add(
        self: &Arc<Self>,
        spec: &WatchSpec,
        on_change: OnChange,
    ) -> Result<Box<dyn Watching>> {
        let canon =
            |p: &str| platform::canonicalize(Path::new(p)).map_err(|e| PwError::from(e).context(p));
        let tree = canon(&spec.tree)?;
        let mut git_dirs: Vec<PathBuf> = spec
            .git_dirs
            .iter()
            .filter_map(|g| canon(g).ok())
            .filter(|g| !g.starts_with(&tree))
            .collect();
        git_dirs.sort_by_key(|g| Reverse(g.as_os_str().len()));
        git_dirs.dedup();
        let ignored = Mutex::new(absolute(&tree, &spec.ignored));
        let root = Arc::new(Root {
            tree: tree.clone(),
            git_dirs: git_dirs.clone(),
            ignored,
            on_change,
            paths: Mutex::default(),
            broken: AtomicBool::new(false),
            skip_noise: self.per_dir,
        });

        let mut wanted: Vec<(PathBuf, RecursiveMode)> = Vec::new();
        let budget = self.max_dirs.saturating_sub(lock(&self.state).paths.len());
        if self.per_dir {
            let dirs = root
                .dirs_under(&tree, budget)
                .ok_or_else(|| PwError::other("too many folders to watch"))?;
            wanted.extend(dirs.into_iter().map(|d| (d, RecursiveMode::NonRecursive)));
        } else {
            wanted.push((tree, RecursiveMode::Recursive));
        }
        for g in &git_dirs {
            wanted.push((g.clone(), RecursiveMode::NonRecursive));
            let refs = g.join("refs");
            if self.per_dir {
                wanted.extend(
                    all_dirs(&refs)
                        .into_iter()
                        .map(|d| (d, RecursiveMode::NonRecursive)),
                );
            } else if refs.is_dir() {
                wanted.push((refs, RecursiveMode::Recursive));
            }
        }
        if self.per_dir && wanted.len() > budget {
            return Err(PwError::other("too many folders to watch"));
        }
        {
            let mut st = lock(&self.state);
            for (i, (path, mode)) in wanted.iter().enumerate() {
                if let Err(e) = st.add(path, *mode) {
                    for (done, _) in &wanted[..i] {
                        st.remove(done);
                    }
                    return Err(e);
                }
            }
        }
        *lock(&root.paths) = wanted.into_iter().map(|(p, _)| p).collect();
        let others = {
            let mut roots = lock(&self.roots);
            let others = roots.clone();
            roots.push(root.clone());
            others
        };
        if self.restarts {
            for r in others {
                (r.on_change)();
            }
        }
        Ok(Box::new(Guard {
            hub: self.clone(),
            root,
        }))
    }

    fn drop_root(&self, root: &Arc<Root>) {
        lock(&self.roots).retain(|r| !Arc::ptr_eq(r, root));
        let paths = std::mem::take(&mut *lock(&root.paths));
        let mut st = lock(&self.state);
        for p in &paths {
            st.remove(p);
        }
        drop(st);
        if self.restarts {
            for r in lock(&self.roots).clone() {
                (r.on_change)();
            }
        }
    }

    /// Watch a folder that just appeared in `root` (one-by-one backends).
    fn add_dir(&self, root: &Root, dir: &Path) {
        if !dir.is_dir() || lock(&self.state).paths.contains_key(dir) {
            return;
        }
        let mut st = lock(&self.state);
        let budget = self.max_dirs.saturating_sub(st.paths.len());
        let Some(dirs) = root.dirs_under(dir, budget) else {
            root.broken.store(true, Ordering::Relaxed);
            return;
        };
        let mut added = Vec::new();
        for d in dirs {
            if st.add(&d, RecursiveMode::NonRecursive).is_err() {
                root.broken.store(true, Ordering::Relaxed);
                break;
            }
            added.push(d);
        }
        drop(st);
        lock(&root.paths).extend(added);
    }

    fn dispatch(&self, ev: notify::Result<Event>) {
        let roots = lock(&self.roots).clone();
        let ev = match ev {
            Ok(ev) => ev,
            // The backend lost track (e.g. an overflowing queue): anything
            // may have changed.
            Err(_) => {
                for r in &roots {
                    (r.on_change)();
                }
                return;
            }
        };
        if matches!(
            ev.kind,
            EventKind::Access(_)
                | EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime))
        ) {
            return;
        }
        let rescan = ev.need_rescan();
        let new_dir = self.per_dir
            && matches!(
                ev.kind,
                EventKind::Create(CreateKind::Folder | CreateKind::Any)
                    | EventKind::Modify(ModifyKind::Name(
                        RenameMode::To | RenameMode::Both | RenameMode::Any
                    ))
            );
        for r in &roots {
            let mut hit = rescan;
            for p in &ev.paths {
                if r.classify(p) == Class::Relevant {
                    hit = true;
                    if new_dir {
                        self.add_dir(r, p);
                    }
                }
            }
            if hit {
                (r.on_change)();
            }
        }
    }
}

fn dispatch_loop(hub: Weak<Hub>, rx: mpsc::Receiver<notify::Result<Event>>) {
    for ev in rx {
        let Some(hub) = hub.upgrade() else { return };
        hub.dispatch(ev);
    }
}

/// Stops watching its checkout when dropped.
struct Guard {
    hub: Arc<Hub>,
    root: Arc<Root>,
}

impl Watching for Guard {
    fn healthy(&self) -> bool {
        !self.root.broken.load(Ordering::Relaxed)
    }

    fn complete(&self) -> bool {
        !self.root.skip_noise
    }

    fn set_ignored(&self, ignored: &[String]) {
        *lock(&self.root.ignored) = absolute(&self.root.tree, ignored);
    }
}

/// `rels` (relative to `tree`, `/`-separated, maybe ending in `/`) as paths.
fn absolute(tree: &Path, rels: &[String]) -> HashSet<PathBuf> {
    rels.iter()
        .map(|rel| tree.join(rel.trim_end_matches('/')))
        .collect()
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.hub.drop_root(&self.root);
    }
}

/// The process-wide hub, made on first use.
pub(super) fn watch(spec: &WatchSpec, on_change: OnChange) -> Result<Box<dyn Watching>> {
    static HUB: OnceLock<std::result::Result<Arc<Hub>, PwError>> = OnceLock::new();
    match HUB.get_or_init(|| Hub::new(None, MAX_DIRS)) {
        Ok(hub) => hub.add(spec, on_change),
        Err(e) => Err(e.clone()),
    }
}

/// Counts changes (tests).
#[cfg(test)]
pub(crate) fn counter() -> (Arc<std::sync::atomic::AtomicU64>, OnChange) {
    let n = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let m = n.clone();
    (
        n,
        Arc::new(move || {
            m.fetch_add(1, Ordering::SeqCst);
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use std::sync::atomic::AtomicU64;
    use std::time::{Duration, Instant};

    fn root(tree: &str, git_dirs: &[&str], ignored: &[&str]) -> Root {
        Root {
            tree: PathBuf::from(tree),
            git_dirs: git_dirs.iter().map(PathBuf::from).collect(),
            ignored: Mutex::new(ignored.iter().map(|i| Path::new(tree).join(i)).collect()),
            on_change: Arc::new(|| {}),
            paths: Mutex::default(),
            broken: AtomicBool::new(false),
            skip_noise: true,
        }
    }

    #[test]
    fn a_whole_tree_watch_skips_only_what_git_ignores() {
        let mut r = root("/r", &[], &["node_modules"]);
        r.skip_noise = false;
        let c = |p: &str| r.classify(Path::new(p));
        assert_eq!(c("/r/node_modules/x/index.js"), Class::Ignored);
        assert_eq!(
            c("/r/target/debug/x"),
            Class::Relevant,
            "not ignored by git: git status shows it"
        );
    }

    #[test]
    fn only_changes_that_git_status_can_see_count() {
        let r = root(
            "/r",
            &["/m/.git/worktrees/w", "/m/.git"],
            &["dist", "notes.log"],
        );
        let c = |p: &str| r.classify(Path::new(p));
        assert_eq!(c("/r/src/a.rs"), Class::Relevant);
        assert_eq!(c("/r/.gitignore"), Class::Relevant);
        assert_eq!(c("/r/.git/index"), Class::Relevant);
        assert_eq!(c("/r/.git/HEAD"), Class::Relevant);
        assert_eq!(c("/r/.git/refs/heads/main"), Class::Relevant);
        assert_eq!(c("/r/.git/packed-refs"), Class::Relevant);
        assert_eq!(c("/r/.git/index.lock"), Class::Ignored);
        assert_eq!(c("/r/.git/refs/heads/main.lock"), Class::Ignored);
        assert_eq!(c("/r/.git/objects/ab/cdef"), Class::Ignored);
        assert_eq!(c("/r/.git/logs/HEAD"), Class::Ignored);
        assert_eq!(c("/r/node_modules/x/index.js"), Class::Ignored);
        assert_eq!(c("/r/crates/a/target/debug/x"), Class::Ignored);
        assert_eq!(c("/r/dist/app.js"), Class::Ignored, "git-ignored folder");
        assert_eq!(c("/r/notes.log"), Class::Ignored, "git-ignored file");
        assert_eq!(
            c("/r/sub/.git"),
            Class::Ignored,
            "a nested checkout's .git file"
        );
        // A linked worktree's own git folder and the repository's refs.
        assert_eq!(c("/m/.git/worktrees/w/HEAD"), Class::Relevant);
        assert_eq!(c("/m/.git/worktrees/w/index"), Class::Relevant);
        assert_eq!(c("/m/.git/refs/heads/feature"), Class::Relevant);
        assert_eq!(c("/m/.git/objects/12/34"), Class::Ignored);
        assert_eq!(c("/elsewhere/a.rs"), Class::Outside);
    }

    fn wait_for(n: &AtomicU64, at_least: u64) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if n.load(Ordering::SeqCst) >= at_least {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// Lets the backend deliver anything still in flight, then forgets it.
    fn settle(n: &AtomicU64) -> u64 {
        std::thread::sleep(Duration::from_millis(700));
        n.load(Ordering::SeqCst)
    }

    fn exercise(hub: &Arc<Hub>) {
        let dir = TempDir::new("watch");
        let tree = dir.path().join("w");
        std::fs::create_dir_all(tree.join("src")).unwrap();
        std::fs::create_dir_all(tree.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(tree.join(".git/refs/heads")).unwrap();
        std::fs::create_dir_all(tree.join(".git/objects")).unwrap();
        let (n, on_change) = counter();
        let spec = WatchSpec {
            tree: tree.to_string_lossy().into_owned(),
            git_dirs: vec![],
            ignored: vec!["out/".into(), "node_modules/".into()],
        };
        let guard = hub.add(&spec, on_change).unwrap();
        assert!(guard.healthy());
        let base = settle(&n);

        std::fs::write(tree.join("node_modules/pkg/a.js"), "x").unwrap();
        std::fs::write(tree.join(".git/objects/blob"), "x").unwrap();
        std::fs::create_dir_all(tree.join("out")).unwrap();
        assert_eq!(
            settle(&n),
            base,
            "noise, git internals and ignored paths don't count"
        );

        std::fs::write(tree.join("src/a.rs"), "fn main() {}").unwrap();
        assert!(wait_for(&n, base + 1), "an edit counts");
        let base = settle(&n);
        std::fs::write(tree.join(".git/HEAD"), "ref: refs/heads/other\n").unwrap();
        assert!(wait_for(&n, base + 1), "a branch switch counts");
        // A folder that appears later is watched too.
        let base = settle(&n);
        std::fs::create_dir_all(tree.join("src/new/deeper")).unwrap();
        let base = settle(&n).max(base);
        std::fs::write(tree.join("src/new/deeper/b.rs"), "x").unwrap();
        assert!(wait_for(&n, base + 1), "an edit in a new folder counts");

        drop(guard);
        let base = settle(&n);
        std::fs::write(tree.join("src/a.rs"), "changed again").unwrap();
        assert_eq!(settle(&n), base, "nothing after the watch is dropped");
        assert!(lock(&hub.state).paths.is_empty(), "every OS watch released");
    }

    #[test]
    fn a_tree_watch_reports_relevant_changes_only() {
        exercise(&Hub::new(Some(false), MAX_DIRS).unwrap());
    }

    #[test]
    fn folder_by_folder_watches_report_the_same() {
        exercise(&Hub::new(Some(true), MAX_DIRS).unwrap());
    }

    #[test]
    fn too_many_folders_fail_so_the_caller_polls() {
        let hub = Hub::new(Some(true), 3).unwrap();
        let dir = TempDir::new("watch-limit");
        for d in ["a", "b", "c", "d"] {
            std::fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        let (_, on_change) = counter();
        let spec = WatchSpec {
            tree: dir.path().to_string_lossy().into_owned(),
            ..Default::default()
        };
        assert!(hub.add(&spec, on_change).is_err());
        assert!(
            lock(&hub.state).paths.is_empty(),
            "nothing left half-watched"
        );
    }
}
