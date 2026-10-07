//! The agent kinds: built-ins plus the user's files, read fresh each time
//! (user files may change). Whether a kind is installed depends on the
//! machine, so providers answer that (`Provider::kinds`).

use std::path::PathBuf;

use super::{custom_kind, load_kinds, AgentKind};
use crate::shell;

pub struct KindCatalog {
    user_dir: PathBuf,
}

/// The program a kind's command runs (first word; `$VAR` expanded).
pub fn program(command: &str) -> String {
    let first = command.split_whitespace().next().unwrap_or("");
    match first.strip_prefix('$') {
        Some(var) => std::env::var(var).unwrap_or_else(|_| {
            if var == "SHELL" { shell::user_shell() } else { String::new() }
        }),
        None => first.to_string(),
    }
}

impl KindCatalog {
    /// Kinds from the built-ins plus `*.toml` files in `user_dir`.
    pub fn new(user_dir: PathBuf) -> KindCatalog {
        KindCatalog { user_dir }
    }

    /// Every kind definition, read fresh (user files may change any time).
    pub fn kinds(&self) -> Vec<AgentKind> {
        load_kinds(&self.user_dir)
    }

    pub fn find(&self, id: &str) -> Option<AgentKind> {
        self.kinds().into_iter().find(|k| k.id == id)
    }

    /// The kind a platform means by `name` (id or alias, ignoring case).
    pub fn resolve(&self, name: &str) -> Option<AgentKind> {
        self.kinds().into_iter().find(|k| k.answers_to(name))
    }

    /// A stored agent's kind: its custom command, or the definition by id.
    pub fn for_record(&self, kind: &str, custom_command: Option<&str>) -> Option<AgentKind> {
        if kind == super::CUSTOM {
            Some(custom_kind(custom_command.unwrap_or("")))
        } else {
            self.find(kind)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_is_first_word() {
        assert_eq!(program("npx foo --bar"), "npx");
        assert_eq!(program("  claude"), "claude");
        assert_eq!(program(""), "");
        std::env::set_var("PITWALL_TEST_PROG", "/bin/x");
        assert_eq!(program("$PITWALL_TEST_PROG -l"), "/bin/x");
    }

    #[test]
    fn finds_kinds_and_custom_commands() {
        let c = KindCatalog::new(PathBuf::from("/nonexistent/pitwall-agents"));
        assert_eq!(c.find("claude").unwrap().name, "Claude Code");
        assert!(c.find("nope").is_none());
        let custom = c.for_record("custom", Some("npx foo")).unwrap();
        assert_eq!((custom.id.as_str(), custom.command.as_str()), ("custom", "npx foo"));
        assert_eq!(c.for_record("codex", None).unwrap().id, "codex");
    }
}
