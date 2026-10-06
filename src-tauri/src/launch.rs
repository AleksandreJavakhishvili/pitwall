//! Building the shell command line an agent runs with.

use crate::agents::{AgentKind, HookMode};
use crate::{hooks, shell};

pub struct Plan {
    pub command_line: String,
    /// Session id the launched process will use (if known up front).
    pub session_id: Option<String>,
    pub resumed: bool,
}

/// `resume`: try to continue `session_id`; falls back to a fresh start when
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
    if kind.hooks == HookMode::ClaudeSettings {
        line.push_str(" --settings ");
        line.push_str(&shell::quote(&hooks::claude_settings_json(hook_command)));
    }
    Plan {
        command_line: line,
        session_id,
        resumed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(src: &str) -> AgentKind {
        toml::from_str(src).unwrap()
    }

    fn claude() -> AgentKind {
        kind(include_str!("../agents/claude.toml"))
    }

    fn codex() -> AgentKind {
        kind(include_str!("../agents/codex.toml"))
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
    fn custom_command_is_kept_as_written() {
        let p = plan(&crate::agents::custom_kind("npx foo --bar"), None, false, "x", None);
        assert_eq!(p.command_line, "npx foo --bar");
        let p = plan(&kind(include_str!("../agents/shell.toml")), None, true, "x", None);
        assert_eq!(p.command_line, "$SHELL");
    }
}
