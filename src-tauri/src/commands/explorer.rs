//! The read-only code explorer (docs/spec/explorer.md). Paths are relative
//! to the agent's folder; the core refuses anything that leads outside it.

use tauri::State;

use pitwall_core::explorer::{
    self, DirListing, EditorSettings, FileIndex, FileView, SearchQuery, SearchResult,
};
use pitwall_core::Shared;

use super::{blocking, Res};

/// One folder's children (`dir` omitted: the agent's folder).
#[tauri::command]
pub async fn list_files(
    core: State<'_, Shared>,
    agent_id: String,
    dir: Option<String>,
) -> Res<DirListing> {
    let core = core.inner().clone();
    blocking(move || explorer::list_files(&core, &agent_id, dir.as_deref().unwrap_or(""))).await
}

/// Every file, for quick open.
#[tauri::command]
pub async fn list_all_files(core: State<'_, Shared>, agent_id: String) -> Res<FileIndex> {
    let core = core.inner().clone();
    blocking(move || explorer::list_all_files(&core, &agent_id)).await
}

#[tauri::command]
pub async fn read_file(core: State<'_, Shared>, agent_id: String, path: String) -> Res<FileView> {
    let core = core.inner().clone();
    blocking(move || explorer::read_file(&core, &agent_id, &path)).await
}

/// A new search for the same agent cancels the one still running (it then
/// fails with "cancelled").
#[tauri::command]
pub async fn search_files(
    core: State<'_, Shared>,
    agent_id: String,
    query: SearchQuery,
) -> Res<SearchResult> {
    let core = core.inner().clone();
    blocking(move || explorer::search(&core, &agent_id, &query)).await
}

#[tauri::command]
pub fn cancel_search(core: State<'_, Shared>, agent_id: String) {
    explorer::cancel_search(&core, &agent_id);
}

/// Only when `caps.openInEditor`; `line`/`column` are 1-based.
#[tauri::command]
pub async fn open_in_editor(
    core: State<'_, Shared>,
    agent_id: String,
    path: String,
    line: Option<u32>,
    column: Option<u32>,
) -> Res<()> {
    let core = core.inner().clone();
    blocking(move || explorer::open_in_editor(&core, &agent_id, &path, line, column)).await
}

#[tauri::command]
pub async fn get_editor(core: State<'_, Shared>) -> Res<EditorSettings> {
    let core = core.inner().clone();
    blocking(move || Ok(explorer::editor_settings(&core))).await
}

/// `command` null: back to the first editor found.
#[tauri::command]
pub async fn set_editor(core: State<'_, Shared>, command: Option<String>) -> Res<EditorSettings> {
    let core = core.inner().clone();
    blocking(move || explorer::set_editor(&core, command.as_deref())).await
}
