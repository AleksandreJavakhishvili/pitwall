//! In-memory rules backend, the browser mock's (`src/rules/mock.ts`): for
//! tests and for screenshots next to the React app (debug builds,
//! `PITWALL_RULES_MOCK=1`). Never touches disk.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use pitwall_core::rules::{
    AgentRulesDetail, AgentRulesView, ApplyResult, EffectiveRule, ImportResult, ProjectRules,
    Reapplied, RuleFile, RuleSet, RuleSource, RulesStatus, SaveRuleSet,
};

use pitwall_proto::{
    GeneratedFiles, ProjectAssigned, ProjectGenerated, ProjectImported, ProjectRule,
    ProjectRuleState, ProjectRuleUpdate, ProjectRulesStatus, ProjectUnassigned, ProjectUpdated,
};

use super::RulesBackend;

type Res<T> = Result<T, String>;

#[derive(Default)]
struct State {
    status: Option<RulesStatus>,
    lib: Vec<RuleFile>,
    sets: Vec<RuleSet>,
    sources: Vec<RuleSource>,
    projects: BTreeMap<String, String>,
    agents: BTreeMap<String, AgentRulesView>,
    /// Project path → rules copied into it (name → (source, state)).
    copied: BTreeMap<String, BTreeMap<String, (String, ProjectRuleState)>>,
    /// Projects whose generated files are up to date.
    generated: Vec<String>,
    /// Projects with a hand-written CLAUDE.md not imported yet.
    hand_written: Vec<String>,
    next: u32,
}

pub struct MockRules {
    s: Mutex<State>,
    /// Listed projects for "Project defaults".
    pub listed: Vec<(String, String)>,
}

fn rule(name: &str, description: &str, root: bool, targets: &[&str]) -> RuleFile {
    RuleFile {
        id: format!("library:{name}"),
        path: format!("~/.pitwall/rules/rules/{name}"),
        description: Some(description.into()),
        targets: targets.iter().map(|t| t.to_string()).collect(),
        root,
        local_root: false,
        skill: false,
        source: "library".into(),
    }
}

impl Default for MockRules {
    fn default() -> Self {
        Self::new()
    }
}

impl MockRules {
    pub fn new() -> MockRules {
        MockRules {
            s: Mutex::new(State {
                status: Some(RulesStatus {
                    available: false,
                    via: None,
                    version: None,
                    npx_allowed: false,
                    npx_found: true,
                }),
                lib: vec![
                    rule("typescript.md", "TypeScript style", false, &["*"]),
                    rule("testing.md", "Run tests before saying done", false, &["*"]),
                    rule(
                        "overview.md",
                        "Team overview",
                        true,
                        &["claudecode", "codexcli"],
                    ),
                    rule("terse.md", "Keep answers short", false, &["*"]),
                ],
                sets: vec![RuleSet {
                    id: "s1".into(),
                    name: "Web defaults".into(),
                    rule_ids: vec!["library:typescript.md".into(), "library:testing.md".into()],
                }],
                next: 2,
                ..Default::default()
            }),
            listed: vec![
                (
                    "/Users/dev/code/orders-api".into(),
                    "~/code/orders-api".into(),
                ),
                (
                    "/Users/dev/code/checkout-web".into(),
                    "~/code/checkout-web".into(),
                ),
            ],
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A rule of `path` now in `state` (an update arrived, a local edit).
    pub fn set_project_rule(&self, path: &str, name: &str, state: ProjectRuleState) {
        if let Some(r) = self
            .lock()
            .copied
            .get_mut(path)
            .and_then(|m| m.get_mut(name))
        {
            r.1 = state;
        }
    }

    /// `path` has a hand-written, tracked CLAUDE.md.
    pub fn hand_written(&self, path: &str) {
        self.lock().hand_written.push(path.into());
    }

    /// Mark an agent's rules as changed (or failed) since its session began.
    pub fn set_agent(&self, agent_id: &str, stale: bool, error: Option<String>) {
        self.lock().agents.insert(
            agent_id.into(),
            AgentRulesView {
                agent_id: agent_id.into(),
                rule_set_id: None,
                project_rule_set_id: None,
                applied_at: None,
                generated: vec![],
                error,
                stale,
                main_checkout: false,
                main_checkout_confirmed: false,
            },
        );
    }
}

impl RulesBackend for MockRules {
    fn status(&self) -> RulesStatus {
        self.lock().status.clone().unwrap_or(RulesStatus {
            available: false,
            via: None,
            version: None,
            npx_allowed: false,
            npx_found: false,
        })
    }
    fn set_npx(&self, enabled: bool) -> Res<RulesStatus> {
        let mut s = self.lock();
        if let Some(st) = s.status.as_mut() {
            st.npx_allowed = enabled;
            st.available = enabled;
            st.via = enabled.then_some("npx");
        }
        drop(s);
        Ok(self.status())
    }
    fn library(&self) -> Vec<RuleFile> {
        self.lock().lib.clone()
    }
    fn library_dir(&self) -> Res<PathBuf> {
        Err("the mock has no rules folder".into())
    }
    fn data_dir(&self) -> String {
        "~/.pitwall".into()
    }
    fn sets(&self) -> Vec<RuleSet> {
        self.lock().sets.clone()
    }
    fn save_set(&self, set: SaveRuleSet) -> Res<RuleSet> {
        let name = set.name.trim().to_string();
        if name.is_empty() {
            return Err("name the rule set".into());
        }
        let mut s = self.lock();
        if s.sets
            .iter()
            .any(|x| x.name.eq_ignore_ascii_case(&name) && Some(&x.id) != set.id.as_ref())
        {
            return Err(format!("a rule set named \"{name}\" already exists"));
        }
        let mut ids: Vec<String> = Vec::new();
        for id in set.rule_ids {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        let id = match set.id {
            Some(id) => id,
            None => {
                s.next += 1;
                format!("s{}", s.next)
            }
        };
        let saved = RuleSet {
            id: id.clone(),
            name,
            rule_ids: ids,
        };
        match s.sets.iter_mut().find(|x| x.id == id) {
            Some(x) => *x = saved.clone(),
            None => s.sets.push(saved.clone()),
        }
        Ok(saved)
    }
    fn delete_set(&self, id: &str) -> Res<()> {
        let mut s = self.lock();
        s.sets.retain(|x| x.id != id);
        s.projects.retain(|_, v| v != id);
        Ok(())
    }
    fn import(&self, kind: &str, source: &str) -> Res<ImportResult> {
        let source = source.trim();
        if source.is_empty() {
            return Err("enter a source".into());
        }
        let available = self.status().available;
        let mut s = self.lock();
        if kind == "file" {
            if !available {
                return Err(
                    "rulesync isn't available, and importing a file needs `rulesync import`."
                        .into(),
                );
            }
            let parts: Vec<&str> = source.split('/').filter(|p| !p.is_empty()).collect();
            let name = parts.iter().rev().nth(1).copied().unwrap_or("imported");
            let id = format!("library:{name}.md");
            s.lib.push(RuleFile {
                id: id.clone(),
                path: format!("~/.pitwall/rules/rules/{name}.md"),
                description: None,
                targets: vec!["*".into()],
                root: true,
                local_root: false,
                skill: false,
                source: "library".into(),
            });
            return Ok(ImportResult {
                added: vec![id],
                log: String::new(),
            });
        }
        let trimmed = source.trim_end_matches(".git");
        let name = trimmed
            .split(['/', ':'])
            .rfind(|p| !p.is_empty())
            .unwrap_or("rules")
            .to_string();
        s.sources.push(RuleSource {
            name: name.clone(),
            kind: if kind == "git" { "git" } else { "project" }.into(),
            origin: source.into(),
            root: format!("{source}/.rulesync"),
        });
        let id = format!("{name}:shared.md");
        s.lib.push(RuleFile {
            id: id.clone(),
            path: format!("{source}/.rulesync/rules/shared.md"),
            description: Some("Shared rule".into()),
            targets: vec!["*".into()],
            root: false,
            local_root: false,
            skill: false,
            source: name.clone(),
        });
        Ok(ImportResult {
            added: vec![id],
            log: format!("Added source \"{name}\" (1 rules)"),
        })
    }
    fn sources(&self) -> Vec<RuleSource> {
        self.lock().sources.clone()
    }
    fn pull_source(&self, name: &str) -> Res<String> {
        Ok(format!("{name}: Already up to date."))
    }
    fn remove_source(&self, name: &str) -> Res<()> {
        let mut s = self.lock();
        s.sources.retain(|x| x.name != name);
        s.lib.retain(|r| r.source != name);
        Ok(())
    }
    fn set_project_rules(&self, project_path: &str, rule_set_id: Option<String>) -> Res<()> {
        let mut s = self.lock();
        match rule_set_id.filter(|x| !x.is_empty()) {
            Some(id) => {
                s.projects.insert(project_path.into(), id);
            }
            None => {
                s.projects.remove(project_path);
            }
        }
        Ok(())
    }
    fn project_rules(&self) -> BTreeMap<String, String> {
        self.lock().projects.clone()
    }
    fn project_rules_for(&self, project_path: &str) -> Option<String> {
        self.lock().projects.get(project_path).cloned()
    }
    fn projects(&self) -> Vec<(String, String)> {
        self.listed.clone()
    }
    fn apply(&self, agent_id: &str, _confirm: bool) -> Res<ApplyResult> {
        if !self.status().available {
            return Err("rulesync isn't available. Install it (npm install -g rulesync) or allow npx in Settings → Rules.".into());
        }
        let generated = vec![".claude/rules/typescript.md".to_string()];
        let mut s = self.lock();
        let e = s.agents.entry(agent_id.into()).or_insert(AgentRulesView {
            agent_id: agent_id.into(),
            rule_set_id: None,
            project_rule_set_id: None,
            applied_at: None,
            generated: vec![],
            error: None,
            stale: false,
            main_checkout: false,
            main_checkout_confirmed: false,
        });
        e.stale = false;
        e.error = None;
        e.generated = generated.clone();
        e.applied_at = Some(pitwall_core::clock::unix_ms());
        Ok(ApplyResult {
            generated,
            log: String::new(),
        })
    }
    fn agent_rules(&self) -> Vec<AgentRulesView> {
        self.lock().agents.values().cloned().collect()
    }
    /// Every agent that needs it (the mock has no projects).
    fn reapply_stale(&self, _project: Option<&str>) -> Vec<Reapplied> {
        let ids: Vec<String> = self
            .agent_rules()
            .into_iter()
            .filter(|v| v.needs_reapply())
            .map(|v| v.agent_id)
            .collect();
        ids.into_iter()
            .map(|id| match self.apply(&id, false) {
                Ok(r) => Reapplied {
                    agent_id: id,
                    generated: r.generated,
                    log: r.log,
                    error: None,
                },
                Err(e) => Reapplied {
                    agent_id: id,
                    generated: vec![],
                    log: String::new(),
                    error: Some(e),
                },
            })
            .collect()
    }
    /// The mock runs no agents.
    fn restart(&self, _agent_id: &str) -> Res<Option<pitwall_proto::AgentView>> {
        Ok(None)
    }
    fn project_status(&self, path: &str) -> Res<ProjectRulesStatus> {
        let s = self.lock();
        let copied = s.copied.get(path).cloned().unwrap_or_default();
        let rules: Vec<ProjectRule> = copied
            .iter()
            .map(|(name, (source, state))| ProjectRule {
                name: name.clone(),
                skill: source.contains(":skills/"),
                source: Some(source.clone()),
                state: *state,
                description: None,
            })
            .collect();
        let updates = rules
            .iter()
            .filter(|r| {
                matches!(
                    r.state,
                    ProjectRuleState::Update | ProjectRuleState::Conflict
                )
            })
            .count();
        let generated = s.generated.iter().any(|p| p == path);
        Ok(ProjectRulesStatus {
            project: path.into(),
            rulesync: !copied.is_empty(),
            updates,
            generated: GeneratedFiles {
                generated,
                out_of_date: !copied.is_empty() && !generated,
                changed: vec![],
                targets: vec!["claudecode".into()],
                features: vec!["skills".into()],
            },
            existing: if s.hand_written.iter().any(|p| p == path) {
                vec!["CLAUDE.md".into()]
            } else {
                vec![]
            },
            fallback_set: s.projects.get(path).cloned().filter(|_| copied.is_empty()),
            rules,
        })
    }
    fn project_assign(
        &self,
        path: &str,
        rule_ids: &[String],
        _overwrite: &[String],
    ) -> Res<ProjectAssigned> {
        let mut s = self.lock();
        let lib: Vec<String> = s.lib.iter().map(|r| r.id.clone()).collect();
        let mut out = ProjectAssigned::default();
        for id in rule_ids {
            if !lib.contains(id) {
                out.skipped.push(format!("{id}: not in the library"));
                continue;
            }
            let name = super::rule_label(id).to_string();
            s.copied
                .entry(path.into())
                .or_default()
                .insert(name.clone(), (id.clone(), ProjectRuleState::Current));
            out.copied.push(name);
        }
        s.generated.retain(|p| p != path);
        Ok(out)
    }
    fn project_unassign(
        &self,
        path: &str,
        rules: &[String],
        force: bool,
    ) -> Res<ProjectUnassigned> {
        let mut s = self.lock();
        let mut out = ProjectUnassigned::default();
        let m = s.copied.entry(path.into()).or_default();
        for key in rules {
            let name = m
                .iter()
                .find(|(n, (src, _))| *n == key || src == key)
                .map(|(n, _)| n.clone());
            match name {
                None => out.unknown.push(key.clone()),
                Some(n) if !force && m[&n].1 == ProjectRuleState::Modified => out.modified.push(n),
                Some(n) => {
                    m.remove(&n);
                    out.removed.push(n);
                }
            }
        }
        s.generated.retain(|p| p != path);
        Ok(out)
    }
    fn project_updates(&self, path: &str) -> Res<Vec<ProjectRuleUpdate>> {
        let s = self.lock();
        Ok(s.copied
            .get(path)
            .into_iter()
            .flatten()
            .filter(|(_, (_, st))| {
                matches!(
                    st,
                    ProjectRuleState::Update
                        | ProjectRuleState::Modified
                        | ProjectRuleState::Conflict
                )
            })
            .map(|(name, (source, state))| ProjectRuleUpdate {
                name: name.clone(),
                skill: source.contains(":skills/"),
                source: source.clone(),
                state: *state,
                library: Some("new\n".into()),
                project: Some("old\n".into()),
                diff: "--- project\n+++ library\n@@ -1 +1 @@\n-old\n+new\n".into(),
            })
            .collect())
    }
    fn project_update(
        &self,
        path: &str,
        rules: &[String],
        take: Option<&str>,
    ) -> Res<ProjectUpdated> {
        let mut s = self.lock();
        let mut out = ProjectUpdated::default();
        let m = s.copied.entry(path.into()).or_default();
        for (name, (source, state)) in m.iter_mut() {
            if !rules.is_empty() && !rules.iter().any(|r| r == name || r == source) {
                continue;
            }
            match (*state, take) {
                (ProjectRuleState::Update, _) | (ProjectRuleState::Conflict, Some("library")) => {
                    *state = ProjectRuleState::Current;
                    out.updated.push(name.clone());
                }
                (ProjectRuleState::Conflict, Some(_)) => {
                    *state = ProjectRuleState::Modified;
                    out.kept.push(name.clone());
                }
                (ProjectRuleState::Conflict, None) => out.conflicts.push(name.clone()),
                _ => {}
            }
        }
        if !out.updated.is_empty() {
            s.generated.retain(|p| p != path);
        }
        Ok(out)
    }
    fn project_import_existing(&self, path: &str) -> Res<ProjectImported> {
        let mut s = self.lock();
        s.hand_written.retain(|p| p != path);
        s.copied.entry(path.into()).or_default();
        Ok(ProjectImported {
            files: vec!["CLAUDE.md".into()],
            added: vec!["claude-overview.md".into()],
            log: String::new(),
        })
    }
    fn project_generate(&self, path: &str, _overwrite: &[String]) -> Res<ProjectGenerated> {
        if !self.status().available {
            return Err("rulesync isn't available.".into());
        }
        self.lock().generated.push(path.into());
        Ok(ProjectGenerated {
            written: vec!["CLAUDE.md".into()],
            targets: vec!["claudecode".into()],
            ..Default::default()
        })
    }
    fn project_move_default(&self, path: &str) -> Res<ProjectAssigned> {
        let set = self
            .lock()
            .projects
            .remove(path)
            .ok_or("this project has no default rule set in Pitwall")?;
        let ids = self
            .sets()
            .into_iter()
            .find(|x| x.id == set)
            .map(|x| x.rule_ids)
            .unwrap_or_default();
        self.project_assign(path, &ids, &[])
    }
    /// Every mock agent, as Claude Code agents of `project_path`.
    fn project_detail(&self, project_path: &str) -> ProjectRules {
        let s = self.lock();
        let default = s.projects.get(project_path).cloned();
        let rules_of = |id: Option<&String>, from: &'static str| -> Vec<EffectiveRule> {
            let ids = id
                .and_then(|id| s.sets.iter().find(|x| &x.id == id))
                .map(|x| x.rule_ids.clone())
                .unwrap_or_default();
            ids.into_iter()
                .map(|id| {
                    let f = s.lib.iter().find(|r| r.id == id);
                    EffectiveRule {
                        skill: id.contains(":skills/"),
                        description: f.and_then(|f| f.description.clone()),
                        missing: f.is_none(),
                        id,
                        from,
                    }
                })
                .collect()
        };
        let agents = s
            .agents
            .values()
            .map(|v| {
                let mut view = v.clone();
                view.project_rule_set_id = default.clone();
                let mut effective = rules_of(default.as_ref(), "project");
                effective.extend(rules_of(v.rule_set_id.as_ref(), "agent"));
                AgentRulesDetail {
                    name: v.agent_id.clone(),
                    kind_name: "Claude Code".into(),
                    target: Some("claudecode".into()),
                    effective,
                    view,
                }
            })
            .collect();
        ProjectRules {
            project: project_path.into(),
            rule_set_id: default,
            agents,
        }
    }
}
