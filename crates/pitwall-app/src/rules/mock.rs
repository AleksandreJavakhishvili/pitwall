//! In-memory rules backend, the browser mock's (`src/rules/mock.ts`): for
//! tests and for screenshots next to the React app (debug builds,
//! `PITWALL_RULES_MOCK=1`). Never touches disk.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use pitwall_core::rules::{
    AgentRulesView, ApplyResult, ImportResult, RuleFile, RuleSet, RuleSource, RulesStatus,
    SaveRuleSet,
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
}
