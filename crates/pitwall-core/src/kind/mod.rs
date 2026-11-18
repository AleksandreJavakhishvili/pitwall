//! Agent definitions: one small TOML file per agent. Built-ins are embedded;
//! files in ~/.pitwall/agents override or extend them without an app update.

pub mod catalog;
pub mod launch;
pub mod mcp_file;

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

/// How Pitwall's `pitwall` MCP server (`agents.showFiles`) is given to the
/// agent at launch, with the tool's own documented launch-time setting; the
/// user's config files are never edited this way. Each adds to the user's
/// own servers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpMode {
    /// `claude --mcp-config '<json>'` (added to the user's own servers),
    /// with the tools allowed in the `--settings` Pitwall passes.
    ClaudeConfig,
    /// `codex -c mcp_servers.pitwall.command=… -c mcp_servers.pitwall.args=…`
    /// (config overrides for this run).
    CodexConfig,
    /// `copilot --additional-mcp-config '<json>' --allow-tool pitwall`
    /// (augments ~/.copilot/mcp-config.json; the server's tools allowed).
    CopilotConfig,
    /// `qwen --mcp-config '<json>' --allowed-tools mcp__pitwall__…`
    /// (merged with the user's and the project's servers).
    QwenConfig,
    /// `amp --mcp-config '<json>'` (merged; servers given this way need
    /// no approval).
    AmpConfig,
    /// `OPENCODE_CONFIG_CONTENT='<json>'` in the environment: opencode's
    /// inline config, merged with the user's and the project's.
    OpencodeEnv,
    /// The tool has no safe launch-time way: no server at launch (see
    /// [`AgentKind::mcp_file`] for the opt-in alternative).
    #[default]
    None,
}

/// A tool without a launch-time way that reads its MCP servers from a
/// file: the user can let Pitwall add its own entry there (Settings →
/// Agents, `pitwall show-files add`), never silently.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpFile {
    /// The JSON file (`~/` is the home folder), with a top-level
    /// `mcpServers` object.
    pub path: String,
    /// Fields added to Pitwall's entry next to `command` and `args`
    /// (Gemini CLI's `trust = true`, Cursor's `type = "stdio"`).
    #[serde(default)]
    pub entry: std::collections::BTreeMap<String, serde_json::Value>,
}

/// How a kind gets `agents.showFiles`, in order of preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShowFilesRoute {
    /// Pitwall's MCP server, added at each launch ([`McpMode`]).
    Launch,
    /// Pitwall's MCP server, in the tool's own MCP file, once the user
    /// lets Pitwall add it ([`McpFile`]).
    ConfigFile,
    /// No MCP: a built-in rule tells the agent to run `pitwall-cli open`.
    ShellRule,
    /// Not available (a plain terminal, or no way to tell the tool).
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
    /// How the `pitwall` MCP server is added at launch (none: it isn't).
    #[serde(default)]
    pub mcp: McpMode,
    /// No launch-time way: the MCP file Pitwall may add its entry to, when
    /// the user asks.
    #[serde(default)]
    pub mcp_file: Option<McpFile>,
    /// rulesync `--targets` id used when applying rules (e.g. "claudecode").
    #[serde(default)]
    pub rulesync_target: Option<String>,
    /// For an agent rulesync has no target for: Pitwall writes the rules
    /// into one file itself (docs/spec/rules.md, "Agents rulesync doesn't
    /// cover"). Ignored when `rulesync_target` is set.
    #[serde(default)]
    pub rules_fallback: Option<RulesFallback>,
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
}

/// How Pitwall gives rules to an agent rulesync doesn't cover: the
/// effective rules' bodies (frontmatter dropped), joined into one file in
/// its working folder, recorded and kept out of git like rulesync's output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesFallback {
    /// The name rules' `targets` use for this agent (`geminicli`); a rule
    /// without `targets`, or with `*`, applies too.
    pub target: String,
    /// The file to write, relative to the working folder (`GEMINI.md`).
    /// Never over a file git tracks or one Pitwall didn't write.
    pub file: String,
    /// Added to the agent's fresh starts while `file` holds its rules;
    /// `{file}` is its absolute path (e.g. Aider's `--read {file}`).
    #[serde(default)]
    pub args: Vec<String>,
    /// Where the rules go when `file` is the project's own (tracked, or
    /// not written by Pitwall). Unset: they aren't written.
    #[serde(default)]
    pub alt_file: Option<String>,
    /// Added to fresh starts while `alt_file` holds the rules, to point
    /// the agent at it (`{file}` as above).
    #[serde(default)]
    pub alt_args: Vec<String>,
}

impl AgentKind {
    /// How this kind gets `agents.showFiles`: an MCP server at launch, in
    /// its MCP file (when the user allows), else a rule saying to run
    /// `pitwall-cli open` for kinds that take rules; plain terminals get
    /// nothing.
    pub fn show_files_route(&self) -> ShowFilesRoute {
        if self.mcp != McpMode::None {
            ShowFilesRoute::Launch
        } else if self.mcp_file.is_some() {
            ShowFilesRoute::ConfigFile
        } else if self.id != "shell" && (self.rulesync_target.is_some() || self.rules_fallback.is_some()) {
            ShowFilesRoute::ShellRule
        } else {
            ShowFilesRoute::None
        }
    }

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
        mcp: McpMode::None,
        rulesync_target: None,
        rules_fallback: None,
        worktree_args: vec![],
        process_names: vec![],
        aliases: vec![],
        account_env: None,
        account_home: None,
        icon: Some("terminal".into()),
        update_command: None,
        update_off_env: Default::default(),
        mcp_file: None,
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
