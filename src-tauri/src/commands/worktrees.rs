//! Worktrees in source control (docs/spec/worktrees-view.md). A worktree is
//! named by its project's `id` (from `list_worktrees`) and its path; the core
//! refuses any path it didn't list.

use tauri::State;

use pitwall_core::review::{FileVersions, MergeResult, MergeStatus};
use pitwall_core::vcs::git::FileChange;
use pitwall_core::worktrees;
use pitwall_core::Shared;
use pitwall_proto::ProjectWorktrees;

use super::{blocking, Res};

/// Every project's worktrees; projects are listed again only when due.
#[tauri::command]
pub async fn list_worktrees(core: State<'_, Shared>) -> Res<Vec<ProjectWorktrees>> {
    let core = core.inner().clone();
    blocking(move || Ok(worktrees::list(&core))).await
}

#[tauri::command]
pub async fn get_worktree_changes(core: State<'_, Shared>, project_id: String, path: String) -> Res<Vec<FileChange>> {
    let core = core.inner().clone();
    blocking(move || worktrees::changes(&core, &project_id, &path)).await
}

#[tauri::command]
pub async fn get_worktree_file_versions(core: State<'_, Shared>, project_id: String, path: String, file: String) -> Res<FileVersions> {
    let core = core.inner().clone();
    blocking(move || worktrees::file_versions(&core, &project_id, &path, &file)).await
}

#[tauri::command]
pub async fn get_worktree_merge_status(core: State<'_, Shared>, project_id: String, path: String) -> Res<MergeStatus> {
    let core = core.inner().clone();
    blocking(move || worktrees::merge_status(&core, &project_id, &path)).await
}

#[tauri::command]
pub async fn commit_worktree(core: State<'_, Shared>, project_id: String, path: String, message: String) -> Res<String> {
    let core = core.inner().clone();
    blocking(move || worktrees::commit(&core, &project_id, &path, &message)).await
}

#[tauri::command]
pub async fn merge_worktree(core: State<'_, Shared>, project_id: String, path: String) -> Res<MergeResult> {
    let core = core.inner().clone();
    blocking(move || worktrees::merge(&core, &project_id, &path)).await
}

#[tauri::command]
pub async fn remove_worktree(core: State<'_, Shared>, project_id: String, path: String) -> Res<()> {
    let core = core.inner().clone();
    blocking(move || worktrees::remove(&core, &project_id, &path)).await
}
