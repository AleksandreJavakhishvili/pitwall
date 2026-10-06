//! Agent kinds for the UI, with "is it installed" resolved once in the
//! background (each lookup spawns a login shell, which is slow).

use std::collections::HashMap;
use std::sync::{Condvar, Mutex, OnceLock};

use crate::agents::{self, AgentKind};
use crate::model::KindView;
use crate::shell;

type Resolved = HashMap<String, Option<String>>;

struct Cache {
    resolved: Mutex<Option<Resolved>>,
    ready: Condvar,
}

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Cache {
        resolved: Mutex::new(None),
        ready: Condvar::new(),
    })
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

/// Start resolving every kind's program in the background.
pub fn warm_up() {
    std::thread::spawn(|| {
        let mut resolved = Resolved::new();
        for kind in agents::load_kinds() {
            let p = program(&kind.command);
            resolved.entry(p.clone()).or_insert_with(|| resolve(&p));
        }
        let c = cache();
        *c.resolved.lock().unwrap_or_else(|e| e.into_inner()) = Some(resolved);
        c.ready.notify_all();
    });
}

/// Blocks until warm-up finished (call off the main thread).
pub fn list() -> Vec<KindView> {
    let c = cache();
    let mut guard = c.resolved.lock().unwrap_or_else(|e| e.into_inner());
    while guard.is_none() {
        guard = c.ready.wait(guard).unwrap_or_else(|e| e.into_inner());
    }
    let resolved = guard.as_mut().expect("checked above");
    let mut views: Vec<KindView> = agents::load_kinds()
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

pub fn find(id: &str) -> Option<AgentKind> {
    agents::load_kinds().into_iter().find(|k| k.id == id)
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
}
