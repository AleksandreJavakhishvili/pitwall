//! Git plumbing. Read-only operations run with GIT_OPTIONAL_LOCKS=0 so polling
//! never fights with the agents for the index lock.

use std::path::Path;
use std::process::Command;

use serde::Serialize;

fn git(cwd: &str) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(cwd).env("GIT_OPTIONAL_LOCKS", "0");
    cmd
}

fn run(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub fn repo_root(path: &str) -> Option<String> {
    run(git(path).args(["rev-parse", "--show-toplevel"]))
        .ok()
        .map(|s| s.trim().to_string())
}

pub fn head(path: &str) -> Option<String> {
    run(git(path).args(["rev-parse", "HEAD"]))
        .ok()
        .map(|s| s.trim().to_string())
}

/// Current branch name, or `None` when detached or not a repo.
pub fn current_branch(path: &str) -> Option<String> {
    run(git(path).args(["symbolic-ref", "--quiet", "--short", "HEAD"]))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `git worktree remove` (never `--force`: refuses on uncommitted changes).
pub fn worktree_remove(repo: &str, path: &str) -> Result<(), String> {
    run(git(repo).args(["worktree", "remove", path])).map(|_| ())
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorktreeEntry {
    pub path: String,
    /// Short branch name; `None` when detached.
    pub branch: Option<String>,
}

/// Every worktree of the repo at `repo` (the main checkout first).
pub fn worktree_list(repo: &str) -> Result<Vec<WorktreeEntry>, String> {
    Ok(parse_worktree_list(&run(git(repo).args(["worktree", "list", "--porcelain"]))?))
}

pub fn parse_worktree_list(out: &str) -> Vec<WorktreeEntry> {
    let mut res: Vec<WorktreeEntry> = Vec::new();
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            res.push(WorktreeEntry { path: p.to_string(), branch: None });
        } else if let (Some(b), Some(last)) = (line.strip_prefix("branch "), res.last_mut()) {
            last.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
        }
    }
    res
}

pub fn merge_base(cwd: &str, a: &str, b: &str) -> Option<String> {
    run(git(cwd).args(["merge-base", a, b])).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct FileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    pub untracked: bool,
    pub binary: bool,
}

/// Everything that differs from `base` in the working tree, including commits
/// the agent made and files it created but never added.
/// Error returned by [`changes`] when the agent works outside a git repository;
/// the UI shows a calm note instead of git's usage text.
pub const NOT_A_REPO: &str = "not-a-git-repo";

pub fn changes(cwd: &str, base: Option<&str>) -> Result<Vec<FileChange>, String> {
    let base = base.unwrap_or("HEAD");
    let mut files = Vec::new();
    let numstat = run(git(cwd).args(["diff", "--numstat", base])).map_err(|e| {
        if e.contains("Not a git repository") || e.contains("not a git repository") {
            NOT_A_REPO.to_string()
        } else {
            e
        }
    })?;
    for line in numstat.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(a), Some(r), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        files.push(FileChange {
            path: path.to_string(),
            added: a.parse().unwrap_or(0),
            removed: r.parse().unwrap_or(0),
            untracked: false,
            binary: a == "-",
        });
    }
    let untracked = run(git(cwd).args(["ls-files", "--others", "--exclude-standard"]))?;
    for path in untracked.lines().filter(|l| !l.is_empty()) {
        let full = Path::new(cwd).join(path);
        let (added, binary) = match std::fs::metadata(&full) {
            Ok(meta) if meta.len() <= 1_000_000 => match std::fs::read(&full) {
                Ok(bytes) if !bytes.contains(&0) => {
                    let lines = bytes.iter().filter(|&&b| b == b'\n').count();
                    let trailing = usize::from(!bytes.is_empty() && !bytes.ends_with(b"\n"));
                    ((lines + trailing) as u32, false)
                }
                _ => (0, true),
            },
            _ => (0, true),
        };
        files.push(FileChange {
            path: path.to_string(),
            added,
            removed: 0,
            untracked: true,
            binary,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

pub fn file_diff(cwd: &str, base: Option<&str>, path: &str, untracked: bool) -> Result<String, String> {
    if untracked {
        // `--no-index` exits 1 when files differ, which is the expected case.
        let out = git(cwd)
            .args(["diff", "--no-index", "--", "/dev/null", path])
            .output()
            .map_err(|e| format!("git: {e}"))?;
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    run(git(cwd).args(["diff", base.unwrap_or("HEAD"), "--", path]))
}
