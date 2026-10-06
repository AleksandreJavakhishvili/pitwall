//! Review screen services (docs/spec/review.md): look up the agent, run the
//! git operation (`vcs::review`), and refresh the agent's numbers after a
//! change. Everything that runs git is blocking.

use std::sync::Arc;

use crate::engine::{Engine, Task};
use crate::exec::Exec;
use crate::model::WorktreeInfo;
use crate::vcs::git::{FileChange, Git};
use crate::vcs::review::{self as ops, toplevel, worktree_branch};
use crate::vcs::snapshot;

pub use ops::{FileVersions, MergeResult, MergeStatus};

type Res<T> = Result<T, String>;

/// What a review call needs to know about an agent (read under the lock).
struct Ctx {
    exec: Arc<dyn Exec>,
    cwd: String,
    base: Option<String>,
    task: Option<Task>,
    worktree: Option<WorktreeInfo>,
}

fn ctx(engine: &Engine, agent_id: &str, task_id: Option<&str>) -> Res<Ctx> {
    // Looked up before taking the registry lock (it takes it too).
    let exec = engine.exec_for(agent_id);
    engine.with(agent_id, |a| {
        let task = match task_id {
            Some(id) => Some(a.rec.tasks.iter().find(|t| t.id == id).cloned().ok_or("task not found")?),
            None => None,
        };
        Ok(Ctx { exec, cwd: a.rec.cwd.clone(), base: a.rec.base_commit.clone(), task, worktree: a.rec.worktree.clone() })
    })?
}

impl Ctx {
    /// Git in the agent's folder, on its machine.
    fn git(&self) -> Git<'_> {
        Git::new(&*self.exec, &self.cwd)
    }
}

/// Ask the ticker for fresh git numbers for this agent.
fn refresh(engine: &Engine, agent_id: &str) {
    let _ = engine.with(agent_id, |a| a.git_wanted = true);
    engine.changed(false);
}

pub fn list_tasks(engine: &Engine, agent_id: &str) -> Res<Vec<Task>> {
    engine.with(agent_id, |a| a.rec.tasks.clone())
}

/// `task_id` `None` = all changes since the agent started (repo-root paths).
pub fn task_changes(engine: &Engine, agent_id: &str, task_id: Option<&str>) -> Res<Vec<FileChange>> {
    let c = ctx(engine, agent_id, task_id)?;
    let top = toplevel(&c.git())?;
    match c.task.clone() {
        None => top.changes(c.base.as_deref()),
        Some(t) => {
            let start = t.start_tree.ok_or("this task's snapshot isn't ready yet")?;
            let end = match t.end_tree {
                Some(e) => e,
                None => snapshot::snapshot(&top)?,
            };
            ops::tree_changes(&top, &start, &end)
        }
    }
}

pub fn file_versions(engine: &Engine, agent_id: &str, path: &str, task_id: Option<&str>) -> Res<FileVersions> {
    let c = ctx(engine, agent_id, task_id)?;
    ops::file_versions(&toplevel(&c.git())?, path, c.base.as_deref(), c.task.as_ref())
}

pub fn discard_file(engine: &Engine, agent_id: &str, path: &str) -> Res<()> {
    let c = ctx(engine, agent_id, None)?;
    ops::discard(&toplevel(&c.git())?, c.base.as_deref(), path)?;
    refresh(engine, agent_id);
    Ok(())
}

pub fn commit(engine: &Engine, agent_id: &str, message: &str) -> Res<String> {
    let c = ctx(engine, agent_id, None)?;
    let id = ops::commit(&c.git(), message)?;
    refresh(engine, agent_id);
    Ok(id)
}

pub fn merge(engine: &Engine, agent_id: &str) -> Res<MergeResult> {
    let c = ctx(engine, agent_id, None)?;
    let wt = c.worktree.clone().ok_or("This agent works in the main checkout: there is no branch to merge. Commit instead.")?;
    let branch =
        worktree_branch(&*c.exec, &wt).ok_or("The agent's worktree isn't on a branch (detached HEAD). Create a branch there, then merge.")?;
    let result = ops::merge(&c.git().at(&wt.repo), &branch)?;
    if let Some(tip) = result.merged.then(|| c.git().head()).flatten() {
        // What was merged is no longer "the agent's changes".
        let _ = engine.with(agent_id, |a| a.rec.base_commit = Some(tip));
        engine.changed(true);
    }
    refresh(engine, agent_id);
    Ok(result)
}

pub fn merge_status(engine: &Engine, agent_id: &str) -> Res<MergeStatus> {
    let c = ctx(engine, agent_id, None)?;
    let branch = c.worktree.as_ref().and_then(|w| worktree_branch(&*c.exec, w));
    let wt = c.worktree.as_ref().map(|w| (w.repo.as_str(), branch.as_deref()));
    ops::merge_status(&c.git(), wt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{record, Harness};
    use crate::vcs::snapshot::TempRepo;

    #[test]
    fn review_of_an_agent_in_a_repo() {
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let mut rec = record("a", r.path());
        rec.base_commit = Some(r.git(&["rev-parse", "HEAD"]).trim().to_string());
        let h = Harness::new(vec![rec]);
        r.write("a.txt", "2\n");

        let files = task_changes(&h.engine, "a", None).unwrap();
        assert_eq!(files.len(), 1);
        assert!(task_changes(&h.engine, "a", Some("no-such-task")).is_err());
        assert!(list_tasks(&h.engine, "a").unwrap().is_empty());
        let v = file_versions(&h.engine, "a", "a.txt", None).unwrap();
        assert_eq!(v.modified.as_deref(), Some("2\n"));

        commit(&h.engine, "a", "agent work").unwrap();
        assert!(task_changes(&h.engine, "a", None).unwrap().len() == 1, "base stays where the agent started");
        assert!(merge(&h.engine, "a").unwrap_err().contains("main checkout"));
        let st = merge_status(&h.engine, "a").unwrap();
        assert!(!st.worktree && st.uncommitted == 0);
        assert!(task_changes(&h.engine, "ghost", None).is_err());
    }
}
