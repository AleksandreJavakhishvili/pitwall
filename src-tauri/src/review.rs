//! Review screen backend (docs/spec/review.md): per-task changes, file versions
//! for the Monaco diff editor, and the explicit user actions discard / commit /
//! merge. Read-only git runs with GIT_OPTIONAL_LOCKS=0. Paths are relative to
//! the repository root of the agent's checkout.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;

use crate::git::{self, FileChange};
use crate::registry::Shared;
use crate::tasks::{self, git as g, run, Task};

type Res<T> = Result<T, String>;

/// Files larger than this are not sent to the editor.
const MAX_EDITOR_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileVersions {
    pub original: Option<String>,
    pub modified: Option<String>,
    /// Binary or too large to show: both sides are `null`.
    pub binary: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MergeResult {
    pub merged: bool,
    pub conflict: bool,
    pub message: String,
    /// Branch of the main checkout the merge went (or would go) into.
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MergeStatus {
    pub worktree: bool,
    /// The agent's branch (for worktree agents: the worktree's current branch).
    pub branch: Option<String>,
    /// The main checkout's current branch (merge target); worktree agents only.
    pub target: Option<String>,
    /// Main checkout has uncommitted changes to tracked files (merge refused).
    pub target_dirty: bool,
    /// Uncommitted entries (`git status --porcelain`) in the agent's checkout.
    pub uncommitted: u32,
    /// Commits on the agent's branch not yet in the target.
    pub ahead: u32,
}

// ------------------------------------------------------------------ helpers

fn valid_path(path: &str) -> Res<()> {
    if path.is_empty() || path.starts_with('/') || path.split('/').any(|p| p == ".." || p == ".git") {
        return Err("invalid path".into());
    }
    Ok(())
}

pub fn toplevel(cwd: &str) -> Res<String> {
    git::repo_root(cwd).ok_or_else(|| "not a git repository".to_string())
}

/// Raw bytes of `<rev>:<path>`, or `None` if it doesn't exist there.
fn blob(top: &str, rev: &str, path: &str) -> Option<Vec<u8>> {
    let out = g(top).args(["cat-file", "blob", &format!("{rev}:{path}")]).output().ok()?;
    out.status.success().then_some(out.stdout)
}

fn disk(top: &str, path: &str) -> Option<Vec<u8>> {
    let p = Path::new(top).join(path);
    if std::fs::symlink_metadata(&p).ok()?.is_dir() {
        return None;
    }
    std::fs::read(p).ok()
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes.len() > MAX_EDITOR_BYTES || bytes[..bytes.len().min(8000)].contains(&0) || std::str::from_utf8(bytes).is_err()
}

fn versions(original: Option<Vec<u8>>, modified: Option<Vec<u8>>) -> FileVersions {
    let binary = original.as_deref().is_some_and(is_binary) || modified.as_deref().is_some_and(is_binary);
    if binary {
        return FileVersions { original: None, modified: None, binary };
    }
    let text = |b: Vec<u8>| String::from_utf8(b).ok();
    FileVersions { original: original.and_then(text), modified: modified.and_then(text), binary }
}

fn base_rev(top: &str, base: Option<&str>) -> String {
    match base {
        Some(b) if run(g(top).args(["cat-file", "-e", &format!("{b}^{{commit}}")])).is_ok() => b.to_string(),
        _ => "HEAD".into(),
    }
}

// ------------------------------------------------------------------ pure git ops (tested)

/// Changes between two trees, NUL-separated so odd paths survive.
pub fn tree_changes(top: &str, from: &str, to: &str) -> Res<Vec<FileChange>> {
    let names = run(g(top).args(["diff", "--name-status", "--no-renames", "-z", from, to]))?;
    let mut added = std::collections::HashSet::new();
    let mut it = names.split('\0');
    while let (Some(st), Some(path)) = (it.next(), it.next()) {
        if st == "A" {
            added.insert(path.to_string());
        }
    }
    let stat = run(g(top).args(["diff", "--numstat", "--no-renames", "-z", from, to]))?;
    let mut files = Vec::new();
    for rec in stat.split('\0').filter(|r| !r.is_empty()) {
        let mut parts = rec.splitn(3, '\t');
        let (Some(a), Some(r), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        files.push(FileChange {
            path: path.to_string(),
            added: a.parse().unwrap_or(0),
            removed: r.parse().unwrap_or(0),
            untracked: added.contains(path),
            binary: a == "-",
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Original/modified text of `path`. With a task: start tree vs end tree (or
/// the working tree while it's running). Without: `base` vs the working tree.
pub fn file_versions(top: &str, path: &str, base: Option<&str>, task: Option<&Task>) -> Res<FileVersions> {
    valid_path(path)?;
    match task {
        Some(t) => {
            let start = t.start_tree.as_deref().ok_or("this task's snapshot isn't ready yet")?;
            let modified = match &t.end_tree {
                Some(end) => blob(top, end, path),
                None => disk(top, path),
            };
            Ok(versions(blob(top, start, path), modified))
        }
        None => {
            let base = base_rev(top, base);
            Ok(versions(blob(top, &base, path), disk(top, path)))
        }
    }
}

/// Undo an agent's change to one file: restore it from `base` (index and
/// working tree), or delete it if it didn't exist there.
pub fn discard(top: &str, base: Option<&str>, path: &str) -> Res<()> {
    valid_path(path)?;
    let base = base_rev(top, base);
    let lit = |cmd: &mut std::process::Command| {
        cmd.arg("--literal-pathspecs");
    };
    let mut probe = g(top);
    lit(&mut probe);
    if run(probe.args(["cat-file", "-e", &format!("{base}:{path}")])).is_ok() {
        let mut cmd = g(top);
        lit(&mut cmd);
        run(cmd.args(["restore", &format!("--source={base}"), "--staged", "--worktree", "--", path]))?;
        return Ok(());
    }
    let mut cmd = g(top);
    lit(&mut cmd);
    run(cmd.args(["rm", "--cached", "--ignore-unmatch", "-q", "--", path]))?;
    let full: PathBuf = Path::new(top).join(path);
    match std::fs::symlink_metadata(&full) {
        Ok(m) if m.is_dir() => Err("refusing to delete a directory".into()),
        Ok(_) => std::fs::remove_file(&full).map_err(|e| e.to_string()),
        Err(_) => Ok(()),
    }
}

fn porcelain(cwd: &str, tracked_only: bool) -> Res<Vec<String>> {
    let mut cmd = g(cwd);
    cmd.args(["status", "--porcelain"]);
    if tracked_only {
        cmd.arg("--untracked-files=no");
    }
    Ok(run(&mut cmd)?.lines().filter(|l| !l.is_empty()).map(String::from).collect())
}

/// `git add -A && git commit -m <message>`; returns the short commit id.
pub fn commit(cwd: &str, message: &str) -> Res<String> {
    if message.trim().is_empty() {
        return Err("commit message is empty".into());
    }
    if porcelain(cwd, false)?.is_empty() {
        return Err("nothing to commit".into());
    }
    run(g(cwd).args(["add", "-A"]))?;
    run(g(cwd).args(["commit", "-q", "-m", message]))?;
    Ok(run(g(cwd).args(["rev-parse", "--short", "HEAD"]))?.trim().to_string())
}

fn merging(repo: &str) -> bool {
    run(g(repo).args(["rev-parse", "-q", "--verify", "MERGE_HEAD"])).is_ok()
}

/// `git merge --no-ff <branch>` into the main checkout's current branch.
/// Refuses on a dirty checkout; aborts on conflicts.
pub fn merge(repo: &str, branch: &str) -> Res<MergeResult> {
    let target = git::current_branch(repo);
    let refuse = |message: String| MergeResult { merged: false, conflict: false, message, branch: target.clone() };
    let Some(tb) = target.clone() else {
        return Ok(refuse(format!("{repo} is on a detached HEAD; check out a branch first.")));
    };
    if merging(repo) {
        return Ok(refuse(format!("{repo} already has a merge in progress.")));
    }
    if !porcelain(repo, true)?.is_empty() {
        return Ok(refuse(format!(
            "{} has uncommitted changes on {tb}. Commit or stash them, then merge again.",
            crate::paths::tildify(repo)
        )));
    }
    if run(g(repo).args(["merge-base", "--is-ancestor", branch, "HEAD"])).is_ok() {
        return Ok(refuse(format!("Nothing to merge: {branch} is already in {tb}.")));
    }
    let out = g(repo)
        .env("GIT_MERGE_AUTOEDIT", "no")
        .args(["merge", "--no-ff", "--no-edit", branch])
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        return Ok(MergeResult { merged: true, conflict: false, message: format!("Merged {branch} into {tb}."), branch: target });
    }
    if merging(repo) {
        let files = run(g(repo).args(["diff", "--name-only", "--diff-filter=U"])).unwrap_or_default();
        let files: Vec<&str> = files.lines().filter(|l| !l.is_empty()).collect();
        run(g(repo).args(["merge", "--abort"]))?;
        let list = if files.is_empty() { String::new() } else { format!(" in {}", files.join(", ")) };
        return Ok(MergeResult {
            merged: false,
            conflict: true,
            message: format!("Merging {branch} into {tb} conflicts{list}. The merge was aborted; nothing changed."),
            branch: target,
        });
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let err = if err.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_string() } else { err };
    Ok(refuse(format!("git merge failed: {err}")))
}

/// The branch a worktree is on now (`git branch --show-current`), else the
/// one it was on when found. Never assumed from the agent's name.
pub fn worktree_branch(wt: &crate::model::WorktreeInfo) -> Option<String> {
    git::current_branch(&wt.path).or_else(|| Some(wt.branch.clone()).filter(|b| !b.is_empty()))
}

/// `wt`: (main repo, the worktree's branch; `None` = detached).
pub fn merge_status(cwd: &str, wt: Option<(&str, Option<&str>)>) -> Res<MergeStatus> {
    let uncommitted = porcelain(cwd, false)?.len() as u32;
    let Some((repo, branch)) = wt else {
        return Ok(MergeStatus {
            worktree: false,
            branch: git::current_branch(cwd),
            target: None,
            target_dirty: false,
            uncommitted,
            ahead: 0,
        });
    };
    let target = git::current_branch(repo);
    let ahead = target
        .as_ref()
        .zip(branch)
        .and_then(|(t, b)| run(g(repo).args(["rev-list", "--count", &format!("{t}..{b}")])).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    Ok(MergeStatus {
        worktree: true,
        branch: branch.map(String::from),
        target,
        target_dirty: !porcelain(repo, true)?.is_empty(),
        uncommitted,
        ahead,
    })
}

// ------------------------------------------------------------------ commands

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Res<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("internal error: {e}"))?
}

struct Ctx {
    cwd: String,
    base: Option<String>,
    task: Option<Task>,
    worktree: Option<crate::model::WorktreeInfo>,
}

fn ctx(core: &Shared, agent_id: &str, task_id: Option<&str>) -> Res<Ctx> {
    core.with(agent_id, |a| {
        let task = match task_id {
            Some(id) => Some(a.rec.tasks.iter().find(|t| t.id == id).cloned().ok_or("task not found")?),
            None => None,
        };
        Ok(Ctx { cwd: a.rec.cwd.clone(), base: a.rec.base_commit.clone(), task, worktree: a.rec.worktree.clone() })
    })?
}

fn refresh(core: &Shared, agent_id: &str) {
    let _ = core.with(agent_id, |a| a.git_wanted = true);
    core.changed(false);
}

#[tauri::command]
pub fn list_tasks(core: State<'_, Shared>, agent_id: String) -> Res<Vec<Task>> {
    core.with(&agent_id, |a| a.rec.tasks.clone())
}

/// `taskId` null = all changes since the agent started (repo-root paths).
#[tauri::command]
pub async fn get_task_changes(core: State<'_, Shared>, agent_id: String, task_id: Option<String>) -> Res<Vec<FileChange>> {
    let c = ctx(&core, &agent_id, task_id.as_deref())?;
    blocking(move || {
        let top = toplevel(&c.cwd)?;
        match c.task {
            None => git::changes(&top, c.base.as_deref()),
            Some(t) => {
                let start = t.start_tree.ok_or("this task's snapshot isn't ready yet")?;
                let end = match t.end_tree {
                    Some(e) => e,
                    None => tasks::snapshot(&top)?,
                };
                tree_changes(&top, &start, &end)
            }
        }
    })
    .await
}

#[tauri::command]
pub async fn get_file_versions(
    core: State<'_, Shared>,
    agent_id: String,
    path: String,
    task_id: Option<String>,
) -> Res<FileVersions> {
    let c = ctx(&core, &agent_id, task_id.as_deref())?;
    blocking(move || file_versions(&toplevel(&c.cwd)?, &path, c.base.as_deref(), c.task.as_ref())).await
}

#[tauri::command]
pub async fn discard_file(core: State<'_, Shared>, agent_id: String, path: String) -> Res<()> {
    let c = ctx(&core, &agent_id, None)?;
    blocking(move || discard(&toplevel(&c.cwd)?, c.base.as_deref(), &path)).await?;
    refresh(&core, &agent_id);
    Ok(())
}

#[tauri::command]
pub async fn commit_agent(core: State<'_, Shared>, agent_id: String, message: String) -> Res<String> {
    let c = ctx(&core, &agent_id, None)?;
    let id = blocking(move || commit(&c.cwd, &message)).await?;
    refresh(&core, &agent_id);
    Ok(id)
}

#[tauri::command]
pub async fn merge_agent(core: State<'_, Shared>, agent_id: String) -> Res<MergeResult> {
    let c = ctx(&core, &agent_id, None)?;
    let wt = c.worktree.ok_or("This agent works in the main checkout: there is no branch to merge. Commit instead.")?;
    let cwd = c.cwd.clone();
    let (result, tip) = blocking(move || {
        let branch = worktree_branch(&wt)
            .ok_or("The agent's worktree isn't on a branch (detached HEAD). Create a branch there, then merge.")?;
        let r = merge(&wt.repo, &branch)?;
        Ok((r.clone(), r.merged.then(|| git::head(&cwd)).flatten()))
    })
    .await?;
    if let Some(tip) = tip {
        // What was merged is no longer "the agent's changes".
        let _ = core.with(&agent_id, |a| a.rec.base_commit = Some(tip));
        core.changed(true);
    }
    refresh(&core, &agent_id);
    Ok(result)
}

#[tauri::command]
pub async fn get_merge_status(core: State<'_, Shared>, agent_id: String) -> Res<MergeStatus> {
    let c = ctx(&core, &agent_id, None)?;
    blocking(move || {
        let branch = c.worktree.as_ref().and_then(worktree_branch);
        let wt = c.worktree.as_ref().map(|w| (w.repo.as_str(), branch.as_deref()));
        merge_status(&c.cwd, wt)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::tests::TempRepo;
    use crate::tasks::snapshot;

    #[test]
    fn task_changes_between_snapshots() {
        let r = TempRepo::new();
        r.write("keep.txt", "a\nb\n");
        r.write("gone.txt", "x\n");
        r.commit_all("init");
        let start = snapshot(r.path()).unwrap();
        r.write("keep.txt", "a\nB\nc\n");
        std::fs::remove_file(r.0.join("gone.txt")).unwrap();
        r.write("dir/new file.txt", "hi\n");
        let end = snapshot(r.path()).unwrap();
        let files = tree_changes(r.path(), &start, &end).unwrap();
        let by = |p: &str| files.iter().find(|f| f.path == p).cloned().unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!((by("keep.txt").added, by("keep.txt").removed), (2, 1));
        assert!(by("dir/new file.txt").untracked);
        assert_eq!(by("gone.txt").removed, 1);
        assert!(!by("gone.txt").untracked);
    }

    #[test]
    fn versions_for_editor() {
        let r = TempRepo::new();
        r.write("a.txt", "old\n");
        r.write("bin.dat", "a\0b");
        r.commit_all("init");
        let base = r.git(&["rev-parse", "HEAD"]).trim().to_string();
        r.write("a.txt", "new\n");
        r.write("n.txt", "fresh\n");
        let v = file_versions(r.path(), "a.txt", Some(&base), None).unwrap();
        assert_eq!(v, FileVersions { original: Some("old\n".into()), modified: Some("new\n".into()), binary: false });
        let v = file_versions(r.path(), "n.txt", Some(&base), None).unwrap();
        assert_eq!((v.original, v.modified.as_deref()), (None, Some("fresh\n")));
        assert!(file_versions(r.path(), "bin.dat", Some(&base), None).unwrap().binary);
        assert!(file_versions(r.path(), "../etc/passwd", None, None).is_err());

        // Task mode: start vs end tree, independent of later edits.
        let start = snapshot(r.path()).unwrap();
        r.write("a.txt", "newer\n");
        let end = snapshot(r.path()).unwrap();
        r.write("a.txt", "even later\n");
        let t = Task {
            id: "t".into(),
            prompt: String::new(),
            started_at: 0,
            ended_at: Some(1),
            start_tree: Some(start),
            end_tree: Some(end),
        };
        let v = file_versions(r.path(), "a.txt", None, Some(&t)).unwrap();
        assert_eq!((v.original.as_deref(), v.modified.as_deref()), (Some("new\n"), Some("newer\n")));
    }

    #[test]
    fn discard_restores_or_deletes() {
        let r = TempRepo::new();
        r.write("a.txt", "orig\n");
        r.commit_all("init");
        let base = r.git(&["rev-parse", "HEAD"]).trim().to_string();
        r.write("a.txt", "changed\n");
        r.git(&["add", "a.txt"]);
        r.write("new.txt", "x\n");
        r.write("staged-new.txt", "y\n");
        r.git(&["add", "staged-new.txt"]);
        r.write("other.txt", "untouched\n");

        discard(r.path(), Some(&base), "a.txt").unwrap();
        assert_eq!(std::fs::read_to_string(r.0.join("a.txt")).unwrap(), "orig\n");
        discard(r.path(), Some(&base), "new.txt").unwrap();
        assert!(!r.0.join("new.txt").exists());
        discard(r.path(), Some(&base), "staged-new.txt").unwrap();
        assert!(!r.0.join("staged-new.txt").exists());
        assert_eq!(r.git(&["status", "--porcelain"]).trim(), "?? other.txt");
        assert!(discard(r.path(), Some(&base), "../x").is_err());
    }

    #[test]
    fn commit_and_merge_from_worktree() {
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let wt_path = r.0.join("wt");
        r.git(&["worktree", "add", "-q", "-b", "worktree-agent", wt_path.to_str().unwrap(), "HEAD"]);
        let wt = wt_path.to_str().unwrap();
        std::fs::write(wt_path.join("b.txt"), "agent\n").unwrap();

        // Snapshots inside a linked worktree use (a copy of) its own index.
        let before = r.git(&["-C", wt, "status", "--porcelain"]);
        let tree = snapshot(wt).unwrap();
        assert!(r.git(&["ls-tree", "--name-only", &tree]).contains("b.txt"));
        assert_eq!(r.git(&["-C", wt, "status", "--porcelain"]), before);

        assert!(commit(wt, "  ").is_err());
        let st = merge_status(wt, Some((r.path(), Some("worktree-agent")))).unwrap();
        assert_eq!((st.uncommitted, st.ahead, st.target.as_deref()), (1, 0, Some("main")));
        commit(wt, "agent work").unwrap();
        assert!(commit(wt, "again").is_err()); // nothing to commit

        // Dirty main checkout: refused, nothing merged.
        std::fs::write(r.0.join("a.txt"), "local edit\n").unwrap();
        let res = merge(r.path(), "worktree-agent").unwrap();
        assert!(!res.merged && !res.conflict && res.message.contains("uncommitted"));
        r.git(&["checkout", "--", "a.txt"]);

        let res = merge(r.path(), "worktree-agent").unwrap();
        assert!(res.merged, "{res:?}");
        assert!(r.0.join("b.txt").exists());
        assert_eq!(r.git(&["rev-list", "--count", "--merges", "HEAD"]).trim(), "1");
        let res = merge(r.path(), "worktree-agent").unwrap();
        assert!(!res.merged && res.message.contains("Nothing to merge"));
    }

    #[test]
    fn worktree_branch_is_read_live() {
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let wt_path = r.0.join("wt");
        r.git(&["worktree", "add", "-q", "-b", "first", wt_path.to_str().unwrap(), "HEAD"]);
        let mut info = crate::model::WorktreeInfo {
            repo: r.path().into(),
            path: wt_path.to_string_lossy().into_owned(),
            branch: "first".into(),
        };
        r.git(&["-C", &info.path, "checkout", "-q", "-b", "renamed"]);
        assert_eq!(worktree_branch(&info).as_deref(), Some("renamed"));
        r.git(&["-C", &info.path, "checkout", "-q", "--detach"]);
        assert_eq!(worktree_branch(&info).as_deref(), Some("first"));
        info.branch.clear();
        assert_eq!(worktree_branch(&info), None);
        let st = merge_status(&info.path, Some((r.path(), None))).unwrap();
        assert!(st.worktree && st.branch.is_none() && st.ahead == 0);
    }

    #[test]
    fn merge_conflict_is_aborted() {
        let r = TempRepo::new();
        r.write("a.txt", "base\n");
        r.commit_all("init");
        let wt_path = r.0.join("wt");
        r.git(&["worktree", "add", "-q", "-b", "worktree-c", wt_path.to_str().unwrap(), "HEAD"]);
        std::fs::write(wt_path.join("a.txt"), "agent\n").unwrap();
        commit(wt_path.to_str().unwrap(), "agent").unwrap();
        r.write("a.txt", "main\n");
        r.commit_all("main change");
        let head = r.git(&["rev-parse", "HEAD"]);

        let res = merge(r.path(), "worktree-c").unwrap();
        assert!(res.conflict && !res.merged, "{res:?}");
        assert!(res.message.contains("a.txt"));
        assert_eq!(res.branch.as_deref(), Some("main"));
        assert!(!merging(r.path()));
        assert_eq!(r.git(&["rev-parse", "HEAD"]), head);
        assert!(r.git(&["status", "--porcelain", "--untracked-files=no"]).trim().is_empty());
    }
}
