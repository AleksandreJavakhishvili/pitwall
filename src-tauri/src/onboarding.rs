//! Commands for first-launch auto-detect and the project list
//! (docs/spec/onboarding.md). Events: `scan-progress`, `projects-changed`.

use tauri::{AppHandle, Emitter, State};

use crate::model::{AgentView, CreateAgentRequest};
use crate::project_list::{self, Project};
use crate::registry::Shared;
use crate::scan::{self, ScanResult};
use crate::{hooks, lifecycle};

type Res<T> = Result<T, String>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Res<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("internal error: {e}"))?
}

fn projects_changed(app: &AppHandle) -> Vec<Project> {
    let list = project_list::list();
    let _ = app.emit("projects-changed", &list);
    list
}

#[tauri::command]
pub async fn scan_environment(app: AppHandle, core: State<'_, Shared>) -> Res<ScanResult> {
    let core = core.inner().clone();
    blocking(move || {
        let mut result = scan::scan(&|p| {
            let _ = app.emit("scan-progress", &p);
        });
        // "Scan again" must not offer sessions Pitwall agents already run.
        let sessions = core.records().into_iter().filter_map(|r| r.session_id).collect();
        scan::mark_in_pitwall(&mut result, &sessions);
        Ok(result)
    })
    .await
}

#[tauri::command]
pub fn get_onboarded() -> bool {
    project_list::onboarded()
}

#[tauri::command]
pub fn list_projects() -> Vec<Project> {
    project_list::list()
}

#[tauri::command]
pub fn add_project(app: AppHandle, path: String) -> Res<Vec<Project>> {
    project_list::add(&[path])?;
    Ok(projects_changed(&app))
}

/// Drops the project from Pitwall's list. Never deletes files.
#[tauri::command]
pub fn remove_project(app: AppHandle, path: String) -> Res<Vec<Project>> {
    project_list::remove(&path)?;
    Ok(projects_changed(&app))
}

#[tauri::command]
pub async fn complete_onboarding(app: AppHandle, projects: Vec<String>, install_codex_hooks: bool) -> Res<Vec<Project>> {
    blocking(move || {
        // Folders that vanished since the scan are skipped, not fatal.
        for p in &projects {
            if let Err(e) = project_list::add(std::slice::from_ref(p)) {
                eprintln!("pitwall: not adding project: {e}");
            }
        }
        project_list::set_onboarded()?;
        let list = projects_changed(&app);
        if install_codex_hooks {
            hooks::install_script()
                .and_then(|_| hooks::install_codex_hooks_at(&hooks::codex_hooks_path(), &hooks::hook_command()))
                .map_err(|e| format!("Projects were saved, but Codex hooks could not be installed: {e}"))?;
        }
        Ok(list)
    })
    .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn continue_conversation(
    app: AppHandle,
    core: State<'_, Shared>,
    kind: String,
    session_id: String,
    project_path: String,
    name: String,
    display_project: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Res<AgentView> {
    if session_id.trim().is_empty() {
        return Err("no session to continue".into());
    }
    let display_project = display_project.filter(|p| !p.trim().is_empty());
    let core = core.inner().clone();
    let remember = (session_id.clone(), display_project.clone());
    let view = blocking(move || {
        lifecycle::create(
            &core,
            CreateAgentRequest {
                name,
                kind,
                project_path,
                custom_command: None,
                worktree: false,
                resume_session_id: Some(session_id),
                rule_set_id: None,
                apply_to_main_checkout: false,
                display_project,
                cols,
                rows,
            },
        )
    })
    .await?;
    // "Scan again" keeps the user's choice of project for this conversation.
    if let (id, Some(p)) = &remember {
        if let Err(e) = project_list::remember_conversation_project(id, Some(p)) {
            eprintln!("pitwall: not remembering the project for {id}: {e}");
        }
    }
    add_agent_project(&app, &view);
    Ok(view)
}

/// The agent's project joins the project list if it wasn't there yet (not
/// `~` etc.), so it shows in the sidebar even after its agents are removed.
pub fn add_agent_project(app: &AppHandle, view: &AgentView) {
    if !scan::is_project_folder(&view.project) {
        return;
    }
    match project_list::add(std::slice::from_ref(&view.project)) {
        Ok(n) if n > 0 => {
            projects_changed(app);
        }
        Ok(_) => {}
        Err(e) => eprintln!("pitwall: not adding project {}: {e}", view.project),
    }
}

