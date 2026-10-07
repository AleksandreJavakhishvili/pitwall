//! Agents: kinds, lifecycle, terminal I/O, queue, changes, Codex hooks.

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;

use pitwall_core::engine::{changes, input, lifecycle};
use pitwall_core::model::{size_of, AgentView, CodexHooksStatus, CreateAgentRequest, KindView};
use pitwall_core::onboarding::{self, recent::RecentProject};
use pitwall_core::vcs::git::FileChange;
use pitwall_core::{hooks, Shared};

use pitwall_proto::{CreateForm, ProviderMachines};

use super::{blocking, Res};

#[tauri::command]
pub async fn list_kinds(core: State<'_, Shared>) -> Res<Vec<KindView>> {
    let core = core.inner().clone();
    blocking(move || core.list_kinds()).await
}

#[tauri::command]
pub async fn recent_projects(core: State<'_, Shared>) -> Res<Vec<RecentProject>> {
    let core = core.inner().clone();
    blocking(move || Ok(onboarding::recent::recent_projects(core.paths(), 30))).await
}

#[tauri::command]
pub fn list_agents(core: State<'_, Shared>) -> Vec<AgentView> {
    core.views()
}

#[tauri::command]
pub async fn create_agent(core: State<'_, Shared>, req: CreateAgentRequest) -> Res<AgentView> {
    let core = core.inner().clone();
    let c = core.clone();
    let view = blocking(move || lifecycle::create(&c, req)).await?;
    onboarding::add_agent_project(&core, &view);
    Ok(view)
}

/// Providers and their machines: the New-agent dialog's "Runs on".
#[tauri::command]
pub async fn list_machines(core: State<'_, Shared>) -> Res<Vec<ProviderMachines>> {
    let core = core.inner().clone();
    blocking(move || Ok(core.machine_list())).await
}

/// How new agents are made on one machine (its fields and choices).
#[tauri::command]
pub async fn create_form(core: State<'_, Shared>, provider: String, machine: String) -> Res<CreateForm> {
    let core = core.inner().clone();
    blocking(move || core.create_form(Some(&provider), Some(&machine))).await
}

/// Output batching window for the webview channel (a lone chunk goes at once).
const OUTPUT_GAP: std::time::Duration = std::time::Duration::from_millis(12);

#[tauri::command]
pub fn attach_output(core: State<'_, Shared>, agent_id: String, on_data: Channel<InvokeResponseBody>) -> Res<u64> {
    let sink = Box::new(move |bytes: &[u8]| on_data.send(InvokeResponseBody::Raw(bytes.to_vec())).is_ok());
    // One webview message per OUTPUT_GAP at most while an agent streams (perf.md).
    input::attach_output(&core, &agent_id, pitwall_core::term::coalesce(sink, OUTPUT_GAP))
}

#[tauri::command]
pub fn detach_output(core: State<'_, Shared>, agent_id: String, subscription_id: u64) -> Res<()> {
    input::detach_output(&core, &agent_id, subscription_id);
    Ok(())
}

#[tauri::command]
pub fn write_input(core: State<'_, Shared>, agent_id: String, data: String) -> Res<()> {
    input::write_input(&core, &agent_id, &data)
}

#[tauri::command]
pub fn resize(core: State<'_, Shared>, agent_id: String, cols: u16, rows: u16) -> Res<()> {
    input::resize(&core, &agent_id, cols, rows)
}

#[tauri::command]
pub fn send_prompt(core: State<'_, Shared>, agent_id: String, text: String) -> Res<()> {
    input::send_prompt(&core, &agent_id, text)
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
    input::queue_send_now(&core, &agent_id, &item_id)
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
    let core = core.inner().clone();
    blocking(move || changes::changes(&core, &agent_id)).await
}

/// Its changes read now, bypassing the polling pace; one already running is
/// awaited. The agent's numbers follow via `agents-changed`.
#[tauri::command]
pub async fn refresh_changes(core: State<'_, Shared>, agent_id: String) -> Res<Vec<FileChange>> {
    let core = core.inner().clone();
    blocking(move || changes::refresh(&core, &agent_id)).await
}

#[tauri::command]
pub async fn get_file_diff(core: State<'_, Shared>, agent_id: String, path: String, untracked: bool) -> Res<String> {
    let core = core.inner().clone();
    blocking(move || changes::file_diff(&core, &agent_id, &path, untracked)).await
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
    let size = size_of(cols, rows);
    blocking(move || lifecycle::restart(&core, &agent_id, size)).await
}

#[tauri::command]
pub async fn remove_agent(core: State<'_, Shared>, agent_id: String, delete_worktree: bool) -> Res<()> {
    let core = core.inner().clone();
    blocking(move || lifecycle::remove(&core, &agent_id, delete_worktree)).await
}

#[tauri::command]
pub async fn codex_hooks_status(core: State<'_, Shared>) -> Res<CodexHooksStatus> {
    let core = core.inner().clone();
    blocking(move || Ok(hooks::codex_status(core.paths()))).await
}

#[tauri::command]
pub async fn install_codex_hooks(core: State<'_, Shared>) -> Res<CodexHooksStatus> {
    let core = core.inner().clone();
    blocking(move || hooks::install_codex(core.paths())).await
}
