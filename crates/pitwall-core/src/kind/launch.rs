//! Building the shell command line an agent runs with.

use super::{AgentKind, HookMode, McpMode};
use crate::provider::McpWiring;
use crate::{hooks, shell};

pub struct Plan {
    pub command_line: String,
    /// Session id the launched process will use (if known up front).
    pub session_id: Option<String>,
    pub resumed: bool,
}

/// The command line for a launch, for providers that launch a command
/// themselves (local, ssh). `resume`: try to continue `session_id`; falls back to a fresh start when
/// the kind can't resume or there is nothing to resume. `worktree`: on a fresh
/// start, add the kind's own worktree flag (`{name}` = this name). `mcp`:
/// Pitwall's MCP server, added the kind's way ([`McpMode`]).
pub fn plan(
    kind: &AgentKind,
    session_id: Option<&str>,
    resume: bool,
    hook_command: &str,
    mcp: Option<&McpWiring>,
    worktree: Option<&str>,
) -> Plan {
    let (args, session_id, resumed) = match session_id {
        Some(id) if resume && !kind.resume_args.is_empty() => {
            (&kind.resume_args, Some(id.to_string()), true)
        }
        _ => {
            let fresh = kind
                .assign_session_id
                .then(|| uuid::Uuid::new_v4().to_string());
            (&kind.new_args, fresh, false)
        }
    };
    let mut line = kind.command.trim().to_string();
    // Codex: config overrides go before its own arguments (`codex -c … resume <id>`).
    if let (Some(m), McpMode::CodexConfig) = (mcp, kind.mcp) {
        for arg in codex_mcp_args(m) {
            line.push(' ');
            line.push_str(&shell::quote(&arg));
        }
    }
    for arg in args {
        if arg.contains("{session_id}") {
            let Some(id) = &session_id else { continue };
            line.push(' ');
            line.push_str(&shell::quote(&arg.replace("{session_id}", id)));
        } else {
            line.push(' ');
            line.push_str(&shell::quote(arg));
        }
    }
    if let (Some(name), false) = (worktree, resumed) {
        for arg in &kind.worktree_args {
            line.push(' ');
            line.push_str(&shell::quote(&arg.replace("{name}", name)));
        }
    }
    let claude_mcp = mcp.filter(|_| kind.mcp == McpMode::ClaudeConfig);
    if let Some(m) = claude_mcp {
        line.push_str(" --mcp-config ");
        line.push_str(&shell::quote(&hooks::json_arg(&claude_mcp_json(m), "claude-mcp")));
    }
    // No hook command: the provider can't deliver hooks (caps.hooks = None).
    let hook = (kind.hooks == HookMode::ClaudeSettings && !hook_command.is_empty()).then_some(hook_command);
    // The MCP tools only show the user files in Pitwall: allowed without a
    // prompt each time (Claude Code's own permission rules, merged with
    // the user's).
    let allow: Vec<String> = match claude_mcp {
        Some(_) => pitwall_proto::show::TOOLS.iter().map(|t| format!("mcp__{}__{t}", pitwall_proto::show::MCP_SERVER)).collect(),
        None => vec![],
    };
    if hook.is_some() || !allow.is_empty() {
        line.push_str(" --settings ");
        line.push_str(&shell::quote(&hooks::json_arg(&hooks::claude_settings(hook, &allow), "claude-hooks")));
    }
    Plan {
        command_line: line,
        session_id,
        resumed,
    }
}

/// `claude --mcp-config`'s JSON: the one stdio server.
pub fn claude_mcp_json(m: &McpWiring) -> String {
    // Spelled out, so the keys keep this order whichever way serde_json
    // was built (`preserve_order` or not).
    fn text(v: &impl serde::Serialize) -> String {
        serde_json::to_string(v).unwrap_or_default()
    }
    format!(
        r#"{{"mcpServers":{{{}:{{"command":{},"args":{}}}}}}}"#,
        text(&pitwall_proto::show::MCP_SERVER),
        text(&m.command),
        text(&m.args),
    )
}

/// `codex -c` overrides defining the server (values are TOML).
pub fn codex_mcp_args(m: &McpWiring) -> Vec<String> {
    let name = pitwall_proto::show::MCP_SERVER;
    let args = toml::Value::Array(m.args.iter().cloned().map(toml::Value::String).collect());
    vec![
        "-c".into(),
        format!("mcp_servers.{name}.command={}", toml::Value::String(m.command.clone())),
        "-c".into(),
        format!("mcp_servers.{name}.args={args}"),
    ]
}

/// A terminal that continues the agent last started in it by hand
/// (docs/spec/terminals.md §3): its login shell runs the agent (resuming
/// `session_id` when `resume` and the kind can) and then, once the agent
/// exits, the terminal's own shell, so the pane is a shell again as before.
/// Returns the kind to launch (a plain command line, the shell's id and
/// name) and the agent's own plan (its conversation id).
pub fn then_shell(
    shell: &AgentKind,
    agent: &AgentKind,
    session_id: Option<&str>,
    resume: bool,
    hook_command: &str,
    mcp: Option<&McpWiring>,
) -> (AgentKind, Plan) {
    let inner = plan(agent, session_id, resume, hook_command, mcp, None);
    let after = plan(shell, None, false, "", None, None);
    let kind = AgentKind {
        id: shell.id.clone(),
        name: shell.name.clone(),
        command: shell::LoginShell::current().then(&inner.command_line, &after.command_line),
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
        icon: None,
        update_command: None,
        update_off_env: Default::default(),
    };
    (kind, inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(src: &str) -> AgentKind {
        toml::from_str(src).unwrap()
    }

    fn claude() -> AgentKind {
        kind(include_str!("../../agents/claude.toml"))
    }

    fn codex() -> AgentKind {
        kind(include_str!("../../agents/codex.toml"))
    }

    #[test]
    fn claude_new_gets_session_and_settings() {
        let p = plan(&claude(), None, false, "sh '/h'", None, None);
        let id = p.session_id.clone().unwrap();
        assert!(!p.resumed);
        assert!(p.command_line.starts_with(&format!("claude --session-id {id} --settings '")));
        assert!(p.command_line.contains(r#""command":"sh '\''/h'\''""#));
    }

    #[test]
    fn no_hook_command_means_no_settings() {
        let p = plan(&claude(), None, false, "", None, None);
        assert!(!p.command_line.contains("--settings"));
    }

    #[test]
    fn claude_resume() {
        let p = plan(&claude(), Some("abc"), true, "x", None, None);
        assert!(p.resumed);
        assert!(p.command_line.starts_with("claude --resume abc --settings "));
    }

    #[test]
    fn codex_resume_needs_session() {
        let p = plan(&codex(), None, true, "x", None, None);
        assert_eq!(p.command_line, "codex");
        assert_eq!(p.session_id, None);
        let p = plan(&codex(), Some("s 1"), true, "x", None, None);
        assert_eq!(p.command_line, "codex resume 's 1'");
    }

    #[test]
    fn worktree_flag_only_on_fresh_starts() {
        let p = plan(&claude(), None, false, "x", None, Some("fix-login"));
        let id = p.session_id.clone().unwrap();
        assert!(p.command_line.starts_with(&format!("claude --session-id {id} --worktree fix-login --settings ")));
        let p = plan(&claude(), Some("abc"), true, "x", None, Some("fix-login"));
        assert!(!p.command_line.contains("--worktree"));
        assert_eq!(plan(&codex(), None, false, "x", None, Some("n")).command_line, "codex --worktree");
        assert_eq!(plan(&codex(), Some("s"), true, "x", None, Some("n")).command_line, "codex resume s");
    }

    #[test]
    fn a_terminal_continues_its_agent_then_is_a_shell() {
        let shell = kind(include_str!("../../agents/shell.toml"));
        let (k, p) = then_shell(&shell, &claude(), Some("abc"), true, "x", None);
        assert!(p.resumed && p.session_id.as_deref() == Some("abc"));
        assert_eq!((k.id.as_str(), k.hooks), ("shell", HookMode::None));
        let line = plan(&k, None, false, "x", None, None).command_line;
        assert!(line.starts_with("claude --resume abc --settings '"), "{line}");
        assert!(line.ends_with("; exec $SHELL"), "{line}");
        // Nothing to resume (or not allowed): the agent starts fresh, with
        // the id it is given up front.
        let (k, p) = then_shell(&shell, &claude(), Some("abc"), false, "", None);
        let id = p.session_id.clone().unwrap();
        assert!(!p.resumed && id != "abc");
        assert_eq!(k.command, format!("claude --session-id {id}; exec $SHELL"));
        let (k, p) = then_shell(&shell, &codex(), None, true, "x", None);
        assert_eq!((k.command.as_str(), p.session_id), ("codex; exec $SHELL", None));
    }

    #[test]
    fn custom_command_is_kept_as_written() {
        let p = plan(&crate::kind::custom_kind("npx foo --bar"), None, false, "x", None, None);
        assert_eq!(p.command_line, "npx foo --bar");
        let p = plan(&kind(include_str!("../../agents/shell.toml")), None, true, "x", None, None);
        assert_eq!(p.command_line, "$SHELL");
    }

    fn mcp() -> McpWiring {
        McpWiring { command: "/Apps/Pit wall/pitwall-cli".into(), args: vec!["mcp".into()] }
    }

    /// The server goes in each kind's documented way, or not at all.
    #[test]
    fn the_mcp_server_per_kind() {
        let m = mcp();
        // Claude Code: --mcp-config with the server, and the tools allowed
        // in the --settings that also carries the hooks.
        let p = plan(&claude(), None, false, "sh '/h'", Some(&m), None);
        let line = &p.command_line;
        let json = r#"{"mcpServers":{"pitwall":{"command":"/Apps/Pit wall/pitwall-cli","args":["mcp"]}}}"#;
        assert!(line.contains(&format!(" --mcp-config '{json}' --settings '")), "{line}");
        assert!(line.contains(r#""permissions":{"allow":["mcp__pitwall__show_files","mcp__pitwall__show_folder"]}"#), "{line}");
        assert!(line.contains(r#""hooks":{"#), "{line}");
        // No hooks (a provider that can't deliver them): still the server
        // and its permissions.
        let p = plan(&claude(), Some("abc"), true, "", Some(&m), None);
        assert!(p.command_line.starts_with("claude --resume abc --mcp-config '"), "{}", p.command_line);
        assert!(p.command_line.ends_with(r#"--settings '{"permissions":{"allow":["mcp__pitwall__show_files","mcp__pitwall__show_folder"]}}'"#));
        // Codex: -c overrides before its own arguments.
        let p = plan(&codex(), Some("s1"), true, "x", Some(&m), None);
        assert_eq!(
            p.command_line,
            r#"codex -c 'mcp_servers.pitwall.command="/Apps/Pit wall/pitwall-cli"' -c 'mcp_servers.pitwall.args=["mcp"]' resume s1"#
        );
        // Kinds without a safe launch-time way: nothing added.
        for src in [include_str!("../../agents/gemini.toml"), include_str!("../../agents/shell.toml"), include_str!("../../agents/opencode.toml")] {
            let k = kind(src);
            assert_eq!(k.mcp, McpMode::None);
            assert_eq!(plan(&k, None, false, "x", Some(&m), None).command_line, plan(&k, None, false, "x", None, None).command_line);
        }
        // Off (no wiring): as before.
        assert!(!plan(&claude(), None, false, "x", None, None).command_line.contains("mcp"));
        // A terminal continuing Claude Code passes it on.
        let shell = kind(include_str!("../../agents/shell.toml"));
        let (k, _) = then_shell(&shell, &claude(), Some("abc"), true, "x", Some(&m));
        assert!(k.command.starts_with("claude --resume abc --mcp-config '"), "{}", k.command);
    }
}
