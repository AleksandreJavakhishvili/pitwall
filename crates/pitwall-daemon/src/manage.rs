//! Managing what Pitwall has (docs/spec/engineer.md): stop, restart,
//! remove, rename, diagnose and wait for agents; their queues; projects;
//! rules; read-only review. The same `pitwall-core` calls the UI makes.
//! Risky requests (stop, restart, remove, deleting a worktree, writing
//! rules into a main checkout) wait for the user's answer in Pitwall unless
//! Pitwall's own window asked.

use std::time::{Duration, Instant};

use serde_json::Value;

use pitwall_core::engine::{diagnose, input, lifecycle};
use pitwall_core::rules::{self, Dirs, ImportRequest, SaveRuleSet};
use pitwall_proto::{
    code, method, AgentQueue, AgentRef, AgentRemove, AgentRename, AgentRulesDetail, AgentRulesInfo, AgentView, AgentWait, ChangedFile, ErrorBody, ProjectPath,
    ProjectRulesView, ProjectView, QueueAdd, QueueFilter, QueueItemRef, EffectiveRuleView, ReviewChanges, ReviewRequest, Risk, RuleFileView, RuleSetRef, RuleSetSave, RuleSetView, RuleSets, RulesApplied,
    RulesApply, RulesDefault, RulesImport, RulesImported, RulesOverview, RulesShow, Status, WaitResult,
};

use crate::approvals::{Ask, Decision};
use crate::identity::Caller;
use crate::server::Server;

type Res = Result<Value, ErrorBody>;

/// How long `agent.wait` waits by default, and at most.
pub const WAIT_DEFAULT: Duration = Duration::from_secs(300);
pub const WAIT_MAX: Duration = Duration::from_secs(24 * 3600);
/// How often `agent.wait` looks.
const WAIT_POLL: Duration = Duration::from_millis(100);

fn ok(v: impl serde::Serialize) -> Res {
    serde_json::to_value(v).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

fn other(e: impl Into<String>) -> ErrorBody {
    ErrorBody::new(code::OTHER, e)
}

fn bad(e: impl Into<String>) -> ErrorBody {
    ErrorBody::new(code::BAD_PARAMS, e)
}

/// The agent as clients see it, or `not_found`.
pub fn agent(s: &Server, id: &str) -> Result<AgentView, ErrorBody> {
    s.engine
        .views()
        .into_iter()
        .find(|v| v.id == id)
        .ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no agent \"{id}\" in Pitwall (see `pitwall agent list`)")))
}

/// Ask the user (Pitwall's own window never asks itself). `what` finishes
/// the denial messages ("Nothing changed.").
fn approve(s: &Server, caller: &Caller, action: &str, summary: String, details: Vec<String>, risk: Risk, what: &str) -> Result<(), ErrorBody> {
    if caller.ui {
        return Ok(());
    }
    let decision = s.approvals.ask(Ask {
        action: action.into(),
        summary: summary.clone(),
        details,
        requester: caller.requester(),
        caller_key: caller.key(),
        risk,
    });
    match decision {
        Decision::Allowed | Decision::Remembered => Ok(()),
        Decision::Denied => Err(ErrorBody::new(code::DENIED, format!("The user didn't allow it ({summary}). {what}"))),
        Decision::TimedOut => Err(ErrorBody::new(code::APPROVAL_TIMEOUT, format!("Nobody answered the approval ({summary}) in time. {what}"))),
    }
}

/// Stopping or starting something on another machine is asked every time.
fn risk_of(a: &AgentView) -> Risk {
    if a.machine.provider == "local" {
        Risk::Low
    } else {
        Risk::High
    }
}

fn where_note(a: &AgentView) -> Option<String> {
    (a.machine.provider != "local").then(|| format!("It runs on {} ({}): this changes that machine.", a.machine.label, a.machine.provider))
}

pub fn stop(s: &Server, caller: &Caller, r: AgentRef) -> Res {
    let a = agent(s, &r.agent_id)?;
    if !a.caps.stop {
        return Err(ErrorBody::new(code::NOT_RUNNING, format!("\"{}\" isn't running", a.name)));
    }
    let mut details = vec!["Its process ends. It stays in Pitwall; restarting starts it again.".to_string()];
    details.extend(where_note(&a));
    approve(s, caller, method::AGENT_STOP, format!("stop \"{}\"", a.name), details, risk_of(&a), "It is still running.")?;
    lifecycle::stop(&s.engine, &a.id).map_err(other)?;
    // Stopping runs in the background (as the UI's button); answer once it
    // has ended, or with how it is after a while.
    let until = Instant::now() + STOP_WAIT;
    loop {
        let now = agent(s, &a.id)?;
        if !now.running || Instant::now() >= until {
            return ok(now);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// How long `agent.stop` waits to see the agent end.
const STOP_WAIT: Duration = Duration::from_secs(10);

pub fn restart(s: &Server, caller: &Caller, r: AgentRef) -> Res {
    let a = agent(s, &r.agent_id)?;
    if !a.caps.restart {
        return Err(ErrorBody::new(code::UNSUPPORTED, format!("Pitwall can't start \"{}\" again", a.name)));
    }
    let mut details = vec![if a.running {
        "It is running: its process is stopped and started again.".to_string()
    } else {
        "It starts again.".to_string()
    }];
    if a.caps.resume {
        details.push("It continues its conversation.".into());
    }
    details.extend(where_note(&a));
    approve(s, caller, method::AGENT_RESTART, format!("restart \"{}\"", a.name), details, risk_of(&a), "Nothing changed.")?;
    ok(lifecycle::restart(&s.engine, &a.id, None).map_err(other)?)
}

pub fn remove(s: &Server, caller: &Caller, r: AgentRemove) -> Res {
    let a = agent(s, &r.agent_id)?;
    if r.delete_worktree && !a.caps.remove_worktree {
        return Err(bad(format!("\"{}\" has no worktree of its own to delete", a.name)));
    }
    let mut details = vec![if a.caps.remove_keeps_session {
        format!("The session keeps running on {}; Pitwall only stops tracking it.", a.machine.label)
    } else {
        "Its process ends and it leaves Pitwall's list.".to_string()
    }];
    let summary = if r.delete_worktree {
        details.push(format!("Also deletes its worktree {} — uncommitted work there is lost.", a.cwd_display));
        format!("remove \"{}\" and delete its worktree", a.name)
    } else {
        format!("remove \"{}\" from Pitwall", a.name)
    };
    let action = if r.delete_worktree { format!("{}:worktree", method::AGENT_REMOVE) } else { method::AGENT_REMOVE.to_string() };
    approve(s, caller, &action, summary, details, Risk::High, "Nothing was removed.")?;
    lifecycle::remove(&s.engine, &a.id, r.delete_worktree).map_err(other)?;
    Ok(Value::Null)
}

pub fn rename(s: &Server, r: AgentRename) -> Res {
    agent(s, &r.agent_id)?;
    ok(s.engine.rename(&r.agent_id, &r.name).map_err(bad)?)
}

pub fn status(s: &Server, r: AgentRef) -> Res {
    agent(s, &r.agent_id)?;
    ok(diagnose::diagnose(&s.engine, &r.agent_id).map_err(other)?)
}

/// Block (this connection only) until the agent reaches a wanted status.
pub fn wait(s: &Server, r: AgentWait) -> Res {
    if r.until.is_empty() {
        return Err(bad("say which status to wait for (until: idle, blocked, done, …)"));
    }
    let timeout = r.timeout_ms.map(Duration::from_millis).unwrap_or(WAIT_DEFAULT).min(WAIT_MAX);
    let start = Instant::now();
    let first = agent(s, &r.agent_id)?;
    let mut left = !r.fresh;
    let ends = |st: Status| matches!(st, Status::Stopped | Status::Exited);
    // Not running counts as stopped/exited whatever the status says yet.
    let reached = |a: &AgentView| r.until.contains(&a.status) || (!a.running && r.until.iter().any(|u| ends(*u)));
    let state = |a: &AgentView| (a.status, a.running);
    loop {
        let a = agent(s, &r.agent_id).map_err(|_| ErrorBody::new(code::NOT_FOUND, format!("\"{}\" was removed while waiting", first.name)))?;
        left |= state(&a) != state(&first);
        if left && reached(&a) {
            return ok(WaitResult { agent: a, waited_ms: start.elapsed().as_millis() as u64 });
        }
        if (ends(a.status) || !a.running) && !r.until.iter().any(|u| ends(*u)) {
            return Err(ErrorBody::new(code::NOT_RUNNING, format!("\"{}\" is {} and won't get there", a.name, word(a.status))));
        }
        if start.elapsed() >= timeout {
            return Err(ErrorBody::new(
                code::TIMEOUT,
                format!("\"{}\" is still {} after {}s", a.name, word(a.status), timeout.as_secs()),
            ));
        }
        std::thread::sleep(WAIT_POLL);
    }
}

fn word(st: Status) -> String {
    serde_json::to_value(st).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

// ------------------------------------------------------------ queue

fn queue_of(a: AgentView) -> AgentQueue {
    AgentQueue { agent_id: a.id, name: a.name, status: a.status, auto_send: a.auto_send, items: a.queue }
}

pub fn queue_add(s: &Server, r: QueueAdd) -> Res {
    agent(s, &r.agent_id)?;
    if r.text.trim().is_empty() {
        return Err(bad("the prompt is empty"));
    }
    ok(s.engine.queue_add(&r.agent_id, r.text).map_err(other)?)
}

pub fn queue_list(s: &Server, r: QueueFilter) -> Res {
    let list: Vec<AgentQueue> = match r.agent_id {
        Some(id) => vec![queue_of(agent(s, &id)?)],
        None => s.engine.views().into_iter().filter(|a| !a.queue.is_empty()).map(queue_of).collect(),
    };
    ok(list)
}

fn item(a: &AgentView, id: Option<&str>) -> Result<String, ErrorBody> {
    let found = match id {
        Some(id) => a.queue.iter().find(|q| q.id == id),
        None => a.queue.first(),
    };
    found.map(|q| q.id.clone()).ok_or_else(|| match id {
        Some(id) => ErrorBody::new(code::NOT_FOUND, format!("\"{}\" has no queued prompt {id}", a.name)),
        None => ErrorBody::new(code::NOT_FOUND, format!("\"{}\" has nothing queued", a.name)),
    })
}

pub fn queue_remove(s: &Server, r: QueueItemRef) -> Res {
    let a = agent(s, &r.agent_id)?;
    let id = r.item_id.as_deref().ok_or_else(|| bad("which item (itemId)"))?;
    let id = item(&a, Some(id))?;
    ok(s.engine.queue_remove(&a.id, &id).map_err(other)?)
}

pub fn queue_send(s: &Server, r: QueueItemRef) -> Res {
    let a = agent(s, &r.agent_id)?;
    let id = item(&a, r.item_id.as_deref())?;
    if !a.running {
        return Err(ErrorBody::new(code::NOT_RUNNING, format!("\"{}\" isn't running", a.name)));
    }
    ok(input::queue_send_now(&s.engine, &a.id, &id).map_err(other)?)
}

// ------------------------------------------------------------ projects

fn projects(list: Vec<pitwall_core::onboarding::project_list::Project>) -> Vec<ProjectView> {
    list.into_iter().map(|p| ProjectView { path: p.path, display: p.display, is_git: p.is_git, added_at: p.added_at }).collect()
}

pub fn project_list(s: &Server) -> Res {
    ok(projects(s.engine.projects().list()))
}

pub fn project_add(s: &Server, r: ProjectPath) -> Res {
    let path = pitwall_core::onboarding::project_list::normalize(&r.path);
    if path.is_empty() || !std::path::Path::new(&path).is_dir() {
        return Err(ErrorBody::new(code::NOT_FOUND, format!("no folder {}", r.path)));
    }
    ok(projects(pitwall_core::onboarding::add_project(&s.engine, path).map_err(other)?))
}

pub fn project_remove(s: &Server, r: ProjectPath) -> Res {
    let path = pitwall_core::onboarding::project_list::normalize(&r.path);
    if !s.engine.projects().list().iter().any(|p| p.path == path) {
        return Err(ErrorBody::new(code::NOT_FOUND, format!("{} isn't one of Pitwall's projects", r.path)));
    }
    ok(projects(pitwall_core::onboarding::remove_project(&s.engine, &path).map_err(other)?))
}

// ------------------------------------------------------------ rules

fn dirs(s: &Server) -> Dirs {
    Dirs::of(s.engine.paths())
}

fn rules_of(s: &Server, id: &str) -> Option<AgentRulesInfo> {
    rules::agent_rules(&s.engine).into_iter().find(|r| r.agent_id == id).map(info)
}

fn info(r: rules::AgentRulesView) -> AgentRulesInfo {
    AgentRulesInfo {
        agent_id: r.agent_id,
        rule_set_id: r.rule_set_id,
        project_rule_set_id: r.project_rule_set_id,
        applied_at: r.applied_at,
        generated: r.generated,
        error: r.error,
        stale: r.stale,
        main_checkout: r.main_checkout && !r.main_checkout_confirmed,
    }
}

pub fn rules_list(s: &Server) -> Res {
    let d = dirs(s);
    let st = rules::status(&d);
    ok(RulesOverview {
        available: st.available,
        via: st.via.map(String::from),
        files: rules::library_files(&d)
            .into_iter()
            .map(|f| RuleFileView { id: f.id, description: f.description, targets: f.targets, source: f.source })
            .collect(),
        agents: rules::agent_rules(&s.engine).into_iter().map(info).collect(),
    })
}

fn sets(s: &Server) -> RuleSets {
    let d = dirs(s);
    RuleSets {
        sets: rules::sets(&d).into_iter().map(|x| RuleSetView { id: x.id, name: x.name, rule_ids: x.rule_ids }).collect(),
        project_defaults: rules::project_rules(&d),
    }
}

pub fn rules_sets(s: &Server) -> Res {
    ok(sets(s))
}

/// A set by id or name.
fn set_id(s: &Server, key: &str) -> Result<String, ErrorBody> {
    let all = sets(s).sets;
    all.iter()
        .find(|x| x.id == key)
        .or_else(|| all.iter().find(|x| x.name.eq_ignore_ascii_case(key)))
        .map(|x| x.id.clone())
        .ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no rule set \"{key}\" (see `pitwall rules sets`)")))
}

pub fn rules_apply(s: &Server, caller: &Caller, r: RulesApply) -> Res {
    let a = agent(s, &r.agent_id)?;
    if !a.caps.rules {
        return Err(ErrorBody::new(code::UNSUPPORTED, format!("rules can't be applied to \"{}\" (its kind or machine)", a.name)));
    }
    let set = match r.set.as_deref().map(str::trim) {
        None => None,
        Some("") | Some("none") => Some(None),
        Some(k) => Some(Some(set_id(s, k)?)),
    };
    let main = rules_of(s, &a.id).is_some_and(|i| i.main_checkout);
    if main {
        if !r.main_checkout {
            return Err(ErrorBody::new(
                code::CONFLICT,
                format!("\"{}\" works in the main checkout {}: pass --main-checkout (asks the user)", a.name, a.cwd_display),
            ));
        }
        approve(
            s,
            caller,
            method::RULES_APPLY,
            format!("write rule files into {} for \"{}\"", a.cwd_display, a.name),
            vec![
                "It works in the main checkout, not a worktree of its own: the rule files are written there.".into(),
                "Pitwall keeps them out of git status (.git/info/exclude).".into(),
            ],
            Risk::High,
            "Nothing was written.",
        )?;
    }
    if let Some(set) = set {
        rules::set_agent_rules(&s.engine, &a.id, set).map_err(other)?;
    }
    let res = rules::apply_to_agent(&s.engine, &a.id, main).map_err(other)?;
    ok(RulesApplied { agent_id: a.id, generated: res.generated, log: res.log })
}

/// Make or change a rule set. New rule ids must be in the library; ones the
/// set already has stay even when they are gone from it.
pub fn rules_set_save(s: &Server, r: RuleSetSave) -> Res {
    let d = dirs(s);
    let existing = match &r.id {
        Some(id) => Some(
            rules::sets(&d)
                .into_iter()
                .find(|x| &x.id == id)
                .ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no rule set \"{id}\" (see `pitwall rules sets`)")))?,
        ),
        None => None,
    };
    let lib = rules::library_files(&d);
    for id in &r.rule_ids {
        let kept = existing.as_ref().is_some_and(|x| x.rule_ids.contains(id));
        if !kept && !lib.iter().any(|f| &f.id == id) {
            return Err(bad(format!("no rule \"{id}\" in the library (see `pitwall rules list`)")));
        }
    }
    let set = rules::save_set(&d, SaveRuleSet { id: r.id, name: r.name, rule_ids: r.rule_ids }).map_err(bad)?;
    ok(RuleSetView { id: set.id, name: set.name, rule_ids: set.rule_ids })
}

pub fn rules_set_delete(s: &Server, caller: &Caller, r: RuleSetRef) -> Res {
    let id = set_id(s, &r.set)?;
    let all = sets(s);
    let name = all.sets.iter().find(|x| x.id == id).map(|x| x.name.clone()).unwrap_or_default();
    let projects = all.project_defaults.values().filter(|v| **v == id).count();
    let mut details = vec!["Agents keep the rule files they have until rules are re-applied.".to_string()];
    if projects > 0 {
        details.insert(0, format!("{projects} project(s) have it as their default set and lose it."));
    }
    approve(s, caller, method::RULES_SET_DELETE, format!("delete the rule set \"{name}\""), details, Risk::High, "Nothing was deleted.")?;
    rules::delete_set(&dirs(s), &id).map_err(other)?;
    ok(sets(s))
}

pub fn rules_import(s: &Server, caller: &Caller, r: RulesImport) -> Res {
    let source = r.source.trim();
    if source.is_empty() {
        return Err(bad("say where to import from"));
    }
    match r.kind.as_str() {
        "project" => {}
        "file" | "git" => {
            let what = if r.kind == "git" { "clones that repository" } else { "reads that file with `rulesync import`" };
            approve(
                s,
                caller,
                method::RULES_IMPORT,
                format!("import rules from {source}"),
                vec![
                    format!("Pitwall {what} and adds its rules to the library in its own folder; nothing else changes."),
                    "The rules reach agents only once they are in a rule set that is applied.".into(),
                ],
                Risk::High,
                "Nothing was imported.",
            )?;
        }
        other => return Err(bad(format!("{other}: import from project, file or git"))),
    }
    let res = rules::import(&dirs(s), ImportRequest { kind: r.kind, source: source.to_string() }).map_err(other)?;
    ok(RulesImported { added: res.added, log: res.log })
}

fn project_view(p: rules::ProjectRules) -> ProjectRulesView {
    ProjectRulesView {
        project: p.project,
        rule_set_id: p.rule_set_id,
        agents: p
            .agents
            .into_iter()
            .map(|a| AgentRulesDetail {
                name: a.name,
                kind_name: a.kind_name,
                target: a.target,
                rules: info(a.view),
                effective: a
                    .effective
                    .into_iter()
                    .map(|r| EffectiveRuleView { id: r.id, description: r.description, from: r.from.into(), missing: r.missing })
                    .collect(),
            })
            .collect(),
    }
}

pub fn rules_show(s: &Server, r: RulesShow) -> Res {
    match (r.project.as_deref().map(str::trim), r.agent_id.as_deref()) {
        (Some(p), None) if !p.is_empty() => ok(project_view(rules::project_detail(&s.engine, p))),
        (None, Some(id)) => {
            let a = agent(s, id)?;
            ok(project_view(rules::agent_detail(&s.engine, &a.id).map_err(other)?))
        }
        _ => Err(bad("give a project or an agent")),
    }
}

pub fn rules_default(s: &Server, r: RulesDefault) -> Res {
    let set = match r.set.as_deref().map(str::trim) {
        None | Some("") | Some("none") => None,
        Some(k) => Some(set_id(s, k)?),
    };
    if r.project.trim().is_empty() {
        return Err(bad("which project"));
    }
    rules::set_project_rules(&dirs(s), &r.project, set).map_err(other)?;
    ok(sets(s))
}

// ------------------------------------------------------------ review

pub fn review_changes(s: &Server, r: ReviewRequest) -> Res {
    let a = agent(s, &r.agent_id)?;
    if !a.caps.diff {
        return Err(ErrorBody::new(code::UNSUPPORTED, format!("\"{}\" doesn't work in a git repository Pitwall can read", a.name)));
    }
    let files = pitwall_core::review::task_changes(&s.engine, &a.id, r.task_id.as_deref()).map_err(other)?;
    let files: Vec<ChangedFile> = files
        .into_iter()
        .map(|f| ChangedFile { path: f.path, added: f.added, removed: f.removed, binary: f.binary, untracked: f.untracked, status: f.status })
        .collect();
    ok(ReviewChanges {
        agent_id: a.id,
        since: r.task_id.unwrap_or_else(|| "agent".into()),
        added: files.iter().map(|f| f.added).sum(),
        removed: files.iter().map(|f| f.removed).sum(),
        files,
    })
}
