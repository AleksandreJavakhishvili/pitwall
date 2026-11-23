//! What an extension would run and set, worked out without running
//! anything (docs/spec/extensions.md, "Setup actions" and "Settings →
//! Extensions"): an adapter's or a step's program found ([`resolve`]), a
//! setup action's steps as they will run or why they won't ([`plan`]),
//! and the variables it sets on agents ([`env_preview`]). Settings shows
//! these, and `pitwall extension check` says them ([`super::check`]).
//!
//! A step `for` a tool runs once per place that tool's agents run from
//! on this computer ([`plan_for`]): its default config folder, then each
//! of Pitwall's accounts of it ([`ToolAccount`], [`tool_accounts`]), with
//! the environment Pitwall gives that account's agents. A recorded run
//! keeps the accounts it went through for ([`missing`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{expand_args, Action, Manifest, SETTING_PLACEHOLDER};
use crate::accounts::Accounts;
use crate::kind::AgentKind;

/// The program `program` names: a path inside `dir` when it has a slash,
/// else found on `path` (the login shell's PATH; this process's when
/// `None`).
pub fn resolve(dir: &Path, program: &str, path: Option<&str>) -> Result<PathBuf, String> {
    if program.contains('/') || program.contains('\\') {
        let p = dir.join(program);
        let (Ok(full), Ok(base)) = (p.canonicalize(), dir.canonicalize()) else {
            return Err(format!("{program} isn't in the extension's folder"));
        };
        if !full.starts_with(&base) {
            return Err(format!("{program} isn't inside the extension's folder"));
        }
        return Ok(full);
    }
    let path = path.map(String::from).or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
    let exts: &[&str] = if std::env::consts::OS == "windows" { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    for d in std::env::split_paths(&path) {
        for e in exts {
            let c = d.join(format!("{program}{e}"));
            if c.is_file() {
                return Ok(c);
            }
        }
    }
    Err(format!("“{program}” isn't installed or isn't on your PATH: install it, then restart the extension"))
}

/// One step as it will run, or why it won't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The program and its arguments, settings put in.
    pub argv: Vec<String>,
    /// Where the program is (found on PATH).
    pub program: Option<PathBuf>,
    /// Why it won't run.
    pub skip: Option<String>,
    pub for_kind: Option<String>,
    pub optional: bool,
    /// The account of `for_kind` it runs for (`None`: the tool's default
    /// folder, or a step for no tool).
    pub account: Option<ToolAccount>,
}

impl Planned {
    /// The command as a line to read (arguments with spaces or quotes
    /// quoted; nothing is run through a shell).
    pub fn line(&self) -> String {
        line(&self.argv)
    }

    /// Where it runs, in plain words ("Claude Code, account Work",
    /// "Claude Code, default"); `None` for a step for no tool.
    pub fn who(&self) -> Option<String> {
        let k = kind_name(self.for_kind.as_deref()?);
        Some(match &self.account {
            Some(a) => format!("{k}, account {}", a.name),
            None => format!("{k}, default"),
        })
    }

    /// The variables it runs with besides the app's own: its account's.
    pub fn env(&self) -> &[(String, String)] {
        self.account.as_ref().map_or(&[], |a| a.env.as_slice())
    }
}

/// One of Pitwall's accounts of a tool (`crate::accounts`) as a place
/// that tool's agents run from: a step for the tool runs there too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolAccount {
    /// The tool's kind (`claude`).
    pub kind: String,
    /// The account's id (`claude-work`).
    pub id: String,
    /// Its name ("Work").
    pub name: String,
    /// What Pitwall gives its agents (`crate::accounts::env`): its config
    /// folder.
    pub env: Vec<(String, String)>,
}

impl ToolAccount {
    /// "Claude Code, account Work".
    pub fn words(&self) -> String {
        format!("{}, account {}", kind_name(&self.kind), self.name)
    }
}

/// The accounts in `list` of the tools `kind` knows (a kind that can have
/// accounts), in their order, each with the environment its agents get.
/// Browser accounts, git identities and extension connections aren't a
/// tool's.
pub fn tool_accounts(list: &Accounts, kind: impl Fn(&str) -> Option<AgentKind>) -> Vec<ToolAccount> {
    list.accounts
        .iter()
        .filter_map(|a| {
            let def = kind(&a.kind).filter(|k| k.account_env.is_some())?;
            Some(ToolAccount { kind: a.kind.clone(), id: a.id.clone(), name: a.name.clone(), env: crate::accounts::env(&def, Some(a)) })
        })
        .collect()
}

/// The kinds action `a` has steps for, in order.
pub fn kinds_of(a: &Action) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for k in a.steps.iter().filter_map(|s| s.for_kind.as_deref()) {
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

/// The accounts of `accounts` that action `a` has steps for and a
/// recorded run didn't go through (`covered`: their ids). A record from
/// before accounts were kept covers none.
pub fn missing<'a>(a: &Action, accounts: &'a [ToolAccount], covered: &[String]) -> Vec<&'a ToolAccount> {
    let kinds = kinds_of(a);
    accounts.iter().filter(|t| kinds.contains(&t.kind.as_str()) && !covered.contains(&t.id)).collect()
}

/// The accounts a run leaves covered: `ok`, the accounts it went through
/// without a failure, added to `before` when it ran for missing accounts
/// only (`partial`), else in its place.
pub fn covered_after(before: &[String], partial: bool, ok: &[String]) -> Vec<String> {
    let mut out: Vec<String> = if partial { before.to_vec() } else { Vec::new() };
    for id in ok {
        if !out.contains(id) {
            out.push(id.clone());
        }
    }
    out
}

/// `argv` as a line to read.
pub fn line(argv: &[String]) -> String {
    argv.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
}

fn quote(a: &str) -> String {
    if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:@#=+,%".contains(c)) {
        a.to_string()
    } else {
        format!("'{}'", a.replace('\'', "'\\''"))
    }
}

/// What an agent kind is called (`claude`: "Claude Code").
pub fn kind_name(id: &str) -> String {
    match id {
        "claude" => "Claude Code".into(),
        "codex" => "Codex".into(),
        "gemini" => "Gemini CLI".into(),
        "aider" => "Aider".into(),
        other => other.to_string(),
    }
}

/// Action `a` of extension `m`, with its plain settings `values`, its
/// programs looked up on `path`, for the tools' default folders only.
pub fn plan(m: &Manifest, a: &Action, values: &BTreeMap<String, String>, path: Option<&str>) -> Vec<Planned> {
    plan_for(m, a, values, path, &[], true)
}

/// [`plan`], each run of consecutive steps for one tool repeated for each
/// place it runs from: the tool's default folder (when `default`), then
/// each of `accounts` of that tool, in their order. Steps for no tool run
/// once, when `default` (`default` false: a run for missing accounts
/// only).
pub fn plan_for(m: &Manifest, a: &Action, values: &BTreeMap<String, String>, path: Option<&str>, accounts: &[ToolAccount], default: bool) -> Vec<Planned> {
    let one = |s: &super::Step, account: Option<&ToolAccount>| {
        let (argv, mut skip) = match expand_args(&s.run, m, values) {
            Ok(v) => (v, None),
            Err(label) => (s.run.clone(), Some(format!("set {label} first"))),
        };
        let program = resolve(Path::new("."), &argv[0], path).ok();
        if program.is_none() && skip.is_none() {
            skip = Some(match &s.for_kind {
                Some(k) => format!("{} isn't installed", kind_name(k)),
                None => format!("“{}” isn't installed", argv[0]),
            });
        }
        Planned { argv, program, skip, for_kind: s.for_kind.clone(), optional: s.optional, account: account.cloned() }
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < a.steps.len() {
        let kind = a.steps[i].for_kind.clone();
        let end = i + a.steps[i..].iter().take_while(|s| s.for_kind == kind).count();
        let block = &a.steps[i..end];
        let mut places: Vec<Option<&ToolAccount>> = Vec::new();
        if default {
            places.push(None);
        }
        if let Some(k) = &kind {
            places.extend(accounts.iter().filter(|t| t.kind == *k).map(Some));
        }
        for place in places {
            out.extend(block.iter().map(|s| one(s, place)));
        }
        i = end;
    }
    out
}

/// Each variable extension `m` sets, as `NAME=value`: its settings put in
/// (the per-agent placeholders left as they are), or why it isn't set.
pub fn env_preview(m: &Manifest, values: &BTreeMap<String, String>) -> Vec<String> {
    m.agent_env
        .iter()
        .map(|v| {
            let mut value = v.value.clone();
            for s in &m.settings {
                let p = format!("{SETTING_PLACEHOLDER}{}}}", s.key);
                if value.contains(&p) {
                    match values.get(&s.key).filter(|x| !x.trim().is_empty()) {
                        Some(x) => value = value.replace(&p, x),
                        None => return format!("{} (not set until {} is)", v.name, s.label),
                    }
                }
            }
            format!("{}={value}", v.name)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_are_found_and_kept_in_their_folder() {
        let t = crate::testing::TempDir::new("plan-resolve");
        std::fs::write(t.path().join("run.sh"), "").unwrap();
        assert!(resolve(t.path(), "./run.sh", None).is_ok());
        assert!(resolve(t.path(), "../outside.sh", None).is_err());
        std::fs::create_dir_all(t.path().join("bin")).unwrap();
        std::fs::write(t.path().join("bin").join("tool"), "").unwrap();
        let path = t.path().join("bin").to_string_lossy().into_owned();
        assert!(resolve(t.path(), "tool", Some(&path)).is_ok());
        assert!(resolve(t.path(), "no-such-program-here", Some(&path)).unwrap_err().contains("isn't installed"));
    }

    #[test]
    fn lines_are_quoted_for_reading() {
        assert_eq!(line(&["a".into(), "it's".into(), "b c".into(), "--x=1".into()]), "a 'it'\\''s' 'b c' --x=1");
        assert_eq!(kind_name("claude"), "Claude Code");
        assert_eq!(kind_name("other"), "other");
    }

    fn two_accounts() -> (Accounts, Vec<AgentKind>) {
        let acc = |id: &str, kind: &str, name: &str, dir: &str| crate::accounts::Account { id: id.into(), kind: kind.into(), name: name.into(), dir: dir.into(), ..Default::default() };
        let list = Accounts {
            accounts: vec![
                acc("claude-work", "claude", "Work", "/data/accounts/claude-work"),
                acc("web-1", crate::accounts::WEB, "Browsing", "/data/webview/x"),
                acc("claude-home", "claude", "Home", "/data/accounts/claude-home"),
                acc("conn-1", &crate::accounts::ext_kind("x"), "Server", ""),
            ],
            ..Default::default()
        };
        (list, crate::kind::load_kinds(Path::new("/nonexistent/pitwall-agents")))
    }

    /// A step for a tool runs for its default folder and for each of
    /// Pitwall's accounts of it, with exactly the environment that
    /// account's agents are launched with; steps for no tool, or for a
    /// tool without accounts, run once.
    #[test]
    fn steps_for_a_tool_run_for_each_of_its_accounts() {
        let t = crate::testing::TempDir::new("plan-accounts");
        std::fs::write(t.path().join("claude"), "").unwrap();
        std::fs::write(t.path().join("prep"), "").unwrap();
        let path = t.path().to_string_lossy().into_owned();
        let m = super::super::parse(r#"{"id": "x", "name": "X", "actions": [{"id": "setup", "label": "Set up", "description": "d", "steps": [
            {"run": ["prep"]},
            {"for": "claude", "run": ["claude", "plugin", "remove", "x"], "optional": true},
            {"for": "claude", "run": ["claude", "plugin", "install", "x"]},
            {"for": "codex", "run": ["codex", "plugin", "add", "x"]}
        ]}]}"#).unwrap();
        let a = &m.actions[0];
        let (list, kinds) = two_accounts();
        let accounts = tool_accounts(&list, |k| kinds.iter().find(|d| d.id == k).cloned());
        assert_eq!(accounts.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["claude-work", "claude-home"], "only tools' accounts");
        let claude = kinds.iter().find(|k| k.id == "claude").unwrap();
        assert_eq!(accounts[0].env, crate::accounts::env(claude, list.get("claude-work")), "what its agents get at launch");
        assert_eq!(accounts[0].env, [("CLAUDE_CONFIG_DIR".to_string(), "/data/accounts/claude-work".to_string())]);

        let p = plan_for(&m, a, &BTreeMap::new(), Some(&path), &accounts, true);
        let rows: Vec<(String, Option<String>, bool)> = p.iter().map(|s| (s.line(), s.who(), s.optional)).collect();
        let r = |l: &str, w: Option<&str>, o: bool| (l.to_string(), w.map(String::from), o);
        assert_eq!(
            rows,
            [
                r("prep", None, false),
                r("claude plugin remove x", Some("Claude Code, default"), true),
                r("claude plugin install x", Some("Claude Code, default"), false),
                r("claude plugin remove x", Some("Claude Code, account Work"), true),
                r("claude plugin install x", Some("Claude Code, account Work"), false),
                r("claude plugin remove x", Some("Claude Code, account Home"), true),
                r("claude plugin install x", Some("Claude Code, account Home"), false),
                r("codex plugin add x", Some("Codex, default"), false),
            ]
        );
        assert!(p[..3].iter().all(|s| s.env().is_empty()), "the default folder: the app's own environment");
        assert_eq!(p[3].env(), accounts[0].env.as_slice());
        assert_eq!(p[5].env(), accounts[1].env.as_slice());
        assert_eq!(p[7].skip.as_deref(), Some("Codex isn't installed"));
        assert_eq!(plan(&m, a, &BTreeMap::new(), Some(&path)), plan_for(&m, a, &BTreeMap::new(), Some(&path), &[], true), "no accounts: as before");
        assert_eq!(plan(&m, a, &BTreeMap::new(), Some(&path)).len(), 4);

        // Missing accounts only: nothing for the default folders or for no
        // tool.
        let only = plan_for(&m, a, &BTreeMap::new(), Some(&path), &accounts[1..], false);
        assert_eq!(only.iter().map(|s| s.who().unwrap()).collect::<Vec<_>>(), ["Claude Code, account Home", "Claude Code, account Home"]);
    }

    /// A tick covers the accounts its run went through: a record from
    /// before accounts were kept covers none, an account added later is
    /// missing, a removed one doesn't matter, and an action without steps
    /// for a tool that has accounts needs none.
    #[test]
    fn a_record_covers_the_accounts_it_ran_for() {
        let m = super::super::parse(r#"{"id": "x", "name": "X", "actions": [
            {"id": "setup", "label": "Set up", "description": "d", "steps": [{"for": "claude", "run": ["claude", "a"]}, {"for": "codex", "run": ["codex", "a"]}]},
            {"id": "other", "label": "Other", "description": "d", "steps": [{"run": ["npm", "i"]}, {"for": "gemini", "run": ["gemini", "a"]}]}
        ]}"#).unwrap();
        let (list, kinds) = two_accounts();
        let accounts = tool_accounts(&list, |k| kinds.iter().find(|d| d.id == k).cloned());
        let ids = |v: Vec<&ToolAccount>| v.into_iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        assert_eq!(kinds_of(&m.actions[0]), ["claude", "codex"]);
        assert_eq!(ids(missing(&m.actions[0], &accounts, &[])), ["claude-work", "claude-home"], "a record from before: every account");
        assert_eq!(ids(missing(&m.actions[0], &accounts, &["claude-work".into()])), ["claude-home"], "added later");
        assert!(missing(&m.actions[0], &accounts, &["claude-home".into(), "claude-work".into(), "claude-gone".into()]).is_empty());
        assert!(missing(&m.actions[1], &accounts, &[]).is_empty(), "no account of its tools");
        assert!(missing(&m.actions[0], &[], &[]).is_empty(), "no accounts at all");

        assert_eq!(covered_after(&["a".into()], true, &["b".into(), "a".into()]), ["a", "b"], "a run for the missing ones adds to the record");
        assert_eq!(covered_after(&["a".into()], false, &["b".into()]), ["b"], "a full run replaces it");
    }
}
