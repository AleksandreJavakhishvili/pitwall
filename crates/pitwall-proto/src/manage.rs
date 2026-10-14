//! Managing what Pitwall has (docs/spec/engineer.md): agents (stop,
//! restart, remove, rename, status, wait), their queues, spaces and
//! windows, projects, rules and read-only review. The parameters and
//! results of those methods (`method::*` in [`crate::api`]); the server
//! offers them when `welcome.caps` has [`caps::MANAGE`](crate::caps::MANAGE)
//! (and [`caps::SPACES`](crate::caps::SPACES) for `space.*` and
//! `agent.move`, which need Pitwall's windows).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::explorer::FileStatus;
use crate::views::{AgentView, QueueItem, Source, Status};

// ------------------------------------------------------------ agents

/// `agent.remove`: take an agent out of Pitwall (its process ends; an
/// adopted session keeps running where it is).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRemove {
    pub agent_id: String,
    /// Also delete its own worktree (`caps.removeWorktree`).
    #[serde(default)]
    pub delete_worktree: bool,
}

/// `agent.rename`: the name Pitwall shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRename {
    pub agent_id: String,
    pub name: String,
}

/// `agent.wait`: block until the agent's status is one of `until`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentWait {
    pub agent_id: String,
    /// Any of these ends the wait (`idle`, `blocked`, `done`, …).
    pub until: Vec<Status>,
    /// Give up after this long (server default: 5 minutes; at most a day).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "number | null")]
    pub timeout_ms: Option<u64>,
    /// Ignore the status it has now: wait until it has left it and then
    /// reached one of `until` (after sending a prompt).
    #[serde(default)]
    pub fresh: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WaitResult {
    pub agent: AgentView,
    #[ts(type = "number")]
    pub waited_ms: u64,
}

/// `agent.status`: why an agent shows the status it does (diagnostics:
/// "why is it unknown?").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentDiagnosis {
    pub agent_id: String,
    pub name: String,
    pub kind: String,
    pub status: Status,
    /// Which input decided it: hooks > screen > activity.
    pub source: Source,
    pub detail: Option<String>,
    pub running: bool,
    /// Its terminal is attached (Pitwall sees its output).
    pub attached: bool,
    /// How long it has had this status.
    #[ts(type = "number")]
    pub status_for_ms: u64,
    pub hooks: HookSignal,
    pub screen: ScreenSignal,
    pub activity: ActivitySignal,
    /// Plain-language reasons, most important first.
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HookSignal {
    /// Its kind reports through hooks and its machine can deliver them.
    pub supported: bool,
    /// `claude-settings` | `codex-global` | `none`.
    pub mode: String,
    /// A hook arrived since it started (hooks then decide its status).
    pub seen: bool,
    /// The last state a hook reported (`working`, `blocked`, `done`, `idle`).
    pub last: Option<String>,
    /// Codex: Pitwall's hooks are in `~/.codex/hooks.json`.
    pub installed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScreenSignal {
    /// Pitwall has screen rules for its kind.
    pub rules: bool,
    /// What the rules read last (`working`, `blocked`, `idle`; null: no match).
    pub last: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySignal {
    /// Since its last output (null: none yet).
    #[ts(type = "number | null")]
    pub last_output_ms_ago: Option<u64>,
    /// Output this recent counts as working.
    #[ts(type = "number")]
    pub window_ms: u64,
}

// ------------------------------------------------------------ queue

/// `queue.add`: queue a prompt (sent when the agent is free, if it
/// auto-sends).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueAdd {
    pub agent_id: String,
    pub text: String,
}

/// `queue.remove` / `queue.send`: one queued prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueItemRef {
    pub agent_id: String,
    /// The item's id; `queue.send` without it sends the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
}

/// `queue.list`: one agent, or every agent with a queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueueFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentQueue {
    pub agent_id: String,
    pub name: String,
    pub status: Status,
    pub auto_send: bool,
    pub items: Vec<QueueItem>,
}

// ------------------------------------------------------------ spaces

/// A space as `space.list` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpaceView {
    pub id: String,
    pub name: String,
    /// `all` | `project` | `custom`.
    pub kind: String,
    pub project: Option<String>,
    /// The window it is in (`main`, `pitwall-2`, …).
    pub window: String,
    /// Agents shown in its panes.
    pub shown: Vec<String>,
    /// Agents that belong to it (custom and project spaces).
    pub members: Vec<String>,
}

/// `space.create`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpaceCreate {
    pub name: String,
}

/// `space.rename`. `space` is an id or a name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpaceRename {
    pub space: String,
    pub name: String,
}

/// `space.move`: into another window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpaceMove {
    pub space: String,
    /// `new` (a new window), `main`, or an open window's label.
    pub window: String,
}

/// `agent.move`: show an agent in a space (as dropping it on the tab).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentMove {
    pub agent_id: String,
    pub space: String,
}

// ------------------------------------------------------------ projects

/// `project.add` / `project.remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPath {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    pub path: String,
    pub display: String,
    pub is_git: bool,
    #[ts(type = "number")]
    pub added_at: u64,
}

// ------------------------------------------------------------ rules

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RuleFileView {
    pub id: String,
    pub description: Option<String>,
    pub targets: Vec<String>,
    pub source: String,
    /// A skill (`<source>:skills/<name>`), not a rule.
    #[serde(default)]
    pub skill: bool,
}

/// `rules.library.new`: a new rule or skill in the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryNew {
    /// A skill (else a rule).
    #[serde(default)]
    pub skill: bool,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub targets: Vec<String>,
    /// A library id, a file, or a skill's folder to start from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// `from` is the other kind (a skill to make a rule of, or the reverse).
    #[serde(default)]
    pub convert: bool,
    /// Replace one of that name (asks the user).
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryAdded {
    pub id: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRulesInfo {
    pub agent_id: String,
    /// Its own (extra) set.
    pub rule_set_id: Option<String>,
    /// Its project's default set.
    pub project_rule_set_id: Option<String>,
    #[ts(type = "number | null")]
    pub applied_at: Option<u64>,
    pub generated: Vec<String>,
    pub error: Option<String>,
    /// The rules changed since they were applied.
    pub stale: bool,
    /// Works in the main checkout (not its own worktree).
    pub main_checkout: bool,
}

/// `rules.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RulesOverview {
    /// rulesync can run (installed, or npx allowed).
    pub available: bool,
    pub via: Option<String>,
    pub files: Vec<RuleFileView>,
    pub agents: Vec<AgentRulesInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RuleSetView {
    pub id: String,
    pub name: String,
    pub rule_ids: Vec<String>,
}

/// `rules.sets`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RuleSets {
    pub sets: Vec<RuleSetView>,
    /// Project folder (repo root) → its default set.
    pub project_defaults: BTreeMap<String, String>,
}

/// `rules.apply`: generate an agent's rule files (they take effect in its
/// next session).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct RulesApply {
    pub agent_id: String,
    /// Give the agent this set of its own first (`""`: none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// It may write into the main checkout (asks the user first).
    #[serde(default)]
    pub main_checkout: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RulesApplied {
    pub agent_id: String,
    pub generated: Vec<String>,
    pub log: String,
}

/// `rules.reapply`: re-apply every agent whose rules changed or failed
/// since its session started (only worktrees and main checkouts the user
/// confirmed), optionally one project's or one set's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct RulesReapply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// A set id or name: only agents that use it (own or project default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RulesFailed {
    pub agent_id: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RulesReapplied {
    /// Re-applied: used from each agent's next session.
    pub applied: Vec<RulesApplied>,
    pub failed: Vec<RulesFailed>,
}

/// `rules.set.save`: a new rule set (no `id`), or a changed one (its
/// name or rules). Rule ids are the library's (`rules.list`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct RuleSetSave {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub rule_ids: Vec<String>,
}

/// `rules.set.delete`: a set by id or name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct RuleSetRef {
    pub set: String,
}

/// `rules.import`: rules into the library from a project's `.rulesync/`
/// (`project`, read in place), a CLAUDE.md / AGENTS.md / GEMINI.md
/// (`file`, through `rulesync import`) or a git repository (`git`, cloned).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct RulesImport {
    pub kind: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RulesImported {
    /// Ids of rules that are new in the library.
    pub added: Vec<String>,
    pub log: String,
}

/// `rules.show`: one project's rules (`project`), or one agent's
/// (`agentId`, with its project's default set).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct RulesShow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

/// One rule an agent gets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveRuleView {
    pub id: String,
    pub description: Option<String>,
    /// `project` (the project's default set), `agent` (its own set) or
    /// `folder` (the working folder's own `.rulesync/`).
    pub from: String,
    /// Picked in a set but no longer in the library (skipped).
    pub missing: bool,
    /// A skill, not a rule.
    #[serde(default)]
    pub skill: bool,
}

/// An agent's rules in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRulesDetail {
    pub name: String,
    pub kind_name: String,
    /// What its files are made for (a rulesync target such as
    /// `claudecode`); null: its kind takes no rules.
    pub target: Option<String>,
    /// Its state: own set, generated files, applied at, stale, error.
    pub rules: AgentRulesInfo,
    /// The project's default set plus its own, for its kind (`targets`).
    pub effective: Vec<EffectiveRuleView>,
}

/// `rules.show`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRulesView {
    /// The project (repo root when inside git).
    pub project: String,
    /// Its default set (id).
    pub rule_set_id: Option<String>,
    pub agents: Vec<AgentRulesDetail>,
}

/// `rules.default`: a project's default set (`set` null: none).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct RulesDefault {
    pub project: String,
    pub set: Option<String>,
}

// ------------------------------------------------------------ project rules
// The project scope (docs/spec/rules.md, "Project rules"): library rules
// copied into `<repo>/.rulesync/rules/`, committed by the user.

/// Where a rule in a project's `.rulesync/rules/` stands against the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ProjectRuleState {
    /// As copied, and the library's is the same.
    Current,
    /// The library's changed since it was copied ("update available").
    Update,
    /// Changed in the project; the library's didn't.
    Modified,
    /// Changed in both: someone has to choose.
    Conflict,
    /// Its library rule is gone.
    SourceGone,
    /// Copied, then deleted from the project.
    Deleted,
    /// The project's own (not copied by Pitwall).
    Own,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRule {
    /// Its path under `.rulesync/rules/` (`style.md`), or a skill's folder
    /// name under `.rulesync/skills/` (`review`).
    pub name: String,
    /// A skill (a folder), not a rule file.
    #[serde(default)]
    pub skill: bool,
    /// The library rule it was copied from (`library:style.md`).
    pub source: Option<String>,
    pub state: ProjectRuleState,
    pub description: Option<String>,
}

/// The files rulesync generates from the project's `.rulesync/`
/// (CLAUDE.md, AGENTS.md, …), which are committed too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedFiles {
    /// Pitwall generated them once (otherwise `outOfDate` says nothing).
    pub generated: bool,
    /// `.rulesync/` changed, or generated files were edited or deleted,
    /// since the last generation.
    pub out_of_date: bool,
    /// Generated files edited or deleted since.
    pub changed: Vec<String>,
    /// rulesync targets it generates for.
    pub targets: Vec<String>,
    /// What it generates: `skills`, and `rules` only once rules were copied
    /// into the project (else its CLAUDE.md / AGENTS.md stay its own).
    #[serde(default)]
    pub features: Vec<String>,
}

/// `rules.project.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRulesStatus {
    /// The project (repo root).
    pub project: String,
    /// It has a `.rulesync/rules/` folder.
    pub rulesync: bool,
    pub rules: Vec<ProjectRule>,
    /// Rules with an update or a conflict.
    pub updates: usize,
    pub generated: GeneratedFiles,
    /// Hand-written instruction files git tracks (CLAUDE.md, AGENTS.md,
    /// GEMINI.md) not imported into `.rulesync/` yet: import them first.
    pub existing: Vec<String>,
    /// The old project default set (Pitwall's own data), used while the
    /// project has no `.rulesync/`: "move to project".
    pub fallback_set: Option<String>,
}

/// A project, by folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRef {
    pub project: String,
}

/// `rules.project.assign`: copy a set's rules and/or single library rules
/// into the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAssign {
    pub project: String,
    /// A set id or name: all its rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    #[serde(default)]
    pub rule_ids: Vec<String>,
    /// Files (names under `.rulesync/rules/`) that exist with other
    /// content and may be replaced.
    #[serde(default)]
    pub overwrite: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAssigned {
    /// Written into `.rulesync/rules/`.
    pub copied: Vec<String>,
    /// Already there with the same content (now tracked).
    pub unchanged: Vec<String>,
    /// There with other content: not written (pass them in `overwrite`).
    pub conflicts: Vec<String>,
    /// Why some weren't copied.
    pub skipped: Vec<String>,
}

/// `rules.project.unassign`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUnassign {
    pub project: String,
    /// Names under `.rulesync/rules/` or library ids.
    pub rules: Vec<String>,
    /// Remove them even if they were changed in the project.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUnassigned {
    pub removed: Vec<String>,
    /// Changed in the project, so kept (pass `force`).
    pub modified: Vec<String>,
    /// Not rules Pitwall copied there.
    pub unknown: Vec<String>,
}

/// One rule with something to decide (`rules.project.updates`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRuleUpdate {
    pub name: String,
    #[serde(default)]
    pub skill: bool,
    pub source: String,
    /// `update`, `modified` or `conflict`.
    pub state: ProjectRuleState,
    /// The library's text now (a skill's SKILL.md; null: gone).
    pub library: Option<String>,
    /// The project's text now (a skill's SKILL.md; null: deleted).
    pub project: Option<String>,
    /// Unified diff from the project's copy to the library's (a skill's:
    /// one per file that differs).
    pub diff: String,
}

/// `rules.project.update`: take library updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUpdate {
    pub project: String,
    /// Names or library ids; empty: every rule with an update.
    #[serde(default)]
    pub rules: Vec<String>,
    /// For conflicts: `library` (replace the project's copy) or `project`
    /// (keep it). Unset: conflicts are left alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUpdated {
    /// The library's version written into the project.
    pub updated: Vec<String>,
    /// Conflicts where the project's copy was kept.
    pub kept: Vec<String>,
    /// Conflicts left alone (say `take`).
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectImported {
    /// The instruction files imported (CLAUDE.md, …).
    pub files: Vec<String>,
    /// Rules written into `.rulesync/rules/`.
    pub added: Vec<String>,
    pub log: String,
}

/// `rules.project.generate`: run rulesync for the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGenerate {
    pub project: String,
    /// rulesync targets (empty: the ones it used last, else those of its
    /// agents' kinds).
    #[serde(default)]
    pub targets: Vec<String>,
    /// Files that may be replaced although Pitwall didn't write them
    /// (a hand-written CLAUDE.md, …).
    #[serde(default)]
    pub overwrite: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGenerated {
    pub written: Vec<String>,
    pub unchanged: Vec<String>,
    /// Not written: someone else's file there (pass them in `overwrite`).
    pub needs_confirm: Vec<String>,
    pub targets: Vec<String>,
    /// rulesync features generated (`skills`, `rules`).
    #[serde(default)]
    pub features: Vec<String>,
    pub log: String,
}

// ------------------------------------------------------------ review

/// `review.changes`: what the agent changed since it started (or in one
/// task). Read-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    pub binary: bool,
    pub untracked: bool,
    pub status: Option<FileStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReviewChanges {
    pub agent_id: String,
    /// `agent` (since it started) or the task id.
    pub since: String,
    pub files: Vec<ChangedFile>,
    pub added: u32,
    pub removed: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn round_trip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(v: &T, wire: Value) {
        assert_eq!(serde_json::to_value(v).unwrap(), wire);
        assert_eq!(&serde_json::from_value::<T>(wire).unwrap(), v);
    }

    #[test]
    fn params_golden_json() {
        round_trip(&AgentRemove { agent_id: "a1".into(), delete_worktree: true }, json!({"agentId":"a1","deleteWorktree":true}));
        let r: AgentRemove = serde_json::from_value(json!({"agentId":"a1"})).unwrap();
        assert!(!r.delete_worktree, "keeps the worktree unless asked");
        round_trip(&AgentRename { agent_id: "a1".into(), name: "api".into() }, json!({"agentId":"a1","name":"api"}));
        round_trip(
            &AgentWait { agent_id: "a1".into(), until: vec![Status::Idle, Status::Done], timeout_ms: Some(500), fresh: true },
            json!({"agentId":"a1","until":["idle","done"],"timeoutMs":500,"fresh":true}),
        );
        let w: AgentWait = serde_json::from_value(json!({"agentId":"a1","until":["blocked"]})).unwrap();
        assert_eq!((w.timeout_ms, w.fresh), (None, false));
        round_trip(&QueueAdd { agent_id: "a1".into(), text: "run tests".into() }, json!({"agentId":"a1","text":"run tests"}));
        round_trip(&QueueItemRef { agent_id: "a1".into(), item_id: None }, json!({"agentId":"a1"}));
        round_trip(&QueueFilter::default(), json!({}));
        round_trip(&SpaceCreate { name: "api".into() }, json!({"name":"api"}));
        round_trip(&SpaceRename { space: "s1".into(), name: "web".into() }, json!({"space":"s1","name":"web"}));
        round_trip(&SpaceMove { space: "s1".into(), window: "new".into() }, json!({"space":"s1","window":"new"}));
        round_trip(&AgentMove { agent_id: "a1".into(), space: "api".into() }, json!({"agentId":"a1","space":"api"}));
        round_trip(&ProjectPath { path: "/p".into() }, json!({"path":"/p"}));
        round_trip(&RulesApply { agent_id: "a1".into(), set: Some("s".into()), main_checkout: false }, json!({"agentId":"a1","set":"s","mainCheckout":false}));
        round_trip(&RulesDefault { project: "/p".into(), set: None }, json!({"project":"/p","set":null}));
        round_trip(&RulesShow { project: Some("/p".into()), agent_id: None }, json!({"project":"/p"}));
        round_trip(
            &LibraryNew { skill: true, name: "review".into(), description: None, targets: vec![], from: None, convert: false, overwrite: false },
            json!({"skill":true,"name":"review","targets":[],"convert":false,"overwrite":false}),
        );
        round_trip(&RulesReapply { project: Some("/p".into()), set: None }, json!({"project":"/p"}));
        round_trip(&RuleSetSave { id: None, name: "Web".into(), rule_ids: vec!["library:a.md".into()] }, json!({"name":"Web","ruleIds":["library:a.md"]}));
        round_trip(&RulesImport { kind: "git".into(), source: "https://x/r.git".into() }, json!({"kind":"git","source":"https://x/r.git"}));
        round_trip(&RulesShow { project: None, agent_id: Some("a1".into()) }, json!({"agentId":"a1"}));
        round_trip(&ReviewRequest { agent_id: "a1".into(), task_id: None }, json!({"agentId":"a1"}));
    }

    #[test]
    fn results_golden_json() {
        round_trip(
            &SpaceView {
                id: "s1".into(),
                name: "api".into(),
                kind: "custom".into(),
                project: None,
                window: "main".into(),
                shown: vec!["a1".into()],
                members: vec!["a1".into(), "a2".into()],
            },
            json!({"id":"s1","name":"api","kind":"custom","project":null,"window":"main","shown":["a1"],"members":["a1","a2"]}),
        );
        round_trip(
            &AgentQueue {
                agent_id: "a1".into(),
                name: "api".into(),
                status: Status::Working,
                auto_send: true,
                items: vec![QueueItem { id: "q1".into(), text: "next".into() }],
            },
            json!({"agentId":"a1","name":"api","status":"working","autoSend":true,"items":[{"id":"q1","text":"next"}]}),
        );
        round_trip(
            &ReviewChanges {
                agent_id: "a1".into(),
                since: "agent".into(),
                files: vec![ChangedFile { path: "src/a.rs".into(), added: 3, removed: 1, binary: false, untracked: false, status: Some(FileStatus::M) }],
                added: 3,
                removed: 1,
            },
            json!({"agentId":"a1","since":"agent","files":[{"path":"src/a.rs","added":3,"removed":1,"binary":false,"untracked":false,"status":"M"}],"added":3,"removed":1}),
        );
        round_trip(
            &AgentDiagnosis {
                agent_id: "a1".into(),
                name: "api".into(),
                kind: "codex".into(),
                status: Status::Unknown,
                source: Source::Activity,
                detail: None,
                running: true,
                attached: true,
                status_for_ms: 10,
                hooks: HookSignal { supported: true, mode: "codex-global".into(), seen: false, last: None, installed: Some(false) },
                screen: ScreenSignal { rules: true, last: None, detail: None },
                activity: ActivitySignal { last_output_ms_ago: Some(5000), window_ms: 1200 },
                explanation: vec!["x".into()],
            },
            json!({"agentId":"a1","name":"api","kind":"codex","status":"unknown","source":"activity","detail":null,"running":true,
                   "attached":true,"statusForMs":10,
                   "hooks":{"supported":true,"mode":"codex-global","seen":false,"last":null,"installed":false},
                   "screen":{"rules":true,"last":null,"detail":null},
                   "activity":{"lastOutputMsAgo":5000,"windowMs":1200},"explanation":["x"]}),
        );
        round_trip(
            &RuleSets { sets: vec![RuleSetView { id: "s".into(), name: "Strict".into(), rule_ids: vec!["library:a.md".into()] }], project_defaults: BTreeMap::from([("/p".into(), "s".into())]) },
            json!({"sets":[{"id":"s","name":"Strict","ruleIds":["library:a.md"]}],"projectDefaults":{"/p":"s"}}),
        );
        round_trip(
            &ProjectView { path: "/p".into(), display: "~/p".into(), is_git: true, added_at: 1 },
            json!({"path":"/p","display":"~/p","isGit":true,"addedAt":1}),
        );
    }
}
