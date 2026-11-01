//! The accounts (`pitwall_core::accounts`) as the app shows them: one model
//! for the window's views (Settings → Accounts, New agent's
//! account chips, the agent and project menus, the account badges), saved
//! to `accounts.json` on every change, with who each one is logged in as.
//! Git identities too (Settings → Accounts, a project's Accounts…).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui::{App, AppContext, Context, Entity, Global};

use pitwall_core::accounts::{self, Account, Accounts, GitFields, GitIdentity, GitService};
use pitwall_core::kind::AgentKind;
use pitwall_core::paths::Paths;
use pitwall_core::usage::{Usage, UsageCache};
use pitwall_core::Shared;

/// Who an account is logged in as is read again at most this often.
const LOGIN_EVERY: Duration = Duration::from_secs(3);
/// Usage is read again at most this often (while something shows it).
const USAGE_EVERY: Duration = Duration::from_secs(60);

pub struct AccountsModel {
    pub list: Accounts,
    paths: Option<Paths>,
    engine: Option<Shared>,
    /// (kind, account id or `default`) → its login's email.
    logins: BTreeMap<(String, String), Option<String>>,
    logins_at: Option<Instant>,
    /// (kind, account id or `default`) → its usage, read off the UI thread.
    usage: BTreeMap<(String, String), Usage>,
    usage_at: Option<Instant>,
    usage_reading: bool,
    /// Parsed logs, kept between reads (logs only grow).
    usage_cache: Arc<Mutex<UsageCache>>,
    /// Kind → whether its command is installed, checked once (in the
    /// background, with the first usage read).
    installed: Option<BTreeMap<String, bool>>,
}

/// Project folder → the account its agents run on (its first kind's, in
/// `accounts.json`), for what doesn't pick a login itself: a terminal in
/// that project sits under the project's account ([`project_account`]).
static PROJECTS: std::sync::RwLock<BTreeMap<String, String>> = std::sync::RwLock::new(BTreeMap::new());

fn remember(list: &Accounts) {
    let map = list
        .projects
        .iter()
        .filter_map(|(project, kinds)| Some((project.clone(), kinds.values().find(|id| list.get(id).is_some())?.clone())))
        .collect();
    if let Ok(mut p) = PROJECTS.write() {
        *p = map;
    }
}

/// Tests: projects and their accounts, as `accounts.json` would say. Kept
/// apart from [`PROJECTS`], which other tests' models replace meanwhile.
#[cfg(test)]
static TEST_PROJECTS: std::sync::RwLock<BTreeMap<String, String>> = std::sync::RwLock::new(BTreeMap::new());

#[cfg(test)]
pub fn set_project_accounts(pairs: &[(&str, &str)]) {
    if let Ok(mut p) = TEST_PROJECTS.write() {
        p.extend(pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())));
    }
}

/// The account `project`'s agents run on, when it has one.
pub fn project_account(project: &str) -> Option<String> {
    #[cfg(test)]
    if let Some(a) = TEST_PROJECTS.read().ok()?.get(project) {
        return Some(a.clone());
    }
    PROJECTS.read().ok()?.get(project).cloned()
}

struct Handle(Entity<AccountsModel>);

impl Global for Handle {}

/// The app's one model (loaded from the engine's folder on first use).
pub fn model(cx: &mut App) -> Entity<AccountsModel> {
    let engine = cx.try_global::<crate::main_screen::EngineHandle>().map(|e| e.0.clone());
    let paths = engine.as_ref().map(|e| Paths::new(e.paths().root().to_path_buf()));
    if let Some(h) = cx.try_global::<Handle>() {
        let m = h.0.clone();
        // Made before the engine was up: from its folder now.
        if m.read(cx).paths.is_none() && paths.is_some() {
            m.update(cx, |m, cx| {
                m.list = paths.as_ref().map(Accounts::load).unwrap_or_default();
                remember(&m.list);
                m.paths = paths;
                m.engine = engine;
                cx.notify();
            });
        }
        return m;
    }
    let list = paths.as_ref().map(Accounts::load).unwrap_or_default();
    remember(&list);
    let m = cx.new(|_| AccountsModel {
        list,
        paths,
        engine,
        logins: BTreeMap::new(),
        logins_at: None,
        usage: BTreeMap::new(),
        usage_at: None,
        usage_reading: false,
        usage_cache: Arc::new(Mutex::new(UsageCache::new())),
        installed: None,
    });
    cx.set_global(Handle(m.clone()));
    m
}

/// The accounts' ids in their order (empty before the model is made).
pub fn ids(cx: &App) -> Vec<String> {
    cx.try_global::<Handle>().map(|h| h.0.read(cx).list.accounts.iter().map(|a| a.id.clone()).collect()).unwrap_or_default()
}

/// The display name of account `id` (`None`: the user's own).
pub fn name_of(cx: &mut App, id: Option<&str>) -> Option<String> {
    let id = id?;
    let m = model(cx);
    let name = m.read(cx).list.get(id).map(|a| a.name.clone());
    name
}

/// The name of the account agent `a` runs on, when not the user's own:
/// another machine's agent's by the name its machine gave it (never one
/// of this Mac's).
pub fn agent_account(cx: &mut App, a: &pitwall_proto::AgentView) -> Option<String> {
    if crate::agents::mirrored(a) {
        return a.account.as_ref().map(|_| a.account_name.clone().unwrap_or_else(|| "Unnamed account".into()));
    }
    name_of(cx, a.account.as_deref())
}

impl AccountsModel {
    /// Accounts can be kept (an engine runs).
    pub fn live(&self) -> bool {
        self.paths.is_some()
    }

    /// A kind's accounts besides Default.
    pub fn of_kind(&self, kind: &str) -> Vec<Account> {
        self.list.of_kind(kind).cloned().collect()
    }

    /// The kinds that can run on more than one login (their definitions
    /// name a login folder variable), in New agent's order, installed or not.
    pub fn kinds(&self) -> Vec<AgentKind> {
        self.engine.as_ref().map(|e| e.kinds().kinds().into_iter().filter(|k| k.account_env.is_some()).collect()).unwrap_or_default()
    }

    /// The kinds shown in Settings → Accounts: installed, or with
    /// accounts already.
    /// Until that's known, the ones whose usage it can read.
    pub fn shown_kinds(&self) -> Vec<AgentKind> {
        self.kinds()
            .into_iter()
            .filter(|k| {
                let on = match &self.installed {
                    Some(i) => i.get(&k.id).copied().unwrap_or(false),
                    None => pitwall_core::usage::readable(&k.id),
                };
                on || self.list.of_kind(&k.id).next().is_some()
            })
            .collect()
    }

    /// `kind`'s usage on account `id` (`None`: the user's own) as last
    /// read; reads every account's again in the background when it's older
    /// than a minute (or `now`), and notifies when done.
    pub fn usage(&mut self, kind: &str, id: Option<&str>, now: bool, cx: &mut Context<Self>) -> Option<Usage> {
        if !self.usage_reading && (now || self.usage_at.is_none_or(|t| t.elapsed() > USAGE_EVERY)) {
            self.read_usage(cx);
        }
        self.usage.get(&(kind.to_string(), id.unwrap_or(accounts::DEFAULT).to_string())).cloned()
    }

    fn read_usage(&mut self, cx: &mut Context<Self>) {
        let mut jobs: Vec<(AgentKind, Option<Account>)> = Vec::new();
        for k in self.kinds().into_iter().filter(|k| pitwall_core::usage::readable(&k.id)) {
            jobs.push((k.clone(), None));
            jobs.extend(self.list.of_kind(&k.id).cloned().map(|a| (k.clone(), Some(a))));
        }
        self.usage_reading = true;
        self.usage_at = Some(Instant::now());
        let cache = self.usage_cache.clone();
        let check: Vec<(String, String)> =
            if self.installed.is_none() { self.kinds().into_iter().map(|k| (k.id, k.command)).collect() } else { vec![] };
        let installed = cx.background_executor().spawn(async move {
            check.into_iter().map(|(id, cmd)| (id, pitwall_core::shell::which(&cmd).is_some())).collect::<BTreeMap<_, _>>()
        });
        let task = cx.background_executor().spawn(async move {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
            let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
            jobs.into_iter()
                .filter_map(|(k, a)| {
                    let u = cache.usage(&k, a.as_ref(), now)?;
                    Some(((k.id.clone(), a.map(|a| a.id).unwrap_or_else(|| accounts::DEFAULT.into())), u))
                })
                .collect::<BTreeMap<_, _>>()
        });
        cx.spawn(async move |this, cx| {
            let usage = task.await;
            let installed = installed.await;
            let _ = this.update(cx, |m, cx| {
                if !installed.is_empty() {
                    m.installed = Some(installed);
                }
                m.usage = usage;
                m.usage_reading = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// A kind's definition, when it can have accounts.
    pub fn kind(&self, id: &str) -> Option<AgentKind> {
        self.engine.as_ref()?.kinds().find(id).filter(|k| k.account_env.is_some())
    }

    fn save(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let paths = self.paths.as_ref().ok_or("Pitwall's engine isn't running")?;
        self.list.save(paths)?;
        remember(&self.list);
        self.logins_at = None;
        cx.notify();
        Ok(())
    }

    pub fn add(&mut self, kind: &str, name: &str, dir: Option<&str>, cx: &mut Context<Self>) -> Result<Account, String> {
        let paths = self.paths.clone().ok_or("Pitwall's engine isn't running")?;
        let def = self.kind(kind).ok_or("That agent has one login only")?;
        let a = self.list.add(&paths, &def, name, dir)?;
        self.save(cx)?;
        Ok(a)
    }

    pub fn remove(&mut self, id: &str, cx: &mut Context<Self>) -> Result<(), String> {
        self.list.remove(id);
        self.save(cx)
    }

    pub fn rename(&mut self, id: &str, name: &str, cx: &mut Context<Self>) -> Result<(), String> {
        self.list.rename(id, name)?;
        self.save(cx)
    }

    /// Move account `id` before `before` (`None`: the end), as dragged.
    pub fn move_before(&mut self, id: &str, before: Option<&str>, cx: &mut Context<Self>) -> Result<(), String> {
        self.list.move_before(id, before);
        self.save(cx)
    }

    pub fn set_project_default(&mut self, project: &str, kind: &str, id: Option<&str>, cx: &mut Context<Self>) -> Result<(), String> {
        self.list.set_project_default(project, kind, id);
        self.save(cx)
    }

    /// Who `id` (`None`: the user's own) is logged in as, read from the
    /// tool's files (again after a few seconds: a login just made shows).
    pub fn login(&mut self, kind: &str, id: Option<&str>) -> Option<String> {
        if self.logins_at.is_none_or(|t| t.elapsed() > LOGIN_EVERY) {
            self.logins.clear();
            self.logins_at = Some(Instant::now());
        }
        let key = (kind.to_string(), id.unwrap_or(accounts::DEFAULT).to_string());
        if let Some(v) = self.logins.get(&key) {
            return v.clone();
        }
        let acc = id.and_then(|i| self.list.get(i)).cloned();
        let v = self.kind(kind).and_then(|def| accounts::login(&def, acc.as_ref()));
        self.logins.insert(key, v.clone());
        v
    }
}

impl AccountsModel {
    pub fn git(&self) -> Vec<GitIdentity> {
        self.list.git.clone()
    }

    pub fn add_git(&mut self, f: &GitFields, cx: &mut Context<Self>) -> Result<GitIdentity, String> {
        let paths = self.paths.clone().ok_or("Pitwall's engine isn't running")?;
        let g = self.list.add_git(&paths, f)?;
        self.save(cx)?;
        Ok(g)
    }

    pub fn edit_git(&mut self, id: &str, f: &GitFields, cx: &mut Context<Self>) -> Result<(), String> {
        self.list.edit_git(id, f)?;
        self.save(cx)
    }

    pub fn remove_git(&mut self, id: &str, cx: &mut Context<Self>) -> Result<(), String> {
        self.list.remove_git(id);
        self.save(cx)
    }

    pub fn set_project_git(&mut self, project: &str, id: Option<&str>, cx: &mut Context<Self>) -> Result<(), String> {
        self.list.set_project_git(project, id);
        self.save(cx)
    }

    /// Who git identity `id` is logged in to `service` as (read again
    /// like [`login`](Self::login)).
    pub fn git_login(&mut self, id: &str, service: GitService) -> Option<String> {
        if self.logins_at.is_none_or(|t| t.elapsed() > LOGIN_EVERY) {
            self.logins.clear();
            self.logins_at = Some(Instant::now());
        }
        let key = (format!("git:{}", service.label()), id.to_string());
        if let Some(v) = self.logins.get(&key) {
            return v.clone();
        }
        let v = self.list.git_identity(id).and_then(|g| service.login(g));
        self.logins.insert(key, v.clone());
        v
    }
}

/// A terminal with git identity `id`'s folders in the home folder where
/// `service`'s CLI logs in (in the browser).
pub fn git_login_request(id: &str, service: GitService) -> pitwall_core::model::CreateAgentRequest {
    let what = match service {
        GitService::GitHub => "github",
        GitService::AzureDevOps => "azure",
    };
    pitwall_core::model::CreateAgentRequest {
        name: format!("{what}-{}", id.trim_start_matches("git-")),
        kind: pitwall_core::kind::CUSTOM.into(),
        custom_command: Some(service.login_command().into()),
        project_path: pitwall_core::paths::home().to_string_lossy().into_owned(),
        git_identity: Some(id.into()),
        ..Default::default()
    }
}

/// The user's own `user.name` (what a new git identity starts from).
pub fn own_git_name() -> String {
    std::process::Command::new("git")
        .args(["config", "--global", "user.name"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// An agent of `kind` on account `id` in `folder`, to log in with: the
/// tool asks for its login on its first run there.
pub fn login_request(kind: &str, id: &str, folder: &str) -> pitwall_core::model::CreateAgentRequest {
    pitwall_core::model::CreateAgentRequest {
        name: format!("{kind}-{}", id.trim_start_matches(&format!("{kind}-"))),
        kind: kind.into(),
        project_path: folder.into(),
        account: Some(id.into()),
        ..Default::default()
    }
}

/// Where `kind`'s account `id` (`None`: the user's own) keeps its login,
/// as shown (`~/.claude`).
pub fn folder_of(cx: &mut App, kind: &str, id: Option<&str>) -> String {
    let m = model(cx);
    let m = m.read(cx);
    let acc = id.and_then(|i| m.list.get(i)).cloned();
    m.kind(kind)
        .and_then(|def| accounts::dir_of(&def, acc.as_ref()))
        .map(|d| pitwall_core::paths::tildify(&d.to_string_lossy()))
        .unwrap_or_default()
}
