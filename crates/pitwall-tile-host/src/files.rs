//! The host side of `files` (files.wit; platform-services.md §14.3,
//! permissions.md §5.1–5.2): files in the project a view is open in, and
//! in the extension's own files folder.
//!
//! Every path a tile gives is relative to a root and checked here before
//! anything touches the disk ([`clean`]): no absolute paths, no `..`, no
//! `.git`. It is then resolved part by part with every link followed, and
//! refused when it leaves the root's resolved folder or reaches Pitwall's
//! data folder; a project's `.pitwall.json` and `.pitwall/` are never
//! changed. Which project a view is in comes from the app
//! ([`crate::ViewSpec::project`], [`crate::TileHandle::set_project`]).
//!
//! Watches ([`Watcher`]) and the Trash ([`Trash`]) are traits, so the
//! test harness follows changes it makes itself ([`FakeWatcher`]) and
//! never fills the user's Trash ([`FolderTrash`]).

use std::any::Any;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use crate::bindings::pitwall::tile::files as wit;
use crate::host::{cut, HostState, Pending};
use crate::ignore::Ignore;
use crate::limits::{FILES_DATA_BYTES, MAX_FILE_BYTES, MAX_LIST_ENTRIES, MAX_WATCHES, WATCH_MOST, WATCH_QUIET};
use crate::worker::Msg;
use crate::{LogLevel, UiRequest};
use pitwall_core::extensions::permissions::{FILES_READ, FILES_WRITE, PROJECTS_READ};

pub use wit::{Entry, EntryKind, FileError, FilesChanged, ProjectInfo as Project, Root};

/// What a watch calls with the paths that changed (absolute, as the
/// system says them), a batch at a time.
pub type Changed = Box<dyn Fn(Vec<PathBuf>) + Send + Sync>;

/// Follows changes under a folder for `files.watch`.
pub trait Watcher: Send + Sync {
    /// Follow everything under `dir` (absolute, resolved) until the
    /// returned guard is dropped; `changed` gets batches of changed paths,
    /// changes close together in one batch.
    fn watch(&self, dir: &Path, changed: Changed) -> Result<Box<dyn Any + Send>, String>;
}

/// The system's file events (FSEvents, inotify, ReadDirectoryChangesW,
/// through `notify`), coalesced: a batch goes [`WATCH_QUIET`] after the
/// last change, or [`WATCH_MOST`] after the first while they keep coming.
pub struct FsWatcher;

impl Watcher for FsWatcher {
    fn watch(&self, dir: &Path, changed: Changed) -> Result<Box<dyn Any + Send>, String> {
        use notify::Watcher as _;
        let (tx, rx) = mpsc::channel::<PathBuf>();
        let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(e) = res {
                if matches!(e.kind, notify::EventKind::Access(_)) {
                    return;
                }
                for p in e.paths {
                    let _ = tx.send(p);
                }
            }
        })
        .map_err(|e| e.to_string())?;
        w.watch(dir, notify::RecursiveMode::Recursive).map_err(|e| e.to_string())?;
        std::thread::Builder::new().name("tile-watch".into()).spawn(move || debounce(&rx, &changed)).map_err(|e| e.to_string())?;
        Ok(Box::new(w))
    }
}

/// Batches of what arrives on `rx` until it closes (the watcher dropped).
fn debounce(rx: &mpsc::Receiver<PathBuf>, changed: &Changed) {
    while let Ok(first) = rx.recv() {
        let start = Instant::now();
        let mut batch = vec![first];
        loop {
            let wait = WATCH_QUIET.min(WATCH_MOST.saturating_sub(start.elapsed()));
            if wait.is_zero() {
                break;
            }
            match rx.recv_timeout(wait) {
                Ok(p) => batch.push(p),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    changed(batch);
                    return;
                }
            }
        }
        changed(batch);
    }
}

type Sinks = Mutex<Vec<(u64, PathBuf, Arc<Changed>)>>;

/// Watches for tests: nothing follows the disk; [`FakeWatcher::changed`]
/// says what changed, at once, as one batch.
#[derive(Default)]
pub struct FakeWatcher {
    sinks: Arc<Sinks>,
    next: Mutex<u64>,
}

impl FakeWatcher {
    pub fn new() -> Arc<FakeWatcher> {
        Arc::new(FakeWatcher::default())
    }

    /// `paths` (absolute) changed: every watch on a folder above them
    /// hears it now.
    pub fn changed(&self, paths: &[PathBuf]) {
        let sinks: Vec<(PathBuf, Arc<Changed>)> = self.sinks.lock().map(|s| s.iter().map(|(_, d, c)| (d.clone(), c.clone())).collect()).unwrap_or_default();
        for (dir, sink) in sinks {
            let hits: Vec<PathBuf> = paths.iter().filter(|p| real_or_parent(p).starts_with(&dir)).cloned().collect();
            if !hits.is_empty() {
                sink(hits);
            }
        }
    }

    /// How many watches are on.
    pub fn watching(&self) -> usize {
        self.sinks.lock().map(|s| s.len()).unwrap_or(0)
    }
}

/// `p` resolved, or its folder resolved with its name (it was removed).
fn real_or_parent(p: &Path) -> PathBuf {
    if let Ok(r) = p.canonicalize() {
        return r;
    }
    match (p.parent().and_then(|d| d.canonicalize().ok()), p.file_name()) {
        (Some(d), Some(n)) => d.join(n),
        _ => p.to_path_buf(),
    }
}

struct FakeGuard {
    id: u64,
    sinks: Arc<Sinks>,
}

impl Drop for FakeGuard {
    fn drop(&mut self) {
        if let Ok(mut s) = self.sinks.lock() {
            s.retain(|(id, _, _)| *id != self.id);
        }
    }
}

impl Watcher for FakeWatcher {
    fn watch(&self, dir: &Path, changed: Changed) -> Result<Box<dyn Any + Send>, String> {
        let id = {
            let mut n = self.next.lock().map_err(|e| e.to_string())?;
            *n += 1;
            *n
        };
        self.sinks.lock().map_err(|e| e.to_string())?.push((id, dir.to_path_buf(), Arc::new(changed)));
        Ok(Box::new(FakeGuard { id, sinks: self.sinks.clone() }))
    }
}

/// Where `files.delete` puts what it removes from a project.
pub trait Trash: Send + Sync {
    /// Move `path` (absolute) there.
    fn trash(&self, path: &Path) -> std::io::Result<()>;
}

/// The system's Trash (Finder's, the Recycle Bin, the freedesktop.org
/// one).
pub struct SystemTrash;

impl Trash for SystemTrash {
    fn trash(&self, path: &Path) -> std::io::Result<()> {
        crate::platform::trash(path)
    }
}

/// A folder of the test's own as the Trash: items moved into it, numbered.
pub struct FolderTrash {
    dir: PathBuf,
    n: Mutex<u32>,
}

impl FolderTrash {
    pub fn new(dir: impl Into<PathBuf>) -> Arc<FolderTrash> {
        Arc::new(FolderTrash { dir: dir.into(), n: Mutex::new(0) })
    }

    /// The names of what was put in it, oldest first.
    pub fn items(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.dir).map(|r| r.flatten().filter_map(|e| e.file_name().to_str().map(str::to_string)).collect()).unwrap_or_default();
        v.sort();
        v.into_iter().map(|n| n.split_once('-').map(|(_, r)| r.to_string()).unwrap_or(n)).collect()
    }
}

impl Trash for FolderTrash {
    fn trash(&self, path: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let n = {
            let mut n = self.n.lock().map_err(|e| std::io::Error::other(e.to_string()))?;
            *n += 1;
            *n
        };
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("item");
        std::fs::rename(path, self.dir.join(format!("{n:04}-{name}")))
    }
}

/// What the files interface keeps for a running tile.
pub(crate) struct Files {
    /// The project each open view is in (by view id).
    pub projects: std::collections::BTreeMap<String, Project>,
    watches: std::collections::BTreeMap<u32, Watch>,
    next_watch: u32,
    /// `<data>/extension-data/<id>/files`.
    data: PathBuf,
    /// Pitwall's data folder: never reached through a project.
    pitwall: PathBuf,
    watcher: Arc<dyn Watcher>,
    trash: Arc<dyn Trash>,
    /// The files folder's size, once measured.
    data_used: Option<u64>,
    /// The last Activity line and when, so a tile saving every second
    /// doesn't fill it.
    last_line: Option<(String, Instant)>,
    /// Where watches' changes go (the worker's own channel).
    done: mpsc::Sender<Msg>,
}

struct Watch {
    /// The view whose project it follows (none: the files folder).
    view: Option<String>,
    _guard: Box<dyn Any + Send>,
}

impl Files {
    pub fn new(data: PathBuf, pitwall: PathBuf, watcher: Arc<dyn Watcher>, trash: Arc<dyn Trash>, done: mpsc::Sender<Msg>) -> Files {
        Files { projects: Default::default(), watches: Default::default(), next_watch: 1, data, pitwall, watcher, trash, data_used: None, last_line: None, done }
    }

    /// View `view` is in `project` now (or none): its watches on the old
    /// one end. Whether it changed.
    pub fn set_project(&mut self, view: &str, project: Option<Project>) -> bool {
        let old = self.projects.get(view).cloned();
        if old == project {
            return false;
        }
        match project {
            Some(p) => self.projects.insert(view.to_string(), p),
            None => self.projects.remove(view),
        };
        self.watches.retain(|_, w| w.view.as_deref() != Some(view));
        true
    }

    /// View `view` closed.
    pub fn view_closed(&mut self, view: &str) {
        self.projects.remove(view);
        self.watches.retain(|_, w| w.view.as_deref() != Some(view));
    }

    /// Whether watch `id` is on (its changes are still wanted).
    pub fn watching(&self, id: u32) -> bool {
        self.watches.contains_key(&id)
    }
}

/// Where a call's path starts, resolved.
struct Base {
    /// The root's folder, resolved.
    dir: PathBuf,
    /// The project's name; `None` for the files folder.
    project: Option<String>,
    /// The view, for a project.
    view: Option<String>,
}

/// A path found under a root.
struct Found {
    /// Where it is: the root's resolved folder, its folders resolved, and
    /// its own name (a link isn't followed here).
    path: PathBuf,
    /// Where it leads: `path` with a last link followed.
    real: PathBuf,
    /// Its path from the root, as given (cleaned).
    rel: String,
    /// What is there, following a link; `None`: nothing.
    meta: Option<std::fs::Metadata>,
    /// Something is there (a link that leads nowhere counts).
    exists: bool,
}

/// A tile's relative path, as its parts. Refused: an absolute path, a
/// drive or `\`, `..`, NUL, more than 4096 bytes, and `.git` anywhere.
pub fn clean(rel: &str) -> Result<Vec<String>, FileError> {
    if rel.len() > 4096 {
        return Err(FileError::Invalid("the path is longer than 4096 bytes".into()));
    }
    if rel.contains('\0') {
        return Err(FileError::Invalid("the path has a NUL".into()));
    }
    if rel.starts_with('/') || rel.contains('\\') || rel.as_bytes().get(1) == Some(&b':') {
        return Err(FileError::NotAllowed(format!("“{}” isn't a path from the root: write it like docs/notes.md", cut(rel, 200))));
    }
    let parts: Vec<String> = rel.split('/').filter(|p| !p.is_empty() && *p != ".").map(str::to_string).collect();
    if parts.iter().any(|p| p == "..") {
        return Err(FileError::NotAllowed(format!("“{}” goes up with ..: paths stay inside the root", cut(rel, 200))));
    }
    if parts.iter().any(|p| is_git(p)) {
        return Err(FileError::NotAllowed(".git is never reached".into()));
    }
    Ok(parts)
}

fn is_git(part: &str) -> bool {
    part.eq_ignore_ascii_case(".git")
}

/// Whether `parts` name one of Pitwall's own files in a project.
fn pitwall_own(parts: &[String]) -> bool {
    parts.first().is_some_and(|p| p.eq_ignore_ascii_case(".pitwall.json") || p.eq_ignore_ascii_case(".pitwall"))
}

/// Milliseconds since 1970 of `t`.
fn ms(t: std::io::Result<std::time::SystemTime>) -> u64 {
    t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn entry(rel: String, meta: &std::fs::Metadata) -> Entry {
    let folder = meta.is_dir();
    Entry { path: rel, kind: if folder { EntryKind::Folder } else { EntryKind::File }, size: if folder { 0 } else { meta.len() }, modified: ms(meta.modified()) }
}

fn io_error(e: &std::io::Error) -> FileError {
    match e.kind() {
        std::io::ErrorKind::NotFound => FileError::NotFound,
        std::io::ErrorKind::AlreadyExists => FileError::Exists,
        _ => FileError::Failed(e.to_string()),
    }
}

/// `rel`'s parts relative to `root`, when `path` is inside it.
fn inside(root: &Path, path: &Path) -> Option<Vec<String>> {
    let r = path.strip_prefix(root).ok()?;
    Some(r.components().filter_map(|c| match c {
        Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
        _ => None,
    }).collect())
}

/// Find `parts` under `base`: its folders resolved one by one (links
/// followed, refused when they lead out of the root or into `.git`).
fn find(base: &Base, parts: &[String], pitwall: &Path) -> Result<Found, FileError> {
    let out = || FileError::NotAllowed("a link there leads out of the root".into());
    let check = |p: &Path| -> Result<(), FileError> {
        let rel = inside(&base.dir, p).ok_or_else(out)?;
        if rel.iter().any(|s| is_git(s)) {
            return Err(FileError::NotAllowed("a link there leads into .git".into()));
        }
        if base.project.is_some() && p.starts_with(pitwall) {
            return Err(FileError::NotAllowed("Pitwall's own data is never reached".into()));
        }
        Ok(())
    };
    let mut cur = base.dir.clone();
    let (last, folders) = match parts.split_last() {
        Some((l, f)) => (Some(l), f),
        None => (None, &[][..]),
    };
    let mut gone = false;
    for part in folders {
        let next = cur.join(part);
        if gone {
            cur = next;
            continue;
        }
        match std::fs::symlink_metadata(&next) {
            Ok(m) if m.file_type().is_symlink() => {
                let real = next.canonicalize().map_err(|e| io_error(&e))?;
                check(&real)?;
                cur = real;
            }
            Ok(m) if m.is_dir() => cur = next,
            Ok(_) => return Err(FileError::NotFound),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                gone = true;
                cur = next;
            }
            Err(e) => return Err(io_error(&e)),
        }
    }
    let rel = parts.join("/");
    let Some(last) = last else {
        let meta = std::fs::metadata(&cur).ok();
        return Ok(Found { path: cur.clone(), real: cur, rel, exists: meta.is_some(), meta });
    };
    let path = cur.join(last);
    if gone {
        return Ok(Found { real: path.clone(), path, rel, meta: None, exists: false });
    }
    match std::fs::symlink_metadata(&path) {
        Ok(m) if m.file_type().is_symlink() => {
            // A link that leads nowhere is there (it can be removed), but
            // has nothing to read.
            match path.canonicalize() {
                Ok(real) => {
                    check(&real)?;
                    let meta = std::fs::metadata(&real).ok();
                    Ok(Found { path, real, rel, meta, exists: true })
                }
                Err(_) => Ok(Found { real: path.clone(), path, rel, meta: None, exists: true }),
            }
        }
        Ok(m) => Ok(Found { real: path.clone(), path, rel, meta: Some(m), exists: true }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Found { real: path.clone(), path, rel, meta: None, exists: false }),
        Err(e) => Err(io_error(&e)),
    }
}

/// Write `bytes` to `path` through a file beside it and a rename, keeping
/// the old file's permissions: a reader never sees half a file.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let tmp = dir.join(format!(".{name}.pitwall-{}-{}.tmp", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let old = std::fs::metadata(path).ok().map(|m| m.permissions());
    let r = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        if let Some(p) = old {
            std::fs::set_permissions(&tmp, p)?;
        }
        std::fs::rename(&tmp, path)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// The size of everything under `dir` (at most 100 000 entries looked at).
fn size_of(dir: &Path) -> u64 {
    let mut total = 0;
    let mut seen = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            seen += 1;
            if seen > 100_000 {
                return total;
            }
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else {
                total += m.len();
            }
        }
    }
    total
}

impl HostState {
    /// The root a call names, resolved, after its permission: `write`
    /// for a call that changes a project.
    fn base(&mut self, what: &str, root: &Root, write: bool) -> Result<Base, FileError> {
        match root {
            Root::Data => {
                let dir = self.files.data.clone();
                crate::platform::private_dir(&dir).map_err(|e| FileError::Failed(e.to_string()))?;
                let dir = dir.canonicalize().map_err(|e| FileError::Failed(e.to_string()))?;
                Ok(Base { dir, project: None, view: None })
            }
            Root::Project(view) => {
                let need = if write { FILES_WRITE } else { FILES_READ };
                if !self.grants.has(need) {
                    self.denied_file(what, need);
                    return Err(FileError::Denied);
                }
                let Some(p) = self.files.projects.get(view).cloned() else { return Err(FileError::NoProject) };
                let dir = Path::new(&p.folder).canonicalize().map_err(|e| FileError::Failed(format!("the project's folder {} can't be opened: {e}", p.folder)))?;
                Ok(Base { dir, project: Some(p.name), view: Some(view.clone()) })
            }
        }
    }

    /// Refused for want of `permission`: in the Log and the Activity.
    fn denied_file(&mut self, what: &str, permission: &str) {
        let why = match self.gated {
            Some(kind) => format!("an agent's {kind} call runs without the tile's acting permissions; only an act call changes files"),
            None => format!("it isn't granted “{permission}”"),
        };
        self.host_log(LogLevel::Info, format!("{what} was refused: {why}"));
        self.activity(what, String::new(), Some(format!("not granted {permission}")));
    }

    /// A line in the extension's Activity (the same line within a minute
    /// once).
    fn activity(&mut self, method: &str, detail: String, refused: Option<String>) {
        let key = format!("{method}\n{detail}\n{refused:?}");
        let now = self.clock.now();
        if self.files.last_line.as_ref().is_some_and(|(k, at)| *k == key && now.duration_since(*at) < Duration::from_secs(60)) {
            return;
        }
        self.files.last_line = Some((key, now));
        self.pending.push(Pending::Ui(UiRequest::Activity { method: method.to_string(), detail, refused }));
    }

    /// A path refused under `base`: in the Activity when it is a
    /// project's.
    fn refused(&mut self, method: &str, base: &Base, rel: &str, e: FileError) -> FileError {
        if let (Some(project), FileError::NotAllowed(why)) = (&base.project, &e) {
            self.activity(method, format!("{project}: {}", cut(rel, 300)), Some(why.clone()));
        }
        e
    }

    /// Find `rel` under `root` for `method`: the permission, the path's
    /// checks, and (`write`) Pitwall's own files.
    fn locate(&mut self, method: &str, root: &Root, rel: &str, write: bool) -> Result<(Base, Found), FileError> {
        if !self.count_call() {
            return Err(FileError::Busy);
        }
        let base = self.base(method, root, write && matches!(root, Root::Project(_)))?;
        let parts = clean(rel).map_err(|e| self.refused(method, &base, rel, e))?;
        if write && base.project.is_some() && pitwall_own(&parts) {
            return Err(self.refused(method, &base, rel, FileError::NotAllowed("Pitwall's own files in a project are never changed".into())));
        }
        let pitwall = self.files.pitwall.clone();
        let found = find(&base, &parts, &pitwall).map_err(|e| self.refused(method, &base, rel, e))?;
        Ok((base, found))
    }

    /// After a change in a project: in the Activity.
    fn changed(&mut self, method: &str, base: &Base, what: String) {
        if let Some(project) = &base.project {
            self.activity(method, format!("{project}: {}", cut(&what, 300)), None);
        }
    }

    /// The files folder's size now (measured once).
    fn data_used(&mut self) -> u64 {
        if self.files.data_used.is_none() {
            self.files.data_used = Some(size_of(&self.files.data));
        }
        self.files.data_used.unwrap_or(0)
    }

    fn data_add(&mut self, delta: i64) {
        let used = self.data_used() as i64;
        self.files.data_used = Some((used + delta).max(0) as u64);
    }

    /// A list of `folder` under `base` (resolved at `real`).
    fn list_under(base: &Base, rel: &str, real: &Path, deep: bool) -> Result<Vec<Entry>, FileError> {
        let mut ignore = base.project.as_ref().map(|_| Ignore::new(&base.dir));
        let mut out = Vec::new();
        let mut stack = vec![(rel.to_string(), real.to_path_buf())];
        while let Some((rel, dir)) = stack.pop() {
            let rd = std::fs::read_dir(&dir).map_err(|e| io_error(&e))?;
            for e in rd.flatten() {
                let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
                if is_git(&name) {
                    continue;
                }
                let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                let Ok(lm) = e.metadata() else { continue };
                let link = lm.file_type().is_symlink();
                let meta = if link {
                    // A link counts when it leads inside the root.
                    let Ok(r) = e.path().canonicalize() else { continue };
                    if inside(&base.dir, &r).is_none_or(|p| p.iter().any(|s| is_git(s))) {
                        continue;
                    }
                    match std::fs::metadata(&r) {
                        Ok(m) => m,
                        Err(_) => continue,
                    }
                } else {
                    lm
                };
                if ignore.as_mut().is_some_and(|ig| ig.matches(&path, meta.is_dir())) {
                    continue;
                }
                if out.len() >= MAX_LIST_ENTRIES {
                    return Err(FileError::TooLarge);
                }
                if deep && meta.is_dir() && !link {
                    stack.push((path.clone(), e.path()));
                }
                out.push(entry(path, &meta));
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }
}

impl wit::Host for HostState {
    fn project(&mut self, view: String) -> Option<Project> {
        if !self.grants.has(FILES_READ) && !self.grants.has(PROJECTS_READ) {
            return None;
        }
        self.files.projects.get(&view).cloned()
    }

    fn read(&mut self, root: Root, path: String) -> Result<Vec<u8>, FileError> {
        let (_, f) = self.locate("files.read", &root, &path, false)?;
        let meta = f.meta.ok_or(FileError::NotFound)?;
        if meta.is_dir() {
            return Err(FileError::Invalid(format!("“{}” is a folder: list it", cut(&f.rel, 200))));
        }
        if meta.len() > MAX_FILE_BYTES as u64 {
            return Err(FileError::TooLarge);
        }
        let bytes = std::fs::read(&f.real).map_err(|e| io_error(&e))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(FileError::TooLarge);
        }
        Ok(bytes)
    }

    fn stat(&mut self, root: Root, path: String) -> Result<Entry, FileError> {
        let (_, f) = self.locate("files.stat", &root, &path, false)?;
        let meta = f.meta.ok_or(FileError::NotFound)?;
        Ok(entry(f.rel, &meta))
    }

    fn list(&mut self, root: Root, folder: String, deep: bool) -> Result<Vec<Entry>, FileError> {
        let (base, f) = self.locate("files.list", &root, &folder, false)?;
        let meta = f.meta.ok_or(FileError::NotFound)?;
        if !meta.is_dir() {
            return Err(FileError::Invalid(format!("“{}” is a file: read it", cut(&f.rel, 200))));
        }
        HostState::list_under(&base, &f.rel, &f.real, deep)
    }

    fn watch(&mut self, root: Root, folder: String) -> Result<u32, FileError> {
        let (base, f) = self.locate("files.watch", &root, &folder, false)?;
        if !f.meta.as_ref().is_some_and(|m| m.is_dir()) {
            return Err(if f.exists { FileError::Invalid(format!("“{}” is a file: watch its folder", cut(&f.rel, 200))) } else { FileError::NotFound });
        }
        if self.files.watches.len() >= MAX_WATCHES {
            return Err(FileError::Busy);
        }
        let id = self.files.next_watch;
        self.files.next_watch = self.files.next_watch.wrapping_add(1).max(1);
        let (done, dir, project) = (self.files.done.clone(), base.dir.clone(), base.project.is_some());
        let changed: Changed = Box::new(move |paths: Vec<PathBuf>| {
            let mut ignore = project.then(|| Ignore::new(&dir));
            let mut out = BTreeSet::new();
            for p in paths {
                let Some(parts) = inside(&dir, &real_or_parent(&p)).or_else(|| inside(&dir, &p)) else { continue };
                if parts.is_empty() || parts.iter().any(|s| is_git(s)) {
                    continue;
                }
                let rel = parts.join("/");
                let is_dir = std::fs::metadata(&p).is_ok_and(|m| m.is_dir());
                if ignore.as_mut().is_some_and(|ig| ig.ignored(&rel, is_dir)) {
                    continue;
                }
                out.insert(rel);
            }
            if !out.is_empty() {
                let _ = done.send(Msg::FilesChanged(FilesChanged { watch: id, paths: out.into_iter().collect() }));
            }
        });
        let guard = self.files.watcher.watch(&f.real, changed).map_err(FileError::Failed)?;
        self.files.watches.insert(id, Watch { view: base.view, _guard: guard });
        Ok(id)
    }

    fn unwatch(&mut self, id: u32) {
        self.files.watches.remove(&id);
    }

    fn write(&mut self, root: Root, path: String, contents: Vec<u8>) -> Result<(), FileError> {
        let (base, f) = self.locate("files.write", &root, &path, true)?;
        if f.rel.is_empty() || f.meta.as_ref().is_some_and(|m| m.is_dir()) {
            return Err(FileError::Invalid(format!("“{}” is a folder", cut(&f.rel, 200))));
        }
        if contents.len() > MAX_FILE_BYTES {
            return Err(FileError::TooLarge);
        }
        let old = f.meta.as_ref().map_or(0, |m| m.len());
        if base.project.is_none() && self.data_used() + contents.len() as u64 > FILES_DATA_BYTES + old {
            return Err(FileError::TooLarge);
        }
        if let Some(dir) = f.real.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io_error(&e))?;
        }
        write_whole(&f.real, &contents).map_err(|e| io_error(&e))?;
        if base.project.is_none() {
            self.data_add(contents.len() as i64 - old as i64);
        }
        self.changed("files.write", &base, f.rel);
        Ok(())
    }

    fn create(&mut self, root: Root, path: String, contents: Vec<u8>) -> Result<(), FileError> {
        use std::io::Write;
        let (base, f) = self.locate("files.create", &root, &path, true)?;
        if f.rel.is_empty() || f.exists {
            return Err(FileError::Exists);
        }
        if contents.len() > MAX_FILE_BYTES || (base.project.is_none() && self.data_used() + contents.len() as u64 > FILES_DATA_BYTES) {
            return Err(FileError::TooLarge);
        }
        if let Some(dir) = f.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io_error(&e))?;
        }
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&f.path).map_err(|e| io_error(&e))?;
        file.write_all(&contents).map_err(|e| io_error(&e))?;
        if base.project.is_none() {
            self.data_add(contents.len() as i64);
        }
        self.changed("files.create", &base, f.rel);
        Ok(())
    }

    fn make_folder(&mut self, root: Root, path: String) -> Result<(), FileError> {
        let (base, f) = self.locate("files.make-folder", &root, &path, true)?;
        if f.rel.is_empty() || f.exists {
            return Err(FileError::Exists);
        }
        std::fs::create_dir_all(&f.path).map_err(|e| io_error(&e))?;
        self.changed("files.make-folder", &base, f.rel);
        Ok(())
    }

    fn rename(&mut self, root: Root, from: String, to: String) -> Result<(), FileError> {
        let (base, a) = self.locate("files.rename", &root, &from, true)?;
        let (_, b) = self.locate("files.rename", &root, &to, true)?;
        if a.rel.is_empty() || b.rel.is_empty() {
            return Err(FileError::Invalid("the root itself can't be renamed".into()));
        }
        if !a.exists {
            return Err(FileError::NotFound);
        }
        // A new name that differs only in case is the same file on
        // macOS and Windows: allowed.
        if b.exists && !a.rel.eq_ignore_ascii_case(&b.rel) {
            return Err(FileError::Exists);
        }
        if b.path.starts_with(&a.path) && b.path != a.path {
            return Err(FileError::Invalid("a folder can't move into itself".into()));
        }
        if let Some(dir) = b.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| io_error(&e))?;
        }
        std::fs::rename(&a.path, &b.path).map_err(|e| io_error(&e))?;
        self.changed("files.rename", &base, format!("{} → {}", a.rel, b.rel));
        Ok(())
    }

    fn delete(&mut self, root: Root, path: String) -> Result<(), FileError> {
        let (base, f) = self.locate("files.delete", &root, &path, true)?;
        if f.rel.is_empty() {
            return Err(FileError::Invalid("the root itself can't be removed".into()));
        }
        if !f.exists {
            return Err(FileError::NotFound);
        }
        let folder = std::fs::symlink_metadata(&f.path).is_ok_and(|m| m.is_dir());
        if base.project.is_some() {
            let trash = self.files.trash.clone();
            trash.trash(&f.path).map_err(|e| FileError::Failed(format!("it couldn't be moved to the Trash: {e}")))?;
            self.changed("files.delete", &base, format!("{} (to the Trash)", f.rel));
        } else {
            let size = if folder { size_of(&f.path) } else { f.meta.as_ref().map_or(0, |m| m.len()) };
            let r = if folder { std::fs::remove_dir_all(&f.path) } else { std::fs::remove_file(&f.path) };
            r.map_err(|e| io_error(&e))?;
            self.data_add(-(size as i64));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_cleaned() {
        assert_eq!(clean("docs//./a.md").unwrap(), ["docs", "a.md"]);
        assert_eq!(clean("").unwrap(), Vec::<String>::new());
        for bad in ["/etc/passwd", "a/../../b", "..", "C:/x", "a\\b", ".git/config", "x/.GIT/HEAD"] {
            assert!(matches!(clean(bad), Err(FileError::NotAllowed(_))), "{bad}");
        }
        assert!(matches!(clean("a\0b"), Err(FileError::Invalid(_))));
        assert!(pitwall_own(&clean(".pitwall.json").unwrap()) && pitwall_own(&clean(".Pitwall/x").unwrap()) && !pitwall_own(&clean("docs/.pitwall.json").unwrap()));
    }

    /// The system's events, coalesced: three writes close together come
    /// as one batch (or few), and the guard's drop ends the watch.
    #[test]
    fn the_system_watcher_coalesces() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().canonicalize().unwrap();
        let (tx, rx) = mpsc::channel::<Vec<PathBuf>>();
        let tx = Mutex::new(tx);
        let guard = FsWatcher.watch(&dir, Box::new(move |b| {
            let _ = tx.lock().unwrap().send(b);
        })).unwrap();
        for n in 0..3 {
            std::fs::write(dir.join(format!("{n}.md")), "x").unwrap();
        }
        let first = rx.recv_timeout(Duration::from_secs(10)).expect("a batch");
        let mut seen: BTreeSet<PathBuf> = first.into_iter().filter_map(|p| p.file_name().map(PathBuf::from)).collect();
        while seen.len() < 3 {
            let Ok(b) = rx.recv_timeout(Duration::from_secs(5)) else { break };
            seen.extend(b.into_iter().filter_map(|p| p.file_name().map(PathBuf::from)));
        }
        assert!(["0.md", "1.md", "2.md"].iter().all(|n| seen.contains(Path::new(n))), "{seen:?}");
        drop(guard);
    }

    #[test]
    fn links_out_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("p");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(d.path().join("secret"), "x").unwrap();
        std::fs::write(root.join("docs/a.md"), "a").unwrap();
        let base = Base { dir: root.canonicalize().unwrap(), project: Some("p".into()), view: Some("main".into()) };
        let none = Path::new("/nonexistent-pitwall");
        assert!(find(&base, &clean("docs/a.md").unwrap(), none).unwrap().exists);
        assert!(!find(&base, &clean("new/b/c.md").unwrap(), none).unwrap().exists);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.path().join("secret"), root.join("out")).unwrap();
            std::os::unix::fs::symlink(d.path(), root.join("up")).unwrap();
            std::os::unix::fs::symlink(root.join("docs"), root.join("in")).unwrap();
            assert!(matches!(find(&base, &clean("out").unwrap(), none), Err(FileError::NotAllowed(_))));
            assert!(matches!(find(&base, &clean("up/secret").unwrap(), none), Err(FileError::NotAllowed(_))));
            assert!(find(&base, &clean("in/a.md").unwrap(), none).unwrap().meta.is_some());
        }
    }
}
