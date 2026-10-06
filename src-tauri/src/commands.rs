//! Tauri commands (see docs/CONTRACT.md). Anything that may block runs on a
//! blocking worker so the main thread stays responsive.

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, State, WebviewWindow};

use crate::git::FileChange;
use crate::model::{AgentView, CodexHooksStatus, CreateAgentRequest, KindView};
use crate::projects::RecentProject;
use crate::registry::Shared;
use crate::{git, hooks, kinds, lifecycle, projects, windows};

type Res<T> = Result<T, String>;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Res<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("internal error: {e}"))?
}

#[tauri::command]
pub async fn list_kinds() -> Res<Vec<KindView>> {
    blocking(|| Ok(kinds::list())).await
}

#[tauri::command]
pub async fn recent_projects() -> Res<Vec<RecentProject>> {
    blocking(|| Ok(projects::recent_projects(30))).await
}

#[tauri::command]
pub fn list_agents(core: State<'_, Shared>) -> Vec<AgentView> {
    core.views()
}

#[tauri::command]
pub async fn create_agent(app: AppHandle, core: State<'_, Shared>, req: CreateAgentRequest) -> Res<AgentView> {
    let core = core.inner().clone();
    let view = blocking(move || lifecycle::create(&core, req)).await?;
    crate::onboarding::add_agent_project(&app, &view);
    Ok(view)
}

#[tauri::command]
pub fn attach_output(core: State<'_, Shared>, agent_id: String, on_data: Channel<InvokeResponseBody>) -> Res<u64> {
    Ok(core.session(&agent_id)?.attach(on_data))
}

#[tauri::command]
pub fn detach_output(core: State<'_, Shared>, agent_id: String, subscription_id: u64) -> Res<()> {
    // Detaching from an agent that is gone or restarted is not an error.
    if let Ok(Some(session)) = core.with(&agent_id, |a| a.session.clone()) {
        session.detach(subscription_id);
    }
    Ok(())
}

#[tauri::command]
pub fn write_input(core: State<'_, Shared>, agent_id: String, data: String) -> Res<()> {
    let session = core.session(&agent_id)?;
    session.touch_input();
    session.write(data.as_bytes())?;
    if data.contains('\r') {
        // Enter in the terminal: there's probably a conversation to resume now.
        let changed = core.with(&agent_id, |a| !std::mem::replace(&mut a.rec.has_conversation, true))?;
        if changed {
            core.changed(true);
        }
        crate::tasks::begin_for(&core, &agent_id, None, false);
    }
    Ok(())
}

#[tauri::command]
pub fn resize(core: State<'_, Shared>, agent_id: String, cols: u16, rows: u16) -> Res<()> {
    // Recorded even with no process (stopped, or mid-restart) so the next
    // spawn starts at this size instead of losing it.
    let (session, recorded) = core.with(&agent_id, |a| (a.session.clone(), a.rec.set_term_size((cols, rows))))?;
    let resized = session.is_some_and(|s| s.resize(cols, rows));
    if recorded || resized {
        core.changed(recorded);
    }
    Ok(())
}

#[tauri::command]
pub fn send_prompt(core: State<'_, Shared>, agent_id: String, text: String) -> Res<()> {
    let session = core.record_sent(&agent_id, &text)?;
    crate::tasks::begin_for(&core, &agent_id, Some(&text), true);
    session.touch_input();
    session.send_text(text);
    Ok(())
}

#[tauri::command]
pub fn queue_add(core: State<'_, Shared>, agent_id: String, text: String) -> Res<AgentView> {
    core.queue_add(&agent_id, text)
}

#[tauri::command]
pub fn queue_remove(core: State<'_, Shared>, agent_id: String, item_id: String) -> Res<AgentView> {
    core.queue_remove(&agent_id, &item_id)
}

#[tauri::command]
pub fn queue_send_now(core: State<'_, Shared>, agent_id: String, item_id: String) -> Res<AgentView> {
    let (session, text, view) = core.take_queued(&agent_id, &item_id)?;
    crate::tasks::begin_for(&core, &agent_id, Some(&text), true);
    session.send_text(text);
    Ok(view)
}

#[tauri::command]
pub fn set_auto_send(core: State<'_, Shared>, agent_id: String, enabled: bool) -> Res<AgentView> {
    core.set_auto_send(&agent_id, enabled)
}

#[tauri::command]
pub fn mark_seen(core: State<'_, Shared>, agent_id: String) -> Res<()> {
    core.mark_seen(&agent_id)
}

#[tauri::command]
pub async fn get_changes(core: State<'_, Shared>, agent_id: String) -> Res<Vec<FileChange>> {
    let (cwd, base) = core.with(&agent_id, |a| (a.rec.cwd.clone(), a.rec.base_commit.clone()))?;
    let files = blocking(move || git::changes(&cwd, base.as_deref())).await?;
    let (added, removed, n) = (
        files.iter().map(|f| f.added).sum(),
        files.iter().map(|f| f.removed).sum(),
        files.len() as u32,
    );
    let differs = core.with(&agent_id, |a| {
        let d = (a.added, a.removed, a.files_changed) != (added, removed, n);
        (a.added, a.removed, a.files_changed) = (added, removed, n);
        d
    })?;
    if differs {
        core.changed(false);
    }
    Ok(files)
}

#[tauri::command]
pub async fn get_file_diff(core: State<'_, Shared>, agent_id: String, path: String, untracked: bool) -> Res<String> {
    let (cwd, base) = core.with(&agent_id, |a| (a.rec.cwd.clone(), a.rec.base_commit.clone()))?;
    if path.is_empty() || path.starts_with('/') || path.split('/').any(|p| p == "..") {
        return Err("invalid path".into());
    }
    blocking(move || git::file_diff(&cwd, base.as_deref(), &path, untracked)).await
}

#[tauri::command]
pub fn stop_agent(core: State<'_, Shared>, agent_id: String) -> Res<()> {
    lifecycle::stop(core.inner(), &agent_id)
}

#[tauri::command]
pub async fn restart_agent(
    core: State<'_, Shared>,
    agent_id: String,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Res<AgentView> {
    let core = core.inner().clone();
    let size = crate::model::size_of(cols, rows);
    blocking(move || lifecycle::restart(&core, &agent_id, size)).await
}

#[tauri::command]
pub async fn remove_agent(core: State<'_, Shared>, agent_id: String, delete_worktree: bool) -> Res<()> {
    let core = core.inner().clone();
    blocking(move || lifecycle::remove(&core, &agent_id, delete_worktree)).await
}

fn codex_status() -> CodexHooksStatus {
    let path = hooks::codex_hooks_path();
    CodexHooksStatus {
        installed: hooks::codex_status_at(&path, &hooks::hook_command()),
        path: path.to_string_lossy().into_owned(),
    }
}

#[tauri::command]
pub async fn codex_hooks_status() -> Res<CodexHooksStatus> {
    blocking(|| Ok(codex_status())).await
}

#[tauri::command]
pub async fn install_codex_hooks() -> Res<CodexHooksStatus> {
    blocking(|| {
        hooks::install_script()?;
        hooks::install_codex_hooks_at(&hooks::codex_hooks_path(), &hooks::hook_command())?;
        Ok(codex_status())
    })
    .await
}

// ------------------------------------------------------------ windows / UI state

#[tauri::command]
pub fn get_ui_state() -> serde_json::Value {
    windows::get_ui_state()
}

#[tauri::command]
pub fn set_ui_state(app: AppHandle, window: WebviewWindow, state: serde_json::Value) -> Res<()> {
    windows::set_ui_state(&app, state, window.label())
}

#[tauri::command]
pub async fn open_window(app: AppHandle, space_id: String) -> Res<String> {
    windows::open_window(&app, &space_id)
}

#[tauri::command]
pub fn focus_window(app: AppHandle, label: String) -> Res<()> {
    windows::focus_window(&app, &label)
}

#[tauri::command]
pub fn list_windows(app: AppHandle) -> Vec<String> {
    windows::list_windows(&app)
}
