//! The accounts (`pitwall_core::accounts`) as the app shows them: one model
//! for the window's views (Settings → Agents → Accounts, New agent's
//! account chips, the agent and project menus, the account badges), saved
//! to `accounts.json` on every change, with who each one is logged in as.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use gpui::{App, AppContext, Context, Entity, Global};

use pitwall_core::accounts::{self, Account, Accounts};
use pitwall_core::kind::AgentKind;
use pitwall_core::paths::Paths;
use pitwall_core::Shared;

/// Who an account is logged in as is read again at most this often.
const LOGIN_EVERY: Duration = Duration::from_secs(3);

pub struct AccountsModel {
    pub list: Accounts,
    paths: Option<Paths>,
    engine: Option<Shared>,
    /// (kind, account id or `default`) → its login's email.
    logins: BTreeMap<(String, String), Option<String>>,
    logins_at: Option<Instant>,
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
                m.paths = paths;
                m.engine = engine;
                cx.notify();
            });
        }
        return m;
    }
    let list = paths.as_ref().map(Accounts::load).unwrap_or_default();
    let m = cx.new(|_| AccountsModel { list, paths, engine, logins: BTreeMap::new(), logins_at: None });
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

    /// A kind's definition, when it can have accounts.
    pub fn kind(&self, id: &str) -> Option<AgentKind> {
        self.engine.as_ref()?.kinds().find(id).filter(|k| k.account_env.is_some())
    }

    fn save(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        let paths = self.paths.as_ref().ok_or("Pitwall's engine isn't running")?;
        self.list.save(paths)?;
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
