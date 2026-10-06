//! Agent kinds for the UI, with "is it installed" resolved once in the
//! background (each lookup spawns a login shell, which is slow).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

use super::{custom_kind, load_kinds, AgentKind};
use crate::model::KindView;
use crate::shell;

type Resolved = HashMap<String, Option<String>>;

/// The agent kinds (built-ins + the user's files) and where each kind's
/// program is installed. Owned by the engine; resolving programs is slow, so
/// [`warm_up`](Self::warm_up) does it once in the background.
pub struct KindCatalog {
    user_dir: PathBuf,
    resolved: Mutex<Option<Resolved>>,
    ready: Condvar,
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

fn resolve(program: &str) -> Option<String> {
    if program.is_empty() {
        return None;
    }
    if program.starts_with('/') {
        return std::path::Path::new(program).exists().then(|| program.to_string());
    }
    shell::which(program)
}

impl KindCatalog {
    /// Kinds from the built-ins plus `*.toml` files in `user_dir`.
    pub fn new(user_dir: PathBuf) -> KindCatalog {
        KindCatalog { user_dir, resolved: Mutex::new(None), ready: Condvar::new() }
    }

    /// Every kind definition, read fresh (user files may change any time).
    pub fn kinds(&self) -> Vec<AgentKind> {
        load_kinds(&self.user_dir)
    }

    pub fn find(&self, id: &str) -> Option<AgentKind> {
        self.kinds().into_iter().find(|k| k.id == id)
    }

    /// A stored agent's kind: its custom command, or the definition by id.
    pub fn for_record(&self, kind: &str, custom_command: Option<&str>) -> Option<AgentKind> {
        if kind == "custom" {
            Some(custom_kind(custom_command.unwrap_or("")))
        } else {
            self.find(kind)
        }
    }

    /// Start resolving every kind's program in the background.
    pub fn warm_up(self: &Arc<Self>) {
        let this = self.clone();
        std::thread::spawn(move || {
            let mut resolved = Resolved::new();
            for kind in this.kinds() {
                let p = program(&kind.command);
                resolved.entry(p.clone()).or_insert_with(|| resolve(&p));
            }
            *this.resolved.lock().unwrap_or_else(|e| e.into_inner()) = Some(resolved);
            this.ready.notify_all();
        });
    }

    /// Kinds for the New-agent dialog. Blocks until warm-up finished (call
    /// off the main thread).
    pub fn list(&self) -> Vec<KindView> {
        let mut guard = self.resolved.lock().unwrap_or_else(|e| e.into_inner());
        while guard.is_none() {
            guard = self.ready.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
        let resolved = guard.as_mut().expect("checked above");
        let mut views: Vec<KindView> = self
            .kinds()
            .into_iter()
            .map(|kind| {
                let p = program(&kind.command);
                // Kinds added since startup get resolved now (rare).
                let path = resolved.entry(p.clone()).or_insert_with(|| resolve(&p)).clone();
                KindView {
                    id: kind.id,
                    name: kind.name,
                    installed: path.is_some(),
                    path,
                    worktree: !kind.worktree_args.is_empty(),
                }
            })
            .collect();
        views.push(KindView {
            id: "custom".into(),
            name: "Custom command".into(),
            installed: true,
            path: None,
            worktree: false,
        });
        views
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
