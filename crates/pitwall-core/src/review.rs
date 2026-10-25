//! Review screen services (docs/spec/review.md): look up the agent, run the
//! git operation (`vcs::review`), and refresh the agent's numbers after a
//! change. Everything that runs git is blocking.

use std::sync::Arc;

use crate::engine::{Engine, Task};
use crate::exec::Exec;
use crate::model::WorktreeInfo;
use crate::vcs::git::{FileChange, Git};
use crate::vcs::review::{self as ops, compare_with, toplevel, worktree_branch};
use crate::vcs::snapshot;

pub use ops::{FileVersions, MergeResult, MergeStatus, ReviewBase};

type Res<T> = Result<T, String>;

/// What a review call needs to know about an agent (read under the lock).
struct Ctx {
    exec: Arc<dyn Exec>,
    cwd: String,
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
        Ok(Ctx { exec, cwd: a.rec.cwd.clone(), task, worktree: a.rec.worktree.clone() })
    })?
}

impl Ctx {
    /// Git in the agent's folder, on its machine.
    fn git(&self) -> Git<'_> {
        Git::new(&*self.exec, &self.cwd)
    }

    /// What its work is compared with in `top` (`compare_with`): `chosen`,
    /// else the branch the user chose for this checkout and branch, else
    /// found automatically.
    fn compare(&self, engine: &Engine, top: &Git, chosen: Option<&str>) -> ReviewBase {
        let remembered = || top.current_branch().and_then(|b| engine.projects().review_base(top.dir(), &b));
        let chosen = chosen.map(String::from).or_else(remembered);
        compare_with(top, self.main_checkout(), chosen.as_deref())
    }

    fn base(&self, engine: &Engine, top: &Git) -> String {
        self.compare(engine, top, None).rev
    }

    fn main_checkout(&self) -> Option<&str> {
        self.worktree.as_ref().map(|w| w.repo.as_str())
    }
}

/// What Review compares an agent's work with, and what else it could.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    /// The agent's branch (`None`: detached HEAD; nothing to choose for).
    pub branch: Option<String>,
    /// What it is compared with now.
    pub base: ReviewBase,
    /// The branch "Automatic" compares with.
    pub automatic: Option<String>,
    /// The branches to choose from (local, then origin's).
    pub branches: Vec<String>,
}

pub fn comparison(engine: &Engine, agent_id: &str) -> Res<Comparison> {
    let c = ctx(engine, agent_id, None)?;
    let top = toplevel(&c.git())?;
    Ok(Comparison {
        branch: top.current_branch(),
        base: c.compare(engine, &top, None),
        automatic: compare_with(&top, c.main_checkout(), None).branch,
        branches: ops::branches(&top),
    })
}

/// Compare the agent's branch with `base` from now on (remembered for its
/// checkout and branch); `None`: automatic again.
pub fn set_compare_with(engine: &Engine, agent_id: &str, base: Option<&str>) -> Res<()> {
    let c = ctx(engine, agent_id, None)?;
    let top = toplevel(&c.git())?;
    let branch = top.current_branch().ok_or("The agent isn't on a branch (detached HEAD).")?;
    if let Some(b) = base {
        if !ops::branches(&top).iter().any(|x| x == b) {
            return Err(format!("There is no branch {b}."));
        }
    }
    engine.projects().set_review_base(top.dir(), &branch, base)?;
    refresh(engine, agent_id);
    Ok(())
}

/// Ask the ticker for fresh git numbers for this agent.
fn refresh(engine: &Engine, agent_id: &str) {
    let _ = engine.with(agent_id, |a| a.git_wanted = true);
    engine.changed(false);
}

pub fn list_tasks(engine: &Engine, agent_id: &str) -> Res<Vec<Task>> {
    engine.with(agent_id, |a| a.rec.tasks.clone())
}

/// `task_id` `None` = all of the agent's changes on its branch (repo-root paths).
pub fn task_changes(engine: &Engine, agent_id: &str, task_id: Option<&str>) -> Res<Vec<FileChange>> {
    review_changes(engine, agent_id, task_id, None).map(|(files, _)| files)
}

/// [`task_changes`], and the branch they are compared with (all of them
/// only): `base`, just this once, else as [`comparison`] says.
pub fn review_changes(
    engine: &Engine,
    agent_id: &str,
    task_id: Option<&str>,
    base: Option<&str>,
) -> Res<(Vec<FileChange>, Option<String>)> {
    let c = ctx(engine, agent_id, task_id)?;
    let top = toplevel(&c.git())?;
    if let Some(b) = base.filter(|b| !ops::branches(&top).iter().any(|x| x == b)) {
        return Err(format!("There is no branch {b}."));
    }
    match c.task.clone() {
        None => {
            let b = c.compare(engine, &top, base);
            Ok((top.changes(Some(&b.rev))?, b.branch))
        }
        Some(t) => {
            let start = t.start_tree.ok_or("this task's snapshot isn't ready yet")?;
            let end = match t.end_tree {
                Some(e) => e,
                None => snapshot::snapshot(&top)?,
            };
            Ok((ops::tree_changes(&top, &start, &end)?, None))
        }
    }
}

pub fn file_versions(engine: &Engine, agent_id: &str, path: &str, task_id: Option<&str>) -> Res<FileVersions> {
    let c = ctx(engine, agent_id, task_id)?;
    let top = toplevel(&c.git())?;
    let base = c.task.is_none().then(|| c.base(engine, &top));
    ops::file_versions(&top, path, base.as_deref(), c.task.as_ref())
}

pub fn discard_file(engine: &Engine, agent_id: &str, path: &str) -> Res<()> {
    let c = ctx(engine, agent_id, None)?;
    let top = toplevel(&c.git())?;
    ops::discard(&top, Some(&c.base(engine, &top)), path)?;
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
    // (Once merged, the branch's work is in the target: no longer "the
    // agent's changes" by `review_base`.)
    let result = ops::merge(&c.git().at(&wt.repo), &branch)?;
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
        let h = Harness::new(vec![record("a", r.path())]);
        r.write("a.txt", "2\n");

        let files = task_changes(&h.engine, "a", None).unwrap();
        assert_eq!(files.len(), 1);
        assert!(task_changes(&h.engine, "a", Some("no-such-task")).is_err());
        assert!(list_tasks(&h.engine, "a").unwrap().is_empty());
        let v = file_versions(&h.engine, "a", "a.txt", None).unwrap();
        assert_eq!(v.modified.as_deref(), Some("2\n"));

        commit(&h.engine, "a", "agent work").unwrap();
        assert!(task_changes(&h.engine, "a", None).unwrap().is_empty(), "committed on main: nothing left to review");
        assert!(merge(&h.engine, "a").unwrap_err().contains("main checkout"));
        let st = merge_status(&h.engine, "a").unwrap();
        assert!(!st.worktree && st.uncommitted == 0);
        assert!(task_changes(&h.engine, "ghost", None).is_err());
    }

    #[test]
    fn the_chosen_branch_is_remembered() {
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        r.git(&["checkout", "-q", "-b", "develop"]);
        r.write("d.txt", "d\n");
        r.commit_all("develop work");
        r.git(&["checkout", "-q", "-b", "feat"]);
        r.write("f.txt", "f\n");
        let h = Harness::new(vec![record("a", r.path())]);
        let c = comparison(&h.engine, "a").unwrap();
        assert_eq!((c.branch.as_deref(), c.base.branch.as_deref(), c.base.chosen), (Some("feat"), Some("develop"), false));
        assert_eq!(c.branches, ["develop", "feat", "main"]);
        assert_eq!(task_changes(&h.engine, "a", None).unwrap().len(), 1);

        set_compare_with(&h.engine, "a", Some("main")).unwrap();
        let c = comparison(&h.engine, "a").unwrap();
        assert_eq!((c.base.branch.as_deref(), c.base.chosen, c.automatic.as_deref()), (Some("main"), true, Some("develop")));
        assert_eq!(task_changes(&h.engine, "a", None).unwrap().len(), 2);
        // Just this once.
        let (files, with) = review_changes(&h.engine, "a", None, Some("develop")).unwrap();
        assert_eq!((files.len(), with.as_deref()), (1, Some("develop")));
        assert!(review_changes(&h.engine, "a", None, Some("nope")).is_err());
        assert!(set_compare_with(&h.engine, "a", Some("nope")).is_err());

        set_compare_with(&h.engine, "a", None).unwrap();
        assert!(!comparison(&h.engine, "a").unwrap().base.chosen);
    }
}
