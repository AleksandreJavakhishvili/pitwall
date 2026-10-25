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

/// What Review compares an agent's work with ([`compare_with`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewBase {
    /// The commit the work is diffed against: where the branch left
    /// `branch` (`git merge-base`), or `HEAD` on that branch itself.
    pub rev: String,
    /// The branch compared with, as the ref used (`develop`,
    /// `origin/main`); `None` when there is none.
    pub branch: Option<String>,
    /// The user chose it (else found automatically).
    pub chosen: bool,
}

/// [`compare_with`]'s commit, found automatically.
#[cfg(test)]
pub fn review_base(top: &Git, main_checkout: Option<&str>) -> String {
    compare_with(top, main_checkout, None).rev
}

/// What Review compares an agent's work with: where its branch left the
/// branch it came from, so neither its own commits drop out nor what
/// others merged there since shows up. On that branch itself: HEAD (its
/// uncommitted work). The branch, in order:
/// - `chosen` by the user (when it still exists), used as it is;
/// - the one this branch was created from, by git's own records
///   ([`parent_branch`]);
/// - for a worktree agent, the main checkout's current branch
///   (`main_checkout`), which its merge goes into;
/// - the project's main branch ([`main_branch`]).
///
/// Found automatically, of a local branch and its `origin/` copy whichever
/// has moved on further ([`copies`]), so a local `main` left months behind
/// doesn't put all that history into Review.
pub fn compare_with(top: &Git, main_checkout: Option<&str>, chosen: Option<&str>) -> ReviewBase {
    let current = top.current_branch();
    let chosen = chosen.filter(|c| is_branch(top, c)).map(String::from);
    let (name, refs) = match &chosen {
        Some(c) => (c.clone(), vec![c.clone()]),
        None => {
            let name = current
                .as_deref()
                .and_then(|b| parent_branch(top, b))
                .or_else(|| main_checkout.and_then(|r| top.at(r).current_branch()))
                .or_else(|| main_branch(top));
            let Some(name) = name else {
                return ReviewBase { rev: "HEAD".into(), branch: None, chosen: false };
            };
            let refs = copies(top, &name);
            (name, refs)
        }
    };
    let found = ReviewBase { rev: "HEAD".into(), branch: Some(name.clone()), chosen: chosen.is_some() };
    if current.as_deref() == Some(name.as_str()) {
        return found;
    }
    let newest = refs.iter().filter_map(|r| top.merge_base("HEAD", r).map(|b| (r, b))).reduce(|best, x| {
        let newer = x.1 != best.1 && top.run(&["merge-base", "--is-ancestor", &best.1, &x.1]).is_ok();
        if newer { x } else { best }
    });
    match newest {
        Some((r, rev)) => ReviewBase { rev, branch: Some(r.clone()), ..found },
        None => found,
    }
}

fn ref_exists(top: &Git, r: &str) -> bool {
    top.run(&["rev-parse", "--verify", "-q", r]).is_ok()
}

/// `name` is a local branch or a remote one (`origin/x`), not `HEAD`.
fn is_branch(top: &Git, name: &str) -> bool {
    !(name.is_empty() || name == "HEAD" || name.ends_with("/HEAD"))
        && (ref_exists(top, &format!("refs/heads/{name}")) || ref_exists(top, &format!("refs/remotes/{name}")))
}

/// The branch `branch` was created from, by git's own records: the oldest
/// entry of its reflog ("branch: Created from develop"); when that says
/// HEAD, the HEAD reflog's oldest switch to it ("checkout: moving from
/// develop to <branch>"). Not when that is the branch itself (or its
/// `origin/` copy), a commit, or a branch that no longer exists; nor when
/// the records are gone (a fresh clone, an expired reflog).
pub fn parent_branch(top: &Git, branch: &str) -> Option<String> {
    let oldest = |args: &[&str], pick: &dyn Fn(&str) -> Option<String>| -> Option<String> {
        top.run(args).ok()?.lines().filter_map(pick).last()
    };
    let created = oldest(&["reflog", "show", "--format=%gs", &format!("refs/heads/{branch}"), "--"], &|l| {
        l.strip_prefix("branch: Created from ").map(String::from)
    });
    let from = match created?.trim() {
        "HEAD" => {
            let to = format!(" to {branch}");
            oldest(&["reflog", "show", "--format=%gs", "HEAD", "--"], &|l| {
                l.strip_prefix("checkout: moving from ")?.strip_suffix(to.as_str()).map(String::from)
            })?
        }
        x => x.to_string(),
    };
    let from = from.strip_prefix("refs/heads/").or_else(|| from.strip_prefix("refs/remotes/")).unwrap_or(&from);
    let own = from == branch || from.split_once('/').is_some_and(|(_, rest)| rest == branch);
    (!own && is_branch(top, from)).then(|| from.to_string())
}

/// The repository's main branch's name: the one `origin/HEAD` names, else
/// `main` or `master`, when there is such a branch here or on origin.
pub fn main_branch(top: &Git) -> Option<String> {
    let remote = top.run(&["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"]).ok();
    let named = remote.as_deref().map(str::trim).and_then(|r| r.split_once('/')).map(|(_, b)| b.to_string());
    named.into_iter().chain(["main".to_string(), "master".to_string()]).find(|b| {
        ref_exists(top, &format!("refs/heads/{b}")) || ref_exists(top, &format!("refs/remotes/origin/{b}"))
    })
}

/// The copies of branch `name` (`develop` or `origin/develop`) that can be
/// compared with: the local branch and its `origin/` copy, those there are
/// (else `name` itself, e.g. `upstream/x`).
pub fn copies(top: &Git, name: &str) -> Vec<String> {
    let local = match name.strip_prefix("origin/") {
        Some(l) if !ref_exists(top, &format!("refs/heads/{name}")) => l,
        _ => name,
    };
    let mut refs = Vec::new();
    if ref_exists(top, &format!("refs/heads/{local}")) {
        refs.push(local.to_string());
    }
    if ref_exists(top, &format!("refs/remotes/origin/{local}")) {
        refs.push(format!("origin/{local}"));
    }
    if refs.is_empty() && is_branch(top, name) {
        refs.push(name.to_string());
    }
    refs
}

/// Every branch Review can compare with: the local ones, then origin's.
pub fn branches(top: &Git) -> Vec<String> {
    let out = top.run(&["for-each-ref", "--format=%(refname)", "refs/heads", "refs/remotes/origin"]).unwrap_or_default();
    out.lines()
        .filter(|r| !r.ends_with("/HEAD"))
        .filter_map(|r| r.strip_prefix("refs/heads/").or_else(|| r.strip_prefix("refs/remotes/")))
        .map(String::from)
        .collect()
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
            repo: None,
            branch: None,
            parts: None,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Original/modified text of `path`. With a task: start tree vs end tree (or
/// the working tree while it's running). Without: `base` vs the working tree.
/// One part of a file's uncommitted change (`FileChange::parts`): what's
/// staged (HEAD → index) or what isn't yet (index → the file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Staged,
    Unstaged,
}

/// The two sides of `path`'s `stage` part.
pub fn stage_versions(top: &Git, path: &str, stage: Stage) -> Res<FileVersions> {
    valid_path(path)?;
    // (`:path`: the index's copy.)
    Ok(match stage {
        Stage::Staged => versions(blob(top, "HEAD", path), blob(top, "", path)),
        Stage::Unstaged => versions(blob(top, "", path), disk(top, path)),
    })
}

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

    fn paths(top: &Git, base: &str) -> Vec<String> {
        top.changes(Some(base)).unwrap().into_iter().map(|f| f.path).collect()
    }

    #[test]
    fn review_compares_with_the_branch_not_where_the_agent_started() {
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let top = r.g();
        assert_eq!(main_branch(&top).as_deref(), Some("main"));

        // On the main branch: only what isn't committed yet.
        r.write("a.txt", "2\n");
        assert_eq!(review_base(&top, None), "HEAD");
        assert_eq!(paths(&top, &review_base(&top, None)), ["a.txt"]);
        r.commit_all("agent work");
        assert!(paths(&top, &review_base(&top, None)).is_empty(), "committed work leaves Review");

        // On a feature branch: everything since it left main, but not what
        // main got afterwards.
        r.git(&["checkout", "-q", "-b", "feature"]);
        r.write("f.txt", "f\n");
        r.commit_all("feature work");
        r.write("wip.txt", "w\n");
        r.git(&["checkout", "-q", "main"]);
        r.write("other.txt", "o\n");
        r.git(&["add", "other.txt"]);
        r.git(&["commit", "-q", "-m", "someone else"]);
        r.git(&["checkout", "-q", "feature"]);
        assert_eq!(paths(&top, &review_base(&top, None)), ["f.txt", "wip.txt"]);

        // A worktree goes back into the main checkout's current branch.
        let wt = format!("{}-wt", r.path());
        r.git(&["worktree", "add", "-q", "-b", "agent", &wt, "main"]);
        let wg = top.at(&wt);
        std::fs::write(format!("{wt}/x.txt"), "x\n").unwrap();
        wg.run(&["add", "x.txt"]).unwrap();
        wg.run(&["commit", "-q", "-m", "agent"]).unwrap();
        // (The main checkout is on `feature` now.)
        r.git(&["checkout", "-q", "main"]);
        assert_eq!(paths(&wg, &review_base(&wg, Some(r.path()))), ["x.txt"]);
        r.git(&["merge", "-q", "--no-ff", "--no-edit", "agent"]);
        assert!(paths(&wg, &review_base(&wg, Some(r.path()))).is_empty(), "merged work leaves Review");
        r.git(&["worktree", "remove", "--force", &wt]);
    }

    /// A local main left far behind `origin/main` (never pulled) doesn't
    /// bring all of origin's history into Review; nor does a stale
    /// `origin/main` when the local one is newer.
    #[test]
    fn review_uses_whichever_main_has_moved_on_further() {
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let top = r.g();
        let old_main = r.git(&["rev-parse", "HEAD"]).trim().to_string();
        r.write("b.txt", "b\n");
        r.commit_all("merged upstream");
        // origin/main is here; local main stays at `init`.
        r.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        r.git(&["checkout", "-q", "-b", "feature"]);
        r.git(&["branch", "-f", "main", &old_main]);
        r.write("f.txt", "f\n");
        r.commit_all("feature work");
        assert_eq!(copies(&top, "main"), ["main", "origin/main"]);
        assert_eq!(compare_with(&top, None, None).branch.as_deref(), Some("origin/main"));
        assert_eq!(paths(&top, &review_base(&top, None)), ["f.txt"]);

        // The other way round: local main ahead, origin/main stale.
        r.git(&["update-ref", "refs/remotes/origin/main", &old_main]);
        r.git(&["branch", "-f", "main", "feature~1"]);
        assert_eq!(compare_with(&top, None, None).branch.as_deref(), Some("main"));
        assert_eq!(paths(&top, &review_base(&top, None)), ["f.txt"]);

        // Only origin/main (no local main): still compared with it.
        r.git(&["branch", "-D", "main"]);
        r.git(&["update-ref", "refs/remotes/origin/main", "feature~1"]);
        assert_eq!(copies(&top, "main"), ["origin/main"]);
        assert_eq!(paths(&top, &review_base(&top, None)), ["f.txt"]);
    }

    fn branch_of(top: &Git, chosen: Option<&str>) -> Option<String> {
        compare_with(top, None, chosen).branch
    }

    /// A branch made from `develop` is compared with `develop`, not with
    /// main (which may be far away): only its own work shows.
    #[test]
    fn review_compares_with_the_branch_it_was_created_from() {
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let top = r.g();
        r.git(&["checkout", "-q", "-b", "develop"]);
        r.write("d.txt", "d\n");
        r.commit_all("develop work");
        // "branch: Created from develop".
        r.git(&["branch", "feat", "develop"]);
        r.git(&["checkout", "-q", "feat"]);
        assert_eq!(parent_branch(&top, "feat").as_deref(), Some("develop"));
        r.write("f.txt", "f\n");
        r.commit_all("feature work");
        r.write("u.txt", "u\n");
        // What develop gets afterwards stays out.
        r.git(&["update-ref", "refs/heads/develop", "feat~1"]);
        assert_eq!(branch_of(&top, None).as_deref(), Some("develop"));
        assert_eq!(paths(&top, &review_base(&top, None)), ["f.txt", "u.txt"]);

        // "branch: Created from HEAD": the HEAD reflog's switch to it.
        r.git(&["checkout", "-q", "develop"]);
        r.git(&["checkout", "-q", "-b", "feat2"]);
        assert_eq!(r.git(&["reflog", "show", "--format=%gs", "refs/heads/feat2"]).trim(), "branch: Created from HEAD");
        assert_eq!(parent_branch(&top, "feat2").as_deref(), Some("develop"));
        r.git(&["checkout", "-q", "feat"]);
        assert_eq!(parent_branch(&top, "feat2").as_deref(), Some("develop"), "the first switch to it counts");

        // Not a commit, nor its own origin copy.
        r.git(&["checkout", "-q", "--detach", "main"]);
        r.git(&["checkout", "-q", "-b", "from-commit"]);
        assert_eq!(parent_branch(&top, "from-commit"), None);
        r.git(&["update-ref", "refs/remotes/origin/tracked", "main"]);
        r.git(&["branch", "tracked", "origin/tracked"]);
        assert_eq!(parent_branch(&top, "tracked"), None);
        assert_eq!(branch_of(&top.at(r.path()), None).as_deref(), Some("main"), "from-commit: main");

        // The parent is gone: main.
        r.git(&["checkout", "-q", "feat"]);
        r.git(&["branch", "-D", "feat2", "develop"]);
        assert_eq!(parent_branch(&top, "feat"), None);
        assert_eq!(branch_of(&top, None).as_deref(), Some("main"));
    }

    /// No record of where the branch came from (a fresh clone, an expired
    /// reflog): the main branch.
    #[test]
    fn review_falls_back_to_main_without_records() {
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let top = r.g();
        r.git(&["checkout", "-q", "-b", "develop"]);
        r.write("d.txt", "d\n");
        r.commit_all("develop work");
        r.git(&["checkout", "-q", "-b", "feat"]);
        r.write("f.txt", "f\n");
        r.commit_all("feature work");
        assert_eq!(branch_of(&top, None).as_deref(), Some("develop"));
        r.git(&["reflog", "expire", "--expire=all", "--all"]);
        assert_eq!(parent_branch(&top, "feat"), None);
        assert_eq!(branch_of(&top, None).as_deref(), Some("main"));
        assert_eq!(paths(&top, &review_base(&top, None)), ["d.txt", "f.txt"]);
    }

    /// The user's choice wins while it exists; on that branch itself, HEAD.
    #[test]
    fn review_compares_with_the_chosen_branch() {
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let top = r.g();
        r.git(&["checkout", "-q", "-b", "develop"]);
        r.write("d.txt", "d\n");
        r.commit_all("develop work");
        r.git(&["checkout", "-q", "-b", "feat"]);
        r.write("f.txt", "f\n");
        r.commit_all("feature work");
        let b = compare_with(&top, None, Some("main"));
        assert_eq!((b.branch.as_deref(), b.chosen), (Some("main"), true));
        assert_eq!(paths(&top, &b.rev), ["d.txt", "f.txt"]);
        // Gone: automatic again.
        let b = compare_with(&top, None, Some("nope"));
        assert_eq!((b.branch.as_deref(), b.chosen), (Some("develop"), false));
        // Its own branch: what isn't committed yet.
        assert_eq!(compare_with(&top, None, Some("feat")).rev, "HEAD");
        r.git(&["update-ref", "refs/remotes/origin/develop", "develop"]);
        assert_eq!(branches(&top), ["develop", "feat", "main", "origin/develop"]);
    }

    /// By hand, read-only: what Review compares a real checkout with.
    /// `PITWALL_REVIEW_REPO=<dir> cargo test -p pitwall-core review_of_a_real_repo -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn review_of_a_real_repo() {
        let dir = std::env::var("PITWALL_REVIEW_REPO").expect("PITWALL_REVIEW_REPO");
        let top = toplevel(&Git::new(&LocalExec, &dir)).unwrap();
        let b = compare_with(&top, None, None);
        let files = top.changes(Some(&b.rev)).unwrap();
        println!("on {:?}: compared with {:?} at {}: {} file(s)", top.current_branch(), b.branch, b.rev, files.len());
        for f in files.iter().take(20) {
            println!("  {}", f.path);
        }
    }

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
