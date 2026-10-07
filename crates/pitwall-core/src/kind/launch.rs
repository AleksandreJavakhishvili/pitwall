//! Building the shell command line an agent runs with.

use super::{AgentKind, HookMode};
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
/// start, add the kind's own worktree flag (`{name}` = this name).
pub fn plan(
    kind: &AgentKind,
    session_id: Option<&str>,
    resume: bool,
    hook_command: &str,
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
    // No hook command: the provider can't deliver hooks (caps.hooks = None).
    if kind.hooks == HookMode::ClaudeSettings && !hook_command.is_empty() {
        line.push_str(" --settings ");
        line.push_str(&shell::quote(&hooks::claude_settings_json(hook_command)));
    }
    Plan {
        command_line: line,
        session_id,
        resumed,
    }
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
) -> (AgentKind, Plan) {
    let inner = plan(agent, session_id, resume, hook_command, None);
    let after = plan(shell, None, false, "", None);
    let kind = AgentKind {
        id: shell.id.clone(),
        name: shell.name.clone(),
        command: format!("{}; exec {}", inner.command_line, after.command_line),
        new_args: vec![],
        resume_args: vec![],
        assign_session_id: false,
        hooks: HookMode::None,
        rulesync_target: None,
        worktree_args: vec![],
        worktree_dirs: vec![],
        process_names: vec![],
        aliases: vec![],
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
        let p = plan(&claude(), None, false, "sh '/h'", None);
        let id = p.session_id.clone().unwrap();
        assert!(!p.resumed);
        assert!(p.command_line.starts_with(&format!("claude --session-id {id} --settings '")));
        assert!(p.command_line.contains(r#""command":"sh '\''/h'\''""#));
    }

    #[test]
    fn no_hook_command_means_no_settings() {
        let p = plan(&claude(), None, false, "", None);
        assert!(!p.command_line.contains("--settings"));
    }

    #[test]
    fn claude_resume() {
        let p = plan(&claude(), Some("abc"), true, "x", None);
        assert!(p.resumed);
        assert!(p.command_line.starts_with("claude --resume abc --settings "));
    }

    #[test]
    fn codex_resume_needs_session() {
        let p = plan(&codex(), None, true, "x", None);
        assert_eq!(p.command_line, "codex");
        assert_eq!(p.session_id, None);
        let p = plan(&codex(), Some("s 1"), true, "x", None);
        assert_eq!(p.command_line, "codex resume 's 1'");
    }

    #[test]
    fn worktree_flag_only_on_fresh_starts() {
        let p = plan(&claude(), None, false, "x", Some("fix-login"));
        let id = p.session_id.clone().unwrap();
        assert!(p.command_line.starts_with(&format!("claude --session-id {id} --worktree fix-login --settings ")));
        let p = plan(&claude(), Some("abc"), true, "x", Some("fix-login"));
        assert!(!p.command_line.contains("--worktree"));
        assert_eq!(plan(&codex(), None, false, "x", Some("n")).command_line, "codex --worktree");
        assert_eq!(plan(&codex(), Some("s"), true, "x", Some("n")).command_line, "codex resume s");
    }

    #[test]
    fn a_terminal_continues_its_agent_then_is_a_shell() {
        let shell = kind(include_str!("../../agents/shell.toml"));
        let (k, p) = then_shell(&shell, &claude(), Some("abc"), true, "x");
        assert!(p.resumed && p.session_id.as_deref() == Some("abc"));
        assert_eq!((k.id.as_str(), k.hooks), ("shell", HookMode::None));
        let line = plan(&k, None, false, "x", None).command_line;
        assert!(line.starts_with("claude --resume abc --settings '"), "{line}");
        assert!(line.ends_with("; exec $SHELL"), "{line}");
        // Nothing to resume (or not allowed): the agent starts fresh, with
        // the id it is given up front.
        let (k, p) = then_shell(&shell, &claude(), Some("abc"), false, "");
        let id = p.session_id.clone().unwrap();
        assert!(!p.resumed && id != "abc");
        assert_eq!(k.command, format!("claude --session-id {id}; exec $SHELL"));
        let (k, p) = then_shell(&shell, &codex(), None, true, "x");
        assert_eq!((k.command.as_str(), p.session_id), ("codex; exec $SHELL", None));
    }

    #[test]
    fn custom_command_is_kept_as_written() {
        let p = plan(&crate::kind::custom_kind("npx foo --bar"), None, false, "x", None);
        assert_eq!(p.command_line, "npx foo --bar");
        let p = plan(&kind(include_str!("../../agents/shell.toml")), None, true, "x", None);
        assert_eq!(p.command_line, "$SHELL");
    }
}
