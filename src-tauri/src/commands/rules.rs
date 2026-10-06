//! Rules via rulesync (docs/spec/rules.md).

use std::collections::BTreeMap;

use tauri::State;

use pitwall_core::rules::{
    self, AgentRulesView, ApplyResult, Dirs, ImportRequest, ImportResult, RuleFile, RuleSet, RuleSource, RulesStatus,
    SaveRuleSet,
};
use pitwall_core::Shared;

use super::{blocking, Res};

fn dirs(core: &Shared) -> Dirs {
    Dirs::of(core.paths())
}

#[tauri::command]
pub async fn rules_status(core: State<'_, Shared>) -> Res<RulesStatus> {
    let d = dirs(&core);
    blocking(move || Ok(rules::status(&d))).await
}

#[tauri::command]
pub async fn set_rules_npx(core: State<'_, Shared>, enabled: bool) -> Res<RulesStatus> {
    let d = dirs(&core);
    blocking(move || rules::set_npx(&d, enabled)).await
}

#[tauri::command]
pub async fn list_rule_library(core: State<'_, Shared>) -> Res<Vec<RuleFile>> {
    let d = dirs(&core);
    blocking(move || Ok(rules::library_files(&d))).await
}

/// Opens ~/.pitwall/rules in Finder (rules are edited in the user's editor).
#[tauri::command]
pub async fn reveal_rule_library(core: State<'_, Shared>) -> Res<String> {
    let d = dirs(&core);
    blocking(move || {
        let dir = rules::library_dir(&d)?;
        std::process::Command::new("open")
            .arg(&dir)
            .status()
            .map_err(|e| e.to_string())?;
        Ok(dir.to_string_lossy().into_owned())
    })
    .await
}

#[tauri::command]
pub async fn list_rule_sets(core: State<'_, Shared>) -> Res<Vec<RuleSet>> {
    let d = dirs(&core);
    blocking(move || Ok(rules::sets(&d))).await
}

#[tauri::command]
pub async fn save_rule_set(core: State<'_, Shared>, set: SaveRuleSet) -> Res<RuleSet> {
    let d = dirs(&core);
    blocking(move || rules::save_set(&d, set)).await
}

#[tauri::command]
pub async fn delete_rule_set(core: State<'_, Shared>, id: String) -> Res<()> {
    let d = dirs(&core);
    blocking(move || rules::delete_set(&d, &id)).await
}

#[tauri::command]
pub async fn import_rules(core: State<'_, Shared>, req: ImportRequest) -> Res<ImportResult> {
    let d = dirs(&core);
    blocking(move || rules::import(&d, req)).await
}

#[tauri::command]
pub async fn list_rule_sources(core: State<'_, Shared>) -> Res<Vec<RuleSource>> {
    let d = dirs(&core);
    blocking(move || Ok(rules::sources(&d))).await
}

#[tauri::command]
pub async fn pull_rule_source(core: State<'_, Shared>, name: String) -> Res<String> {
    let d = dirs(&core);
    blocking(move || rules::pull_source(&d, &name)).await
}

#[tauri::command]
pub async fn remove_rule_source(core: State<'_, Shared>, name: String) -> Res<()> {
    let d = dirs(&core);
    blocking(move || rules::remove_source(&d, &name)).await
}

#[tauri::command]
pub async fn set_project_rules(core: State<'_, Shared>, project_path: String, rule_set_id: Option<String>) -> Res<()> {
    let d = dirs(&core);
    blocking(move || rules::set_project_rules(&d, &project_path, rule_set_id)).await
}

/// Project path (repo root) → default rule set id.
#[tauri::command]
pub async fn list_project_rules(core: State<'_, Shared>) -> Res<BTreeMap<String, String>> {
    let d = dirs(&core);
    blocking(move || Ok(rules::project_rules(&d))).await
}

/// The default set that would apply to a folder (resolved to its repo root).
#[tauri::command]
pub async fn get_project_rules(core: State<'_, Shared>, project_path: String) -> Res<Option<String>> {
    let d = dirs(&core);
    blocking(move || Ok(rules::project_rules_for(&d, &project_path))).await
}

#[tauri::command]
pub async fn apply_rules(
    core: State<'_, Shared>,
    agent_id: String,
    confirm_main_checkout: Option<bool>,
) -> Res<ApplyResult> {
    let core = core.inner().clone();
    blocking(move || rules::apply_to_agent(&core, &agent_id, confirm_main_checkout.unwrap_or(false))).await
}

/// Change an agent's own (extra) rule set; takes effect on the next apply.
#[tauri::command]
pub async fn set_agent_rules(core: State<'_, Shared>, agent_id: String, rule_set_id: Option<String>) -> Res<()> {
    let core = core.inner().clone();
    blocking(move || rules::set_agent_rules(&core, &agent_id, rule_set_id)).await
}

/// Rules info for every agent (stale flags drive "Re-apply & restart").
#[tauri::command]
pub async fn agent_rules(core: State<'_, Shared>) -> Res<Vec<AgentRulesView>> {
    let core = core.inner().clone();
    blocking(move || Ok(rules::agent_rules(&core))).await
}
