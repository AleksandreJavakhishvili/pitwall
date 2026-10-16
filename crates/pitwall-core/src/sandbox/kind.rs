//! A kind's `[share]` data resolved on this machine, for the sandbox
//! policy (docs/spec/collaboration/sessions.md §6.7): which folders hold
//! the agent's install, which programs it needs to run, which hosts its
//! model is at.

use std::path::{Path, PathBuf};

use crate::kind::AgentKind;

/// The agent's install on this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Install {
    /// Readable folders and files (`SandboxPolicy::read`).
    pub read: Vec<PathBuf>,
    /// Programs it may run (`ExecRule::Allow`; a folder stands for the
    /// programs inside it): its command, its runtimes, its install folders.
    pub exec: Vec<PathBuf>,
    /// Folders to put on the session's `PATH`, in order.
    pub path: Vec<PathBuf>,
    /// The model hosts for the egress proxy.
    pub hosts: Vec<String>,
}

/// `~/x` in `home`; other paths as they are.
pub fn expand(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if path == "~" => home.to_path_buf(),
        None => PathBuf::from(path),
    }
}

/// `name` on `path` (a `PATH` value), as an absolute path.
pub fn which(name: &str, path: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return p.is_absolute().then_some(p).filter(|p| p.is_file());
    }
    path.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join(name)).find(|p| p.is_file())
}

/// Resolve `kind`'s `[share]` data: `home` is the owner's home (for `~/`),
/// `program` the kind's command already found on the owner's PATH (e.g.
/// `/opt/homebrew/bin/claude`), `search` the PATH to find `programs` on.
pub fn resolve(kind: &AgentKind, home: &Path, program: Option<&Path>, search: &str) -> Install {
    let mut out = Install::default();
    let share = kind.share.clone().unwrap_or_default();
    for p in &share.install_paths {
        let p = expand(p, home);
        if p.exists() {
            push(&mut out.read, p.clone());
            push(&mut out.exec, p);
        }
    }
    let mut programs: Vec<PathBuf> = program.map(Path::to_path_buf).into_iter().collect();
    programs.extend(share.programs.iter().filter_map(|n| which(n, search)));
    for p in programs {
        // The link (`~/.local/bin/claude`) and what it points at, readable
        // and runnable; its folder on PATH so the agent finds itself.
        if let Some(dir) = p.parent() {
            push(&mut out.path, dir.to_path_buf());
        }
        if let Ok(real) = std::fs::canonicalize(&p) {
            if real != p {
                push(&mut out.read, real.clone());
                push(&mut out.exec, real);
            }
        }
        push(&mut out.read, p.clone());
        push(&mut out.exec, p);
    }
    out.hosts = share.model_hosts;
    out
}

fn push(list: &mut Vec<PathBuf>, p: PathBuf) {
    if !list.contains(&p) {
        list.push(p);
    }
}

/// Programs a share's sessions may run by default at Read only (the
/// allowlist of sessions.md §6.3, plus the agent's own from [`resolve`]):
/// the shell, git and common read-only tools. Names; resolve with
/// [`which`].
pub const DEFAULT_ALLOW: &[&str] = &[
    "sh", "bash", "zsh", "env", "git", "ls", "cat", "head", "tail", "wc", "sort", "uniq", "cut", "tr", "diff", "rg",
    "grep", "egrep", "fgrep", "find", "xargs", "sed", "awk", "dirname", "basename", "pwd", "echo", "test", "true",
    "false", "stat", "file", "tree", "less", "realpath", "readlink", "date", "uname", "which",
];

/// Programs denied by default with a denylist (sessions.md §6.3).
pub const DEFAULT_DENY: &[&str] = &["curl", "wget", "ssh", "scp", "sftp", "nc", "ncat", "telnet", "osascript", "security"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn bundled_kinds_have_share_data() {
        let kinds = crate::kind::load_kinds(Path::new("/nonexistent/pitwall-agents"));
        let find = |id: &str| kinds.iter().find(|k| k.id == id).unwrap().share.clone();
        let claude = find("claude").expect("claude has [share]");
        assert!(claude.model_hosts.contains(&"api.anthropic.com".to_string()));
        assert!(claude.keychain);
        assert!(find("codex").unwrap().model_hosts.contains(&"api.openai.com".to_string()));
        assert!(find("gemini").unwrap().model_hosts.contains(&"generativelanguage.googleapis.com".to_string()));
        for id in ["claude", "codex", "gemini"] {
            for h in find(id).unwrap().model_hosts {
                assert!(super::super::proxy::HostRule::parse(&h).is_some(), "{id}: {h}");
            }
        }
        assert!(find("shell").is_none());
    }

    #[test]
    fn resolves_install_programs_and_hosts() {
        let dir = TempDir::new("share-kind");
        let home = dir.path().join("home");
        let pkg = home.join(".local/share/drift/1.0");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("drift"), "#!/bin/sh\n").unwrap();
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        crate::platform::symlink(&pkg.join("drift"), &bin.join("drift")).unwrap();
        let rt = dir.path().join("rt");
        std::fs::create_dir_all(&rt).unwrap();
        std::fs::write(rt.join("node"), "").unwrap();

        let mut kind = crate::kind::custom_kind("drift");
        kind.share = Some(crate::kind::ShareKind {
            model_hosts: vec!["api.example.com".into()],
            install_paths: vec!["~/.local/share/drift".into(), "~/missing".into()],
            programs: vec!["node".into(), "nope".into()],
            keychain: false,
            ..Default::default()
        });
        let i = resolve(&kind, &home, Some(&bin.join("drift")), &rt.to_string_lossy());
        let real_pkg = std::fs::canonicalize(home.join(".local/share/drift")).unwrap();
        let real_bin = std::fs::canonicalize(pkg.join("drift")).unwrap();
        assert!(i.read.contains(&home.join(".local/share/drift")) || i.read.contains(&real_pkg));
        assert!(i.exec.contains(&bin.join("drift")) && i.exec.contains(&real_bin));
        assert!(i.exec.contains(&rt.join("node")));
        assert!(!i.read.iter().any(|p| p.ends_with("missing")));
        assert_eq!(i.path, [bin, rt]);
        assert_eq!(i.hosts, ["api.example.com"]);
        assert_eq!(expand("~/x", Path::new("/h")), Path::new("/h/x"));
        assert_eq!(expand("/abs", Path::new("/h")), Path::new("/abs"));
    }
}
