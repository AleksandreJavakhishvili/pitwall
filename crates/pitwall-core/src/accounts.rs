//! More than one login per agent kind: a kind whose definition names the
//! variable for its login folder (`account_env`, e.g. Claude Code's
//! `CLAUDE_CONFIG_DIR`) can run on any of several; an account is a name and
//! such a folder. Each kind has an implicit "Default" account (the user's
//! own, `account_home`). Agents remember theirs
//! (`AgentRecord::account`) and run with its folder in their environment;
//! a project can name the account its new agents use. Kept in
//! `~/.pitwall/accounts.json`. Local agents only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::kind::AgentKind;
use crate::paths::Paths;

/// The implicit account of every kind: the user's own login.
pub const DEFAULT: &str = "default";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// Stable id (`claude-work`).
    pub id: String,
    /// `claude` or `codex`.
    pub kind: String,
    /// What the user calls it ("Work").
    pub name: String,
    /// Its config folder.
    pub dir: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accounts {
    #[serde(default)]
    pub accounts: Vec<Account>,
    /// Project folder → kind → account id its new agents use.
    #[serde(default)]
    pub projects: BTreeMap<String, BTreeMap<String, String>>,
}

/// A kind's default login folder (`account_home`; `None`: no accounts).
pub fn default_dir(kind: &AgentKind) -> Option<PathBuf> {
    kind.account_home.as_deref().map(expand)
}

fn file(paths: &Paths) -> PathBuf {
    paths.root().join("accounts.json")
}

impl Accounts {
    pub fn load(paths: &Paths) -> Accounts {
        std::fs::read(file(paths)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, paths: &Paths) -> Result<(), String> {
        let path = file(paths);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    pub fn get(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    /// An account of `kind` by id or name (any case); `default` (or the
    /// kind's own name for it) → [`DEFAULT`].
    pub fn find(&self, kind: &str, key: &str) -> Option<String> {
        let key = key.trim();
        if key.eq_ignore_ascii_case(DEFAULT) {
            return Some(DEFAULT.into());
        }
        self.of_kind(kind).find(|a| a.id == key || a.name.eq_ignore_ascii_case(key)).map(|a| a.id.clone())
    }

    /// A kind's accounts besides Default.
    pub fn of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a Account> + 'a {
        self.accounts.iter().filter(move |a| a.kind == kind)
    }

    /// Add one. `dir`: a folder already logged in elsewhere; `None` makes a
    /// fresh one under `~/.pitwall/accounts/`.
    pub fn add(&mut self, paths: &Paths, kind: &AgentKind, name: &str, dir: Option<&str>) -> Result<Account, String> {
        let name = name.trim();
        if kind.account_env.is_none() {
            return Err(format!("{} has one login only", kind.name));
        }
        let kind = kind.id.as_str();
        if name.is_empty() {
            return Err("Name it".into());
        }
        if name.eq_ignore_ascii_case("default") || self.of_kind(kind).any(|a| a.name.eq_ignore_ascii_case(name)) {
            return Err(format!("There's already an account called {name}"));
        }
        let base = format!("{kind}-{}", crate::slug::slug(name));
        let mut id = base.clone();
        let mut n = 2;
        while self.get(&id).is_some() {
            id = format!("{base}-{n}");
            n += 1;
        }
        let dir = match dir.map(str::trim).filter(|d| !d.is_empty()) {
            Some(d) => expand(d),
            None => paths.root().join("accounts").join(&id),
        };
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let a = Account { id, kind: kind.into(), name: name.into(), dir: dir.to_string_lossy().into_owned() };
        self.accounts.push(a.clone());
        Ok(a)
    }

    /// Forget one (its folder and login stay on disk) and the projects
    /// that named it.
    pub fn remove(&mut self, id: &str) {
        self.accounts.retain(|a| a.id != id);
        for kinds in self.projects.values_mut() {
            kinds.retain(|_, a| a != id);
        }
        self.projects.retain(|_, k| !k.is_empty());
    }

    pub fn rename(&mut self, id: &str, name: &str) -> Result<(), String> {
        let name = name.trim();
        let kind = self.get(id).map(|a| a.kind.clone()).ok_or("No such account")?;
        if name.is_empty() {
            return Err("Name it".into());
        }
        if self.of_kind(&kind).any(|a| a.id != id && a.name.eq_ignore_ascii_case(name)) {
            return Err(format!("There's already an account called {name}"));
        }
        if let Some(a) = self.accounts.iter_mut().find(|a| a.id == id) {
            a.name = name.into();
        }
        Ok(())
    }

    /// The account a project's new agents of `kind` use (`None`: Default).
    pub fn project_default(&self, project: &str, kind: &str) -> Option<&Account> {
        let id = self.projects.get(project)?.get(kind)?;
        self.get(id)
    }

    /// Name the account a project's new agents of `kind` use (`None` or
    /// [`DEFAULT`]: Default).
    pub fn set_project_default(&mut self, project: &str, kind: &str, id: Option<&str>) {
        let entry = self.projects.entry(project.to_string()).or_default();
        match id.filter(|i| *i != DEFAULT) {
            Some(i) => {
                entry.insert(kind.to_string(), i.to_string());
            }
            None => {
                entry.remove(kind);
            }
        }
        self.projects.retain(|_, k| !k.is_empty());
    }

    /// The account a new agent uses: the one asked for ([`DEFAULT`]: the
    /// user's own), else the project's, else Default (`None`).
    pub fn resolve(&self, kind: &str, project: &str, asked: Option<&str>) -> Option<&Account> {
        match asked {
            Some(DEFAULT) => None,
            Some(id) => self.get(id).filter(|a| a.kind == kind),
            None => self.project_default(project, kind),
        }
    }

    /// Projects that name `id`.
    pub fn projects_of(&self, id: &str) -> Vec<String> {
        self.projects.iter().filter(|(_, k)| k.values().any(|a| a == id)).map(|(p, _)| p.clone()).collect()
    }
}

/// `~/x` → the home folder's `x`.
fn expand(dir: &str) -> PathBuf {
    match dir.strip_prefix("~/") {
        Some(rest) => crate::paths::home().join(rest),
        None if dir == "~" => crate::paths::home(),
        None => PathBuf::from(dir),
    }
}

/// The environment an agent of `kind` on `account` runs with.
pub fn env(kind: &AgentKind, account: Option<&Account>) -> Vec<(String, String)> {
    match (&kind.account_env, account) {
        (Some(var), Some(a)) => vec![(var.clone(), a.dir.clone())],
        _ => vec![],
    }
}

/// The login folder of `account` (`None`: the kind's default one).
pub fn dir_of(kind: &AgentKind, account: Option<&Account>) -> Option<PathBuf> {
    match account {
        Some(a) => Some(PathBuf::from(&a.dir)),
        None => default_dir(kind),
    }
}

/// Who a login folder is logged in as (an email), where Pitwall can read
/// the tool's files: Claude Code's `.claude.json` (`oauthAccount`), Codex's
/// `auth.json` (the id token's claims). `None`: not logged in, an API key,
/// or a tool whose login it can't read.
pub fn login(kind: &AgentKind, account: Option<&Account>) -> Option<String> {
    match kind.id.as_str() {
        "claude" => {
            // Its settings file: in the folder when named, else next to it.
            let file = match account {
                Some(a) => PathBuf::from(&a.dir).join(".claude.json"),
                None => crate::paths::home().join(".claude.json"),
            };
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
            v.get("oauthAccount")?.get("emailAddress")?.as_str().map(String::from)
        }
        "codex" => {
            let file = dir_of(kind, account)?.join("auth.json");
            let v: serde_json::Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
            let token = v.get("tokens")?.get("id_token")?.as_str()?;
            let claims = token.split('.').nth(1).and_then(base64url)?;
            let c: serde_json::Value = serde_json::from_slice(&claims).ok()?;
            c.get("email")?.as_str().map(String::from)
        }
        _ => None,
    }
}

/// Unpadded base64url (a JWT's parts).
fn base64url(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        } as u32)
    };
    let bytes: Vec<u8> = s.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= val(c)? << (18 - 6 * i);
        }
        let take = match chunk.len() {
            4 => 3,
            3 => 2,
            2 => 1,
            _ => return None,
        };
        out.extend_from_slice(&n.to_be_bytes()[1..1 + take]);
    }
    Some(out)
}

/// Make conversation `session_id` (of `kind`, started in `cwd`) readable
/// from config folder `to`, copying it from `from`: Claude Code keeps it in
/// `projects/<cwd as a name>/<id>.jsonl` (and a folder `<id>/` beside it),
/// Codex in a dated `sessions/…/rollout-…-<id>.jsonl`. Only newer over
/// older: a copy already there that is as new or newer is kept. Nothing to
/// copy: done. So `--resume` / `resume` finds it after a switch.
pub fn carry_conversation(kind: &str, from: &Path, to: &Path, cwd: &str, session_id: &str) -> Result<(), String> {
    if from == to || session_id.is_empty() {
        return Ok(());
    }
    let rels: Vec<PathBuf> = match kind {
        "claude" => {
            let dir = PathBuf::from("projects").join(crate::onboarding::latest::claude_project_dir(&crate::onboarding::project_list::normalize(cwd)));
            vec![dir.join(format!("{session_id}.jsonl")), dir.join(session_id)]
        }
        "codex" => match find_codex(from, session_id) {
            Some(rel) => vec![rel],
            None => return Ok(()),
        },
        _ => return Ok(()),
    };
    for rel in rels {
        copy_newer(&from.join(&rel), &to.join(&rel)).map_err(|e| format!("Couldn't carry the conversation over: {e}"))?;
    }
    Ok(())
}

fn modified(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// `src` (a file, or a folder of them) to `dst`, file by file, where `dst`
/// is missing or older.
fn copy_newer(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)?.flatten() {
            copy_newer(&e.path(), &dst.join(e.file_name()))?;
        }
        return Ok(());
    }
    if !src.is_file() {
        return Ok(());
    }
    if let (Some(a), Some(b)) = (modified(src), modified(dst)) {
        if b >= a {
            return Ok(());
        }
    }
    if let Some(dir) = dst.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::copy(src, dst).map(|_| ())
}

/// Before an agent on `account` picks up conversation `session_id`: the
/// newest copy of it among the kind's login folders (the user's own and
/// every account) is brought into that account's (it may have gone on
/// elsewhere after a switch).
pub fn freshen_conversation(kind: &AgentKind, list: &Accounts, account: Option<&Account>, cwd: &str, session_id: &str) {
    let Some(to) = dir_of(kind, account) else { return };
    let mut homes: Vec<PathBuf> = default_dir(kind).into_iter().collect();
    homes.extend(list.of_kind(&kind.id).map(|a| PathBuf::from(&a.dir)));
    for from in homes.iter().filter(|h| **h != to) {
        let _ = carry_conversation(&kind.id, from, &to, cwd, session_id);
    }
}

/// Whether Pitwall can tell `account` (`None`: the user's own) is logged
/// in: `Some(false)` when it can tell it isn't; `None` when it can't read
/// that tool's login.
pub fn logged_in(kind: &AgentKind, account: Option<&Account>) -> Option<bool> {
    matches!(kind.id.as_str(), "claude" | "codex").then(|| login(kind, account).is_some())
}

/// A Codex session's log, relative to its config folder.
fn find_codex(home: &Path, session_id: &str) -> Option<PathBuf> {
    let sessions = home.join("sessions");
    let mut stack = vec![sessions.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| {
                let n = n.to_string_lossy();
                n.starts_with("rollout-") && n.ends_with(&format!("{session_id}.jsonl"))
            }) {
                return p.strip_prefix(home).ok().map(Path::to_path_buf);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    /// A built-in kind's definition.
    fn kind(id: &str) -> AgentKind {
        crate::kind::load_kinds(Path::new("/nonexistent/pitwall-agents")).into_iter().find(|k| k.id == id).unwrap()
    }

    #[test]
    fn add_rename_remove_and_project_defaults() {
        let t = TempDir::new("accounts");
        let paths = Paths::new(t.path().join("pw"));
        let mut a = Accounts::default();
        let work = a.add(&paths, &kind("claude"), "Work", None).unwrap();
        assert_eq!(work.id, "claude-work");
        assert!(Path::new(&work.dir).is_dir(), "a fresh folder");
        assert!(a.add(&paths, &kind("claude"), "work", None).is_err(), "names are unique per kind");
        assert!(a.add(&paths, &kind("claude"), "Default", None).is_err());
        assert!(a.add(&paths, &kind("shell"), "X", None).is_err(), "only kinds with logins");
        let codex = a.add(&paths, &kind("codex"), "Work", None).unwrap();
        assert_eq!(codex.id, "codex-work", "the same name for another kind");

        a.set_project_default("/p", "claude", Some("claude-work"));
        assert_eq!(a.resolve("claude", "/p", None).map(|x| x.id.as_str()), Some("claude-work"));
        assert_eq!(a.resolve("claude", "/p", Some(DEFAULT)), None, "asked for Default");
        assert_eq!(a.resolve("claude", "/q", None), None);
        assert_eq!(a.resolve("codex", "/p", None), None, "per kind");
        assert_eq!(a.projects_of("claude-work"), ["/p"]);

        a.rename("claude-work", "Job").unwrap();
        assert_eq!(a.get("claude-work").unwrap().name, "Job");
        a.save(&paths).unwrap();
        assert_eq!(Accounts::load(&paths), a);

        a.remove("claude-work");
        assert!(a.get("claude-work").is_none());
        assert!(a.projects.is_empty(), "projects forget it");
        assert_eq!(env(&kind("claude"), a.get("codex-work")), [("CLAUDE_CONFIG_DIR".to_string(), codex.dir.clone())]);
    }

    #[test]
    fn who_is_logged_in() {
        let t = TempDir::new("accounts-login");
        let dir = t.path().join("c");
        std::fs::create_dir_all(&dir).unwrap();
        let acc = |kind: &str| Account { id: "x".into(), kind: kind.into(), name: "X".into(), dir: dir.to_string_lossy().into() };
        assert_eq!(login(&kind("claude"), Some(&acc("claude"))), None, "not logged in");
        std::fs::write(dir.join(".claude.json"), r#"{"oauthAccount":{"emailAddress":"a@example.com"}}"#).unwrap();
        assert_eq!(login(&kind("claude"), Some(&acc("claude"))).as_deref(), Some("a@example.com"));
        // {"email":"b@example.com"} as a token's claims.
        let claims = "eyJlbWFpbCI6ImJAZXhhbXBsZS5jb20ifQ";
        std::fs::write(dir.join("auth.json"), format!(r#"{{"tokens":{{"id_token":"h.{claims}.s"}}}}"#)).unwrap();
        assert_eq!(login(&kind("codex"), Some(&acc("codex"))).as_deref(), Some("b@example.com"));
    }

    #[test]
    fn a_conversation_is_carried_to_the_other_folder() {
        let t = TempDir::new("accounts-carry");
        let (from, to) = (t.path().join("a"), t.path().join("b"));
        let rel = Path::new("projects").join(crate::onboarding::latest::claude_project_dir("/work/shop"));
        std::fs::create_dir_all(from.join(&rel)).unwrap();
        std::fs::write(from.join(&rel).join("s1.jsonl"), "{}\n").unwrap();
        carry_conversation("claude", &from, &to, "/work/shop", "s1").unwrap();
        assert_eq!(std::fs::read_to_string(to.join(&rel).join("s1.jsonl")).unwrap(), "{}\n");

        let day = from.join("sessions/2026/10/12");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-2026-10-12T10-00-00-c9.jsonl"), "{}\n").unwrap();
        carry_conversation("codex", &from, &to, "/work/shop", "c9").unwrap();
        assert!(to.join("sessions/2026/10/12/rollout-2026-10-12T10-00-00-c9.jsonl").exists());
        // Nothing to carry: fine.
        carry_conversation("codex", &from, &to, "/w", "none").unwrap();
        // A newer copy where it goes is kept (a switch back never loses turns).
        let newer = to.join(&rel).join("s1.jsonl");
        std::fs::write(&newer, "{}\n{\"more\":1}\n").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        std::fs::File::options().write(true).open(&newer).unwrap().set_modified(later).unwrap();
        carry_conversation("claude", &from, &to, "/work/shop", "s1").unwrap();
        assert_eq!(std::fs::read_to_string(&newer).unwrap(), "{}\n{\"more\":1}\n");
    }
}
