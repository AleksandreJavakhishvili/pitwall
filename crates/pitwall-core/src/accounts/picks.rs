//! What a project picks, and the one place it is worked out
//! (docs/spec/profiles.md).
//!
//! A **slot** is one thing an agent is launched with: a tool kind's account
//! (`claude`), the browser account ([`WEB`]), the git identity ([`GIT`]),
//! or an extension's connection ([`ext_kind`]). A project picks for a slot
//! itself (an override), through its **profile** (a named bundle of picks),
//! or not at all (the slot's Default). [`Accounts::resolve_slot`] and
//! [`Accounts::resolve_launch`] are the only places that order is applied.
//!
//! An **extension connection** is an account of kind `ext:<id>`: one set of
//! the extension's settings (`values`) and secrets (in the secure store,
//! `secretsSet` says which are set). An extension's default connection
//! (`defaults`) applies where nothing picks one.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{Account, Accounts, DEFAULT, WEB};

/// The git identity's slot.
pub const GIT: &str = "git";
/// An extension slot's "no connection".
pub const NONE: &str = "none";
/// The prefix of an extension's slot and connections' kind.
pub const EXT: &str = "ext:";

/// The slot (and connections' kind) of extension `id`: `ext:<id>`.
pub fn ext_kind(id: &str) -> String {
    format!("{EXT}{id}")
}

/// The extension whose slot `kind` is.
pub fn ext_of(kind: &str) -> Option<&str> {
    kind.strip_prefix(EXT).filter(|id| !id.is_empty())
}

/// Kind ids no agent kind may have: they are other slots' names or the
/// pick words.
pub(crate) fn reserved_kind(id: &str) -> bool {
    id == WEB || id == GIT || id == DEFAULT || id == NONE || id.contains(':')
}

/// A named bundle of picks: slot → an account id, [`DEFAULT`], or [`NONE`]
/// (extension slots). A slot it leaves out isn't decided by it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// Stable id (`profile-work`).
    pub id: String,
    /// What the user calls it ("Work").
    pub name: String,
    #[serde(default)]
    pub picks: BTreeMap<String, String>,
    /// Keys of later versions, written back untouched.
    #[serde(flatten)]
    pub rest: serde_json::Map<String, serde_json::Value>,
}

/// Where a slot's answer came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum From {
    /// The project's own pick (an override when it has a profile).
    Project,
    /// Its profile's (by id).
    Profile(String),
    /// Nothing picked: the slot's Default.
    Default,
}

/// A slot's answer for a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The account, git identity or connection; `None`: the user's own
    /// (tools, browser, git) or no connection (extensions).
    pub id: Option<String>,
    pub from: From,
}

/// How many profiles there may be (a list to pick from, not a database).
pub const MAX_PROFILES: usize = 50;

impl Accounts {
    /// The project's own pick for `slot` (`None`: it inherits).
    pub fn project_pick(&self, folder: &str, slot: &str) -> Option<&str> {
        if slot == GIT {
            return self.git_projects.get(folder).map(String::as_str);
        }
        self.projects.get(folder)?.get(slot).map(String::as_str)
    }

    /// Set the project's own pick for `slot` (`None`: inherit). Unchecked:
    /// see [`Accounts::check_pick`].
    pub fn set_project_pick(&mut self, folder: &str, slot: &str, value: Option<&str>) {
        if slot == GIT {
            match value {
                Some(v) => self.git_projects.insert(folder.to_string(), v.to_string()),
                None => self.git_projects.remove(folder),
            };
            return;
        }
        let entry = self.projects.entry(folder.to_string()).or_default();
        match value {
            Some(v) => {
                entry.insert(slot.to_string(), v.to_string());
            }
            None => {
                entry.remove(slot);
            }
        }
        self.projects.retain(|_, k| !k.is_empty());
    }

    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// A profile by id or name (any case).
    pub fn find_profile(&self, key: &str) -> Option<&Profile> {
        let key = key.trim();
        self.profile(key).or_else(|| self.profiles.iter().find(|p| p.name.eq_ignore_ascii_case(key)))
    }

    /// The profile project `folder` uses.
    pub fn profile_of(&self, folder: &str) -> Option<&Profile> {
        self.profile(self.project_profiles.get(folder)?)
    }

    /// Make `id` the project's profile (`None`: none). Its own picks stay
    /// as overrides.
    pub fn set_project_profile(&mut self, folder: &str, id: Option<&str>) -> Result<(), String> {
        match id {
            Some(id) => {
                self.profile(id).ok_or("There's no such profile")?;
                self.project_profiles.insert(folder.to_string(), id.to_string());
            }
            None => {
                self.project_profiles.remove(folder);
            }
        }
        Ok(())
    }

    /// `id` is something `slot` can name.
    fn fits(&self, slot: &str, id: &str) -> bool {
        if slot == GIT {
            return self.git_identity(id).is_some();
        }
        self.get(id).is_some_and(|a| a.kind == slot)
    }

    /// The slot's Default: an extension's default connection; the user's
    /// own (`None`) for the rest.
    fn default_of(&self, slot: &str) -> Option<String> {
        ext_of(slot)?;
        self.defaults.get(slot).filter(|id| self.fits(slot, id)).cloned()
    }

    /// What decides `slot` for `folder` before its Default: its own pick,
    /// else its profile's.
    fn decided(&self, folder: &str, slot: &str) -> Option<(&str, From)> {
        if let Some(v) = self.project_pick(folder, slot) {
            return Some((v, From::Project));
        }
        let p = self.profile_of(folder)?;
        p.picks.get(slot).map(|v| (v.as_str(), From::Profile(p.id.clone())))
    }

    /// **The** resolution of `slot` for project `folder`: its own pick,
    /// else its profile's, else the slot's Default. [`DEFAULT`] means the
    /// Default; [`NONE`] no connection (extension slots); an id that names
    /// nothing of the slot counts as Default.
    pub fn resolve_slot(&self, folder: &str, slot: &str) -> Resolved {
        let default = || Resolved { id: self.default_of(slot), from: From::Default };
        let Some((raw, from)) = self.decided(folder, slot) else { return default() };
        match raw {
            DEFAULT => Resolved { id: self.default_of(slot), from },
            NONE if ext_of(slot).is_some() => Resolved { id: None, from },
            id if self.fits(slot, id) => Resolved { id: Some(id.to_string()), from },
            _ => default(),
        }
    }

    /// The resolution for something launched in `cwd` for `project`: the
    /// folder's own answer when it decides the slot (a folder that is a
    /// project itself), else its project's.
    pub fn resolve_launch(&self, cwd: &str, project: &str, slot: &str) -> Resolved {
        if cwd != project && self.decided(cwd, slot).is_some() {
            return self.resolve_slot(cwd, slot);
        }
        self.resolve_slot(project, slot)
    }

    /// Extension `ext`'s connection for an agent in `cwd` of `project`.
    pub fn connection_for(&self, ext: &str, cwd: &str, project: &str) -> Option<&Account> {
        self.get(self.resolve_launch(cwd, project, &ext_kind(ext)).id.as_deref()?)
    }

    /// Every folder that picks something or has a profile.
    pub fn known_projects(&self) -> Vec<String> {
        let mut all: BTreeSet<&String> = self.projects.keys().collect();
        all.extend(self.git_projects.keys());
        all.extend(self.project_profiles.keys());
        all.into_iter().cloned().collect()
    }

    /// The slots worth showing for `folder`: [`GIT`], [`WEB`], every kind
    /// with an account, and whatever the folder, its profile or the
    /// defaults name.
    pub fn slots(&self, folder: &str) -> BTreeSet<String> {
        let mut s: BTreeSet<String> = [GIT.to_string(), WEB.to_string()].into();
        s.extend(self.accounts.iter().map(|a| a.kind.clone()));
        s.extend(self.projects.get(folder).into_iter().flat_map(|k| k.keys().cloned()));
        s.extend(self.profile_of(folder).into_iter().flat_map(|p| p.picks.keys().cloned()));
        s.extend(self.defaults.keys().cloned());
        s
    }

    /// Each slot's answer for `folder` ([`Accounts::slots`]).
    pub fn effective(&self, folder: &str) -> Vec<(String, Resolved)> {
        self.slots(folder).into_iter().map(|s| {
            let r = self.resolve_slot(folder, &s);
            (s, r)
        }).collect()
    }

    /// The slots whose answer for `folder` differs between `before` and
    /// `after` (what a profile change moves).
    pub fn changed_slots(before: &Accounts, after: &Accounts, folder: &str) -> Vec<String> {
        let mut slots = before.slots(folder);
        slots.extend(after.slots(folder));
        slots.into_iter().filter(|s| before.resolve_slot(folder, s).id != after.resolve_slot(folder, s).id).collect()
    }

    /// What `key` picks for `slot`, as kept: an account's (or git
    /// identity's) id found by id or name, [`DEFAULT`], or [`NONE`]
    /// (extension slots only).
    pub fn check_pick(&self, slot: &str, key: &str) -> Result<String, String> {
        let key = key.trim();
        if key.eq_ignore_ascii_case(DEFAULT) {
            return Ok(DEFAULT.into());
        }
        if key.eq_ignore_ascii_case(NONE) {
            return match ext_of(slot) {
                Some(_) => Ok(NONE.into()),
                None => Err(format!("\"none\" is only for an extension; {slot} takes an account or default")),
            };
        }
        if slot == GIT {
            return self
                .git
                .iter()
                .find(|g| g.id == key || g.name.eq_ignore_ascii_case(key))
                .map(|g| g.id.clone())
                .ok_or_else(|| format!("There's no git identity called {key}"));
        }
        self.of_kind(slot)
            .find(|a| a.id == key || a.name.eq_ignore_ascii_case(key))
            .map(|a| a.id.clone())
            .ok_or_else(|| match ext_of(slot) {
                Some(ext) => format!("{ext} has no connection called {key}"),
                None if slot == WEB => format!("There's no browser account called {key}"),
                None => format!("There's no {slot} account called {key}"),
            })
    }

    // ── Profiles ──────────────────────────────────────────────────────

    fn check_profile_name(&self, id: Option<&str>, name: &str) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Name it".into());
        }
        if name.chars().count() > 40 || name.chars().any(char::is_control) {
            return Err("A profile's name is one line of at most 40 characters".into());
        }
        if self.profiles.iter().any(|p| Some(p.id.as_str()) != id && p.name.eq_ignore_ascii_case(name)) {
            return Err(format!("There's already a profile called {name}"));
        }
        Ok(name.to_string())
    }

    /// A new, empty profile called `name`.
    pub fn add_profile(&mut self, name: &str) -> Result<Profile, String> {
        let name = self.check_profile_name(None, name)?;
        if self.profiles.len() >= MAX_PROFILES {
            return Err(format!("There can be {MAX_PROFILES} profiles at most"));
        }
        let base = format!("profile-{}", crate::slug::slug(&name));
        let mut id = base.clone();
        let mut n = 2;
        while self.profile(&id).is_some() {
            id = format!("{base}-{n}");
            n += 1;
        }
        let p = Profile { id, name, ..Default::default() };
        self.profiles.push(p.clone());
        Ok(p)
    }

    pub fn rename_profile(&mut self, id: &str, name: &str) -> Result<(), String> {
        self.profile(id).ok_or("There's no such profile")?;
        let name = self.check_profile_name(Some(id), name)?;
        if let Some(p) = self.profiles.iter_mut().find(|p| p.id == id) {
            p.name = name;
        }
        Ok(())
    }

    /// Set profile `id`'s pick for `slot` (`None`: it doesn't decide it);
    /// `value` as [`Accounts::check_pick`] keeps it.
    pub fn set_profile_pick(&mut self, id: &str, slot: &str, value: Option<&str>) -> Result<(), String> {
        let value = value.map(|v| self.check_pick(slot, v)).transpose()?;
        let p = self.profiles.iter_mut().find(|p| p.id == id).ok_or("There's no such profile")?;
        match value {
            Some(v) => {
                p.picks.insert(slot.to_string(), v);
            }
            None => {
                p.picks.remove(slot);
            }
        }
        Ok(())
    }

    /// Remove profile `id`, leaving its accounts alone. Its projects go
    /// back to no profile with what they run under kept: each slot it
    /// decided that a project didn't override becomes that project's own
    /// pick.
    pub fn remove_profile(&mut self, id: &str) {
        let Some(p) = self.profile(id).cloned() else { return };
        let projects: Vec<String> = self.project_profiles.iter().filter(|(_, x)| **x == p.id).map(|(f, _)| f.clone()).collect();
        for folder in &projects {
            for (slot, raw) in &p.picks {
                let keep = match raw.as_str() {
                    // With no profile, nothing picked is the Default already.
                    DEFAULT => false,
                    NONE => ext_of(slot).is_some(),
                    id => self.fits(slot, id),
                };
                if keep && self.project_pick(folder, slot).is_none() {
                    self.set_project_pick(folder, slot, Some(raw));
                }
            }
            self.project_profiles.remove(folder);
        }
        self.profiles.retain(|x| x.id != p.id);
    }

    /// The profiles that pick account or git identity `id`, by name.
    pub fn profiles_of(&self, id: &str) -> Vec<String> {
        self.profiles.iter().filter(|p| p.picks.values().any(|v| v == id)).map(|p| p.name.clone()).collect()
    }

    /// The projects that use profile `id`.
    pub fn profile_projects(&self, id: &str) -> Vec<String> {
        self.project_profiles.iter().filter(|(_, p)| *p == id).map(|(f, _)| f.clone()).collect()
    }

    // ── Extension connections ─────────────────────────────────────────

    /// Extension `ext`'s connections, in their order.
    pub fn connections<'a>(&'a self, ext: &str) -> impl Iterator<Item = &'a Account> + 'a {
        let kind = ext_kind(ext);
        self.accounts.iter().filter(move |a| a.kind == kind)
    }

    /// Extension `ext`'s default connection.
    pub fn default_connection(&self, ext: &str) -> Option<&Account> {
        self.get(self.default_of(&ext_kind(ext))?.as_str())
    }

    /// Make `id` the default connection of its extension slot `slot`
    /// (`None`: none).
    pub fn set_default(&mut self, slot: &str, id: Option<&str>) -> Result<(), String> {
        ext_of(slot).ok_or("Only an extension has a default connection")?;
        match id {
            Some(id) if self.fits(slot, id) => {
                self.defaults.insert(slot.to_string(), id.to_string());
            }
            Some(_) => return Err("That connection is gone".into()),
            None => {
                self.defaults.remove(slot);
            }
        }
        Ok(())
    }

    /// A new connection of extension `ext` called `name` (no settings yet);
    /// the extension's first becomes its default.
    pub fn add_connection(&mut self, ext: &str, name: &str) -> Result<Account, String> {
        crate::extensions::check_id(ext)?;
        let name = name.trim();
        let kind = ext_kind(ext);
        if name.is_empty() {
            return Err("Name it".into());
        }
        if name.chars().count() > 40 || name.chars().any(char::is_control) {
            return Err("A connection's name is one line of at most 40 characters".into());
        }
        if [DEFAULT, NONE].iter().any(|w| name.eq_ignore_ascii_case(w)) || self.of_kind(&kind).any(|a| a.name.eq_ignore_ascii_case(name)) {
            return Err(format!("There's already a connection called {name}"));
        }
        let first = self.of_kind(&kind).next().is_none();
        let id = self.free_id(&format!("ext-{ext}-{}", crate::slug::slug(name)), |_| false);
        let a = Account { id, kind: kind.clone(), name: name.into(), ..Default::default() };
        self.accounts.push(a.clone());
        if first {
            self.defaults.insert(kind, a.id.clone());
        }
        Ok(a)
    }

    fn connection_mut(&mut self, id: &str) -> Result<&mut Account, String> {
        self.accounts.iter_mut().find(|a| a.id == id && ext_of(&a.kind).is_some()).ok_or_else(|| "That connection is gone".to_string())
    }

    /// Set connection `id`'s plain setting `key` (`None`: unset).
    pub fn set_connection_value(&mut self, id: &str, key: &str, value: Option<&str>) -> Result<(), String> {
        let a = self.connection_mut(id)?;
        match value.map(str::trim).filter(|v| !v.is_empty()) {
            Some(v) => {
                a.values.insert(key.to_string(), v.to_string());
            }
            None => {
                a.values.remove(key);
            }
        }
        Ok(())
    }

    /// Record that connection `id`'s secret `key` is set, or not.
    pub fn set_connection_secret(&mut self, id: &str, key: &str, set: bool) -> Result<(), String> {
        let a = self.connection_mut(id)?;
        a.secrets_set.retain(|k| k != key);
        if set {
            a.secrets_set.push(key.to_string());
            a.secrets_set.sort();
        }
        Ok(())
    }

    /// Record that connection `id`'s setup action `action` last ran
    /// without a failure, or not.
    pub fn set_connection_action(&mut self, id: &str, action: &str, done: bool) -> Result<(), String> {
        let a = self.connection_mut(id)?;
        a.actions_done.retain(|x| x != action);
        if done {
            a.actions_done.push(action.to_string());
        }
        Ok(())
    }

    /// The connection of `ext` carried over from before connections (its
    /// secrets under the extension's id).
    pub fn carried_over(&self, ext: &str) -> Option<&Account> {
        self.connections(ext).find(|a| a.secret_key.as_deref() == Some(ext))
    }

    /// An older Pitwall (run after this one) changed the settings it keeps
    /// in `ui.json`: they go into the carried-over connection. `true` when
    /// anything changed.
    pub fn sync_carried_over(&mut self, ext: &str, values: &BTreeMap<String, String>, secrets_set: &[String], actions: &[String]) -> bool {
        let Some(id) = self.carried_over(ext).map(|a| a.id.clone()) else { return false };
        let Ok(a) = self.connection_mut(&id) else { return false };
        let values: BTreeMap<String, String> = values.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k.clone(), v.clone())).collect();
        let mut secrets = secrets_set.to_vec();
        secrets.sort();
        secrets.dedup();
        let mut acts = actions.to_vec();
        acts.sort();
        acts.dedup();
        let mut have = a.actions_done.clone();
        have.sort();
        if a.values == values && a.secrets_set == secrets && have == acts {
            return false;
        }
        a.values = values;
        a.secrets_set = secrets;
        a.actions_done = acts;
        true
    }

    /// Before connections, an extension had one set of settings (in
    /// `ui.json`): it becomes one connection called `name`, with those
    /// `values` and `secrets_set`, its secrets kept under the extension's
    /// id (where they already are), and the extension's default, so it
    /// applies wherever it applied before. `None` when the extension has a
    /// connection already (nothing to carry over).
    pub fn adopt_connection(&mut self, ext: &str, name: &str, values: &BTreeMap<String, String>, secrets_set: &[String], actions: &[String]) -> Option<Account> {
        if self.connections(ext).next().is_some() {
            return None;
        }
        let name = name.trim();
        let name = if name.is_empty() || [DEFAULT, NONE].iter().any(|w| name.eq_ignore_ascii_case(w)) { ext } else { name };
        let mut a = self.add_connection(ext, name).ok()?;
        a.values = values.iter().filter(|(_, v)| !v.trim().is_empty()).map(|(k, v)| (k.clone(), v.clone())).collect();
        a.secrets_set = secrets_set.to_vec();
        a.secrets_set.sort();
        a.secrets_set.dedup();
        a.secret_key = Some(ext.to_string());
        a.actions_done = actions.to_vec();
        if let Some(x) = self.accounts.iter_mut().find(|x| x.id == a.id) {
            *x = a.clone();
        }
        self.defaults.insert(ext_kind(ext), a.id.clone());
        Some(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{GitFields, GitIdentity};
    use crate::paths::Paths;
    use crate::testing::TempDir;

    fn acc(id: &str, kind: &str) -> Account {
        Account { id: id.into(), kind: kind.into(), name: id.into(), dir: "/x".into(), ..Default::default() }
    }

    fn git(id: &str) -> GitIdentity {
        GitIdentity { id: id.into(), name: id.into(), user_name: "S".into(), email: "s@x.example".into(), ssh_key: None, dir: "/g".into() }
    }

    /// Accounts with two of each, a profile "Work", and two connections.
    fn sample() -> Accounts {
        let mut a = Accounts::default();
        for (id, kind) in [("claude-work", "claude"), ("claude-home", "claude"), ("web-work", WEB), ("codex-work", "codex")] {
            a.accounts.push(acc(id, kind));
        }
        a.git = vec![git("git-work"), git("git-home")];
        let work = a.add_connection("inbox", "Work").unwrap();
        let home = a.add_connection("inbox", "Home").unwrap();
        assert_eq!((work.id.as_str(), home.id.as_str()), ("ext-inbox-work", "ext-inbox-home"));
        let p = a.add_profile("Work").unwrap();
        for (slot, v) in [("claude", "claude-work"), (GIT, "git-work"), (WEB, "web-work"), ("ext:inbox", "Home"), ("codex", "default")] {
            a.set_profile_pick(&p.id, slot, Some(v)).unwrap();
        }
        a
    }

    fn id(r: Resolved) -> Option<String> {
        r.id
    }

    /// Project override → its profile → the slot's Default, for every slot;
    /// `default` and `none`; an id that names nothing counts as Default.
    #[test]
    fn resolution_order_is_override_then_profile_then_default() {
        let mut a = sample();
        // No profile, no picks: the Defaults (an extension's default
        // connection: its first).
        assert_eq!(a.resolve_slot("/p", "claude"), Resolved { id: None, from: From::Default });
        assert_eq!(id(a.resolve_slot("/p", "ext:inbox")).as_deref(), Some("ext-inbox-work"));
        assert_eq!(a.resolve_slot("/p", GIT).id, None);
        // The profile.
        a.set_project_profile("/p", Some("profile-work")).unwrap();
        let from_profile = From::Profile("profile-work".into());
        assert_eq!(a.resolve_slot("/p", "claude"), Resolved { id: Some("claude-work".into()), from: from_profile.clone() });
        assert_eq!(id(a.resolve_slot("/p", GIT)).as_deref(), Some("git-work"));
        assert_eq!(id(a.resolve_slot("/p", WEB)).as_deref(), Some("web-work"));
        assert_eq!(id(a.resolve_slot("/p", "ext:inbox")).as_deref(), Some("ext-inbox-home"));
        assert_eq!(a.resolve_slot("/p", "codex"), Resolved { id: None, from: from_profile.clone() }, "default: the user's own");
        assert_eq!(a.resolve_slot("/p", "gemini").from, From::Default, "a slot it doesn't decide");
        // Overrides win, `default` and `none` included.
        a.set_project_pick("/p", "claude", Some("claude-home"));
        a.set_project_pick("/p", GIT, Some(DEFAULT));
        a.set_project_pick("/p", "ext:inbox", Some(NONE));
        assert_eq!(a.resolve_slot("/p", "claude"), Resolved { id: Some("claude-home".into()), from: From::Project });
        assert_eq!(a.resolve_slot("/p", GIT), Resolved { id: None, from: From::Project });
        assert_eq!(a.resolve_slot("/p", "ext:inbox"), Resolved { id: None, from: From::Project });
        assert_eq!(a.connection_for("inbox", "/p", "/p"), None, "none: no connection");
        // The legacy helpers go through it too.
        assert_eq!(a.project_default("/p", "claude").map(|x| x.id.as_str()), Some("claude-home"));
        assert_eq!(a.project_web("/p").map(|x| x.id.as_str()), Some("web-work"));
        assert_eq!(a.project_git("/p"), None);
        assert_eq!(a.resolve("claude", "/p", None).map(|x| x.id.as_str()), Some("claude-home"));
        assert_eq!(a.resolve("claude", "/p", Some(DEFAULT)), None);
        // An extension's `default` follows its default connection.
        a.set_project_pick("/p", "ext:inbox", Some(DEFAULT));
        assert_eq!(id(a.resolve_slot("/p", "ext:inbox")).as_deref(), Some("ext-inbox-work"));
        a.set_default("ext:inbox", None).unwrap();
        assert_eq!(a.resolve_slot("/p", "ext:inbox").id, None, "no default: none");
        // A dangling id is the Default; `none` means nothing for a tool.
        a.set_project_pick("/q", "claude", Some("claude-gone"));
        a.set_project_pick("/q", "codex", Some(NONE));
        assert_eq!(a.resolve_slot("/q", "claude"), Resolved { id: None, from: From::Default });
        assert_eq!(a.resolve_slot("/q", "codex").id, None);
        // Another kind's account is never a slot's answer.
        a.set_project_pick("/q", WEB, Some("claude-work"));
        assert_eq!(a.resolve_slot("/q", WEB).id, None);
    }

    /// A launch in a folder that decides the slot itself uses its answer;
    /// otherwise its project's.
    #[test]
    fn a_launch_asks_its_folder_then_its_project() {
        let mut a = sample();
        a.set_project_profile("/repo", Some("profile-work")).unwrap();
        assert_eq!(id(a.resolve_launch("/repo/wt", "/repo", "claude")).as_deref(), Some("claude-work"));
        a.set_project_pick("/repo/wt", "claude", Some("claude-home"));
        assert_eq!(id(a.resolve_launch("/repo/wt", "/repo", "claude")).as_deref(), Some("claude-home"));
        assert_eq!(id(a.resolve_launch("/repo/wt", "/repo", GIT)).as_deref(), Some("git-work"), "other slots: the project's");
        assert_eq!(a.resolve_git("/repo/wt", "/repo", None).map(|g| g.id.as_str()), Some("git-work"));
        assert_eq!(a.connection_for("inbox", "/repo/wt", "/repo").map(|c| c.id.as_str()), Some("ext-inbox-home"));
    }

    /// A project with no profile behaves exactly as before profiles: its
    /// picks, else the Defaults; accounts.json from before loads and saves
    /// the same.
    #[test]
    fn without_profiles_nothing_changes() {
        let old = r#"{"accounts":[{"id":"claude-work","kind":"claude","name":"Work","dir":"/x"},{"id":"web-work","kind":"web","name":"Work","dir":"/w"}],"projects":{"/p":{"claude":"claude-work","web":"web-work"}},"git":[{"id":"git-work","name":"Work","userName":"S","email":"s@x.example","dir":"/g"}],"gitProjects":{"/p":"git-work"}}"#;
        let a: Accounts = serde_json::from_str(old).unwrap();
        assert_eq!(a.project_default("/p", "claude").map(|x| x.id.as_str()), Some("claude-work"));
        assert_eq!(a.project_web("/p").map(|x| x.id.as_str()), Some("web-work"));
        assert_eq!(a.resolve_git("/p", "/p", None).map(|g| g.id.as_str()), Some("git-work"));
        assert_eq!(a.resolve_slot("/q", "claude").id, None);
        let back: serde_json::Value = serde_json::to_value(&a).unwrap();
        assert_eq!(back, serde_json::from_str::<serde_json::Value>(old).unwrap(), "written back as it was");
    }

    /// Keys a later Pitwall adds survive a load and save.
    #[test]
    fn later_keys_are_kept() {
        let text = r#"{"accounts":[{"id":"a","kind":"claude","name":"A","dir":"/x","color":"red"}],"projects":{},"profiles":[{"id":"profile-w","name":"W","picks":{},"icon":"star"}],"teams":[1]}"#;
        let a: Accounts = serde_json::from_str(text).unwrap();
        let v = serde_json::to_value(&a).unwrap();
        assert_eq!(v["teams"], serde_json::json!([1]));
        assert_eq!(v["accounts"][0]["color"], "red");
        assert_eq!(v["profiles"][0]["icon"], "star");
    }

    #[test]
    fn profiles_are_made_renamed_and_checked() {
        let mut a = sample();
        assert!(a.add_profile("work").is_err(), "names are unique");
        assert!(a.add_profile("  ").is_err());
        let p = a.add_profile("Personal").unwrap();
        assert_eq!(p.id, "profile-personal");
        a.rename_profile(&p.id, "Home").unwrap();
        assert_eq!(a.find_profile("home").map(|p| p.id.as_str()), Some("profile-personal"));
        assert!(a.rename_profile(&p.id, "WORK").is_err());
        // Picks: by name or id, only the slot's kind; none for extensions only.
        assert!(a.set_profile_pick(&p.id, "claude", Some("web-work")).is_err());
        assert!(a.set_profile_pick(&p.id, "claude", Some(NONE)).is_err());
        assert!(a.set_profile_pick(&p.id, GIT, Some("nobody")).is_err());
        a.set_profile_pick(&p.id, "claude", Some("claude-home")).unwrap();
        a.set_profile_pick(&p.id, "ext:inbox", Some("none")).unwrap();
        a.set_profile_pick(&p.id, GIT, Some("git-home")).unwrap();
        assert_eq!(a.profile(&p.id).unwrap().picks.get("ext:inbox").map(String::as_str), Some(NONE));
        a.set_profile_pick(&p.id, GIT, None).unwrap();
        assert!(!a.profile(&p.id).unwrap().picks.contains_key(GIT));
        assert!(a.set_project_profile("/p", Some("profile-gone")).is_err());
        let t = TempDir::new("profiles-save");
        let paths = Paths::new(t.path().join("pw"));
        a.save(&paths).unwrap();
        assert_eq!(Accounts::load(&paths), a);
    }

    /// Removing a profile leaves its accounts alone and keeps what its
    /// projects run under, as their own picks.
    #[test]
    fn removing_a_profile_keeps_its_projects_picks() {
        let mut a = sample();
        a.set_project_profile("/p", Some("profile-work")).unwrap();
        a.set_project_profile("/q", Some("profile-work")).unwrap();
        a.set_project_pick("/q", "claude", Some("claude-home"));
        let before = a.clone();
        a.remove_profile("profile-work");
        assert!(a.profiles.is_empty() && a.project_profiles.is_empty());
        assert_eq!(a.accounts, before.accounts, "accounts stay");
        for folder in ["/p", "/q"] {
            assert!(Accounts::changed_slots(&before, &a, folder).is_empty(), "{folder}: nothing it runs under changed");
        }
        assert_eq!(a.project_pick("/p", "claude"), Some("claude-work"));
        assert_eq!(a.project_pick("/q", "claude"), Some("claude-home"), "an override stays as it was");
        assert_eq!(a.project_pick("/p", "codex"), None, "default needs no pick without a profile");
        assert_eq!(a.project_pick("/p", GIT), Some("git-work"));
    }

    /// Removing an account, connection or git identity: projects, profiles
    /// and defaults forget it and fall back a step.
    #[test]
    fn removing_an_account_falls_back_a_step() {
        let mut a = sample();
        a.set_project_profile("/p", Some("profile-work")).unwrap();
        a.set_project_pick("/p", "claude", Some("claude-home"));
        assert_eq!(a.projects_of("claude-home"), ["/p"]);
        assert_eq!(a.projects_of("claude-work"), Vec::<String>::new(), "overridden");
        assert_eq!(a.profiles_of("claude-work"), ["Work"]);
        a.remove("claude-home");
        assert_eq!(id(a.resolve_slot("/p", "claude")).as_deref(), Some("claude-work"), "back to the profile's");
        a.remove("claude-work");
        assert_eq!(a.resolve_slot("/p", "claude"), Resolved { id: None, from: From::Default });
        assert!(!a.profile("profile-work").unwrap().picks.contains_key("claude"));
        a.remove("ext-inbox-work");
        assert!(!a.defaults.contains_key("ext:inbox"), "the default goes");
        assert_eq!(id(a.resolve_slot("/p", "ext:inbox")).as_deref(), Some("ext-inbox-home"), "the profile's still");
        assert_eq!(a.git_projects_of("git-work"), ["/p"]);
        a.remove_git("git-work");
        assert!(!a.profile("profile-work").unwrap().picks.contains_key(GIT));
        assert_eq!(a.resolve_slot("/p", GIT).id, None);
    }

    #[test]
    fn changed_slots_say_what_a_profile_change_moves() {
        let mut a = sample();
        let before = a.clone();
        a.set_project_profile("/p", Some("profile-work")).unwrap();
        let changed = Accounts::changed_slots(&before, &a, "/p");
        assert_eq!(changed, ["claude", "ext:inbox", "git", "web"]);
    }

    /// Connections: unique names per extension, their settings and which
    /// secrets are set; the first is the default; never any other kind's.
    #[test]
    fn connections_are_accounts_of_their_extension() {
        let mut a = Accounts::default();
        let w = a.add_connection("inbox", "Work").unwrap();
        assert_eq!((w.kind.as_str(), w.dir.as_str()), ("ext:inbox", ""));
        assert!(a.add_connection("inbox", "work").is_err());
        assert!(a.add_connection("inbox", "None").is_err());
        assert!(a.add_connection("Bad Id", "X").is_err());
        let other = a.add_connection("tracker", "Work").unwrap();
        assert_eq!(other.id, "ext-tracker-work");
        assert_eq!(a.default_connection("inbox").map(|c| c.id.as_str()), Some("ext-inbox-work"));
        let h = a.add_connection("inbox", "Home").unwrap();
        assert_eq!(a.default_connection("inbox").map(|c| c.id.as_str()), Some("ext-inbox-work"), "the first stays the default");
        a.set_connection_value(&h.id, "server", Some(" https://h.example ")).unwrap();
        a.set_connection_secret(&h.id, "apiKey", true).unwrap();
        assert_eq!(a.get(&h.id).unwrap().values["server"], "https://h.example");
        assert_eq!(a.get(&h.id).unwrap().secrets_name(), "ext-inbox-home");
        a.set_connection_value(&h.id, "server", Some("")).unwrap();
        assert!(a.get(&h.id).unwrap().values.is_empty());
        assert!(a.set_connection_value("claude-x", "k", Some("v")).is_err(), "only connections");
        assert!(a.set_default("claude", Some("x")).is_err());
        assert!(a.set_default("ext:inbox", Some(&other.id)).is_err(), "another extension's");
        assert_eq!(a.connections("inbox").count(), 2);
        // A tool kind can't take a slot's name.
        let t = TempDir::new("reserved-kinds");
        let paths = Paths::new(t.path().join("pw"));
        let mut kind = crate::kind::load_kinds(std::path::Path::new("/nonexistent")).into_iter().find(|k| k.id == "claude").unwrap();
        for bad in [GIT, "ext:inbox", NONE] {
            kind.id = bad.into();
            assert!(a.add(&paths, &kind, "X", None).is_err(), "{bad}");
        }
        let _ = GitFields::default();
    }

    /// An extension set up before connections becomes one connection,
    /// named after it, the default, its secrets where they were.
    #[test]
    fn an_extension_set_up_before_becomes_its_default_connection() {
        let mut a = Accounts::default();
        let values = BTreeMap::from([("server".to_string(), "https://s.example".to_string()), ("blank".to_string(), " ".to_string())]);
        let c = a.adopt_connection("inbox", "Inbox", &values, &["apiKey".to_string()], &["setup".to_string()]).unwrap();
        assert_eq!((c.id.as_str(), c.name.as_str(), c.secrets_name()), ("ext-inbox-inbox", "Inbox", "inbox"));
        assert_eq!(c.actions_done, ["setup"]);
        assert_eq!(c.values, BTreeMap::from([("server".to_string(), "https://s.example".to_string())]));
        assert_eq!(c.secrets_set, ["apiKey"]);
        assert_eq!(a.get(&c.id), Some(&c));
        assert_eq!(a.defaults["ext:inbox"], c.id);
        // Everywhere: projects named nowhere, and new ones.
        assert_eq!(a.connection_for("inbox", "/any", "/any").map(|x| x.id.as_str()), Some("ext-inbox-inbox"));
        assert_eq!(a.adopt_connection("inbox", "Inbox", &values, &[], &[]), None, "only once");
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(json["accounts"][0]["secretKey"], "inbox");
        assert_eq!(json["accounts"][0]["secretsSet"], serde_json::json!(["apiKey"]));
        assert_eq!(json["accounts"][0]["actionsDone"], serde_json::json!(["setup"]));
        // An older Pitwall changed what it keeps in ui.json: taken over;
        // the same again changes nothing.
        let newer = BTreeMap::from([("server".to_string(), "https://t.example".to_string())]);
        assert!(a.sync_carried_over("inbox", &newer, &[], &[]));
        let c = a.carried_over("inbox").unwrap();
        assert_eq!((c.values["server"].as_str(), c.secrets_set.len(), c.actions_done.len()), ("https://t.example", 0, 0));
        assert!(!a.sync_carried_over("inbox", &newer, &[], &[]));
        // A connection made later isn't the carried-over one.
        let other = a.add_connection("inbox", "Home").unwrap();
        assert_eq!(a.carried_over("inbox").map(|x| x.id.clone()), Some(c_id(&a)));
        assert_ne!(other.id, c_id(&a));
        a.set_connection_action(&other.id, "setup", true).unwrap();
        assert_eq!(a.get(&other.id).unwrap().actions_done, ["setup"]);
    }

    fn c_id(a: &Accounts) -> String {
        a.carried_over("inbox").unwrap().id.clone()
    }
}
