//! The read-only code explorer (docs/spec/explorer.md). Paths are relative
//! to the agent's folder; the core refuses anything that leads outside it.

use tauri::State;

use pitwall_core::explorer::{
    self, DirListing, FileIndex, FileView, SearchQuery, SearchResult,
};
use pitwall_core::Shared;

use super::{blocking, Res};

/// One folder's children (`dir` omitted: the agent's folder); `ignored`:
/// what git ignores too ("Show ignored files").
#[tauri::command]
pub async fn list_files(
    core: State<'_, Shared>,
    agent_id: String,
    dir: Option<String>,
    ignored: Option<bool>,
) -> Res<DirListing> {
    let core = core.inner().clone();
    blocking(move || {
        explorer::list_files(
            &core,
            &agent_id,
            dir.as_deref().unwrap_or(""),
            ignored.unwrap_or(false),
        )
    })
    .await
}

/// Every file, for quick open.
#[tauri::command]
pub async fn list_all_files(core: State<'_, Shared>, agent_id: String) -> Res<FileIndex> {
    let core = core.inner().clone();
    blocking(move || explorer::list_all_files(&core, &agent_id)).await
}

/// Text up to 2 MiB; `large` ("Load anyway"): up to 10 MiB.
#[tauri::command]
pub async fn read_file(
    core: State<'_, Shared>,
    agent_id: String,
    path: String,
    large: Option<bool>,
) -> Res<FileView> {
    let core = core.inner().clone();
    blocking(move || explorer::read_file(&core, &agent_id, &path, large.unwrap_or(false))).await
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
