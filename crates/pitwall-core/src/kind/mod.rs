//! Agent definitions: one small TOML file per agent. Built-ins are embedded;
//! files in ~/.pitwall/agents override or extend them without an app update.

pub mod catalog;
pub mod launch;

use std::path::Path;

use serde::{Deserialize, Serialize};

pub use catalog::KindCatalog;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookMode {
    /// Hooks passed per launch via `claude --settings <json>`.
    ClaudeSettings,
    /// Hooks merged into ~/.codex/hooks.json (only with the user's approval).
    CodexGlobal,
    /// No hooks: status falls back to output activity.
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentKind {
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub new_args: Vec<String>,
    #[serde(default)]
    pub resume_args: Vec<String>,
    #[serde(default)]
    pub assign_session_id: bool,
    pub hooks: HookMode,
    /// rulesync `--targets` id used when applying rules (e.g. "claudecode").
    #[serde(default)]
    pub rulesync_target: Option<String>,
    /// The agent's own "work in a new git worktree" flag (docs/spec/worktrees.md).
    /// `{name}` is replaced with a name Pitwall picks; empty = not supported.
    #[serde(default)]
    pub worktree_args: Vec<String>,
    /// Folders, relative to the agent's working folder, where the agent's
    /// tool makes worktrees of its own (sub-agents, `--worktree`), e.g.
    /// `.claude/worktrees`. Worktrees found there are shown under the agent
    /// (docs/spec/worktrees-view.md).
    #[serde(default)]
    pub worktree_dirs: Vec<String>,
    /// Program names this agent shows up as in the process table (e.g. when
    /// the user starts it by hand in a terminal, docs/spec/terminals.md).
    /// Empty = the program of `command`.
    #[serde(default)]
    pub process_names: Vec<String>,
    /// Names other platforms use for this agent (agw harness integrations,
    /// e.g. "claude-code"); matched like `id` when adopting their sessions.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// The variable naming the folder the tool keeps its login in
    /// (`CLAUDE_CONFIG_DIR`): set, the kind can run on more than one
    /// account (`crate::accounts`). Unset: one login only.
    #[serde(default)]
    pub account_env: Option<String>,
    /// That folder when the variable isn't set (`~/.claude`).
    #[serde(default)]
    pub account_home: Option<String>,
    /// The logo shown for it: a bundled logo's id (`claude`, `codex`, …,
    /// `terminal`) or an SVG file. None: the kind's own, else a letter.
    #[serde(default)]
    pub icon: Option<String>,
    /// The command that updates the tool (`claude update`), run by Pitwall
    /// one at a time when it keeps tools up to date (`crate::tool_updates`).
    #[serde(default)]
    pub update_command: Option<String>,
    /// The environment that turns off the tool's own updater
    /// (`DISABLE_AUTOUPDATER = "1"`), given to everything Pitwall launches
    /// while it updates the tool itself.
    #[serde(default)]
    pub update_off_env: std::collections::BTreeMap<String, String>,
    /// What hosting a shared session of this kind needs (`[share]`,
    /// docs/spec/collaboration/sessions.md §6.7). None: it can still be
    /// shared, with defaults: replies from the terminal, no fork, no model
    /// hosts allowed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share: Option<ShareKind>,
}

/// A kind's `[share]` table. The sandbox's keys (`model_hosts`,
/// `install_paths`, `programs`, `keychain`) and the conversation core's
/// (`reply`, `fork_args`, `instruction_files`, `limit_screen`,
/// `own_sandbox_off_args`, `own_sandbox_off_env`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareKind {
    /// Hosts the agent reaches its model and login at; the share's egress
    /// proxy allows these (plus what the owner adds).
    #[serde(default)]
    pub model_hosts: Vec<String>,
    /// Where the agent's own install may be (`~/` = the owner's home):
    /// readable inside the sandbox, and its programs may run.
    #[serde(default)]
    pub install_paths: Vec<String>,
    /// Programs the agent needs besides its command (its runtime, `node`).
    #[serde(default)]
    pub programs: Vec<String>,
    /// It keeps its login in the OS credential store (the macOS Keychain).
    #[serde(default)]
    pub keychain: bool,
    /// How a turn's reply is read (§7.2).
    #[serde(default)]
    pub reply: ReplyTier,
    /// Start a session as a fork of a base conversation (§6.4), instead of
    /// `new_args`: `{base}` is the base's id, `{session_id}` a new id when
    /// the kind takes one. Empty: it can't fork; the approved summary is
    /// sent as the first message instead.
    #[serde(default)]
    pub fork_args: Vec<String>,
    /// The instruction files it reads by itself (§6.5): the owner's text
    /// goes into these in the session folder.
    #[serde(default)]
    pub instruction_files: Vec<String>,
    /// Its own usage-limit message (§5.8), a regex on its screen; a group
    /// named `until` is the reset time it shows.
    #[serde(default)]
    pub limit_screen: Option<String>,
    /// Arguments and environment that turn the agent's own sandbox off: it
    /// can't start inside Pitwall's (Seatbelt doesn't nest), which is the
    /// one that holds; its tool rules stay as the second layer (§6.8).
    #[serde(default)]
    pub own_sandbox_off_args: Vec<String>,
    #[serde(default)]
    pub own_sandbox_off_env: std::collections::BTreeMap<String, String>,
}

impl ShareKind {
    /// The files a kind without `instruction_files` gets.
    pub fn instruction_files_or_default(&self) -> Vec<String> {
        if self.instruction_files.is_empty() {
            ["AGENTS.md", "CLAUDE.md", "GEMINI.md", "QWEN.md"].iter().map(|s| s.to_string()).collect()
        } else {
            self.instruction_files.clone()
        }
    }
}

/// How the reply of a turn is read (§7.2), best first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReplyTier {
    /// (a) The kind's hooks carry the last message (Claude Code `Stop`).
    Hook,
    /// (a) The kind's own session files.
    SessionFile,
    /// (b) Each turn through the kind's non-interactive mode.
    NonInteractive,
    /// (c) The terminal output of the turn.
    #[default]
    Terminal,
}

impl AgentKind {
    /// This kind is what a platform calls `name` (its id or an alias,
    /// ignoring case).
    pub fn answers_to(&self, name: &str) -> bool {
        let name = name.trim();
        !name.is_empty() && (self.id.eq_ignore_ascii_case(name) || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(name)))
    }
}

const BUILTIN: &[&str] = &[
    include_str!("../../agents/claude.toml"),
    include_str!("../../agents/codex.toml"),
    include_str!("../../agents/gemini.toml"),
    include_str!("../../agents/opencode.toml"),
    include_str!("../../agents/cursor.toml"),
    include_str!("../../agents/copilot.toml"),
    include_str!("../../agents/qwen.toml"),
    include_str!("../../agents/amp.toml"),
    include_str!("../../agents/aider.toml"),
    include_str!("../../agents/shell.toml"),
];

/// The built-ins, overridden/extended by `*.toml` files in `user_dir`.
pub fn load_kinds(user_dir: &Path) -> Vec<AgentKind> {
    let mut kinds: Vec<AgentKind> = BUILTIN
        .iter()
        .map(|src| toml::from_str(src).expect("built-in agent definition is valid"))
        .collect();

    if let Ok(entries) = std::fs::read_dir(user_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else { continue };
            match toml::from_str::<AgentKind>(&src) {
                Ok(kind) => match kinds.iter_mut().find(|k| k.id == kind.id) {
                    Some(existing) => *existing = kind,
                    None => kinds.push(kind),
                },
                Err(err) => eprintln!("pitwall: skipping {}: {err}", path.display()),
            }
        }
    }
    kinds
}

/// The id of the "custom command" kind.
pub const CUSTOM: &str = "custom";

/// Kind used for "custom command" agents; the command comes from the user.
pub fn custom_kind(command: &str) -> AgentKind {
    AgentKind {
        id: CUSTOM.into(),
        name: "Custom".into(),
        command: command.into(),
        new_args: vec![],
        resume_args: vec![],
        assign_session_id: false,
        hooks: HookMode::None,
        rulesync_target: None,
        worktree_args: vec![],
        worktree_dirs: vec![],
        process_names: vec![],
        aliases: vec![],
        account_env: None,
        account_home: None,
        icon: Some("terminal".into()),
        update_command: None,
        update_off_env: Default::default(),
        share: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_files_override_and_extend_builtins() {
        let dir = crate::testing::TempDir::new("kinds");
        std::fs::write(dir.path().join("shell.toml"), "id = \"shell\"\nname = \"My shell\"\ncommand = \"/bin/sh\"\nhooks = \"none\"\n").unwrap();
        std::fs::write(dir.path().join("mine.toml"), "id = \"mine\"\nname = \"Mine\"\ncommand = \"mine\"\nhooks = \"none\"\n").unwrap();
        std::fs::write(dir.path().join("broken.toml"), "id = ").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();
        let kinds = load_kinds(dir.path());
        assert_eq!(kinds.iter().find(|k| k.id == "shell").unwrap().name, "My shell");
        assert!(kinds.iter().any(|k| k.id == "mine"));
        assert!(kinds.iter().any(|k| k.id == "claude"));
    }

    /// Every built-in agent definition with screen rules has a matching rule
    /// file, and every rule file belongs to a built-in agent.
    #[test]
    fn new_agent_kinds_have_rules() {
        let kinds = super::load_kinds(std::path::Path::new("/nonexistent/pitwall-agents"));
        let rules: Vec<&str> = pitwall_detect::builtin_rule_kinds().collect();
        for id in ["gemini", "opencode", "cursor", "copilot", "qwen", "amp", "aider"] {
            assert!(kinds.iter().any(|k| k.id == id), "agent kind {id} missing");
            assert!(rules.contains(&id), "rules for {id} missing");
        }
        for kind in rules {
            assert!(kinds.iter().any(|k| k.id == kind), "rules for unknown kind {kind}");
        }
    }

    /// docs/reference/agents.md names every key of an agent definition
    /// (all of `AgentKind`'s fields, so every key a bundled file uses).
    #[test]
    fn reference_doc_names_every_key() {
        let doc = include_str!("../../../../docs/reference/agents.md");
        let kind = serde_json::to_value(custom_kind("x")).unwrap();
        let mut keys: Vec<String> = kind.as_object().unwrap().keys().cloned().collect();
        for src in BUILTIN {
            keys.extend(src.parse::<toml::Table>().unwrap().keys().cloned());
        }
        for key in keys {
            assert!(doc.contains(&format!("`{key}`")), "docs/reference/agents.md doesn't describe `{key}`");
        }
    }

    #[test]
    fn kinds_answer_to_their_aliases() {
        let kinds = super::load_kinds(std::path::Path::new("/nonexistent/pitwall-agents"));
        let named = |n: &str| kinds.iter().find(|k| k.answers_to(n)).map(|k| k.id.as_str());
        assert_eq!(named("claude-code"), Some("claude"));
        assert_eq!(named("Claude"), Some("claude"));
        assert_eq!(named("codex"), Some("codex"));
        assert_eq!(named("shell"), Some("shell"));
        assert_eq!(named("grok"), None);
        assert_eq!(named(""), None);
    }
}
