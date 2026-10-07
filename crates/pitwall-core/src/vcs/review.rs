//! Git operations behind the Review screen (docs/spec/review.md): per-task
//! changes, file versions for the diff editor, and the explicit user actions
//! discard / commit / merge. Read-only git runs with GIT_OPTIONAL_LOCKS=0.
//! Paths are relative to the repository root of the agent's checkout.

use serde::Serialize;

use super::git::{FileChange, FileStatus, Git};
use crate::engine::tasks::Task;
use crate::exec::{self, Exec, FileKind};

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

/// The repository root of the agent's checkout, as a [`Git`] there.
pub fn toplevel<'a>(git: &Git<'a>) -> Res<Git<'a>> {
    git.repo_root().map(|top| git.at(&top)).ok_or_else(|| "not a git repository".to_string())
}

/// Raw bytes of `<rev>:<path>`, or `None` if it doesn't exist there.
fn blob(top: &Git, rev: &str, path: &str) -> Option<Vec<u8>> {
    let out = top.output(&["cat-file", "blob", &format!("{rev}:{path}")], &[], None).ok()?;
    out.ok().then_some(out.stdout)
}

/// The file in the working tree (cut just past the editor's limit, which
/// still reads as too large).
fn disk(top: &Git, path: &str) -> Option<Vec<u8>> {
    let exec = top.exec();
    let p = exec::join(top.dir(), path);
    if exec.stat(&p).ok()??.kind == FileKind::Dir {
        return None;
    }
    exec.read_file(&p, MAX_EDITOR_BYTES as u64 + 1).ok()?
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

fn base_rev(top: &Git, base: Option<&str>) -> String {
    match base {
        Some(b) if top.run(&["cat-file", "-e", &format!("{b}^{{commit}}")]).is_ok() => b.to_string(),
        _ => "HEAD".into(),
    }
}

// ------------------------------------------------------------------ pure git ops (tested)

/// Changes between two trees, NUL-separated so odd paths survive.
pub fn tree_changes(top: &Git, from: &str, to: &str) -> Res<Vec<FileChange>> {
    let names = top.run(&["diff", "--name-status", "--no-renames", "-z", from, to])?;
    let mut status = std::collections::HashMap::new();
    let mut it = names.split('\0');
    while let (Some(st), Some(path)) = (it.next(), it.next()) {
        status.insert(path.to_string(), FileStatus::from_git(st));
    }
    let stat = top.run(&["diff", "--numstat", "--no-renames", "-z", from, to])?;
    let mut files = Vec::new();
    for rec in stat.split('\0').filter(|r| !r.is_empty()) {
        let mut parts = rec.splitn(3, '\t');
        let (Some(a), Some(r), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        files.push(FileChange {
            path: path.to_string(),
            added: a.parse().unwrap_or(0),
            removed: r.parse().unwrap_or(0),
            untracked: status.get(path) == Some(&FileStatus::A),
            binary: a == "-",
            status: Some(status.get(path).copied().unwrap_or(FileStatus::M)),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Original/modified text of `path`. With a task: start tree vs end tree (or
/// the working tree while it's running). Without: `base` vs the working tree.
pub fn file_versions(top: &Git, path: &str, base: Option<&str>, task: Option<&Task>) -> Res<FileVersions> {
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
pub fn discard(top: &Git, base: Option<&str>, path: &str) -> Res<()> {
    valid_path(path)?;
    let base = base_rev(top, base);
    let lit = "--literal-pathspecs";
    if top.run(&[lit, "cat-file", "-e", &format!("{base}:{path}")]).is_ok() {
        top.run(&[lit, "restore", &format!("--source={base}"), "--staged", "--worktree", "--", path])?;
        return Ok(());
    }
    top.run(&[lit, "rm", "--cached", "--ignore-unmatch", "-q", "--", path])?;
    let exec = top.exec();
    let full = exec::join(top.dir(), path);
    match exec.stat(&full) {
        Ok(Some(st)) if st.kind == FileKind::Dir => Err("refusing to delete a directory".into()),
        Ok(Some(_)) => Ok(exec.remove_file(&full)?),
        _ => Ok(()),
    }
}

fn porcelain(git: &Git, tracked_only: bool) -> Res<Vec<String>> {
    let mut args = vec!["status", "--porcelain"];
    if tracked_only {
        args.push("--untracked-files=no");
    }
    Ok(git.run(&args)?.lines().filter(|l| !l.is_empty()).map(String::from).collect())
}

/// `git add -A && git commit -m <message>`; returns the short commit id.
pub fn commit(git: &Git, message: &str) -> Res<String> {
    if message.trim().is_empty() {
        return Err("commit message is empty".into());
    }
    if porcelain(git, false)?.is_empty() {
        return Err("nothing to commit".into());
    }
    git.run(&["add", "-A"])?;
    git.run(&["commit", "-q", "-m", message])?;
    Ok(git.run(&["rev-parse", "--short", "HEAD"])?.trim().to_string())
}

fn merging(repo: &Git) -> bool {
    repo.run(&["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok()
}

/// `git merge --no-ff <branch>` into the main checkout's current branch.
/// Refuses on a dirty checkout; aborts on conflicts.
pub fn merge(repo_git: &Git, branch: &str) -> Res<MergeResult> {
    let repo = repo_git.dir();
    let target = repo_git.current_branch();
    let refuse = |message: String| MergeResult { merged: false, conflict: false, message, branch: target.clone() };
    let Some(tb) = target.clone() else {
        return Ok(refuse(format!("{repo} is on a detached HEAD; check out a branch first.")));
    };
    if merging(repo_git) {
        return Ok(refuse(format!("{repo} already has a merge in progress.")));
    }
    if !porcelain(repo_git, true)?.is_empty() {
        return Ok(refuse(format!(
            "{} has uncommitted changes on {tb}. Commit or stash them, then merge again.",
            crate::paths::tildify(repo)
        )));
    }
    if repo_git.run(&["merge-base", "--is-ancestor", branch, "HEAD"]).is_ok() {
        return Ok(refuse(format!("Nothing to merge: {branch} is already in {tb}.")));
    }
    let out = repo_git.output(&["merge", "--no-ff", "--no-edit", branch], &[("GIT_MERGE_AUTOEDIT", "no")], None)?;
    if out.ok() {
        return Ok(MergeResult { merged: true, conflict: false, message: format!("Merged {branch} into {tb}."), branch: target });
    }
    if merging(repo_git) {
        let files = repo_git.run(&["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
        let files: Vec<&str> = files.lines().filter(|l| !l.is_empty()).collect();
        repo_git.run(&["merge", "--abort"])?;
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
pub fn worktree_branch(exec: &dyn Exec, wt: &crate::model::WorktreeInfo) -> Option<String> {
    Git::new(exec, &wt.path).current_branch().or_else(|| Some(wt.branch.clone()).filter(|b| !b.is_empty()))
}

/// `cwd`: the agent's checkout. `wt`: (main repo, the worktree's branch;
/// `None` = detached).
pub fn merge_status(cwd: &Git, wt: Option<(&str, Option<&str>)>) -> Res<MergeStatus> {
    let uncommitted = porcelain(cwd, false)?.len() as u32;
    let Some((repo, branch)) = wt else {
        return Ok(MergeStatus {
            worktree: false,
            branch: cwd.current_branch(),
            target: None,
            target_dirty: false,
            uncommitted,
            ahead: 0,
        });
    };
    let repo = cwd.at(repo);
    let target = repo.current_branch();
    let ahead = target
        .as_ref()
        .zip(branch)
        .and_then(|(t, b)| repo.run(&["rev-list", "--count", &format!("{t}..{b}")]).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    Ok(MergeStatus {
        worktree: true,
        branch: branch.map(String::from),
        target,
        target_dirty: !porcelain(&repo, true)?.is_empty(),
        uncommitted,
        ahead,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExec;
    use crate::testing::FakeExec;
    use crate::vcs::snapshot::{snapshot, TempRepo};

    #[test]
    fn task_changes_between_snapshots() {
        let r = TempRepo::new();
        r.write("keep.txt", "a\nb\n");
        r.write("gone.txt", "x\n");
        r.commit_all("init");
        let start = snapshot(&r.g()).unwrap();
        r.write("keep.txt", "a\nB\nc\n");
        std::fs::remove_file(r.0.join("gone.txt")).unwrap();
        r.write("dir/new file.txt", "hi\n");
        let end = snapshot(&r.g()).unwrap();
        let files = tree_changes(&r.g(), &start, &end).unwrap();
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
        let v = file_versions(&r.g(), "a.txt", Some(&base), None).unwrap();
        assert_eq!(v, FileVersions { original: Some("old\n".into()), modified: Some("new\n".into()), binary: false });
        let v = file_versions(&r.g(), "n.txt", Some(&base), None).unwrap();
        assert_eq!((v.original, v.modified.as_deref()), (None, Some("fresh\n")));
        assert!(file_versions(&r.g(), "bin.dat", Some(&base), None).unwrap().binary);
        assert!(file_versions(&r.g(), "../etc/passwd", None, None).is_err());

        // Task mode: start vs end tree, independent of later edits.
        let start = snapshot(&r.g()).unwrap();
        r.write("a.txt", "newer\n");
        let end = snapshot(&r.g()).unwrap();
        r.write("a.txt", "even later\n");
        let t = Task {
            id: "t".into(),
            prompt: String::new(),
            started_at: 0,
            ended_at: Some(1),
            start_tree: Some(start),
            end_tree: Some(end),
        };
        let v = file_versions(&r.g(), "a.txt", None, Some(&t)).unwrap();
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

        discard(&r.g(), Some(&base), "a.txt").unwrap();
        assert_eq!(std::fs::read_to_string(r.0.join("a.txt")).unwrap(), "orig\n");
        discard(&r.g(), Some(&base), "new.txt").unwrap();
        assert!(!r.0.join("new.txt").exists());
        discard(&r.g(), Some(&base), "staged-new.txt").unwrap();
        assert!(!r.0.join("staged-new.txt").exists());
        assert_eq!(r.git(&["status", "--porcelain"]).trim(), "?? other.txt");
        assert!(discard(&r.g(), Some(&base), "../x").is_err());
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
        let tree = snapshot(&Git::new(&LocalExec, wt)).unwrap();
        assert!(r.git(&["ls-tree", "--name-only", &tree]).contains("b.txt"));
        assert_eq!(r.git(&["-C", wt, "status", "--porcelain"]), before);

        assert!(commit(&Git::new(&LocalExec, wt), "  ").is_err());
        let st = merge_status(&Git::new(&LocalExec, wt), Some((r.path(), Some("worktree-agent")))).unwrap();
        assert_eq!((st.uncommitted, st.ahead, st.target.as_deref()), (1, 0, Some("main")));
        commit(&Git::new(&LocalExec, wt), "agent work").unwrap();
        assert!(commit(&Git::new(&LocalExec, wt), "again").is_err()); // nothing to commit

        // Dirty main checkout: refused, nothing merged.
        std::fs::write(r.0.join("a.txt"), "local edit\n").unwrap();
        let res = merge(&r.g(), "worktree-agent").unwrap();
        assert!(!res.merged && !res.conflict && res.message.contains("uncommitted"));
        r.git(&["checkout", "--", "a.txt"]);

        let res = merge(&r.g(), "worktree-agent").unwrap();
        assert!(res.merged, "{res:?}");
        assert!(r.0.join("b.txt").exists());
        assert_eq!(r.git(&["rev-list", "--count", "--merges", "HEAD"]).trim(), "1");
        let res = merge(&r.g(), "worktree-agent").unwrap();
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
        assert_eq!(worktree_branch(&LocalExec, &info).as_deref(), Some("renamed"));
        r.git(&["-C", &info.path, "checkout", "-q", "--detach"]);
        assert_eq!(worktree_branch(&LocalExec, &info).as_deref(), Some("first"));
        info.branch.clear();
        assert_eq!(worktree_branch(&LocalExec, &info), None);
        let st = merge_status(&Git::new(&LocalExec, &info.path), Some((r.path(), None))).unwrap();
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
        commit(&Git::new(&LocalExec, wt_path.to_str().unwrap()), "agent").unwrap();
        r.write("a.txt", "main\n");
        r.commit_all("main change");
        let head = r.git(&["rev-parse", "HEAD"]);

        let res = merge(&r.g(), "worktree-c").unwrap();
        assert!(res.conflict && !res.merged, "{res:?}");
        assert!(res.message.contains("a.txt"));
        assert_eq!(res.branch.as_deref(), Some("main"));
        assert!(!merging(&r.g()));
        assert_eq!(r.git(&["rev-parse", "HEAD"]), head);
        assert!(r.git(&["status", "--porcelain", "--untracked-files=no"]).trim().is_empty());
    }

    #[test]
    fn editor_reads_go_through_the_agents_machine() {
        let x = FakeExec::new();
        x.on(&["git", "-C", "/r", "cat-file", "-e", "b0^{commit}"], "")
            .on_exit(&["git", "-C", "/r", "cat-file", "blob", "*"], 128, "", "missing")
            .on(&["git", "-C", "/r", "cat-file", "blob", "b0:a.txt"], "old\n")
            .file("/r/a.txt", b"new\n")
            .file("/r/huge.txt", &vec![b'x'; MAX_EDITOR_BYTES + 10])
            .dir("/r/folder");
        let top = Git::new(&*x, "/r");
        let v = file_versions(&top, "a.txt", Some("b0"), None).unwrap();
        assert_eq!((v.original.as_deref(), v.modified.as_deref()), (Some("old\n"), Some("new\n")));
        assert!(file_versions(&top, "huge.txt", Some("b0"), None).unwrap().binary, "too large for the editor");
        let v = file_versions(&top, "folder", Some("b0"), None).unwrap();
        assert_eq!((v.original, v.modified, v.binary), (None, None, false));
    }

    #[test]
    fn discard_never_deletes_a_folder() {
        let x = FakeExec::new();
        x.on(&["git", "-C", "/r", "cat-file", "-e", "HEAD^{commit}"], "")
            .on_exit(&["git", "-C", "/r", "--literal-pathspecs", "cat-file", "-e", "*"], 128, "", "")
            .on(&["git", "-C", "/r", "--literal-pathspecs", "rm", ".."], "")
            .dir("/r/d")
            .file("/r/d/x", b"x")
            .file("/r/new.txt", b"x");
        let top = Git::new(&*x, "/r");
        assert_eq!(discard(&top, Some("HEAD"), "d").unwrap_err(), "refusing to delete a directory");
        discard(&top, Some("HEAD"), "new.txt").unwrap();
        assert_eq!(x.contents("/r/new.txt"), None);
        discard(&top, Some("HEAD"), "never-existed").unwrap();
    }

    #[test]
    fn merge_refuses_a_detached_target_without_touching_it() {
        let x = FakeExec::new();
        x.on_exit(&["git", "-C", "/r", "symbolic-ref", ".."], 1, "", "");
        let res = merge(&Git::new(&*x, "/r"), "feature").unwrap();
        assert!(!res.merged && !res.conflict && res.message.contains("detached HEAD"));
        assert_eq!(x.ran(&["git", "-C", "/r", "merge", ".."]), 0);
    }
}
