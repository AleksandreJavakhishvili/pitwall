//! Worktrees in source control (docs/spec/worktrees-view.md): every linked
//! worktree of each project agents work in, attributed to an agent where
//! possible, with its own changes and actions.
//!
//! Cheap by design (perf.md): one `git worktree list --porcelain` per project
//! per refresh, through the [`Exec`] of the machine the project is on (so an
//! agw project goes through its slow exec, and is listed at most every
//! `git_poll_ms`). A project is listed again only when something about its
//! agents changed or the last list is ~30 s old; the UI asks while it is
//! visible. A worktree's changes are read only when the UI shows it.
//! Reads never touch an index or working tree (`GIT_OPTIONAL_LOCKS=0`, no
//! `git status` refresh); commit, merge and remove are explicit user actions
//! and never use `--force`.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard};

use pitwall_proto::{MachineView, ProjectWorktrees, WorktreeCaps, WorktreeVia, WorktreeView};

use crate::engine::Engine;
use crate::exec::Exec;
use crate::paths;
use crate::provider::Locator;
use crate::vcs::git::{FileChange, Git, WorktreeEntry};
use crate::vcs::review::{self as ops, FileVersions, MergeResult, MergeStatus};

type Res<T> = Result<T, String>;

/// A project is listed again at least this often while the UI asks.
pub const PERIOD_MS: u64 = 30_000;
/// The UI asks every `PERIOD_MS`; a list a little younger than that still
/// counts as due, so the period doesn't silently double.
const PERIOD_SLACK_MS: u64 = 5_000;
/// Fastest re-list after a change in the project (providers with a slow
/// exec: their `git_poll_ms`).
const MIN_GAP_MS: u64 = 2_000;

// ------------------------------------------------------------------ attribution (pure)

/// An agent, as the attribution rules see it.
#[derive(Debug, Clone)]
pub struct Spot {
    pub id: String,
    /// Its working folder, resolved on its machine.
    pub folder: String,
    /// Tool-managed worktree folders, relative to its checkout's root
    /// (`AgentKind::worktree_dirs`).
    pub tool_dirs: Vec<String>,
    pub created_at: u64,
}

/// `path` is `root` or inside it.
fn inside(path: &str, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    path == root || path.strip_prefix(root).is_some_and(|rest| rest.starts_with('/'))
}

/// The index of the deepest entry containing `path`.
fn deepest(entries: &[WorktreeEntry], path: &str) -> Option<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.bare && inside(path, &e.path))
        .max_by_key(|(_, e)| e.path.len())
        .map(|(i, _)| i)
}

/// Who each entry belongs to, by the rules in order: the agent's own folder,
/// a tool-managed folder under the agent's checkout, a process of the agent
/// working in it (`proc_cwds`: agent id → folder), else nobody. Ties go to
/// the agent with a process there, then the deepest checkout, then the
/// oldest agent. One answer per entry (the main checkout and a bare
/// repository included; callers skip them).
pub fn attribute(entries: &[WorktreeEntry], spots: &[Spot], proc_cwds: &[(String, String)]) -> Vec<(Option<String>, WorktreeVia)> {
    let own: Vec<Option<usize>> = spots.iter().map(|s| deepest(entries, &s.folder)).collect();
    let procs: Vec<(&str, usize)> = proc_cwds.iter().filter_map(|(id, cwd)| Some((id.as_str(), deepest(entries, cwd)?))).collect();
    let has_proc = |id: &str, i: usize| procs.iter().any(|(p, e)| *p == id && *e == i);
    let order = |a: &&Spot, b: &&Spot| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id));

    entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            // 1. Its own working folder.
            if let Some(s) = spots.iter().zip(&own).filter(|(_, o)| **o == Some(i)).map(|(s, _)| s).min_by(order) {
                return (Some(s.id.clone()), WorktreeVia::Own);
            }
            // 2. Under the agent's checkout, where its tool keeps worktrees.
            let mut tool: Vec<(&Spot, usize)> = spots
                .iter()
                .zip(&own)
                .filter_map(|(s, o)| {
                    let root = o.map_or(s.folder.as_str(), |o| entries[o].path.as_str());
                    let hit = s.tool_dirs.iter().any(|d| {
                        let d = d.trim_matches('/');
                        !d.is_empty() && !d.split('/').any(|p| p == "..") && inside(&e.path, &format!("{}/{d}", root.trim_end_matches('/')))
                    });
                    hit.then_some((s, root.len()))
                })
                .collect();
            tool.sort_by(|(a, ra), (b, rb)| {
                has_proc(&b.id, i).cmp(&has_proc(&a.id, i)).then(rb.cmp(ra)).then(order(a, b))
            });
            if let Some((s, _)) = tool.first() {
                return (Some(s.id.clone()), WorktreeVia::ToolDir);
            }
            // 3. A process in the agent's terminal works in it.
            if let Some(s) = spots.iter().filter(|s| has_proc(&s.id, i)).min_by(order) {
                return (Some(s.id.clone()), WorktreeVia::Process);
            }
            (None, WorktreeVia::Other)
        })
        .collect()
}

// ------------------------------------------------------------------ cache

/// One listing of a project, from one agent's folder.
#[derive(Debug, Clone)]
struct Listed {
    /// Engine mono ms when it was listed (0 = must list again).
    at: u64,
    /// The project's agents' state when listed (`Member::sig`).
    sig: u64,
    entries: Vec<WorktreeEntry>,
    /// (agent id, folder a process of it works in), when rule 3 was needed.
    proc_cwds: Vec<(String, String)>,
    error: Option<String>,
}

/// The engine's worktree listings, keyed by [`group_key`].
#[derive(Default)]
pub struct Cache {
    listed: Mutex<HashMap<String, Listed>>,
    /// One refresh at a time (two windows asking at once list once).
    refresh: Mutex<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the refresh needs to know about one agent (read under the lock).
#[derive(Debug, Clone)]
struct Member {
    id: String,
    loc: Locator,
    project: String,
    cwd: String,
    /// `cwd` resolved, when known.
    real: Option<String>,
    kind: String,
    created_at: u64,
    pids: Vec<u32>,
    running: bool,
    process_cwd: bool,
    machine: MachineView,
    home: Option<String>,
    /// Slow exec: re-list at most this often after changes.
    min_gap: u64,
    sig: u64,
}

/// Agents that work in one place: provider, machine and project folder.
fn group_key(m: &Member) -> String {
    format!("{}:{}:{}", m.loc.provider, m.loc.machine, m.project)
}

fn members(engine: &Engine) -> Vec<Member> {
    engine
        .agents()
        .iter()
        .filter(|a| a.caps().worktrees)
        .map(|a| {
            let r = &a.rec;
            let loc = r.locator();
            let p = &a.facts.provider;
            let mut h = DefaultHasher::new();
            (&r.cwd, a.added, a.removed, a.files_changed, &a.branch, a.status as u8).hash(&mut h);
            Member {
                id: r.id.clone(),
                project: r.project.clone(),
                cwd: r.cwd.clone(),
                real: a.real_cwd.as_ref().filter(|(from, _)| *from == r.cwd).map(|(_, real)| real.clone()),
                kind: a.kind().to_string(),
                created_at: r.created_at,
                pids: [a.local_pid(), a.inner.as_ref().map(|i| i.pid).filter(|_| p.local_process)].into_iter().flatten().collect(),
                running: a.running(),
                process_cwd: p.process_cwd,
                machine: MachineView {
                    provider: loc.provider.to_string(),
                    id: loc.machine.to_string(),
                    label: if a.facts.machine_label.is_empty() { loc.machine.to_string() } else { a.facts.machine_label.clone() },
                    can_create: p.create && !p.platform_create,
                },
                home: a.facts.home.clone(),
                min_gap: u64::from(p.git_poll_ms).max(MIN_GAP_MS),
                sig: h.finish(),
                loc,
            }
        })
        .collect()
}

/// Resolve members' folders on their machines (once per folder; kept on the agent).
fn resolve(engine: &Engine, members: &mut [Member], execs: &HashMap<String, Arc<dyn Exec>>) {
    for m in members.iter_mut().filter(|m| m.real.is_none()) {
        let Some(exec) = execs.get(&m.id) else { continue };
        let real = exec.real_path(&m.cwd).unwrap_or_else(|_| m.cwd.trim_end_matches('/').to_string());
        let _ = engine.with(&m.id, |a| a.real_cwd = Some((m.cwd.clone(), real.clone())));
        m.real = Some(real);
    }
}

/// Members by [`group_key`], in order.
fn grouped(ms: &[Member]) -> Vec<(String, Vec<&Member>)> {
    let mut groups: Vec<(String, Vec<&Member>)> = Vec::new();
    for m in ms {
        let key = group_key(m);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(m),
            None => groups.push((key, vec![m])),
        }
    }
    groups
}

fn group_sig(ms: &[&Member]) -> u64 {
    let mut h = DefaultHasher::new();
    for m in ms {
        (&m.id, m.sig).hash(&mut h);
    }
    h.finish()
}

fn spots(engine: &Engine, ms: &[&Member]) -> Vec<Spot> {
    let kinds = engine.kinds().kinds();
    ms.iter()
        .map(|m| Spot {
            id: m.id.clone(),
            folder: m.real.clone().unwrap_or_else(|| m.cwd.clone()),
            tool_dirs: kinds.iter().find(|k| k.id == m.kind).map(|k| k.worktree_dirs.clone()).unwrap_or_default(),
            created_at: m.created_at,
        })
        .collect()
}

/// Linked worktrees nobody claimed by folder: worth asking processes (rule 3).
fn unclaimed(entries: &[WorktreeEntry], spots: &[Spot]) -> bool {
    attribute(entries, spots, &[]).iter().zip(entries).skip(1).any(|((_, via), e)| *via == WorktreeVia::Other && !e.bare && !e.prunable)
}

/// Folders processes of these agents work in, where their provider can
/// tell (one batched ask per provider: one `lsof` on this Mac).
fn process_cwds(engine: &Engine, ms: &[&Member]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // Per provider: (agent id, what to ask).
    type Asks<'a> = Vec<(&'a str, (Locator, Option<u32>))>;
    let mut by_provider: Vec<(&str, Asks)> = Vec::new();
    for m in ms.iter().filter(|m| m.process_cwd && m.running) {
        let asks: Vec<_> = if m.pids.is_empty() {
            vec![(m.id.as_str(), (m.loc.clone(), None))]
        } else {
            m.pids.iter().map(|p| (m.id.as_str(), (m.loc.clone(), Some(*p)))).collect()
        };
        let provider = m.loc.provider.as_str();
        match by_provider.iter_mut().find(|(p, _)| *p == provider) {
            Some((_, list)) => list.extend(asks),
            None => by_provider.push((provider, asks)),
        }
    }
    for (_, asks) in by_provider {
        let Ok(p) = engine.providers().for_locator(&asks[0].1 .0) else { continue };
        let locs: Vec<(Locator, Option<u32>)> = asks.iter().map(|(_, a)| a.clone()).collect();
        for ((id, _), cwd) in asks.iter().zip(p.process_cwds(&locs)) {
            if let Some(cwd) = cwd {
                out.push((id.to_string(), cwd));
            }
        }
    }
    out
}

/// List the projects that are due (all of them with `force`) and build the
/// views. Blocking (git, `lsof`).
fn refresh(engine: &Engine, force: bool) -> Vec<ProjectWorktrees> {
    let _one = lock(&engine.worktrees.refresh);
    let mut ms = members(engine);
    let execs: HashMap<String, Arc<dyn Exec>> = ms.iter().map(|m| (m.id.clone(), engine.exec_for(&m.id))).collect();
    resolve(engine, &mut ms, &execs);

    let groups = grouped(&ms);
    let now = engine.now();
    for (key, list) in &groups {
        let sig = group_sig(list);
        let due = match lock(&engine.worktrees.listed).get(key) {
            None => true,
            Some(l) => {
                let age = now.saturating_sub(l.at);
                force || l.at == 0 || age + PERIOD_SLACK_MS >= PERIOD_MS || (l.sig != sig && age >= list[0].min_gap)
            }
        };
        if !due {
            continue;
        }
        let exec = &execs[&list[0].id];
        let listed = match Git::new(&**exec, &list[0].cwd).worktree_list() {
            Ok(entries) => {
                let sp = spots(engine, list);
                let proc_cwds = if unclaimed(&entries, &sp) { process_cwds(engine, list) } else { vec![] };
                Listed { at: now, sig, entries, proc_cwds, error: None }
            }
            Err(e) => {
                let old = lock(&engine.worktrees.listed).get(key).cloned();
                let (entries, proc_cwds) = old.map(|o| (o.entries, o.proc_cwds)).unwrap_or_default();
                Listed { at: now, sig, entries, proc_cwds, error: Some(e) }
            }
        };
        lock(&engine.worktrees.listed).insert(key.clone(), listed);
    }
    let listed = {
        let mut listed = lock(&engine.worktrees.listed);
        // Forget projects nobody works in any more.
        listed.retain(|k, _| groups.iter().any(|(g, _)| g == k));
        listed.clone()
    };
    build(engine, &groups, &listed)
}

/// Views from the listings: groups that turned out to be the same repository
/// (same machine and main checkout) become one project.
fn build(engine: &Engine, groups: &[(String, Vec<&Member>)], listed: &HashMap<String, Listed>) -> Vec<ProjectWorktrees> {
    let mut out: Vec<(ProjectWorktrees, Vec<&Member>, Listed)> = Vec::new();
    for (key, list) in groups {
        let Some(l) = listed.get(key) else { continue };
        let m0 = list[0];
        let Some(main) = l.entries.first() else {
            // Nothing listed yet (an error before the first good list).
            continue;
        };
        let id = format!("{}:{}:{}", m0.loc.provider, m0.loc.machine, main.path);
        if let Some((_, ms, prev)) = out.iter_mut().find(|(p, _, _)| p.id == id) {
            ms.extend(list.iter().copied());
            if l.at > prev.at {
                *prev = l.clone();
            }
            continue;
        }
        let view = ProjectWorktrees {
            id,
            repo: main.path.clone(),
            repo_display: paths::tildify_in(m0.home.as_deref(), &main.path),
            branch: if main.bare { None } else { main.branch.clone() },
            machine: m0.machine.clone(),
            agent_ids: vec![],
            worktrees: vec![],
            error: None,
        };
        out.push((view, list.clone(), l.clone()));
    }
    out.into_iter()
        .map(|(mut view, ms, l)| {
            let sp = spots(engine, &ms);
            let who = attribute(&l.entries, &sp, &l.proc_cwds);
            let main = &l.entries[0];
            view.agent_ids = ms.iter().map(|m| m.id.clone()).collect();
            view.error = l.error.clone();
            view.worktrees = l
                .entries
                .iter()
                .zip(who)
                .skip(1)
                .filter(|(e, _)| !e.bare)
                .map(|(e, (agent_id, via))| worktree_view(e, main, agent_id, via, &view, ms[0].home.as_deref()))
                .collect();
            view
        })
        .collect()
}

fn worktree_view(e: &WorktreeEntry, main: &WorktreeEntry, agent_id: Option<String>, via: WorktreeVia, p: &ProjectWorktrees, home: Option<&str>) -> WorktreeView {
    let diff = !e.prunable;
    let target = p.branch.as_deref().filter(|_| !main.bare);
    WorktreeView {
        path: e.path.clone(),
        path_display: paths::tildify_in(home, &e.path),
        name: e.path.trim_end_matches('/').rsplit('/').next().unwrap_or(&e.path).to_string(),
        branch: e.branch.clone(),
        head: e.head.clone(),
        locked: e.locked,
        lock_reason: e.lock_reason.clone(),
        prunable: e.prunable,
        agent_id,
        via,
        caps: WorktreeCaps {
            diff,
            commit: diff,
            merge: diff && e.branch.is_some() && target.is_some() && e.branch.as_deref() != target,
            remove: diff && !e.locked && via != WorktreeVia::Own,
            terminal: diff && p.machine.can_create,
        },
    }
}

// ------------------------------------------------------------------ services

/// Every project's worktrees, listing the projects that are due. Blocking.
pub fn list(engine: &Engine) -> Vec<ProjectWorktrees> {
    refresh(engine, false)
}

/// One worktree, as the UI was last shown it (never a path it wasn't).
struct Target {
    exec: Arc<dyn Exec>,
    /// Its project's agents (refreshed after changes).
    agents: Vec<String>,
    repo: String,
    wt: WorktreeView,
    /// Main checkout's branch, else its HEAD commit (merge-base target).
    target: Option<String>,
}

fn target(engine: &Engine, project: &str, path: &str) -> Res<Target> {
    let ms = members(engine);
    let listed = lock(&engine.worktrees.listed).clone();
    let p = build(engine, &grouped(&ms), &listed)
        .into_iter()
        .find(|v| v.id == project)
        .ok_or("That project's worktrees aren't listed any more.")?;
    let wt = p.worktrees.iter().find(|w| w.path == path).cloned().ok_or("That worktree isn't listed any more.")?;
    let owner = p.agent_ids.first().cloned().ok_or("No agent works in that project any more.")?;
    let main_head = listed.values().find_map(|l| l.entries.first().filter(|m| m.path == p.repo).and_then(|m| m.head.clone()));
    let target = p.branch.clone().map(|b| format!("refs/heads/{b}")).or(main_head);
    Ok(Target { exec: engine.exec_for(&owner), agents: p.agent_ids, repo: p.repo, wt, target })
}

impl Target {
    fn git(&self) -> Git<'_> {
        Git::new(&*self.exec, &self.wt.path)
    }

    fn need(&self, ok: bool, why: &str) -> Res<()> {
        if ok {
            Ok(())
        } else {
            Err(format!("{}: {why}", self.wt.path_display))
        }
    }

    /// Merge-base with the project's current branch (`None`: unrelated or
    /// unknown; then its own HEAD).
    fn base(&self) -> Option<String> {
        self.git().merge_base(self.target.as_deref()?, "HEAD")
    }

    /// The agents of the project show fresh numbers (and the list is read again).
    fn touched(&self, engine: &Engine) {
        for id in &self.agents {
            let _ = engine.with(id, |a| a.git_wanted = true);
        }
        for l in lock(&engine.worktrees.listed).values_mut() {
            l.at = 0;
        }
        engine.changed(false);
    }
}

/// Its changes against the merge-base with the project's current branch,
/// including uncommitted and untracked files. Blocking.
pub fn changes(engine: &Engine, project: &str, path: &str) -> Res<Vec<FileChange>> {
    let t = target(engine, project, path)?;
    t.need(t.wt.caps.diff, "its folder is gone")?;
    t.git().changes(t.base().as_deref())
}

pub fn file_versions(engine: &Engine, project: &str, path: &str, file: &str) -> Res<FileVersions> {
    let t = target(engine, project, path)?;
    t.need(t.wt.caps.diff, "its folder is gone")?;
    ops::file_versions(&t.git(), file, t.base().as_deref(), None)
}

pub fn merge_status(engine: &Engine, project: &str, path: &str) -> Res<MergeStatus> {
    let t = target(engine, project, path)?;
    t.need(t.wt.caps.diff, "its folder is gone")?;
    let branch = t.git().current_branch();
    ops::merge_status(&t.git(), Some((&t.repo, branch.as_deref())))
}

/// `git add -A && git commit -m <message>` in the worktree.
pub fn commit(engine: &Engine, project: &str, path: &str, message: &str) -> Res<String> {
    let t = target(engine, project, path)?;
    t.need(t.wt.caps.commit, "can't commit there")?;
    let id = ops::commit(&t.git(), message)?;
    t.touched(engine);
    Ok(id)
}

/// Merge the worktree's branch into the project's current branch, with the
/// Review merge rules (dirty main checkout refused, conflicts aborted).
pub fn merge(engine: &Engine, project: &str, path: &str) -> Res<MergeResult> {
    let t = target(engine, project, path)?;
    t.need(t.wt.caps.diff, "its folder is gone")?;
    let branch = t.git().current_branch().ok_or("This worktree isn't on a branch (detached HEAD). Create a branch there, then merge.")?;
    let result = ops::merge(&t.git().at(&t.repo), &branch)?;
    t.touched(engine);
    Ok(result)
}

/// `git worktree remove <path>` — never `--force`, so git refuses when it has
/// uncommitted changes; locked worktrees are refused before git is asked.
/// The branch is kept.
pub fn remove(engine: &Engine, project: &str, path: &str) -> Res<()> {
    let t = target(engine, project, path)?;
    if t.wt.via == WorktreeVia::Own {
        return Err(format!("{} is where an agent works: remove the agent (with its worktree) instead.", t.wt.path_display));
    }
    // Locked now? (The list the UI saw may be older.)
    let main = Git::new(&*t.exec, &t.repo);
    let now = main.worktree_list()?;
    let e = now.iter().skip(1).find(|e| e.path == t.wt.path).ok_or("That worktree isn't listed any more.")?;
    if e.locked {
        let why = e.lock_reason.as_deref().map(|r| format!(" ({r})")).unwrap_or_default();
        return Err(format!("{} is locked{why}. Pitwall doesn't remove locked worktrees.", t.wt.path_display));
    }
    main.worktree_remove(&t.wt.path)?;
    t.touched(engine);
    Ok(())
}

#[cfg(test)]
mod tests;
