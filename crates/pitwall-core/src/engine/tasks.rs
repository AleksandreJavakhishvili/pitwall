//! Per-task snapshots (docs/spec/review.md). A task is one prompt delivered to
//! an agent, until the agent is done/idle. At task start and end the working
//! tree is snapshotted without side effects on the user's checkout
//! (`vcs::snapshot`).
//!
//! Bookkeeping happens under the registry lock (in-memory only); the git work
//! runs on one background worker, in order, so a task's start snapshot is always
//! taken before its end snapshot.

use std::sync::mpsc::{channel, Sender};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::{Agent, Shared};
use crate::clock::unix_ms;
use crate::exec::{self, Exec};
use crate::model::{AgentRecord, Status};
use crate::vcs::git::Git;
use crate::vcs::snapshot::{drop_refs, keep, snapshot};

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
        core.submit(jobs);
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

/// The engine's one snapshot worker: jobs run in order, so a task's start
/// snapshot is always taken before its end snapshot. Jobs submitted before
/// [`start`](Self::start) are dropped.
#[derive(Default)]
pub(crate) struct Worker {
    tx: OnceLock<Sender<Job>>,
}

impl Worker {
    pub fn submit(&self, jobs: Vec<Job>) {
        if let Some(tx) = self.tx.get() {
            for j in jobs {
                let _ = tx.send(j);
            }
        }
    }

    pub fn start(&self, core: Shared) {
        let (tx, rx) = channel::<Job>();
        if self.tx.set(tx).is_err() {
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
}

fn process(core: &Shared, job: Job) {
    let (Job::Start { agent, .. } | Job::End { agent, .. } | Job::Drop { agent, .. }) = &job;
    let exec = core.exec_for(agent);
    match job {
        Job::Start { agent, task, cwd } => match snapshot(&Git::new(&*exec, &cwd)) {
            Ok(tree) => {
                let _ = core.with(&agent, |a| {
                    if let Some(t) = a.rec.tasks.iter_mut().find(|t| t.id == task) {
                        t.start_tree = Some(tree.clone());
                    }
                });
                let _ = keep(&Git::new(&*exec, &cwd), &agent, &task, Some(&tree), None);
                core.changed(true);
            }
            Err(_) => {
                // Not a git repository (or git failed): no per-task diffs.
                let _ = core.with(&agent, |a| a.rec.tasks.retain(|t| t.id != task));
                core.changed(true);
            }
        },
        Job::End { agent, task, cwd } => {
            let git = Git::new(&*exec, &cwd);
            let Ok(tree) = snapshot(&git) else { return };
            let start = core
                .with(&agent, |a| {
                    let t = a.rec.tasks.iter_mut().find(|t| t.id == task)?;
                    t.end_tree = Some(tree.clone());
                    t.start_tree.clone()
                })
                .ok()
                .flatten();
            if start.is_some() {
                let _ = keep(&git, &agent, &task, start.as_deref(), Some(&tree));
            }
            core.changed(true);
        }
        Job::Drop { agent, task, cwd } => {
            let _ = drop_refs(&Git::new(&*exec, &cwd), &agent, Some(&task));
        }
    }
}

/// Agent removed: delete all its snapshot refs (best effort; call before its
/// worktree goes away). `exec`: the agent's machine.
pub fn forget(exec: &dyn Exec, rec: &AgentRecord) {
    let dirs = [Some(rec.cwd.as_str()), rec.worktree.as_ref().map(|w| w.repo.as_str()), Some(rec.project.as_str())];
    for dir in dirs.into_iter().flatten() {
        if exec::is_dir(exec, dir) && drop_refs(&Git::new(exec, dir), &rec.id, None).is_ok() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(cwd: &str) -> AgentRecord {
        crate::testing::record("agent1", cwd)
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
        let mut a = Agent::new(rec("/tmp"), None, 1);
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
