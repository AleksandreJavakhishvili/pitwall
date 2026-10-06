//! Agents that work in their own git worktree (docs/spec/worktrees.md).
//! Pitwall never creates worktrees: it passes the agent's own flag
//! (`claude --worktree <name>`, `codex --worktree`) and then finds out where
//! the agent actually works, from (in order) the hook payload's `cwd`, the
//! process's cwd (`lsof`), or a worktree that appeared after the launch.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::git;
use crate::model::{mono_ms, WorktreeInfo};
use crate::registry::{Agent, Shared};

/// How long after a launch the ticker keeps looking (hooks can still find it later).
pub const WATCH_MS: u64 = 120_000;
/// Gap between two looks.
pub const POLL_MS: u64 = 1_500;
const LSOF_TIMEOUT: Duration = Duration::from_secs(3);

/// In-memory: an agent launched with its worktree flag that we're watching.
#[derive(Debug, Clone)]
pub struct Watch {
    /// Worktree roots of the project before the launch.
    pub before: Vec<String>,
    /// The name passed to the agent (if its flag takes one).
    pub hint: Option<String>,
    pub started: u64,
    pub next_at: u64,
    pub inflight: bool,
}

impl Watch {
    pub fn new(before: Vec<String>, hint: Option<String>) -> Watch {
        let now = mono_ms();
        Watch { before, hint, started: now, next_at: now + 500, inflight: false }
    }
}

/// Where the agent turned out to work.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// Worktree root.
    pub path: String,
    /// The agent's working folder (the root or a folder inside it).
    pub cwd: String,
    pub branch: String,
    /// Merge-base with the project's current branch.
    pub base: Option<String>,
}

fn canon(p: &str) -> String {
    std::fs::canonicalize(p)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| p.trim_end_matches('/').to_string())
}

/// Roots of every worktree of `repo` (canonical paths).
pub fn roots(repo: &str) -> Vec<String> {
    git::worktree_list(repo).unwrap_or_default().iter().map(|e| canon(&e.path)).collect()
}

/// A name for the agent's flag that no existing worktree of `repo` uses yet
/// (by folder name or branch), so two agents never land in the same one.
pub fn pick_name(repo: &str, base: &str) -> String {
    let taken: Vec<String> = git::worktree_list(repo)
        .unwrap_or_default()
        .into_iter()
        .flat_map(|e| {
            let dir = Path::new(&e.path).file_name().map(|n| n.to_string_lossy().into_owned());
            let branch = e.branch.map(|b| b.rsplit('/').next().unwrap_or(&b).to_string());
            [dir, branch].into_iter().flatten()
        })
        .collect();
    let clash = |n: &str| taken.iter().any(|t| t == n || t.ends_with(&format!("-{n}")));
    (1..100)
        .map(|i| if i == 1 { base.to_string() } else { format!("{base}-{i}") })
        .find(|n| !clash(n))
        .unwrap_or_else(|| format!("{base}-{}", &uuid::Uuid::new_v4().to_string()[..8]))
}

/// `dir` is (inside) a linked worktree of `repo` — not the main checkout and
/// not one another agent already has.
pub fn check(repo: &str, dir: &str, claimed: &[String]) -> Option<Found> {
    if !Path::new(dir).is_dir() {
        return None;
    }
    let top = canon(&git::repo_root(dir)?);
    let roots = roots(repo);
    if roots.first() == Some(&top) || !roots.contains(&top) || claimed.iter().any(|c| canon(c) == top) {
        return None;
    }
    Some(found(repo, &top, &canon(dir)))
}

fn found(repo: &str, top: &str, cwd: &str) -> Found {
    let branch = git::current_branch(top).unwrap_or_default();
    let target = git::current_branch(repo).unwrap_or_else(|| "HEAD".into());
    let base = git::merge_base(repo, &target, &git::head(top).unwrap_or_else(|| "HEAD".into()))
        .or_else(|| git::head(top));
    Found { path: top.to_string(), cwd: cwd.to_string(), branch, base }
}

/// The current folder of process `pid`.
pub fn process_cwd(pid: u32) -> Option<String> {
    let mut lsof = Command::new("/usr/sbin/lsof");
    lsof.args(["-a", "-d", "cwd", "-p", &pid.to_string(), "-Fn"]);
    let out = crate::scan::run_with_timeout(lsof, LSOF_TIMEOUT)?;
    crate::scan::parse_lsof_cwd(&out).into_iter().find(|(p, _)| *p == pid).map(|(_, cwd)| cwd)
}

/// Everything the ticker's look needs (gathered under the lock).
#[derive(Debug, Clone)]
pub struct Probe {
    pub id: String,
    pub repo: String,
    pub pid: Option<u32>,
    pub before: Vec<String>,
    pub hint: Option<String>,
    pub claimed: Vec<String>,
}

/// Process cwd first, else a worktree that appeared since the launch.
pub fn discover(p: &Probe) -> Option<Found> {
    if let Some(f) = p.pid.and_then(process_cwd).and_then(|cwd| check(&p.repo, &cwd, &p.claimed)) {
        return Some(f);
    }
    discover_new(p)
}

fn discover_new(p: &Probe) -> Option<Found> {
    let entries = git::worktree_list(&p.repo).ok()?;
    let mut fresh: Vec<_> = entries
        .into_iter()
        .skip(1)
        .filter(|e| {
            let c = canon(&e.path);
            !p.before.contains(&c) && !p.claimed.iter().any(|x| canon(x) == c) && Path::new(&c).is_dir()
        })
        .collect();
    // The one named after what we passed, if the agent's flag took a name.
    if let Some(hint) = &p.hint {
        let named = |e: &git::WorktreeEntry| {
            Path::new(&e.path).file_name().is_some_and(|n| n.to_string_lossy() == *hint)
                || e.branch.as_deref().is_some_and(|b| b == hint || b.ends_with(&format!("/{hint}")) || b.ends_with(&format!("-{hint}")))
        };
        if let Some(i) = fresh.iter().position(named) {
            fresh.swap(0, i);
        }
    }
    let e = fresh.into_iter().next()?;
    let top = canon(&e.path);
    Some(found(&p.repo, &top, &top))
}

/// Paths other agents already work in.
pub fn claimed(agents: &[Agent], except: &str) -> Vec<String> {
    agents
        .iter()
        .filter(|a| a.rec.id != except)
        .filter_map(|a| a.rec.worktree.as_ref().map(|w| w.path.clone()))
        .collect()
}

/// Make `found` the agent's working dir (no-op unless still pending).
pub fn adopt(a: &mut Agent, f: Found) -> bool {
    if !a.rec.worktree_pending {
        return false;
    }
    a.rec.worktree = Some(WorktreeInfo { repo: a.rec.project.clone(), path: f.path, branch: f.branch });
    a.rec.cwd = f.cwd;
    if f.base.is_some() {
        a.rec.base_commit = f.base;
    }
    a.rec.worktree_pending = false;
    a.watch = None;
    a.git_wanted = true;
    true
}

/// Adopt `found` for agent `id`, then write its rules into the worktree.
pub fn apply(core: &Shared, id: &str, found: Option<Found>) {
    let adopted = {
        let mut agents = core.agents();
        // Another agent may have taken it while we looked (checked unlocked).
        let taken = found.as_ref().is_some_and(|f| claimed(&agents, id).contains(&f.path));
        agents.iter_mut().find(|a| a.rec.id == id).and_then(|a| {
            if let Some(w) = a.watch.as_mut() {
                w.inflight = false;
                w.next_at = mono_ms() + POLL_MS;
            }
            let f = found.filter(|_| !taken)?;
            adopt(a, f).then(|| a.rec.clone())
        })
    };
    if let Some(rec) = adopted {
        core.changed(true);
        std::thread::spawn(move || crate::rules::on_worktree_found(&rec));
    }
}

/// Ticker: the agents due for a look (call under the registry lock).
pub fn due(agents: &mut [Agent], now: u64) -> Vec<Probe> {
    let mut probes = Vec::new();
    for i in 0..agents.len() {
        let a = &agents[i];
        let Some(w) = &a.watch else { continue };
        if !a.rec.worktree_pending || w.inflight || now < w.next_at {
            continue;
        }
        if now.saturating_sub(w.started) > WATCH_MS {
            agents[i].watch = None;
            continue;
        }
        let probe = Probe {
            id: a.rec.id.clone(),
            repo: a.rec.project.clone(),
            pid: a.session.as_ref().and_then(|s| s.pid),
            before: w.before.clone(),
            hint: w.hint.clone(),
            claimed: claimed(agents, &a.rec.id),
        };
        if let Some(w) = agents[i].watch.as_mut() {
            w.inflight = true;
        }
        probes.push(probe);
    }
    probes
}

/// Hook payloads carry the session's `cwd`: the most direct answer.
pub fn on_hook(core: &Shared, id: &str, payload: &Value) {
    let Some(cwd) = payload.get("cwd").and_then(Value::as_str).filter(|c| !c.is_empty()) else { return };
    let pending = {
        let agents = core.agents();
        agents
            .iter()
            .find(|a| a.rec.id == id && a.rec.worktree_pending)
            .map(|a| (a.rec.project.clone(), claimed(&agents, id)))
    };
    let Some((repo, claimed)) = pending else { return };
    if let Some(f) = check(&repo, cwd, &claimed) {
        apply(core, id, Some(f));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::tests::TempRepo;

    fn setup() -> (TempRepo, String) {
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let repo = canon(r.path());
        (r, repo)
    }

    #[test]
    fn parses_porcelain() {
        let out = "worktree /r\nHEAD abc\nbranch refs/heads/main\n\nworktree /r/.claude/worktrees/x\nHEAD def\ndetached\n\n";
        let got = git::parse_worktree_list(out);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].branch.as_deref(), Some("main"));
        assert_eq!(got[1].path, "/r/.claude/worktrees/x");
        assert_eq!(got[1].branch, None);
    }

    #[test]
    fn check_accepts_only_linked_worktrees() {
        let (r, repo) = setup();
        assert_eq!(check(&repo, &repo, &[]), None, "main checkout is not a worktree");
        std::fs::create_dir_all(r.0.join("sub")).unwrap();
        assert_eq!(check(&repo, &format!("{repo}/sub"), &[]), None);
        let wt = r.0.join(".agent/worktrees/fix");
        r.git(&["worktree", "add", "-q", "-b", "worktree-fix", wt.to_str().unwrap(), "HEAD"]);
        std::fs::create_dir_all(wt.join("deep")).unwrap();
        let f = check(&repo, wt.join("deep").to_str().unwrap(), &[]).unwrap();
        assert_eq!(f.path, canon(wt.to_str().unwrap()));
        assert!(f.cwd.ends_with("/deep"));
        assert_eq!(f.branch, "worktree-fix");
        assert_eq!(f.base.as_deref(), Some(r.git(&["rev-parse", "HEAD"]).trim()));
        assert_eq!(check(&repo, wt.to_str().unwrap(), std::slice::from_ref(&f.path)), None, "claimed by another agent");
        let other = TempRepo::new();
        assert_eq!(check(&repo, other.path(), &[]), None, "a different repo");
    }

    #[test]
    fn names_avoid_existing_worktrees() {
        let (r, repo) = setup();
        assert_eq!(pick_name(&repo, "fix"), "fix");
        let wt = r.0.join("wts/fix");
        r.git(&["worktree", "add", "-q", "-b", "worktree-fix", wt.to_str().unwrap(), "HEAD"]);
        assert_eq!(pick_name(&repo, "fix"), "fix-2");
    }

    /// A harmless stand-in for an agent: makes its own worktree in a temp repo
    /// and `cd`s into it, like `claude --worktree` does.
    #[test]
    fn finds_worktree_a_fake_agent_made() {
        let (r, repo) = setup();
        let before = roots(&repo);
        let wt = r.0.join(".fake/worktrees/job");
        let script = format!(
            "git worktree add -q -b fake/job '{}' HEAD && cd '{}' && exec sleep 30",
            wt.display(),
            wt.display()
        );
        let mut child = Command::new("/bin/sh").arg("-c").arg(script).current_dir(&repo).spawn().unwrap();
        let pid = child.id();
        let probe = Probe {
            id: "a".into(),
            repo: repo.clone(),
            pid: Some(pid),
            before: before.clone(),
            hint: Some("job".into()),
            claimed: vec![],
        };
        let mut got = None;
        for _ in 0..100 {
            got = discover(&probe);
            if got.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        let f = got.expect("found the worktree");
        assert_eq!(f.path, canon(wt.to_str().unwrap()));
        assert_eq!(f.branch, "fake/job");

        // Without a pid (or lsof): the newly listed worktree is found too.
        let f2 = discover(&Probe { pid: None, ..probe.clone() }).unwrap();
        assert_eq!(f2.path, f.path);
        // ...but not when it existed before the launch.
        assert_eq!(discover(&Probe { pid: None, before: roots(&repo), ..probe }), None);
    }
}
