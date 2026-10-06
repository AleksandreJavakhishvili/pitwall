//! Per-task snapshots (docs/spec/review.md). A task is one prompt delivered to
//! an agent, until the agent is done/idle. At task start and end we snapshot
//! the working tree into a tree object WITHOUT side effects on the user's
//! checkout: a temporary index (`GIT_INDEX_FILE`) + `git add -A` + `write-tree`.
//! The real index and the working tree are never touched. Trees are kept alive
//! by one ref per task, `refs/pitwall/<agent>/<task>`, pointing at a small tree
//! with `start` (and later `end`) entries.
//!
//! Bookkeeping happens under the registry lock (in-memory only); the git work
//! runs on one background worker, in order, so a task's start snapshot is always
//! taken before its end snapshot.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::model::{unix_ms, AgentRecord, Status};
use crate::registry::{Agent, Shared};

/// A hook arriving this soon after a task started belongs to that same prompt.
const SAME_PROMPT_MS: u64 = 5000;
/// Tasks kept per agent; older ones (and their refs) are dropped.
const MAX_TASKS: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    /// The prompt as delivered; empty when it was typed in the terminal and no
    /// hook told us the text.
    pub prompt: String,
    pub started_at: u64,
    #[serde(default)]
    pub ended_at: Option<u64>,
    /// `None` until the start snapshot has been taken.
    #[serde(default)]
    pub start_tree: Option<String>,
    #[serde(default)]
    pub end_tree: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    Start { agent: String, task: String, cwd: String },
    End { agent: String, task: String, cwd: String },
    Drop { agent: String, task: String, cwd: String },
}

// ------------------------------------------------------------------ git plumbing

/// git with `GIT_OPTIONAL_LOCKS=0`, like git.rs.
pub(crate) fn git(cwd: &str) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true");
    cmd
}

pub(crate) fn run(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn temp_index_path() -> PathBuf {
    std::env::temp_dir().join(format!("pitwall-index-{}", uuid::Uuid::new_v4()))
}

struct TempIndex(PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("lock"));
    }
}

/// Tree id of the working tree as it is now, untracked (non-ignored) files
/// included. Uses a throwaway copy of the index; never writes the real one.
pub fn snapshot(cwd: &str) -> Result<String, String> {
    let top = run(git(cwd).args(["rev-parse", "--show-toplevel"]))?.trim().to_string();
    let tmp = TempIndex(temp_index_path());
    // Seed with a copy of the real index: keeps tracked-but-ignored files and
    // lets git reuse its stat cache. Fall back to HEAD's tree, then to empty.
    let real = run(git(&top).args(["rev-parse", "--path-format=absolute", "--git-path", "index"]))
        .map(|s| PathBuf::from(s.trim()))
        .ok();
    let seeded = real.as_ref().is_some_and(|p| std::fs::copy(p, &tmp.0).is_ok());
    let with_index = |args: &[&str]| run(git(&top).env("GIT_INDEX_FILE", &tmp.0).args(args));
    if !seeded {
        let _ = with_index(&["read-tree", "HEAD"]);
    }
    if with_index(&["add", "-A"]).is_err() {
        // A seeded index we can't use (e.g. split index): start from HEAD.
        let _ = std::fs::remove_file(&tmp.0);
        let _ = with_index(&["read-tree", "HEAD"]);
        with_index(&["add", "-A"])?;
    }
    Ok(with_index(&["write-tree"])?.trim().to_string())
}

pub fn task_ref(agent: &str, task: &str) -> String {
    format!("refs/pitwall/{agent}/{task}")
}

/// Point the task's ref at a tree holding `start` / `end`, keeping both alive.
pub fn keep(cwd: &str, agent: &str, task: &str, start: Option<&str>, end: Option<&str>) -> Result<(), String> {
    let mut listing = String::new();
    for (name, tree) in [("end", end), ("start", start)] {
        if let Some(t) = tree {
            listing.push_str(&format!("040000 tree {t}\t{name}\n"));
        }
    }
    if listing.is_empty() {
        return Ok(());
    }
    let holder = run_with_stdin(git(cwd).arg("mktree"), &listing)?;
    run(git(cwd).args(["update-ref", &task_ref(agent, task), holder.trim()])).map(|_| ())
}

fn run_with_stdin(cmd: &mut Command, input: &str) -> Result<String, String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("git: no stdin")?
        .write_all(input.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Delete one task's ref (or every ref of the agent when `task` is `None`).
pub fn drop_refs(cwd: &str, agent: &str, task: Option<&str>) -> Result<(), String> {
    let prefix = match task {
        Some(t) => task_ref(agent, t),
        None => format!("refs/pitwall/{agent}/"),
    };
    let refs = run(git(cwd).args(["for-each-ref", "--format=%(refname)", &prefix]))?;
    for r in refs.lines().filter(|r| !r.is_empty()) {
        run(git(cwd).args(["update-ref", "-d", r]))?;
    }
    Ok(())
}

// ------------------------------------------------------------------ bookkeeping

fn open_task(rec: &mut AgentRecord) -> Option<&mut Task> {
    rec.tasks.last_mut().filter(|t| t.ended_at.is_none())
}

pub fn current_task_id(rec: &AgentRecord) -> Option<String> {
    rec.tasks.last().filter(|t| t.ended_at.is_none()).map(|t| t.id.clone())
}

/// A prompt went to the agent. `explicit` = sent by Pitwall or reported by a
/// UserPromptSubmit hook; a plain Enter typed in the terminal only starts a
/// task when none is open (otherwise it's answering a question).
pub fn begin(a: &mut Agent, prompt: Option<&str>, explicit: bool) -> Vec<Job> {
    begin_at(&mut a.rec, prompt, explicit, unix_ms())
}

fn begin_at(rec: &mut AgentRecord, prompt: Option<&str>, explicit: bool, now: u64) -> Vec<Job> {
    let mut jobs = Vec::new();
    let (agent, cwd) = (rec.id.clone(), rec.cwd.clone());
    if let Some(open) = open_task(rec) {
        if !explicit {
            return jobs;
        }
        if now.saturating_sub(open.started_at) < SAME_PROMPT_MS {
            // Same prompt reported twice (send + hook): just learn its text.
            if open.prompt.is_empty() {
                if let Some(p) = prompt {
                    open.prompt = p.to_string();
                }
            }
            return jobs;
        }
        open.ended_at = Some(now);
        jobs.push(Job::End { agent: agent.clone(), task: open.id.clone(), cwd: cwd.clone() });
    }
    let id = uuid::Uuid::new_v4().to_string();
    rec.tasks.push(Task {
        id: id.clone(),
        prompt: prompt.unwrap_or("").to_string(),
        started_at: now,
        ended_at: None,
        start_tree: None,
        end_tree: None,
    });
    jobs.push(Job::Start { agent: agent.clone(), task: id, cwd: cwd.clone() });
    while rec.tasks.len() > MAX_TASKS {
        let old = rec.tasks.remove(0);
        jobs.push(Job::Drop { agent: agent.clone(), task: old.id, cwd: cwd.clone() });
    }
    jobs
}

/// The agent finished (done/idle/exited): close the open task, if any.
pub fn end(a: &mut Agent) -> Vec<Job> {
    let (agent, cwd) = (a.rec.id.clone(), a.rec.cwd.clone());
    match open_task(&mut a.rec) {
        Some(t) => {
            t.ended_at = Some(unix_ms());
            vec![Job::End { agent, task: t.id.clone(), cwd }]
        }
        None => vec![],
    }
}

/// Status transition seen by the ticker: a turn that ends closes the task.
pub fn on_status(a: &mut Agent, prev: Status, now: Status) -> Vec<Job> {
    let finished = matches!(now, Status::Done | Status::Idle | Status::Exited);
    let was_busy = !matches!(prev, Status::Done | Status::Idle);
    if prev != now && finished && (was_busy || now == Status::Exited) {
        end(a)
    } else {
        vec![]
    }
}

/// `begin` for callers that don't hold the lock.
pub fn begin_for(core: &Shared, id: &str, prompt: Option<&str>, explicit: bool) {
    if let Ok(jobs) = core.with(id, |a| begin(a, prompt, explicit)) {
        if !jobs.is_empty() {
            core.changed(true);
        }
        submit(jobs);
    }
}

/// Hook payloads: UserPromptSubmit starts a task (with its prompt text).
pub fn on_hook(core: &Shared, id: &str, payload: &serde_json::Value) {
    if payload.get("hook_event_name").and_then(|v| v.as_str()) == Some("UserPromptSubmit") {
        let prompt = payload.get("prompt").and_then(|v| v.as_str());
        begin_for(core, id, prompt, true);
    }
}

// ------------------------------------------------------------------ worker

static WORKER: OnceLock<Sender<Job>> = OnceLock::new();

pub fn submit(jobs: Vec<Job>) {
    if let Some(tx) = WORKER.get() {
        for j in jobs {
            let _ = tx.send(j);
        }
    }
}

pub fn start_worker(core: Shared) {
    let (tx, rx) = channel::<Job>();
    if WORKER.set(tx).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("snapshots".into())
        .spawn(move || {
            for job in rx {
                process(&core, job);
            }
        })
        .expect("spawn snapshot worker");
}

fn process(core: &Shared, job: Job) {
    match job {
        Job::Start { agent, task, cwd } => match snapshot(&cwd) {
            Ok(tree) => {
                let _ = core.with(&agent, |a| {
                    if let Some(t) = a.rec.tasks.iter_mut().find(|t| t.id == task) {
                        t.start_tree = Some(tree.clone());
                    }
                });
                let _ = keep(&cwd, &agent, &task, Some(&tree), None);
                core.changed(true);
            }
            Err(_) => {
                // Not a git repository (or git failed): no per-task diffs.
                let _ = core.with(&agent, |a| a.rec.tasks.retain(|t| t.id != task));
                core.changed(true);
            }
        },
        Job::End { agent, task, cwd } => {
            let Ok(tree) = snapshot(&cwd) else { return };
            let start = core
                .with(&agent, |a| {
                    let t = a.rec.tasks.iter_mut().find(|t| t.id == task)?;
                    t.end_tree = Some(tree.clone());
                    t.start_tree.clone()
                })
                .ok()
                .flatten();
            if start.is_some() {
                let _ = keep(&cwd, &agent, &task, start.as_deref(), Some(&tree));
            }
            core.changed(true);
        }
        Job::Drop { agent, task, cwd } => {
            let _ = drop_refs(&cwd, &agent, Some(&task));
        }
    }
}

/// Agent removed: delete all its snapshot refs (best effort; call before its
/// worktree goes away).
pub fn forget(rec: &AgentRecord) {
    let dirs = [Some(rec.cwd.as_str()), rec.worktree.as_ref().map(|w| w.repo.as_str()), Some(rec.project.as_str())];
    for dir in dirs.into_iter().flatten() {
        if Path::new(dir).is_dir() && drop_refs(dir, &rec.id, None).is_ok() {
            return;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A throwaway repo under the system temp dir, deleted on drop.
    pub struct TempRepo(pub PathBuf);

    impl TempRepo {
        pub fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("pitwall-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let r = TempRepo(dir);
            r.git(&["init", "-q", "-b", "main"]);
            r.git(&["config", "user.name", "Test"]);
            r.git(&["config", "user.email", "test@example.com"]);
            r.git(&["config", "commit.gpgsign", "false"]);
            r
        }
        pub fn path(&self) -> &str {
            self.0.to_str().unwrap()
        }
        pub fn git(&self, args: &[&str]) -> String {
            run(git(self.path()).args(args)).unwrap_or_else(|e| panic!("git {args:?}: {e}"))
        }
        pub fn write(&self, rel: &str, content: &str) {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        pub fn commit_all(&self, msg: &str) {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "-m", msg]);
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn rec(cwd: &str) -> AgentRecord {
        serde_json::from_value(serde_json::json!({
            "id": "agent1", "name": "a", "kind": "shell", "kindName": "Shell",
            "cwd": cwd, "project": cwd, "createdAt": 1
        }))
        .unwrap()
    }

    #[test]
    fn snapshot_includes_untracked_and_leaves_index_alone() {
        let r = TempRepo::new();
        r.write("a.txt", "one\n");
        r.write(".gitignore", "ignored/\n");
        r.commit_all("init");
        r.write("a.txt", "two\n");
        r.write("new.txt", "fresh\n");
        r.write("ignored/x.txt", "nope\n");
        let index = std::fs::read(r.0.join(".git/index")).unwrap();
        let status_before = r.git(&["status", "--porcelain"]);

        let tree = snapshot(r.path()).unwrap();
        let files = r.git(&["ls-tree", "-r", "--name-only", &tree]);
        assert!(files.contains("a.txt") && files.contains("new.txt"));
        assert!(!files.contains("ignored/x.txt"));
        assert_eq!(r.git(&["cat-file", "blob", &format!("{tree}:a.txt")]), "two\n");
        // Real index and status untouched.
        assert_eq!(std::fs::read(r.0.join(".git/index")).unwrap(), index);
        assert_eq!(r.git(&["status", "--porcelain"]), status_before);
        assert!(status_before.contains("?? new.txt"));
    }

    #[test]
    fn snapshot_works_in_empty_repo_and_fails_outside_git() {
        let r = TempRepo::new();
        r.write("f", "x");
        assert!(snapshot(r.path()).is_ok());
        let not_git = std::env::temp_dir().join(format!("pitwall-nogit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&not_git).unwrap();
        assert!(snapshot(not_git.to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&not_git);
    }

    #[test]
    fn refs_keep_trees_and_are_dropped() {
        let r = TempRepo::new();
        r.write("a", "1");
        r.commit_all("init");
        let t1 = snapshot(r.path()).unwrap();
        r.write("a", "2");
        let t2 = snapshot(r.path()).unwrap();
        keep(r.path(), "ag", "t1", Some(&t1), Some(&t2)).unwrap();
        let held = r.git(&["rev-parse", &format!("{}:start", task_ref("ag", "t1"))]);
        assert_eq!(held.trim(), t1);
        let held = r.git(&["rev-parse", &format!("{}:end", task_ref("ag", "t1"))]);
        assert_eq!(held.trim(), t2);
        keep(r.path(), "ag", "t2", Some(&t1), None).unwrap();
        keep(r.path(), "other", "t9", Some(&t1), None).unwrap();
        drop_refs(r.path(), "ag", Some("t1")).unwrap();
        assert!(!r.git(&["for-each-ref", "refs/pitwall/ag/"]).contains("/t1"));
        drop_refs(r.path(), "ag", None).unwrap();
        assert!(r.git(&["for-each-ref", "refs/pitwall/ag/"]).trim().is_empty());
        assert!(!r.git(&["for-each-ref", "refs/pitwall/other/"]).trim().is_empty());
    }

    #[test]
    fn task_boundaries() {
        let mut r = rec("/tmp");
        // Typed Enter with nothing open starts a task; the hook fills in the text.
        let jobs = begin_at(&mut r, None, false, 1000);
        assert_eq!(jobs.len(), 1);
        assert_eq!(begin_at(&mut r, Some("fix it"), true, 2000), vec![]);
        assert_eq!(r.tasks[0].prompt, "fix it");
        // Enter while a task is open is an answer, not a new task.
        assert_eq!(begin_at(&mut r, None, false, 9000), vec![]);
        assert_eq!(current_task_id(&r), Some(r.tasks[0].id.clone()));
        // A later explicit prompt closes the open task and opens another.
        let jobs = begin_at(&mut r, Some("next"), true, 20_000);
        assert!(matches!(jobs[0], Job::End { .. }) && matches!(jobs[1], Job::Start { .. }));
        assert_eq!(r.tasks.len(), 2);
        assert_eq!(r.tasks[0].ended_at, Some(20_000));
        assert_eq!(r.tasks[1].prompt, "next");
        // Prompt text is kept verbatim.
        let weird = "  spaced\n\ttext  ";
        begin_at(&mut r, Some(weird), true, 40_000);
        assert_eq!(r.tasks[2].prompt, weird);
    }

    #[test]
    fn old_tasks_are_pruned() {
        let mut r = rec("/tmp");
        let mut drops = 0;
        for i in 0..(MAX_TASKS as u64 + 3) {
            let jobs = begin_at(&mut r, Some("p"), true, i * 10_000);
            drops += jobs.iter().filter(|j| matches!(j, Job::Drop { .. })).count();
        }
        assert_eq!(r.tasks.len(), MAX_TASKS);
        assert_eq!(drops, 3);
    }

    #[test]
    fn status_transitions_end_tasks() {
        let mut a = Agent::new(rec("/tmp"), None);
        begin(&mut a, Some("go"), true);
        assert!(on_status(&mut a, Status::Idle, Status::Idle).is_empty());
        assert!(on_status(&mut a, Status::Working, Status::Blocked).is_empty());
        // idle → done without work in between is not an end.
        assert!(on_status(&mut a, Status::Idle, Status::Done).is_empty());
        assert_eq!(on_status(&mut a, Status::Working, Status::Done).len(), 1);
        assert!(current_task_id(&a.rec).is_none());
        assert!(on_status(&mut a, Status::Working, Status::Done).is_empty());
    }
}
