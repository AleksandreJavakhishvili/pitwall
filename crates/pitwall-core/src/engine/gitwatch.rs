//! Refresh an agent's git numbers when its files change instead of polling
//! (perf.md "Git refresh"). Where the provider says its machines can watch
//! folders (`ProviderCaps::fs_events`), the first refresh of an agent also
//! starts watching its checkout through the machine's `Exec::watch`; from
//! then on the ticker refreshes it when the watch saw a change (at the
//! normal pace at most) plus a slow safety poll while it works
//! (ticker.rs `git_due`). One watch per checkout, shared by every agent in
//! it; at most [`MAX_TREES`] at once. Where watching isn't possible (remote
//! machines, an OS limit, too many checkouts) agents are polled as before.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use super::Engine;
use crate::exec::{Exec, WatchSpec, Watching};
use crate::vcs::git::Git;

/// Checkouts watched at once; agents beyond are polled.
pub(crate) const MAX_TREES: usize = 64;
/// After watching failed, try again this much later (ms).
pub(crate) const RETRY_MS: u64 = 5 * 60_000;
/// At most this many git-ignored paths are skipped by name (the rest still
/// count as changes: harmless, only an extra refresh).
const MAX_IGNORED: usize = 2_000;
/// Git's ignored paths are listed again at most this often per checkout.
pub(crate) const RELIST_MS: u64 = 60_000;

/// One watched checkout.
pub struct Tree {
    /// Bumped on every change.
    gen: Arc<AtomicU64>,
    watching: Box<dyn Watching>,
    /// Its root on the machine.
    top: String,
    /// When git's ignored paths were last listed for it (mono ms).
    listed_at: AtomicU64,
}

impl Tree {
    pub fn gen(&self) -> u64 {
        self.gen.load(Ordering::SeqCst)
    }

    pub fn healthy(&self) -> bool {
        self.watching.healthy()
    }
}

/// Watched checkouts by machine and root, while some agent uses them.
#[derive(Default)]
pub(crate) struct Trees {
    by_key: Mutex<HashMap<String, Weak<Tree>>>,
}

impl Trees {
    /// The working watch kept for `key`.
    fn get(&self, key: &str) -> Option<Arc<Tree>> {
        let mut map = self.by_key.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, t| t.strong_count() > 0);
        map.get(key).and_then(Weak::upgrade).filter(|t| t.healthy())
    }

    /// Checkouts watched now.
    pub fn len(&self) -> usize {
        let mut map = self.by_key.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, t| t.strong_count() > 0);
        map.len()
    }

    /// Keep `tree` for `key`, or the one another caller kept meanwhile.
    fn put(&self, key: String, tree: Arc<Tree>) -> Arc<Tree> {
        let mut map = self.by_key.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = map
            .get(&key)
            .and_then(Weak::upgrade)
            .filter(|t| t.healthy())
        {
            return existing;
        }
        map.insert(key, Arc::downgrade(&tree));
        tree
    }
}

/// An agent's watch.
#[derive(Default)]
pub enum FsWatch {
    /// Not tried yet (or given up for a new folder).
    #[default]
    None,
    /// Watching `cwd`; `seen` is the change count of the last refresh.
    On {
        cwd: String,
        tree: Arc<Tree>,
        seen: u64,
    },
    /// Watching `cwd` failed at `at` (mono ms): poll.
    Off { cwd: String, at: u64 },
}

impl FsWatch {
    /// Whether the watch saw a change since the last refresh; `None` when
    /// there's no working watch of `cwd` (poll).
    pub fn changed(&self, cwd: &str) -> Option<bool> {
        match self {
            FsWatch::On { cwd: c, tree, seen } if c == cwd && tree.healthy() => {
                Some(tree.gen() != *seen)
            }
            _ => None,
        }
    }

    /// The watch sees everything `git status` could show (`Watching::complete`).
    pub fn complete(&self) -> bool {
        matches!(self, FsWatch::On { tree, .. } if tree.watching.complete())
    }
}

/// The checkout an agent at `cwd` works in, as the machine sees it.
struct Checkout {
    top: String,
    git_dir: String,
    common_dir: String,
}

fn checkout(git: &Git) -> Option<Checkout> {
    let out = git
        .run(&[
            "rev-parse",
            "--path-format=absolute",
            "--show-toplevel",
            "--git-dir",
            "--git-common-dir",
        ])
        .ok()?;
    let mut lines = out.lines().map(str::trim).filter(|l| !l.is_empty());
    Some(Checkout {
        top: lines.next()?.to_string(),
        git_dir: lines.next()?.to_string(),
        common_dir: lines.next()?.to_string(),
    })
}

/// What git ignores in the checkout (folders collapsed), for the filter.
fn ignored(git: &Git) -> Vec<String> {
    git.run(&[
        "ls-files",
        "--others",
        "--ignored",
        "--exclude-standard",
        "--directory",
        "--no-empty-directory",
    ])
    .map(|out| {
        out.lines()
            .filter(|l| !l.is_empty())
            .take(MAX_IGNORED)
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

/// Start (or join) the watch of the checkout agent `id` works in at `cwd`,
/// when its provider can and it isn't watched or recently failed. Returns
/// the watch to use for this refresh, if any. Blocking (git, once per
/// checkout); never under the registry lock.
pub(crate) fn ensure(core: &Engine, id: &str, exec: &dyn Exec, cwd: &str) -> Option<Arc<Tree>> {
    let now = core.now();
    let (want, have, machine) = core
        .with(id, |a| {
            if !a.facts.provider.fs_events {
                return (false, None, String::new());
            }
            let loc = a.rec.locator();
            let machine = format!("{}\n{}", loc.provider, loc.machine);
            match &a.fs {
                FsWatch::On { cwd: c, tree, .. } if c == cwd && tree.healthy() => {
                    (false, Some(tree.clone()), machine)
                }
                FsWatch::Off { cwd: c, at } if c == cwd && now.saturating_sub(*at) < RETRY_MS => {
                    (false, None, machine)
                }
                _ => (true, None, machine),
            }
        })
        .ok()?;
    if have.is_some() || !want {
        return have;
    }
    let tree = open(&core.trees, &machine, exec, cwd, now);
    if tree.is_none() {
        let _ = core.with(id, |a| {
            a.fs = FsWatch::Off {
                cwd: cwd.to_string(),
                at: now,
            }
        });
    }
    tree
}

/// The shared watch of the checkout at `cwd` on `machine`, started if needed.
fn open(trees: &Trees, machine: &str, exec: &dyn Exec, cwd: &str, now: u64) -> Option<Arc<Tree>> {
    let git = Git::new(exec, cwd);
    let c = checkout(&git)?;
    let key = format!("{machine}\n{}", c.top);
    if let Some(t) = trees.get(&key) {
        return Some(t);
    }
    if trees.len() >= MAX_TREES {
        return None;
    }
    let inside =
        |d: &str| d == c.top || d.starts_with(&format!("{}/", c.top.trim_end_matches('/')));
    let spec = WatchSpec {
        tree: c.top.clone(),
        git_dirs: [&c.git_dir, &c.common_dir]
            .into_iter()
            .filter(|d| !inside(d))
            .cloned()
            .collect(),
        ignored: ignored(&git.at(&c.top)),
    };
    let gen = Arc::new(AtomicU64::new(0));
    let bump = gen.clone();
    let watching = exec
        .watch(
            &spec,
            Arc::new(move || {
                bump.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .ok()?;
    let tree = Tree {
        gen,
        watching,
        top: c.top,
        listed_at: AtomicU64::new(now),
    };
    Some(trees.put(key, Arc::new(tree)))
}

/// A refresh the watch asked for found nothing new: perhaps something git
/// ignores changed that wasn't there when its list was made (a build's new
/// output folder). List git's ignored paths again, at most every
/// [`RELIST_MS`] per checkout. Blocking (git).
pub(crate) fn relist_ignored(tree: &Tree, exec: &dyn Exec, now: u64) {
    let last = tree.listed_at.load(Ordering::SeqCst);
    if now.saturating_sub(last) < RELIST_MS
        || tree
            .listed_at
            .compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
    {
        return;
    }
    tree.watching
        .set_ignored(&ignored(&Git::new(exec, &tree.top)));
}

#[cfg(test)]
pub(crate) mod tests;
