//! The accounts (`pitwall_core::accounts`) as the app shows them: one model
//! for the window's views (Settings → Accounts, New agent's
//! account chips, the agent and project menus, the account badges), saved
//! to `accounts.json` on every change, with who each one is logged in as.
//! Git identities too (Settings → Accounts, a project's Accounts…), and
//! extension connections and profiles (docs/spec/profiles.md).
//!
//! `accounts.json` changed by someone else (`pitwall profile …` through
//! the socket, an editor) is read again within a second ([`watch`]).

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
    /// `accounts.json` as this model last wrote or read it (its stamp).
    seen: Option<(std::time::SystemTime, u64)>,
}

/// Project folder → the account its agents run on (its first kind's, in
/// `accounts.json`), for what doesn't pick a login itself: a terminal in
/// that project sits under the project's account ([`project_account`]).
static PROJECTS: std::sync::RwLock<BTreeMap<String, String>> = std::sync::RwLock::new(BTreeMap::new());

/// Project folder → the account its terminals go with: its first agent
/// kind's, its own or its profile's (a browser account, git identity or
/// extension connection is no login an agent runs on).
fn project_accounts(list: &Accounts) -> BTreeMap<String, String> {
    list.known_projects()
        .into_iter()
        .filter_map(|project| {
            let id = list.effective(&project).into_iter().filter(|(slot, _)| is_tool_kind(slot)).find_map(|(_, r)| r.id)?;
            Some((project, id))
        })
        .collect()
}

/// `slot` is an agent tool's kind (not the browser, git or an extension).
pub fn is_tool_kind(slot: &str) -> bool {
    slot != accounts::WEB && slot != accounts::GIT && accounts::ext_of(slot).is_none()
}

fn remember(list: &Accounts) {
    let map = project_accounts(list);
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
        seen: None,
    });
    cx.set_global(Handle(m.clone()));
    if m.read(cx).paths.is_some() {
        m.update(cx, |m, _| m.seen = m.paths.as_ref().and_then(|p| stamp(&file_of(p))));
        watch(&m, cx);
    }
    m
}

/// `accounts.json` in `paths`.
fn file_of(paths: &Paths) -> std::path::PathBuf {
    paths.root().join("accounts.json")
}

/// What changes when a file changes: its modification time and size.
fn stamp(path: &std::path::Path) -> Option<(std::time::SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

/// How often `accounts.json` is checked for changes made elsewhere.
const WATCH_EVERY: Duration = Duration::from_millis(700);

/// Read `accounts.json` again whenever someone else changes it.
fn watch(m: &Entity<AccountsModel>, cx: &mut App) {
    let weak = m.downgrade();
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(WATCH_EVERY).await;
        let alive = weak.update(cx, |m, cx| m.reload_if_changed(cx));
        if alive.is_err() {
            return;
        }
    })
    .detach();
}

/// The accounts as last kept (`None` before the model is made): for
/// drawing code that can't make it.
pub fn list(cx: &App) -> Option<&Accounts> {
    cx.try_global::<Handle>().map(|h| &h.0.read(cx).list)
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
        self.seen = stamp(&file_of(paths));
        remember(&self.list);
        self.logins_at = None;
        cx.notify();
        Ok(())
    }

    /// `accounts.json` changed since this model last wrote or read it:
    /// read it again.
    pub fn reload_if_changed(&mut self, cx: &mut Context<Self>) {
        let Some(paths) = self.paths.clone() else { return };
        let now = stamp(&file_of(&paths));
        if now == self.seen {
            return;
        }
        self.seen = now;
        let list = Accounts::load(&paths);
        if list != self.list {
            self.list = list;
            remember(&self.list);
            self.logins_at = None;
            cx.notify();
        }
    }

    /// Change the list with `f` and keep it: nothing changes when `f` or
    /// saving fails.
    pub fn change<T>(&mut self, f: impl FnOnce(&mut Accounts) -> Result<T, String>, cx: &mut Context<Self>) -> Result<T, String> {
        self.paths.as_ref().ok_or("Pitwall's engine isn't running")?;
        self.reload_if_changed(cx);
        let before = self.list.clone();
        let out = match f(&mut self.list) {
            Ok(v) => v,
            Err(e) => {
                self.list = before;
                return Err(e);
            }
        };
        if let Err(e) = self.save(cx) {
            self.list = before;
            return Err(e);
        }
        Ok(out)
    }

    pub fn add(&mut self, kind: &str, name: &str, dir: Option<&str>, cx: &mut Context<Self>) -> Result<Account, String> {
        let paths = self.paths.clone().ok_or("Pitwall's engine isn't running")?;
        let def = self.kind(kind).ok_or("That agent has one login only")?;
        let a = self.list.add(&paths, &def, name, dir)?;
        self.save(cx)?;
        Ok(a)
    }

    /// Add browser account `name` (`crate::web::profiles`).
    pub fn add_web(&mut self, name: &str, cx: &mut Context<Self>) -> Result<Account, String> {
        let paths = self.paths.clone().ok_or("Pitwall's engine isn't running")?;
        let a = self.list.add_web(&paths, name)?;
        self.save(cx)?;
        Ok(a)
    }

    /// Tests: accounts kept in `paths` (no engine).
    #[cfg(test)]
    pub fn set_paths(&mut self, paths: Paths) {
        self.list = Accounts::load(&paths);
        self.seen = stamp(&file_of(&paths));
        self.paths = Some(paths);
    }

    /// Forget account `id` (an extension connection's secrets are
    /// `crate::extensions::connections::remove`'s).
    pub fn remove(&mut self, id: &str, cx: &mut Context<Self>) -> Result<(), String> {
        self.change(
            |l| {
                l.remove(id);
                Ok(())
            },
            cx,
        )
    }

    pub fn rename(&mut self, id: &str, name: &str, cx: &mut Context<Self>) -> Result<(), String> {
        self.change(|l| l.rename(id, name), cx)
    }

    /// Move account `id` before `before` (`None`: the end), as dragged.
    pub fn move_before(&mut self, id: &str, before: Option<&str>, cx: &mut Context<Self>) -> Result<(), String> {
        self.change(
            |l| {
                l.move_before(id, before);
                Ok(())
            },
            cx,
        )
    }

    pub fn set_project_default(&mut self, project: &str, kind: &str, id: Option<&str>, cx: &mut Context<Self>) -> Result<(), String> {
        self.change(
            |l| {
                l.set_project_default(project, kind, id);
                Ok(())
            },
            cx,
        )
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

/// Profiles and what projects pick (docs/spec/profiles.md).
impl AccountsModel {
    pub fn add_profile(&mut self, name: &str, cx: &mut Context<Self>) -> Result<accounts::Profile, String> {
        self.change(|l| l.add_profile(name), cx)
    }

    pub fn rename_profile(&mut self, id: &str, name: &str, cx: &mut Context<Self>) -> Result<(), String> {
        self.change(|l| l.rename_profile(id, name), cx)
    }

    /// Remove profile `id`; its projects keep what they run under.
    pub fn remove_profile(&mut self, id: &str, cx: &mut Context<Self>) -> Result<(), String> {
        self.change(
            |l| {
                l.remove_profile(id);
                Ok(())
            },
            cx,
        )
    }

    /// Profile `id`'s pick for `slot` (`None`: it doesn't decide it).
    pub fn set_profile_pick(&mut self, id: &str, slot: &str, value: Option<&str>, cx: &mut Context<Self>) -> Result<(), String> {
        self.change(|l| l.set_profile_pick(id, slot, value), cx)
    }

    /// The profile `project` uses (`None`: none). What it changes for the
    /// project's running agents is the caller's ([`Accounts::changed_slots`]).
    pub fn set_project_profile(&mut self, project: &str, id: Option<&str>, cx: &mut Context<Self>) -> Result<(), String> {
        self.change(|l| l.set_project_profile(project, id), cx)
    }

    /// The project's own pick for `slot` (`None`: it inherits), checked.
    pub fn set_project_pick(&mut self, project: &str, slot: &str, value: Option<&str>, cx: &mut Context<Self>) -> Result<(), String> {
        self.change(
            |l| {
                let v = value.map(|v| l.check_pick(slot, v)).transpose()?;
                l.set_project_pick(project, slot, v.as_deref());
                Ok(())
            },
            cx,
        )
    }
}

/// Extension connections (`crate::extensions::connections` adds their
/// secrets).
impl AccountsModel {
    pub fn add_connection(&mut self, ext: &str, name: &str, cx: &mut Context<Self>) -> Result<Account, String> {
        self.change(|l| l.add_connection(ext, name), cx)
    }

    pub fn set_connection_value(&mut self, id: &str, key: &str, value: Option<&str>, cx: &mut Context<Self>) -> Result<(), String> {
        self.change(|l| l.set_connection_value(id, key, value), cx)
    }

    pub fn set_connection_secret(&mut self, id: &str, key: &str, set: bool, cx: &mut Context<Self>) -> Result<(), String> {
        self.change(|l| l.set_connection_secret(id, key, set), cx)
    }

    /// Make `id` extension `ext`'s default connection (`None`: none).
    pub fn set_default_connection(&mut self, ext: &str, id: Option<&str>, cx: &mut Context<Self>) -> Result<(), String> {
        self.change(|l| l.set_default(&accounts::ext_kind(ext), id), cx)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The sidebar grouped by account files a project's terminals under its
    /// tool's account, never under its browser account.
    #[test]
    fn a_browser_profile_is_no_projects_account() {
        let t = pitwall_core::testing::TempDir::new("app-accounts");
        let paths = Paths::new(t.path().join("pw"));
        let mut list = Accounts::default();
        let web = list.add_web(&paths, "Work").unwrap();
        list.accounts.push(Account { id: "zed-work".into(), kind: "zed".into(), name: "Work".into(), dir: "/x".into(), ..Default::default() });
        list.set_project_default("/only-web", accounts::WEB, Some(&web.id));
        list.set_project_default("/both", accounts::WEB, Some(&web.id));
        list.set_project_default("/both", "zed", Some("zed-work"));
        let map = project_accounts(&list);
        assert_eq!(map.get("/only-web"), None);
        assert_eq!(map.get("/both").map(String::as_str), Some("zed-work"));
    }

    /// A project with a profile is filed under its profile's tool account;
    /// its own pick overrides it; an extension connection is no login.
    #[test]
    fn a_profiles_tool_account_files_its_projects() {
        let mut list = Accounts::default();
        for (id, kind) in [("zed-work", "zed"), ("zed-home", "zed")] {
            list.accounts.push(Account { id: id.into(), kind: kind.into(), name: id.into(), dir: "/x".into(), ..Default::default() });
        }
        list.add_connection("inbox", "Work").unwrap();
        let p = list.add_profile("Work").unwrap();
        list.set_profile_pick(&p.id, "zed", Some("zed-work")).unwrap();
        list.set_project_profile("/p", Some(&p.id)).unwrap();
        list.set_project_profile("/q", Some(&p.id)).unwrap();
        list.set_project_pick("/q", "zed", Some("zed-home"));
        list.set_project_pick("/r", "ext:inbox", Some("none"));
        let map = project_accounts(&list);
        assert_eq!(map.get("/p").map(String::as_str), Some("zed-work"));
        assert_eq!(map.get("/q").map(String::as_str), Some("zed-home"));
        assert_eq!(map.get("/r"), None);
        assert!(is_tool_kind("claude") && !is_tool_kind("web") && !is_tool_kind("git") && !is_tool_kind("ext:inbox"));
    }
}
