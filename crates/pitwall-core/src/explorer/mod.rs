//! The read-only code explorer (docs/spec/explorer.md): list, read and
//! search the folder an agent works in, on its machine (through its
//! [`Exec`]). Nothing here writes to the agent's folder or starts anything
//! but read-only commands.
//!
//! Every path from a client is relative to the agent's folder (its "root").
//! It is checked ([`clean`]) before anything runs, and then resolved on the
//! machine with every symlink followed. It must still be inside the
//! resolved root ([`inside`]). All calls are blocking.

mod read;
mod search;
mod tree;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub use pitwall_proto::{
    ContentKind, DirListing, EntryKind, FileEntry, FileIndex, FileView, MatchRange, SearchEngine,
    SearchMatch, SearchQuery, SearchResult,
};

use crate::engine::Engine;
use crate::exec::{self, Exec};
use crate::provider::ProviderCaps;

type Res<T> = Result<T, String>;

/// Longest relative path accepted from a client.
const MAX_PATH: usize = 4096;

/// What the engine keeps for the explorer while Pitwall runs.
#[derive(Default)]
pub(crate) struct State {
    /// Whether ripgrep runs on a machine ("provider/machine"), once seen.
    rg: Mutex<HashMap<String, bool>>,
    /// Each agent's running search, to cancel it.
    searches: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What an explorer call needs to know about an agent (read under the lock).
struct Ctx {
    exec: Arc<dyn Exec>,
    cwd: String,
    /// `cwd` resolved on its machine, when known already.
    real_cwd: Option<String>,
    base: Option<String>,
    /// `Some(false)`: not a git repository (Changes found out).
    git_repo: Option<bool>,
    /// "provider/machine".
    machine: String,
    machine_label: String,
    caps: ProviderCaps,
}

fn ctx(engine: &Engine, agent_id: &str) -> Res<Ctx> {
    // Looked up before taking the registry lock (it takes it too).
    let exec = engine.exec_for(agent_id);
    let c = engine.with(agent_id, |a| {
        let loc = a.rec.locator();
        Ctx {
            exec,
            cwd: a.rec.cwd.clone(),
            real_cwd: a
                .real_cwd
                .as_ref()
                .filter(|(from, _)| *from == a.rec.cwd)
                .map(|(_, real)| real.clone()),
            base: a.rec.base_commit.clone(),
            git_repo: a.git_repo,
            machine: format!("{}/{}", loc.provider, loc.machine),
            machine_label: a.facts.machine_label.clone(),
            caps: a.facts.provider,
        }
    })?;
    if !c.caps.exec {
        return Err("Pitwall can't read files where this agent runs.".into());
    }
    Ok(c)
}

impl Ctx {
    /// The agent's folder with every symlink resolved (cached on the agent).
    fn root(&self, engine: &Engine, agent_id: &str) -> Res<String> {
        if let Some(r) = &self.real_cwd {
            return Ok(r.clone());
        }
        let real = self
            .exec
            .real_path(&self.cwd)
            .map_err(|e| format!("The agent's folder can't be read: {e}"))?;
        let cwd = self.cwd.clone();
        let _ = engine.with(agent_id, |a| a.real_cwd = Some((cwd, real.clone())));
        Ok(real)
    }

    /// `rel` (already [`clean`]) resolved on the machine; refused when it
    /// leads outside `root` (a symlink pointing elsewhere).
    fn resolve(&self, root: &str, rel: &str) -> Res<String> {
        if rel.is_empty() {
            return Ok(root.to_string());
        }
        let real = self.exec.real_path(&exec::join(root, rel)).map_err(|e| {
            if e.is(crate::error::ErrorCode::NotFound)
                || e.contains("No such file")
                || e.contains("not found")
            {
                format!("{rel}: not found")
            } else {
                String::from(e)
            }
        })?;
        if !inside(root, &real) {
            return Err(format!("{rel}: leads outside the agent's folder"));
        }
        Ok(real)
    }
}

/// A client's relative path, normalised to `a/b/c` ("" = the root). Refused:
/// absolute paths (`/x`, `\x`, `C:…`), `..` anywhere, NUL, and very long ones.
pub fn clean(rel: &str) -> Res<String> {
    let bad = || Err(format!("invalid path: {rel}"));
    if rel.len() > MAX_PATH || rel.contains('\0') || rel.starts_with(['/', '\\']) {
        return bad();
    }
    let b = rel.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return bad();
    }
    if rel.split(['/', '\\']).any(|s| s == "..") {
        return bad();
    }
    Ok(rel
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect::<Vec<_>>()
        .join("/"))
}

/// `path` is `root` or below it (either separator: Windows paths).
pub fn inside(root: &str, path: &str) -> bool {
    let root = root.trim_end_matches(['/', '\\']);
    if root.is_empty() {
        return path.starts_with('/');
    }
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with(['/', '\\']))
}

/// The children of folder `dir` ("" = the agent's folder): git's view when
/// it is a repository (ignore files apply; `ignored`: ignored entries are
/// listed too, marked), else a plain listing. With the Changes panel's
/// letters. One git round trip (plus resolving `dir`).
pub fn list_files(engine: &Engine, agent_id: &str, dir: &str, ignored: bool) -> Res<DirListing> {
    let dir = clean(dir)?;
    let c = ctx(engine, agent_id)?;
    let root = c.root(engine, agent_id)?;
    let real = c.resolve(&root, &dir)?;
    tree::list(&c, &dir, &real, ignored)
}

/// Every file of the agent's folder, for quick open.
pub fn list_all_files(engine: &Engine, agent_id: &str) -> Res<FileIndex> {
    let c = ctx(engine, agent_id)?;
    let root = c.root(engine, agent_id)?;
    let rg = rg_known(engine, &c);
    let (index, rg_seen) = tree::index(&c, &root, rg);
    remember_rg(engine, &c, rg_seen);
    Ok(index)
}

/// One file for the viewer: text (≤ 2 MiB, or ≤ 10 MiB with `large`:
/// "Load anyway"), binary or too large.
pub fn read_file(engine: &Engine, agent_id: &str, path: &str, large: bool) -> Res<FileView> {
    let rel = clean(path)?;
    if rel.is_empty() {
        return Err("invalid path: a file is needed".into());
    }
    let c = ctx(engine, agent_id)?;
    let root = c.root(engine, agent_id)?;
    let real = c.resolve(&root, &rel)?;
    read::read(&*c.exec, &rel, &real, if large { read::MAX_LARGE } else { read::MAX_TEXT })
}

/// Search the agent's folder (ripgrep, else `git grep`). A new search for
/// the same agent cancels the one still running.
pub fn search(engine: &Engine, agent_id: &str, query: &SearchQuery) -> Res<SearchResult> {
    let c = ctx(engine, agent_id)?;
    let root = c.root(engine, agent_id)?;
    let flag = Arc::new(AtomicBool::new(false));
    if let Some(old) = lock(&engine.explorer.searches).insert(agent_id.to_string(), flag.clone()) {
        old.store(true, Ordering::Relaxed);
    }
    let rg = rg_known(engine, &c);
    let (res, rg_seen) = search::run(&c, &root, query, &flag, rg);
    remember_rg(engine, &c, rg_seen);
    let mut running = lock(&engine.explorer.searches);
    if running.get(agent_id).is_some_and(|f| Arc::ptr_eq(f, &flag)) {
        running.remove(agent_id);
    }
    if flag.load(Ordering::Relaxed) {
        return Err(search::CANCELLED.into());
    }
    res
}

/// Stop agent `agent_id`'s running search, if any (it then answers
/// "cancelled").
pub fn cancel_search(engine: &Engine, agent_id: &str) {
    if let Some(flag) = lock(&engine.explorer.searches).remove(agent_id) {
        flag.store(true, Ordering::Relaxed);
    }
}

fn rg_known(engine: &Engine, c: &Ctx) -> Option<bool> {
    lock(&engine.explorer.rg).get(&c.machine).copied()
}

fn remember_rg(engine: &Engine, c: &Ctx, seen: Option<bool>) {
    if let Some(s) = seen {
        lock(&engine.explorer.rg).insert(c.machine.clone(), s);
    }
}

#[cfg(test)]
mod tests;
