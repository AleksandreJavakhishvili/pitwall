//! First-launch auto-detect and the project list (docs/spec/onboarding.md).
//! Events: `scan-progress`, `projects-changed` (sent by the core's EventSink).

use tauri::State;

use pitwall_core::model::{AdoptSessionRequest, AgentView};
use pitwall_core::onboarding::project_list::Project;
use pitwall_core::onboarding::scan::ScanResult;
use pitwall_core::onboarding::{self, ContinueRequest};
use pitwall_core::Shared;

use super::{blocking, Res};

#[tauri::command]
pub async fn scan_environment(core: State<'_, Shared>) -> Res<ScanResult> {
    let core = core.inner().clone();
    blocking(move || Ok(onboarding::scan_environment(&core))).await
}

#[tauri::command]
pub fn get_onboarded(core: State<'_, Shared>) -> bool {
    core.projects().onboarded()
}

#[tauri::command]
pub fn list_projects(core: State<'_, Shared>) -> Vec<Project> {
    core.projects().list()
}

#[tauri::command]
pub fn add_project(core: State<'_, Shared>, path: String) -> Res<Vec<Project>> {
    onboarding::add_project(&core, path)
}

/// Drops the project from Pitwall's list. Never deletes files.
#[tauri::command]
pub fn remove_project(core: State<'_, Shared>, path: String) -> Res<Vec<Project>> {
    onboarding::remove_project(&core, &path)
}

#[tauri::command]
pub async fn complete_onboarding(core: State<'_, Shared>, projects: Vec<String>, install_codex_hooks: bool) -> Res<Vec<Project>> {
    let core = core.inner().clone();
    blocking(move || onboarding::complete(&core, &projects, install_codex_hooks)).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn continue_conversation(
    core: State<'_, Shared>,
    kind: String,
    session_id: String,
    project_path: String,
    name: String,
    display_project: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Res<AgentView> {
    let core = core.inner().clone();
    let req = ContinueRequest { kind, session_id, project_path, name, display_project, cols, rows };
    blocking(move || onboarding::continue_conversation(&core, req)).await
}

/// "Add to Pitwall" on a session the scan found on another machine (agw).
/// Nothing changes there; a running session is attached.
#[tauri::command]
pub async fn adopt_session(core: State<'_, Shared>, req: AdoptSessionRequest) -> Res<AgentView> {
    let core = core.inner().clone();
    blocking(move || onboarding::adopt_session(&core, req)).await
}
