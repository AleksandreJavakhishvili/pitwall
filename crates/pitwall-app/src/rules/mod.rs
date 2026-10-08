//! Rules via rulesync (inventory §21; docs/spec/rules.md): a port of
//! `src/components/rules/*` and `src/rules/*`.
//!
//! - [`settings`]: Settings → Rules (status, npx, library, import, sources,
//!   rule sets, project defaults), drawn through `settings::ExtraSections`;
//! - [`field`]: the New agent dialog's "Rules" field;
//! - [`stale`]: the shared per-agent rules poller and the pane header's
//!   "Re-apply rules & restart" button.
//!
//! Every call goes through one [`RulesBackend`]: the hosted engine's
//! `pitwall_core::rules` services (the same calls as the Tauri commands in
//! `src-tauri/src/commands/rules.rs`, same `rules.json` and library), or,
//! in debug builds and tests, the in-memory [`mock`] (`PITWALL_RULES_MOCK=1`,
//! the browser mock's data).
//!
//! Wiring: `lib.rs` calls [`init`] after `settings::init`; the pane header
//! calls [`stale::button`]; the New agent dialog owns a [`field::RulesField`].

pub mod field;
#[cfg(any(test, debug_assertions))]
pub mod mock;
pub mod settings;
pub mod stale;
mod widgets;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{App, AppContext, Context, Global};

use pitwall_core::rules::{
    self, AgentRulesView, ApplyResult, Dirs, ImportRequest, ImportResult, RuleFile, RuleSet,
    RuleSource, RulesStatus, SaveRuleSet,
};
use pitwall_core::Shared;

type Res<T> = Result<T, String>;

/// The rules services. Blocking (files, git, rulesync): call them on the
/// background executor ([`call`]).
pub trait RulesBackend: Send + Sync + 'static {
    fn status(&self) -> RulesStatus;
    fn set_npx(&self, enabled: bool) -> Res<RulesStatus>;
    fn library(&self) -> Vec<RuleFile>;
    /// The library's rules folder, created if missing.
    fn library_dir(&self) -> Res<PathBuf>;
    /// Pitwall's data folder as the UI writes it (`~/.pitwall`).
    fn data_dir(&self) -> String;
    fn sets(&self) -> Vec<RuleSet>;
    fn save_set(&self, set: SaveRuleSet) -> Res<RuleSet>;
    fn delete_set(&self, id: &str) -> Res<()>;
    fn import(&self, kind: &str, source: &str) -> Res<ImportResult>;
    fn sources(&self) -> Vec<RuleSource>;
    fn pull_source(&self, name: &str) -> Res<String>;
    fn remove_source(&self, name: &str) -> Res<()>;
    fn set_project_rules(&self, project_path: &str, rule_set_id: Option<String>) -> Res<()>;
    /// Project path (repo root) → default rule set id.
    fn project_rules(&self) -> BTreeMap<String, String>;
    /// The default set that would apply to a folder.
    fn project_rules_for(&self, project_path: &str) -> Option<String>;
    /// Listed projects, then recent ones: (path, display).
    fn projects(&self) -> Vec<(String, String)>;
    fn apply(&self, agent_id: &str, confirm_main_checkout: bool) -> Res<ApplyResult>;
    fn agent_rules(&self) -> Vec<AgentRulesView>;
}

/// The hosted engine (`src-tauri/src/commands/rules.rs`).
pub struct EngineRules(pub Shared);

impl EngineRules {
    fn dirs(&self) -> Dirs {
        Dirs::of(self.0.paths())
    }
}

impl RulesBackend for EngineRules {
    fn status(&self) -> RulesStatus {
        rules::status(&self.dirs())
    }
    fn set_npx(&self, enabled: bool) -> Res<RulesStatus> {
        rules::set_npx(&self.dirs(), enabled)
    }
    fn library(&self) -> Vec<RuleFile> {
        rules::library_files(&self.dirs())
    }
    fn library_dir(&self) -> Res<PathBuf> {
        rules::library_dir(&self.dirs())
    }
    fn data_dir(&self) -> String {
        pitwall_core::paths::tildify(&self.0.paths().root().to_string_lossy())
    }
    fn sets(&self) -> Vec<RuleSet> {
        rules::sets(&self.dirs())
    }
    fn save_set(&self, set: SaveRuleSet) -> Res<RuleSet> {
        rules::save_set(&self.dirs(), set)
    }
    fn delete_set(&self, id: &str) -> Res<()> {
        rules::delete_set(&self.dirs(), id)
    }
    fn import(&self, kind: &str, source: &str) -> Res<ImportResult> {
        rules::import(
            &self.dirs(),
            ImportRequest {
                kind: kind.into(),
                source: source.into(),
            },
        )
    }
    fn sources(&self) -> Vec<RuleSource> {
        rules::sources(&self.dirs())
    }
    fn pull_source(&self, name: &str) -> Res<String> {
        rules::pull_source(&self.dirs(), name)
    }
    fn remove_source(&self, name: &str) -> Res<()> {
        rules::remove_source(&self.dirs(), name)
    }
    fn set_project_rules(&self, project_path: &str, rule_set_id: Option<String>) -> Res<()> {
        rules::set_project_rules(&self.dirs(), project_path, rule_set_id)
    }
    fn project_rules(&self) -> BTreeMap<String, String> {
        rules::project_rules(&self.dirs())
    }
    fn project_rules_for(&self, project_path: &str) -> Option<String> {
        rules::project_rules_for(&self.dirs(), project_path)
    }
    fn projects(&self) -> Vec<(String, String)> {
        let listed = self
            .0
            .projects()
            .list()
            .into_iter()
            .map(|p| (p.path, p.display));
        let recent = pitwall_core::onboarding::recent::recent_projects(self.0.paths(), 30)
            .into_iter()
            .map(|p| (p.path, p.display));
        listed.chain(recent).collect()
    }
    fn apply(&self, agent_id: &str, confirm_main_checkout: bool) -> Res<ApplyResult> {
        rules::apply_to_agent(&self.0, agent_id, confirm_main_checkout)
    }
    fn agent_rules(&self) -> Vec<AgentRulesView> {
        rules::agent_rules(&self.0)
    }
}

/// The app's rules backend (absent when no engine is hosted).
#[derive(Clone)]
pub struct Rules(pub Arc<dyn RulesBackend>);

impl Global for Rules {}

/// The backend, if any.
pub fn backend(cx: &App) -> Option<Arc<dyn RulesBackend>> {
    cx.try_global::<Rules>().map(|r| r.0.clone())
}

/// Debug builds: `PITWALL_RULES_MOCK=1` uses the browser mock's data (for
/// screenshots next to the React app).
fn wants_mock() -> bool {
    cfg!(debug_assertions) && std::env::var_os("PITWALL_RULES_MOCK").is_some()
}

/// Once at start, after `settings::init`: the backend, the Settings
/// section and the shared agent-rules poller.
pub fn init(cx: &mut App, engine: Option<Shared>) {
    let backend: Option<Arc<dyn RulesBackend>> = if wants_mock() {
        #[cfg(debug_assertions)]
        {
            Some(Arc::new(mock::MockRules::new()))
        }
        #[cfg(not(debug_assertions))]
        {
            None
        }
    } else {
        engine.map(|e| Arc::new(EngineRules(e)) as Arc<dyn RulesBackend>)
    };
    let Some(backend) = backend else {
        return;
    };
    install(cx, backend);
}

/// Use `backend` (tests and the mock).
pub fn install(cx: &mut App, backend: Arc<dyn RulesBackend>) {
    cx.set_global(Rules(backend));
    if !cx.has_global::<crate::settings::ExtraSections>() {
        cx.set_global(crate::settings::ExtraSections::default());
    }
    cx.global_mut::<crate::settings::ExtraSections>()
        .0
        .push(settings::section);
    stale::start(cx);
}

/// Run `f` on the background executor with the backend, then `then` on the
/// view (skipped when the view is gone or there is no backend).
pub fn call<V: 'static, T: Send + 'static>(
    cx: &mut Context<V>,
    f: impl FnOnce(&dyn RulesBackend) -> T + Send + 'static,
    then: impl FnOnce(&mut V, T, &mut Context<V>) + 'static,
) {
    let Some(b) = backend(cx) else {
        return;
    };
    let task = cx.background_executor().spawn(async move { f(&*b) });
    cx.spawn(async move |this, cx| {
        let r = task.await;
        let _ = this.update(cx, |v, cx| then(v, r, cx));
    })
    .detach();
}

/// After New agent: rule problems never block the agent, so they come back
/// as an error for a toast ("Couldn't apply rules: …").
pub fn error_after_create(cx: &mut App, agent_id: String) -> Option<gpui::Task<Option<String>>> {
    let b = backend(cx)?;
    stale::refresh(cx);
    Some(cx.background_spawn(async move {
        b.agent_rules()
            .into_iter()
            .find(|r| r.agent_id == agent_id)
            .and_then(|r| r.error)
    }))
}

/// "library:web/style.md" → "web/style.md" (`ruleLabel`).
pub fn rule_label(id: &str) -> &str {
    match id.find(':') {
        Some(i) => &id[i + 1..],
        None => id,
    }
}
