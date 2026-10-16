//! What keeps a share from going live (§6.3 "The agent account", decided
//! 2026-10-16): the login its sessions would run on.
//!
//! - **A separate account.** A session can read and write its agent
//!   account's config folder, and that folder holds the account's other
//!   conversations. For kinds with accounts (`account_env`, Pitwall's
//!   several logins per kind), a share goes live only on an account of its
//!   own: never the kind's Default (the owner's own login, e.g.
//!   `~/.claude`), and never the account the shared agent itself runs on.
//! - **Gemini CLI** keeps its login in `~/.gemini` with no variable to move
//!   it (`[share] login_dir`), so a session (its own `HOME`) has no login
//!   and `~/.gemini` is never mounted or copied. Such a share goes live
//!   only with an API key the owner passes explicitly (`[share]
//!   api_key_env`, e.g. `GEMINI_API_KEY`), set where Pitwall can read it.
//!
//! Going live, resuming, and widening a live share to such a login are
//! refused ([`super::Collab::go_live`], `set_share`); a live session never
//! starts on one either. Guest tries may use any login: the owner is the
//! only one asking (the Share dialog warns, amber).

use std::path::{Path, PathBuf};

use super::session::{share_kind, OwnerAgents, SharedAgent};
use crate::accounts::DEFAULT;
use pitwall_proto::collab::ShareSettings;

/// One of the owner's agent accounts (`accounts.json`), resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountInfo {
    pub id: String,
    /// What the owner calls it ("Sharing").
    pub name: String,
    /// Its config folder.
    pub dir: PathBuf,
}

/// Why a share can't go live yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveBlock {
    /// Sessions would run on the kind's Default account: the owner's own
    /// login (`dir`, e.g. `~/.claude`).
    OwnLogin { kind: String, dir: Option<String> },
    /// Sessions would run on the account the shared agent itself uses.
    AgentsAccount { kind: String, account: String },
    /// The share names an account this kind doesn't have.
    UnknownAccount { kind: String, account: String },
    /// The kind keeps its login where sessions can't reach it, and none of
    /// its API key variables is passed.
    NeedsApiKey { kind: String, login_dir: Option<String>, vars: Vec<String> },
    /// The API key variable is passed but has no value where Pitwall looks.
    KeyNotSet { var: String },
}

impl LiveBlock {
    /// One plain sentence, then how to fix it (the CLI's and the daemon's
    /// error, the Share dialog's amber line). `slug`: the share's name in
    /// the CLI line.
    pub fn message(&self, slug: &str) -> String {
        let pick = format!(
            "Pick or add one in Share… → Secrets and access, or with `pitwall share set {slug} --account <name>` (Settings → Agents → Accounts adds one)."
        );
        match self {
            LiveBlock::OwnLogin { kind, dir } => {
                let dir = dir.as_deref().map(|d| format!(" ({d})")).unwrap_or_default();
                format!(
                    "{slug} would run on your own {kind} login{dir}. A shared session can read and write its account's folder, which holds your other conversations, so sharing needs a separate account. {pick}"
                )
            }
            LiveBlock::AgentsAccount { kind, account } => format!(
                "{slug} would run on the {kind} account \"{account}\", which the shared agent itself uses. A shared session can read and write that account's folder, which holds its other conversations, so sharing needs a separate account. {pick}"
            ),
            LiveBlock::UnknownAccount { kind, account } => format!("There's no {kind} account \"{account}\". {pick}"),
            LiveBlock::NeedsApiKey { kind, login_dir, vars } => {
                let dir = login_dir.as_deref().unwrap_or("its own folder");
                let first = vars.first().map(String::as_str).unwrap_or("an API key variable");
                let others = if vars.len() > 1 { format!(" (or {})", vars[1..].join(", ")) } else { String::new() };
                format!(
                    "{kind} keeps its login in {dir}, which shared sessions can't use; add an API key for sharing: pass {first}{others} in Share… → Secrets and access, or with `pitwall share set {slug} --env {first}`."
                )
            }
            LiveBlock::KeyNotSet { var } => format!(
                "{var} is passed to {slug}, but it isn't set in Pitwall's environment or your login shell: set it (for example in your shell profile) and try again."
            ),
        }
    }
}

/// The account sessions of `share` would run on: the share's, else the
/// shared agent's own; `None`: the kind's Default.
pub fn effective_account<'a>(share: &'a ShareSettings, agent: &'a SharedAgent) -> Option<&'a str> {
    share.access.account.as_deref().or(agent.account.as_deref()).map(str::trim).filter(|a| !a.is_empty() && !a.eq_ignore_ascii_case(DEFAULT))
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

/// What keeps `share` (of `agent`) from going live; empty: nothing.
/// `owner` resolves accounts and reads the owner's variables.
pub fn live_blocks(share: &ShareSettings, agent: &SharedAgent, owner: &dyn OwnerAgents) -> Vec<LiveBlock> {
    let kind = &agent.kind;
    let sk = share_kind(kind);
    let mut out = Vec::new();
    if kind.account_env.is_some() {
        let own = || LiveBlock::OwnLogin { kind: kind.name.clone(), dir: kind.account_home.clone() };
        match effective_account(share, agent) {
            None => out.push(own()),
            Some(key) => match owner.account(kind, key) {
                None => out.push(LiveBlock::UnknownAccount { kind: kind.name.clone(), account: key.to_string() }),
                Some(acc) => {
                    let default_dir = crate::accounts::default_dir(kind);
                    let agents = agent.account.as_deref().filter(|a| !a.eq_ignore_ascii_case(DEFAULT)).and_then(|a| owner.account(kind, a));
                    if default_dir.is_some_and(|d| same_dir(&d, &acc.dir)) {
                        out.push(own());
                    } else if agents.is_some_and(|a| a.id == acc.id || same_dir(&a.dir, &acc.dir)) {
                        out.push(LiveBlock::AgentsAccount { kind: kind.name.clone(), account: acc.name });
                    }
                }
            },
        }
    }
    if !sk.api_key_env.is_empty() {
        let passed: Vec<&String> = sk.api_key_env.iter().filter(|v| share.access.env.contains(v)).collect();
        match passed.first() {
            None => out.push(LiveBlock::NeedsApiKey { kind: kind.name.clone(), login_dir: sk.login_dir.clone(), vars: sk.api_key_env.clone() }),
            Some(first) => {
                if !passed.iter().any(|v| owner.env_value(v).is_some_and(|x| !x.trim().is_empty())) {
                    out.push(LiveBlock::KeyNotSet { var: (*first).clone() });
                }
            }
        }
    }
    out
}

/// How the share's sessions log in, for the summary's account sentence
/// and the Share dialog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Login {
    /// The kind has no accounts and no API key rule: whatever it finds in
    /// the session's own `HOME`.
    #[default]
    Unknown,
    /// An account of the kind: its name (`None`: the owner's own login).
    Account { name: Option<String> },
    /// An API key from one of these variables (`passed`: the one the share
    /// passes, if any); the kind's own login folder is never used.
    ApiKey { vars: Vec<String>, passed: Option<String>, login_dir: Option<String> },
}

/// How sessions of `share` would log in.
pub fn login(share: &ShareSettings, agent: &SharedAgent, owner: &dyn OwnerAgents) -> Login {
    let kind = &agent.kind;
    let sk = share_kind(kind);
    if !sk.api_key_env.is_empty() {
        let passed = sk.api_key_env.iter().find(|v| share.access.env.contains(v)).cloned();
        return Login::ApiKey { vars: sk.api_key_env, passed, login_dir: sk.login_dir };
    }
    if kind.account_env.is_some() {
        let name = effective_account(share, agent).map(|k| owner.account(kind, k).map_or_else(|| k.to_string(), |a| a.name));
        return Login::Account { name };
    }
    Login::Unknown
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::collab::session::SharedAgent;
    use crate::kind::AgentKind;

    struct Owner {
        accounts: Vec<AccountInfo>,
        env: BTreeMap<String, String>,
    }

    impl OwnerAgents for Owner {
        fn agent(&self, _id: &str) -> Option<SharedAgent> {
            None
        }
        fn account_env(&self, _kind: &AgentKind, _account: Option<&str>) -> Vec<(String, String)> {
            vec![]
        }
        fn account(&self, _kind: &AgentKind, key: &str) -> Option<AccountInfo> {
            self.accounts.iter().find(|a| a.id == key || a.name.eq_ignore_ascii_case(key)).cloned()
        }
        fn env_value(&self, name: &str) -> Option<String> {
            self.env.get(name).cloned()
        }
    }

    fn kind(id: &str) -> AgentKind {
        crate::kind::load_kinds(Path::new("/nonexistent")).into_iter().find(|k| k.id == id).unwrap()
    }

    fn share(account: Option<&str>, env: &[&str]) -> ShareSettings {
        let mut s: ShareSettings = serde_json::from_value(serde_json::json!({
            "agent": "a1", "slug": "api-expert", "name": "api", "description": "",
            "scope": {"repo": "/code/api", "ref": "main"},
        }))
        .unwrap();
        s.access.account = account.map(String::from);
        s.access.env = env.iter().map(|e| e.to_string()).collect();
        s
    }

    fn owner() -> Owner {
        Owner {
            accounts: vec![
                AccountInfo { id: "claude-sharing".into(), name: "Sharing".into(), dir: "/accounts/claude-sharing".into() },
                AccountInfo { id: "claude-work".into(), name: "Work".into(), dir: "/accounts/claude-work".into() },
            ],
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn a_share_needs_an_account_of_its_own() {
        let o = owner();
        let agent = SharedAgent { kind: kind("claude"), project: "/code/api".into(), account: None };
        // The owner's own login: refused, and the message says why and how.
        let b = live_blocks(&share(None, &[]), &agent, &o);
        assert_eq!(b, [LiveBlock::OwnLogin { kind: "Claude Code".into(), dir: Some("~/.claude".into()) }]);
        let m = b[0].message("api-expert");
        assert!(m.contains("your own Claude Code login (~/.claude)") && m.contains("holds your other conversations"), "{m}");
        assert!(m.contains("pitwall share set api-expert --account <name>"), "{m}");
        assert_eq!(live_blocks(&share(Some("default"), &[]), &agent, &o).len(), 1);
        // A separate account (by id or name): fine.
        assert!(live_blocks(&share(Some("claude-sharing"), &[]), &agent, &o).is_empty());
        assert!(live_blocks(&share(Some("Sharing"), &[]), &agent, &o).is_empty());
        assert!(matches!(live_blocks(&share(Some("nope"), &[]), &agent, &o)[..], [LiveBlock::UnknownAccount { .. }]));
        // The account the shared agent itself runs on is not separate.
        let on_work = SharedAgent { account: Some("claude-work".into()), ..agent.clone() };
        assert_eq!(live_blocks(&share(None, &[]), &on_work, &o), [LiveBlock::AgentsAccount { kind: "Claude Code".into(), account: "Work".into() }]);
        assert_eq!(live_blocks(&share(Some("Work"), &[]), &on_work, &o).len(), 1);
        assert!(live_blocks(&share(Some("claude-sharing"), &[]), &on_work, &o).is_empty());
        assert_eq!(login(&share(Some("claude-sharing"), &[]), &agent, &o), Login::Account { name: Some("Sharing".into()) });
        assert_eq!(login(&share(None, &[]), &agent, &o), Login::Account { name: None });
    }

    #[test]
    fn an_account_folder_that_is_the_default_one_is_the_owners_own_login() {
        let mut o = owner();
        let home = crate::accounts::default_dir(&kind("codex")).unwrap();
        o.accounts.push(AccountInfo { id: "codex-alias".into(), name: "Alias".into(), dir: home });
        let agent = SharedAgent { kind: kind("codex"), project: "/code/api".into(), account: None };
        assert!(matches!(live_blocks(&share(Some("codex-alias"), &[]), &agent, &o)[..], [LiveBlock::OwnLogin { .. }]));
    }

    #[test]
    fn gemini_goes_live_only_with_an_api_key_passed_explicitly() {
        let mut o = owner();
        let agent = SharedAgent { kind: kind("gemini"), project: "/code/docs".into(), account: None };
        let b = live_blocks(&share(None, &[]), &agent, &o);
        assert_eq!(b.len(), 1, "{b:?}");
        assert_eq!(
            b[0].message("docs"),
            "Gemini CLI keeps its login in ~/.gemini, which shared sessions can't use; add an API key for sharing: \
             pass GEMINI_API_KEY (or GOOGLE_API_KEY) in Share… → Secrets and access, or with `pitwall share set docs --env GEMINI_API_KEY`."
        );
        // Passed, but not set anywhere Pitwall looks.
        assert_eq!(live_blocks(&share(None, &["GEMINI_API_KEY"]), &agent, &o), [LiveBlock::KeyNotSet { var: "GEMINI_API_KEY".into() }]);
        o.env.insert("GEMINI_API_KEY".into(), "test-key-not-real".into());
        assert!(live_blocks(&share(None, &["GEMINI_API_KEY"]), &agent, &o).is_empty());
        // Another variable doesn't count, and never ~/.gemini.
        assert_eq!(live_blocks(&share(None, &["OTHER"]), &agent, &o).len(), 1);
        assert_eq!(
            login(&share(None, &["GEMINI_API_KEY"]), &agent, &o),
            Login::ApiKey { vars: vec!["GEMINI_API_KEY".into(), "GOOGLE_API_KEY".into()], passed: Some("GEMINI_API_KEY".into()), login_dir: Some("~/.gemini".into()) }
        );
    }

    #[test]
    fn kinds_without_accounts_or_keys_have_nothing_to_check() {
        let agent = SharedAgent { kind: kind("opencode"), project: "/code/api".into(), account: None };
        assert!(live_blocks(&share(None, &[]), &agent, &owner()).is_empty());
        assert_eq!(login(&share(None, &[]), &agent, &owner()), Login::Unknown);
    }
}
