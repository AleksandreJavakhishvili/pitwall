//! Review screen (docs/spec/review.md).

use tauri::State;

use pitwall_core::engine::Task;
use pitwall_core::review::{self, FileVersions, MergeResult, MergeStatus};
use pitwall_core::vcs::git::FileChange;
use pitwall_core::Shared;

use super::{blocking, Res};

#[tauri::command]
pub fn list_tasks(core: State<'_, Shared>, agent_id: String) -> Res<Vec<Task>> {
    review::list_tasks(&core, &agent_id)
}

/// `taskId` null = all changes since the agent started (repo-root paths).
#[tauri::command]
pub async fn get_task_changes(core: State<'_, Shared>, agent_id: String, task_id: Option<String>) -> Res<Vec<FileChange>> {
    let core = core.inner().clone();
    blocking(move || review::task_changes(&core, &agent_id, task_id.as_deref())).await
}

#[tauri::command]
pub async fn get_file_versions(
    core: State<'_, Shared>,
    agent_id: String,
    path: String,
    task_id: Option<String>,
) -> Res<FileVersions> {
    let core = core.inner().clone();
    blocking(move || review::file_versions(&core, &agent_id, &path, task_id.as_deref())).await
}

#[tauri::command]
pub async fn discard_file(core: State<'_, Shared>, agent_id: String, path: String) -> Res<()> {
    let core = core.inner().clone();
    blocking(move || review::discard_file(&core, &agent_id, &path)).await
}

#[tauri::command]
pub async fn commit_agent(core: State<'_, Shared>, agent_id: String, message: String) -> Res<String> {
    let core = core.inner().clone();
    blocking(move || review::commit(&core, &agent_id, &message)).await
}

#[tauri::command]
pub async fn merge_agent(core: State<'_, Shared>, agent_id: String) -> Res<MergeResult> {
    let core = core.inner().clone();
    blocking(move || review::merge(&core, &agent_id)).await
}

#[tauri::command]
pub async fn get_merge_status(core: State<'_, Shared>, agent_id: String) -> Res<MergeStatus> {
    let core = core.inner().clone();
    blocking(move || review::merge_status(&core, &agent_id)).await
}
