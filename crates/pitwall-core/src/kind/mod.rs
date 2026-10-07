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
    /// Program names this agent shows up as in the process table (e.g. when
    /// the user starts it by hand in a terminal, docs/spec/terminals.md).
    /// Empty = the program of `command`.
    #[serde(default)]
    pub process_names: Vec<String>,
    /// Names other platforms use for this agent (agw harness integrations,
    /// e.g. "claude-code"); matched like `id` when adopting their sessions.
    #[serde(default)]
    pub aliases: Vec<String>,
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
        process_names: vec![],
        aliases: vec![],
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
