//! Rules via rulesync (docs/spec/rules.md). Pitwall is a tool here: it keeps a
//! library and named rule sets, and calls the `rulesync` CLI to generate each
//! agent's instruction files. It never reimplements rulesync, never installs
//! it, and only writes into a main checkout after explicit confirmation.
//!
//! State lives in ~/.pitwall/rules.json (sets, project defaults, sources,
//! per-agent apply records).

mod apply;
mod library;
mod runner;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::clock::unix_ms;
use crate::engine::Engine;
use crate::exec::{self, Exec, LocalExec};
use crate::kind::{AgentKind, KindCatalog};
use crate::model::AgentRecord;
use crate::paths::{expand_tilde, Paths};
use crate::vcs::git::Git;

// ------------------------------------------------------------------ shapes

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RulesStatus {
    pub available: bool,
    /// "rulesync" | "npx" | null
    pub via: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The user allowed `npx -y rulesync` in Settings.
    pub npx_allowed: bool,
    /// `npx` exists on the login PATH.
    pub npx_found: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuleFile {
    /// `<source>:<path under rules/>`, e.g. `library:style.md`.
    pub id: String,
    pub path: String,
    pub description: Option<String>,
    pub targets: Vec<String>,
    /// rulesync `root: true` (the tool's main instruction file).
    pub root: bool,
    pub local_root: bool,
    /// "library" or an imported source's name.
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuleSet {
    pub id: String,
    pub name: String,
    pub rule_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveRuleSet {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub rule_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuleSource {
    pub name: String,
    /// "project" (read in place) | "git" (cloned into ~/.pitwall/rules-sources)
    pub kind: String,
    /// Project path or git URL.
    pub origin: String,
    /// The rulesync source tree (folder holding `rules/`).
    pub root: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    /// "project" | "file" | "git"
    pub kind: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    /// Ids of rules that are new in the library.
    pub added: Vec<String>,
    pub log: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    /// Generated paths, relative to the agent's working directory.
    pub generated: Vec<String>,
    pub log: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GenFile {
    pub path: String,
    pub hash: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRulesState {
    #[serde(default)]
    pub rule_set_id: Option<String>,
    #[serde(default)]
    pub main_checkout_confirmed: bool,
    #[serde(default)]
    pub applied_at: Option<u64>,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub files: Vec<GenFile>,
    #[serde(default)]
    pub exclude_file: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRulesView {
    pub agent_id: String,
    pub rule_set_id: Option<String>,
    /// The project default set that also applies.
    pub project_rule_set_id: Option<String>,
    pub applied_at: Option<u64>,
    pub generated: Vec<String>,
    pub error: Option<String>,
    /// Rules changed since they were applied: "Re-apply & restart".
    pub stale: bool,
    /// Agent works in the user's main checkout (no own worktree).
    pub main_checkout: bool,
    pub main_checkout_confirmed: bool,
}

// ------------------------------------------------------------------ store

#[derive(Debug, Clone)]
pub struct Dirs {
    pub base: PathBuf,
}

impl Dirs {
    /// The rules files under Pitwall's root.
    pub fn of(paths: &Paths) -> Dirs {
        Dirs { base: paths.root().to_path_buf() }
    }
    /// The library input root (~/.pitwall/rules).
    pub fn library(&self) -> PathBuf {
        self.base.join("rules")
    }
    pub fn library_rules(&self) -> PathBuf {
        self.library().join("rules")
    }
    pub fn sources(&self) -> PathBuf {
        self.base.join("rules-sources")
    }
    pub fn run(&self) -> PathBuf {
        self.base.join("run").join("rules")
    }
    fn store(&self) -> PathBuf {
        self.base.join("rules.json")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    #[serde(default)]
    pub allow_npx: bool,
    #[serde(default)]
    pub sets: Vec<RuleSet>,
    /// Project path → default rule set id.
    #[serde(default)]
    pub project_defaults: BTreeMap<String, String>,
    #[serde(default)]
    pub sources: Vec<RuleSource>,
    #[serde(default)]
    pub agents: BTreeMap<String, AgentRulesState>,
}

static LOCK: Mutex<()> = Mutex::new(());

fn load(dirs: &Dirs) -> Store {
    let Ok(src) = std::fs::read_to_string(dirs.store()) else { return Store::default() };
    serde_json::from_str(&src).unwrap_or_else(|e| {
        eprintln!("pitwall: ignoring unreadable rules.json: {e}");
        let _ = std::fs::copy(dirs.store(), dirs.store().with_extension("json.corrupt"));
        Store::default()
    })
}

fn save(dirs: &Dirs, store: &Store) -> Result<(), String> {
    std::fs::create_dir_all(&dirs.base).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    let tmp = dirs.store().with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dirs.store()).map_err(|e| e.to_string())
}

fn read(dirs: &Dirs) -> Store {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load(dirs)
}

fn update<T>(dirs: &Dirs, f: impl FnOnce(&mut Store) -> Result<T, String>) -> Result<T, String> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = load(dirs);
    let out = f(&mut store)?;
    save(dirs, &store)?;
    Ok(out)
}

/// Projects are keyed by repo root when inside git.
fn project_key(path: &str) -> String {
    let p = path.trim().trim_end_matches('/');
    // Projects are folders on this Mac.
    Git::new(&LocalExec, p).repo_root().unwrap_or_else(|| p.to_string())
}

// ------------------------------------------------------------------ core logic

/// rulesync `--targets` id for an agent kind.
pub fn target_for(kind: &AgentKind) -> Option<String> {
    kind.rulesync_target.clone().filter(|t| !t.trim().is_empty()).or_else(|| {
        match kind.id.as_str() {
            "claude" => Some("claudecode".into()),
            "codex" => Some("codexcli".into()),
            _ => None,
        }
    })
}

/// What `apply` needs to know about an agent.
#[derive(Debug, Clone)]
pub struct AgentCtx {
    pub id: String,
    pub cwd: String,
    pub project: String,
    pub worktree: bool,
    pub kind_name: String,
    pub target: Option<String>,
}

impl AgentCtx {
    pub fn from_record(rec: &AgentRecord, kinds: &KindCatalog) -> AgentCtx {
        let kind = kinds.for_record(&rec.kind, rec.custom_command.as_deref());
        AgentCtx {
            id: rec.id.clone(),
            cwd: rec.cwd.clone(),
            project: rec.project.clone(),
            worktree: rec.worktree.is_some(),
            kind_name: rec.kind_name.clone(),
            target: kind.as_ref().and_then(target_for),
        }
    }
}

fn set_ids(store: &Store, id: Option<&str>) -> Vec<String> {
    id.and_then(|id| store.sets.iter().find(|s| s.id == id))
        .map(|s| s.rule_ids.clone())
        .unwrap_or_default()
}

fn project_set(store: &Store, ctx: &AgentCtx) -> Option<String> {
    store.project_defaults.get(&ctx.project).cloned()
}

fn inputs_for(dirs: &Dirs, store: &Store, ctx: &AgentCtx, target: &str) -> apply::Inputs {
    let state = store.agents.get(&ctx.id).cloned().unwrap_or_default();
    let shared = set_ids(store, project_set(store, ctx).as_deref());
    let extra = set_ids(store, state.rule_set_id.as_deref());
    let lib = library::list(dirs, &store.sources);
    apply::collect(&lib, &shared, &extra, target, Path::new(&ctx.cwd))
}

/// Generate this agent's rule files now. Takes effect on the next session.
/// `exec`: the machine where the agent works.
pub fn apply_with(
    dirs: &Dirs,
    runner: Option<&runner::Runner>,
    exec: &dyn Exec,
    ctx: &AgentCtx,
    confirm_main_checkout: bool,
) -> Result<ApplyResult, String> {
    let store = read(dirs);
    let state = store.agents.get(&ctx.id).cloned().unwrap_or_default();
    if !(ctx.worktree || confirm_main_checkout || state.main_checkout_confirmed) {
        return Err("This agent works in your main checkout. Confirm before Pitwall writes rule files there.".into());
    }
    let cwd = ctx.cwd.as_str();
    if !exec::is_dir(exec, cwd) {
        return Err(format!("{} no longer exists", ctx.cwd));
    }
    let any_set = project_set(&store, ctx).is_some() || state.rule_set_id.is_some();
    let mut log = Vec::new();
    let (files, exclude_file, fingerprint) = if !any_set {
        // Nothing assigned: just clean up what an earlier apply wrote.
        apply::remove_generated(exec, cwd, &state.files, &mut log);
        if let Some(f) = &state.exclude_file {
            apply::write_exclude(exec, f, &ctx.id, &[])?;
        }
        log.push("No rule set assigned to this agent or its project.".into());
        (vec![], None, None)
    } else {
        let target = ctx
            .target
            .clone()
            .ok_or_else(|| format!("{} has no rulesync target (set rulesync_target in its agent definition)", ctx.kind_name))?;
        let runner = runner.ok_or("rulesync isn't available. Install it (npm install -g rulesync) or allow npx in Settings → Rules.")?;
        let inputs = inputs_for(dirs, &store, ctx, &target);
        let stage = dirs.run().join(&ctx.id);
        let out = apply::run(runner, &stage, exec, &ctx.id, cwd, &target, &inputs, &state.files)?;
        let _ = std::fs::remove_dir_all(&stage);
        log.extend(out.log);
        (out.files, out.exclude_file, Some(apply::fingerprint(&inputs, &target)))
    };
    let generated: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
    if generated.is_empty() && any_set {
        log.push("rulesync generated nothing that could be placed.".into());
    }
    update(dirs, |s| {
        let e = s.agents.entry(ctx.id.clone()).or_default();
        e.main_checkout_confirmed |= confirm_main_checkout;
        e.applied_at = Some(unix_ms());
        e.fingerprint = fingerprint;
        e.cwd = ctx.cwd.clone();
        e.files = files;
        e.exclude_file = exclude_file;
        e.error = None;
        Ok(())
    })?;
    Ok(ApplyResult { generated, log: log.join("\n") })
}

fn stale(dirs: &Dirs, store: &Store, ctx: &AgentCtx) -> bool {
    let Some(target) = &ctx.target else { return false };
    let Some(state) = store.agents.get(&ctx.id) else {
        return project_set(store, ctx).is_some() && ctx.worktree;
    };
    if !ctx.worktree && !state.main_checkout_confirmed {
        return false;
    }
    let any_set = project_set(store, ctx).is_some() || state.rule_set_id.is_some();
    match (&state.fingerprint, any_set) {
        (None, false) => !state.files.is_empty(),
        (None, true) => true,
        (Some(_), false) => true,
        (Some(fp), true) => *fp != apply::fingerprint(&inputs_for(dirs, store, ctx, target), target),
    }
}

/// Called by `create_agent` before the agent's first launch. Never fails the
/// creation: problems are kept as the agent's rules `error`.
pub fn on_create(
    dirs: &Dirs,
    kinds: &KindCatalog,
    exec: &dyn Exec,
    rec: &AgentRecord,
    kind: &AgentKind,
    rule_set_id: Option<String>,
    apply_to_main_checkout: bool,
) {
    let rule_set_id = rule_set_id.filter(|s| !s.is_empty());
    let mut ctx = AgentCtx::from_record(rec, kinds);
    ctx.target = target_for(kind);
    let wants = {
        let store = read(dirs);
        rule_set_id.is_some() || project_set(&store, &ctx).is_some()
    };
    let _ = update(dirs, |s| {
        let e = s.agents.entry(rec.id.clone()).or_default();
        e.rule_set_id = rule_set_id.clone();
        e.main_checkout_confirmed = apply_to_main_checkout && rec.worktree.is_none();
        e.cwd = rec.cwd.clone();
        Ok(())
    });
    if !wants || (rec.worktree.is_none() && !apply_to_main_checkout) {
        return;
    }
    let allow = read(dirs).allow_npx;
    let r = runner::runner(allow);
    if let Err(err) = apply_with(dirs, r.as_ref(), exec, &ctx, apply_to_main_checkout) {
        let _ = update(dirs, |s| {
            s.agents.entry(rec.id.clone()).or_default().error = Some(err.clone());
            Ok(())
        });
    }
}

/// The agent made its own worktree (worktree.rs) and now works there: write
/// its rules into it. They take effect on its next session.
pub fn on_worktree_found(dirs: &Dirs, kinds: &KindCatalog, exec: &dyn Exec, rec: &AgentRecord) {
    let ctx = AgentCtx::from_record(rec, kinds);
    let wants = {
        let store = read(dirs);
        let own = store.agents.get(&rec.id).and_then(|s| s.rule_set_id.clone());
        own.is_some() || project_set(&store, &ctx).is_some()
    };
    let _ = update(dirs, |s| {
        s.agents.entry(rec.id.clone()).or_default().cwd = rec.cwd.clone();
        Ok(())
    });
    if !wants || !ctx.worktree {
        return;
    }
    let r = runner::runner(read(dirs).allow_npx);
    let res = apply_with(dirs, r.as_ref(), exec, &ctx, false);
    let _ = update(dirs, |s| {
        s.agents.entry(rec.id.clone()).or_default().error = res.err();
        Ok(())
    });
}

/// Called when an agent is removed: delete the files Pitwall generated for it
/// (unless edited) and drop its block from info/exclude.
pub fn forget_agent(dirs: &Dirs, exec: &dyn Exec, agent_id: &str) {
    let _ = update(dirs, |s| {
        if let Some(st) = s.agents.remove(agent_id) {
            if exec::is_dir(exec, &st.cwd) {
                let mut log = vec![];
                apply::remove_generated(exec, &st.cwd, &st.files, &mut log);
            }
            if let Some(f) = &st.exclude_file {
                let _ = apply::write_exclude(exec, f, agent_id, &[]);
            }
        }
        Ok(())
    });
    let _ = std::fs::remove_dir_all(dirs.run().join(agent_id));
}

pub fn status(dirs: &Dirs) -> RulesStatus {
    let allow = read(dirs).allow_npx;
    let found = runner::detect();
    let r = runner::runner(allow);
    RulesStatus {
        available: r.is_some(),
        via: r.as_ref().map(|r| r.via),
        // Asking npx for a version could download rulesync; only ask a real install.
        version: r.as_ref().filter(|r| r.via == "rulesync").and_then(|r| r.version()),
        npx_allowed: allow,
        npx_found: found.npx.is_some(),
    }
}

// ------------------------------------------------------------------ services
// Each one is blocking (files, git, rulesync); hosts call them off the UI thread.

type Res<T> = Result<T, String>;

pub fn set_npx(dirs: &Dirs, enabled: bool) -> Res<RulesStatus> {
    update(dirs, |s| {
        s.allow_npx = enabled;
        Ok(())
    })?;
    Ok(status(dirs))
}

pub fn library_files(dirs: &Dirs) -> Vec<RuleFile> {
    let store = read(dirs);
    library::list(dirs, &store.sources)
}

/// The library's rules folder (~/.pitwall/rules/rules), created if missing,
/// for the user to edit in their own editor.
pub fn library_dir(dirs: &Dirs) -> Res<PathBuf> {
    let dir = dirs.library_rules();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub fn sets(dirs: &Dirs) -> Vec<RuleSet> {
    read(dirs).sets
}

pub fn save_set(dirs: &Dirs, set: SaveRuleSet) -> Res<RuleSet> {
    let name = set.name.trim().to_string();
    if name.is_empty() {
        return Err("name the rule set".into());
    }
    let mut ids: Vec<String> = Vec::new();
    for id in set.rule_ids {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    update(dirs, |s| {
        if s.sets.iter().any(|x| x.name.eq_ignore_ascii_case(&name) && Some(&x.id) != set.id.as_ref()) {
            return Err(format!("a rule set named \"{name}\" already exists"));
        }
        let saved = RuleSet {
            id: set.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: name.clone(),
            rule_ids: ids.clone(),
        };
        match s.sets.iter_mut().find(|x| x.id == saved.id) {
            Some(existing) => *existing = saved.clone(),
            None if set.id.is_some() => return Err("that rule set no longer exists".into()),
            None => s.sets.push(saved.clone()),
        }
        Ok(saved)
    })
}

pub fn delete_set(dirs: &Dirs, id: &str) -> Res<()> {
    update(dirs, |s| {
        s.sets.retain(|x| x.id != id);
        s.project_defaults.retain(|_, v| v.as_str() != id);
        for a in s.agents.values_mut() {
            if a.rule_set_id.as_deref() == Some(id) {
                a.rule_set_id = None;
            }
        }
        Ok(())
    })
}

pub fn import(dirs: &Dirs, req: ImportRequest) -> Res<ImportResult> {
    let source = expand(&req.source);
    match req.kind.as_str() {
        "project" | "git" => {
            let current = read(dirs).sources;
            let src = if req.kind == "project" {
                library::add_project(&source, &current)?
            } else {
                library::add_git(dirs, &req.source, &current)?
            };
            let added: Vec<String> = library::rules_in(&src.name, Path::new(&src.root)).into_iter().map(|f| f.id).collect();
            let log = format!("Added source \"{}\" ({} rules) from {}", src.name, added.len(), src.root);
            update(dirs, |s| {
                s.sources.push(src);
                Ok(())
            })?;
            Ok(ImportResult { added, log })
        }
        "file" => {
            let allow = read(dirs).allow_npx;
            let r = runner::runner(allow).ok_or("rulesync isn't available, and importing a file needs `rulesync import`.")?;
            let (names, log) = library::import_file(dirs, &r, &source)?;
            Ok(ImportResult {
                added: names.into_iter().map(|n| format!("{}:{n}", library::LIBRARY)).collect(),
                log,
            })
        }
        other => Err(format!("unknown import kind \"{other}\"")),
    }
}

fn expand(p: &str) -> String {
    expand_tilde(p.trim()).to_string_lossy().into_owned()
}

pub fn sources(dirs: &Dirs) -> Vec<RuleSource> {
    read(dirs).sources
}

pub fn pull_source(dirs: &Dirs, name: &str) -> Res<String> {
    let src = read(dirs).sources.into_iter().find(|s| s.name == name).ok_or("no such source")?;
    library::pull(dirs, &src)
}

pub fn remove_source(dirs: &Dirs, name: &str) -> Res<()> {
    update(dirs, |s| {
        if let Some(pos) = s.sources.iter().position(|x| x.name == name) {
            library::remove(dirs, &s.sources[pos])?;
            s.sources.remove(pos);
        }
        Ok(())
    })
}

pub fn set_project_rules(dirs: &Dirs, project_path: &str, rule_set_id: Option<String>) -> Res<()> {
    let key = project_key(&expand(project_path));
    update(dirs, |s| {
        match rule_set_id.filter(|id| !id.is_empty()) {
            Some(id) if !s.sets.iter().any(|x| x.id == id) => return Err("no such rule set".into()),
            Some(id) => {
                s.project_defaults.insert(key, id);
            }
            None => {
                s.project_defaults.remove(&key);
            }
        }
        Ok(())
    })
}

/// Project path (repo root) → default rule set id.
pub fn project_rules(dirs: &Dirs) -> BTreeMap<String, String> {
    read(dirs).project_defaults
}

/// The default set that would apply to a folder (resolved to its repo root).
pub fn project_rules_for(dirs: &Dirs, project_path: &str) -> Option<String> {
    let key = project_key(&expand(project_path));
    read(dirs).project_defaults.get(&key).cloned()
}

/// Generate an agent's rule files now (takes effect on its next session).
pub fn apply_to_agent(engine: &Engine, agent_id: &str, confirm_main_checkout: bool) -> Res<ApplyResult> {
    let rec = engine.with(agent_id, |a| a.rec.clone())?;
    let dirs = Dirs::of(engine.paths());
    let ctx = AgentCtx::from_record(&rec, engine.kinds());
    let r = runner::runner(read(&dirs).allow_npx);
    let res = apply_with(&dirs, r.as_ref(), &*engine.exec_for(agent_id), &ctx, confirm_main_checkout);
    if let Err(e) = &res {
        let _ = update(&dirs, |s| {
            s.agents.entry(ctx.id.clone()).or_default().error = Some(e.clone());
            Ok(())
        });
    }
    res
}

/// Change an agent's own (extra) rule set; takes effect on the next apply.
pub fn set_agent_rules(engine: &Engine, agent_id: &str, rule_set_id: Option<String>) -> Res<()> {
    engine.with(agent_id, |_| ())?;
    update(&Dirs::of(engine.paths()), |s| {
        s.agents.entry(agent_id.to_string()).or_default().rule_set_id = rule_set_id.filter(|x| !x.is_empty());
        Ok(())
    })
}

/// Rules info for every agent (stale flags drive "Re-apply & restart").
pub fn agent_rules(engine: &Engine) -> Vec<AgentRulesView> {
    let recs = engine.records();
    let dirs = Dirs::of(engine.paths());
    let store = read(&dirs);
    recs.iter()
        .map(|rec| {
            let ctx = AgentCtx::from_record(rec, engine.kinds());
            let st = store.agents.get(&rec.id).cloned().unwrap_or_default();
            AgentRulesView {
                agent_id: rec.id.clone(),
                rule_set_id: st.rule_set_id.clone(),
                project_rule_set_id: project_set(&store, &ctx),
                applied_at: st.applied_at,
                generated: st.files.iter().map(|f| f.path.clone()).collect(),
                error: st.error.clone(),
                stale: stale(&dirs, &store, &ctx),
                main_checkout: !ctx.worktree,
                main_checkout_confirmed: st.main_checkout_confirmed,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
