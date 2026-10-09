//! Review's calls into the engine (Tauri: `src-tauri/src/commands/review.rs`,
//! `worktrees.rs`, `agents.rs`). Everything here runs git or talks to a
//! holder, so it runs on the background executor; results come back to the
//! view on the main thread.

use std::path::PathBuf;

use gpui::{App, Context, Global};

use pitwall_core::engine::{changes, input, Task};
use pitwall_core::review::{self, FileVersions, MergeResult, MergeStatus};
use pitwall_core::vcs::git::FileChange;
use pitwall_core::{worktrees, Shared};
use pitwall_proto::ProjectWorktrees;

pub type Res<T> = Result<T, String>;

/// The hosted engine and the data folder, for Review (set once by `run`).
#[derive(Clone)]
pub struct ReviewEngine {
    pub engine: Shared,
    /// Where Review keeps its one preference (`review.json`).
    pub root: Option<PathBuf>,
}

impl Global for ReviewEngine {}

pub fn engine(cx: &App) -> Option<Shared> {
    cx.try_global::<ReviewEngine>().map(|e| e.engine.clone())
}

/// Run `work` with the engine off the main thread, then `done` on the view.
pub fn spawn<V: 'static, T: Send + 'static>(
    cx: &mut Context<V>,
    work: impl FnOnce(&Shared) -> T + Send + 'static,
    done: impl FnOnce(&mut V, T, &mut Context<V>) + 'static,
) {
    let Some(engine) = engine(cx) else { return };
    let task = cx.background_executor().spawn(async move { work(&engine) });
    cx.spawn(async move |this, cx| {
        let out = task.await;
        let _ = this.update(cx, |v, cx| done(v, out, cx));
    })
    .detach();
}

// The engine calls, one per Tauri command.

pub fn task_changes(e: &Shared, agent: &str, task: Option<&str>) -> Res<Vec<FileChange>> {
    review::task_changes(e, agent, task)
}

pub fn refresh_changes(e: &Shared, agent: &str) -> Res<Vec<FileChange>> {
    changes::refresh(e, agent)
}

pub fn list_tasks(e: &Shared, agent: &str) -> Res<Vec<Task>> {
    review::list_tasks(e, agent)
}

pub fn file_versions(e: &Shared, agent: &str, path: &str, task: Option<&str>) -> Res<FileVersions> {
    review::file_versions(e, agent, path, task)
}

pub fn discard_file(e: &Shared, agent: &str, path: &str) -> Res<()> {
    review::discard_file(e, agent, path)
}

pub fn send_prompt(e: &Shared, agent: &str, text: String) -> Res<()> {
    input::send_prompt(e, agent, text)
}

/// Queue into Next up; the agent's updated view (its queue) comes back.
pub fn queue_add(e: &Shared, agent: &str, text: String) -> Res<pitwall_proto::AgentView> {
    e.queue_add(agent, text)
}

pub fn list_worktrees(e: &Shared) -> Vec<ProjectWorktrees> {
    worktrees::list(e)
}

pub fn refresh_worktrees(e: &Shared, project: Option<&str>) -> Vec<ProjectWorktrees> {
    worktrees::refresh_now(e, project)
}

pub fn worktree_changes(e: &Shared, project: &str, path: &str) -> Res<Vec<FileChange>> {
    worktrees::changes(e, project, path)
}

pub fn worktree_file_versions(
    e: &Shared,
    project: &str,
    path: &str,
    file: &str,
) -> Res<FileVersions> {
    worktrees::file_versions(e, project, path, file)
}

pub fn remove_worktree(e: &Shared, project: &str, path: &str) -> Res<()> {
    worktrees::remove(e, project, path)
}

/// What Commit / Commit & merge acts on: an agent's folder, or one worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitTarget {
    Agent {
        id: String,
        /// Shown in the title.
        name: String,
        /// Where the commit runs.
        where_: String,
        project_display: String,
    },
    Worktree {
        project: String,
        path: String,
        name: String,
        where_: String,
        project_display: String,
    },
}

impl CommitTarget {
    pub fn title_name(&self) -> String {
        match self {
            CommitTarget::Agent { name, .. } => name.clone(),
            CommitTarget::Worktree { name, .. } => format!("worktree {name}"),
        }
    }

    pub fn where_(&self) -> &str {
        match self {
            CommitTarget::Agent { where_, .. } | CommitTarget::Worktree { where_, .. } => where_,
        }
    }

    pub fn project_display(&self) -> &str {
        match self {
            CommitTarget::Agent {
                project_display, ..
            }
            | CommitTarget::Worktree {
                project_display, ..
            } => project_display,
        }
    }

    pub fn status(&self, e: &Shared) -> Res<MergeStatus> {
        match self {
            CommitTarget::Agent { id, .. } => review::merge_status(e, id),
            CommitTarget::Worktree { project, path, .. } => {
                worktrees::merge_status(e, project, path)
            }
        }
    }

    /// Short id of the new commit.
    pub fn commit(&self, e: &Shared, message: &str) -> Res<String> {
        match self {
            CommitTarget::Agent { id, .. } => review::commit(e, id, message),
            CommitTarget::Worktree { project, path, .. } => {
                worktrees::commit(e, project, path, message)
            }
        }
    }

    pub fn merge(&self, e: &Shared) -> Res<MergeResult> {
        match self {
            CommitTarget::Agent { id, .. } => review::merge(e, id),
            CommitTarget::Worktree { project, path, .. } => worktrees::merge(e, project, path),
        }
    }
}

/// What a Commit / Commit & merge run ended with.
#[derive(Debug, Clone, PartialEq)]
pub enum CommitOutcome {
    Done(String),
    Conflict(MergeResult),
    /// Not merged (refused): the message, and the status read again.
    Failed(String, Option<MergeStatus>),
}

/// Commit if `commit` (with `message`), then merge if `merge`: each step
/// as the React dialog does it.
pub fn commit_and_merge(
    e: &Shared,
    t: &CommitTarget,
    st: &MergeStatus,
    commit: bool,
    merge: bool,
    message: &str,
) -> CommitOutcome {
    let mut done = String::new();
    if commit {
        match t.commit(e, message) {
            Ok(id) => {
                done = format!(
                    "Committed {id} on {}.",
                    st.branch.as_deref().unwrap_or("the current branch")
                )
            }
            Err(err) => return CommitOutcome::Failed(err, None),
        }
    }
    if merge {
        match t.merge(e) {
            Ok(r) if r.conflict => return CommitOutcome::Conflict(r),
            Ok(r) if !r.merged => {
                let msg = if done.is_empty() {
                    r.message
                } else {
                    format!("{done} {}", r.message)
                };
                return CommitOutcome::Failed(msg, t.status(e).ok());
            }
            Ok(r) => {
                done = if done.is_empty() {
                    r.message
                } else {
                    format!("{done} {}", r.message)
                };
            }
            Err(err) => {
                let msg = if done.is_empty() {
                    err
                } else {
                    format!("{done} {err}")
                };
                return CommitOutcome::Failed(msg, t.status(e).ok());
            }
        }
    }
    CommitOutcome::Done(done)
}

// ── the one preference: side by side or inline ─────────────────────────────

/// What the old `<data>/review.json` (`{"sideBySide": bool}`, written by
/// earlier builds) says, if anything.
pub fn read_review_json(root: &std::path::Path) -> Option<bool> {
    std::fs::read_to_string(root.join("review.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("sideBySide").and_then(|b| b.as_bool()))
}

/// The preference from `ui.reviewSideBySide`, else the old `review.json`
/// in `root`, else side by side; and what to store in `ui.json` (the old
/// file's choice, moved over).
pub fn side_by_side_from(ui: Option<bool>, root: Option<&std::path::Path>) -> (bool, Option<bool>) {
    if let Some(on) = ui {
        return (on, None);
    }
    let old = root.and_then(read_review_json);
    (old.unwrap_or(true), old)
}

/// `pitwall.review.sideBySide` (default side by side): `ui.reviewSideBySide`.
/// Unset, `review.json` is read once and moved into `ui.json` (the old file
/// is left as it was, for older builds).
pub fn load_side_by_side(cx: &mut App) -> bool {
    let root = cx.try_global::<ReviewEngine>().and_then(|e| e.root.clone());
    let (on, store) = side_by_side_from(crate::ui_state::get(cx).review_side_by_side, root.as_deref());
    if let Some(v) = store {
        crate::ui_state::update(cx, |s| s.review_side_by_side = Some(v));
    }
    on
}

pub fn save_side_by_side(cx: &mut App, on: bool) {
    crate::ui_state::update(cx, |s| s.review_side_by_side = Some(on));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_preference_moves_from_review_json_into_ui_json(cx: &mut gpui::TestAppContext) {
        let dir = std::env::temp_dir().join(format!("pw-rv-sbs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(side_by_side_from(None, Some(&dir)), (true, None), "default");
        std::fs::write(dir.join("review.json"), r#"{"sideBySide":false}"#).unwrap();
        assert_eq!(side_by_side_from(None, Some(&dir)), (false, Some(false)));
        assert_eq!(side_by_side_from(Some(true), Some(&dir)), (true, None), "ui.json wins");
        let root = dir.clone();
        cx.update(|cx| {
            crate::ui_state::init(Some(root), cx);
            save_side_by_side(cx, false);
            assert!(!load_side_by_side(cx));
        });
        cx.executor().advance_clock(std::time::Duration::from_millis(200));
        cx.run_until_parked();
        let ui: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("ui.json")).unwrap()).unwrap();
        assert_eq!(ui["reviewSideBySide"], false);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
