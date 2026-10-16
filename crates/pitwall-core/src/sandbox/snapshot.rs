//! A shared session's copy (docs/spec/collaboration/sessions.md §6.2): a
//! fresh repository in the session's `work/` folder with **one commit**
//! holding only the scope's tracked files minus the exclusions (and, if
//! chosen, the uncommitted changes to them). No link to the owner's
//! repository: no `.git` pointer back, no alternates, no remote, no other
//! branches, no history.
//!
//! Committed files are carried over as the owner's own blobs (packed with
//! `git pack-objects` from exactly those ids), so every file has the same
//! blob id as in the owner's repository, which is what lets the caller's
//! `git am -3` merge three ways (§7.5). Uncommitted changes are hashed from
//! the owner's working tree as they are (`--no-filters`).
//!
//! The owner's repository is only read: no index, ref or object is written
//! there.

use crate::error::{PwError, Result};
use crate::exec::{self, Cmd, Exec};

/// What a session's copy holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotSpec {
    /// The owner's checkout (any folder inside it).
    pub repo: String,
    /// The commit to copy: a branch, tag or `HEAD`.
    pub git_ref: String,
    /// The scope: files or folders relative to the repository's top, taken
    /// literally (no globs). Empty: the whole repository.
    pub paths: Vec<String>,
    /// Files or folders (relative, as in `paths`) left out: the sensitive
    /// files the owner kept ticked, and anything else they excluded.
    pub exclude: Vec<String>,
    /// Include the uncommitted changes to tracked files in the scope (the
    /// working tree as it is now; untracked files stay out).
    pub uncommitted: bool,
}

/// One file of a copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// `/`-separated, relative to the top.
    pub path: String,
    /// git's mode: `100644`, `100755` or `120000` (a symlink).
    pub mode: String,
    pub blob: String,
}

/// A copy that was built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The copy's one commit.
    pub commit: String,
    /// The owner's commit it was made from.
    pub source: String,
    /// The commit's message: "snapshot of main @ 3f2a1c9".
    pub message: String,
    pub files: Vec<Entry>,
    /// What the scope held but the copy can't: submodules, changed
    /// symlinks (with why).
    pub skipped: Vec<String>,
}

/// What happened to the session's own earlier changes on a refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Earlier {
    /// There were none.
    None,
    /// They still applied and are on top of the new copy (uncommitted).
    Reapplied,
    /// They no longer apply: here they are as a patch ("files updated;
    /// earlier changes kept as patch").
    KeptAsPatch(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refreshed {
    pub snapshot: Snapshot,
    pub earlier: Earlier,
}

/// Fixed identity for snapshot commits (never the owner's).
const ENV: &[(&str, &str)] = &[
    ("GIT_AUTHOR_NAME", "Pitwall"),
    ("GIT_AUTHOR_EMAIL", "pitwall@localhost"),
    ("GIT_COMMITTER_NAME", "Pitwall"),
    ("GIT_COMMITTER_EMAIL", "pitwall@localhost"),
    ("GIT_TERMINAL_PROMPT", "0"),
];

fn git(exec: &dyn Exec, dir: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut argv = vec!["git", "--literal-pathspecs", "-C", dir];
    argv.extend_from_slice(args);
    let mut cmd = Cmd::new(&argv).env(ENV);
    if let Some(input) = stdin {
        cmd = cmd.stdin(input);
    }
    let out = exec.run(&cmd)?;
    if !out.ok() {
        return Err(PwError::other(format!("git {}: {}", args.first().unwrap_or(&""), out.stderr_text())));
    }
    Ok(out.stdout)
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).trim().to_string()
}

fn excluded(path: &str, exclude: &[String]) -> bool {
    exclude.iter().any(|e| {
        let e = e.trim_start_matches("./").trim_end_matches('/');
        !e.is_empty() && (path == e || path.starts_with(&format!("{e}/")))
    })
}

fn pathspecs(spec: &SnapshotSpec) -> Vec<String> {
    spec.paths
        .iter()
        .map(|p| p.trim_start_matches("./").trim_end_matches('/').to_string())
        .filter(|p| !p.is_empty() && p != ".")
        .collect()
}

/// The owner's repository top and the commit `spec.git_ref` names.
fn source(exec: &dyn Exec, spec: &SnapshotSpec) -> Result<(String, String, String)> {
    let top = text(git(exec, &spec.repo, &["rev-parse", "--show-toplevel"], None)?);
    let r = if spec.git_ref.is_empty() { "HEAD" } else { spec.git_ref.as_str() };
    let commit = text(git(exec, &top, &["rev-parse", "--verify", "--end-of-options", &format!("{r}^{{commit}}")], None)?);
    let label = if r == "HEAD" {
        text(git(exec, &top, &["rev-parse", "--abbrev-ref", "HEAD"], None).unwrap_or_default())
    } else {
        r.to_string()
    };
    Ok((top, commit, label))
}

/// The scope's committed files at `commit`, minus exclusions; submodules
/// go to `skipped`.
fn committed(exec: &dyn Exec, top: &str, commit: &str, spec: &SnapshotSpec, skipped: &mut Vec<String>) -> Result<Vec<Entry>> {
    let specs = pathspecs(spec);
    let mut args = vec!["ls-tree", "-r", "-z", "--full-tree", commit, "--"];
    args.extend(specs.iter().map(String::as_str));
    let out = git(exec, top, &args, None)?;
    let mut entries = Vec::new();
    for rec in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let rec = String::from_utf8_lossy(rec);
        let Some((meta, path)) = rec.split_once('\t') else { continue };
        let mut m = meta.split(' ');
        let (mode, kind, blob) = (m.next().unwrap_or(""), m.next().unwrap_or(""), m.next().unwrap_or(""));
        if excluded(path, &spec.exclude) {
            continue;
        }
        if kind != "blob" {
            skipped.push(format!("{path}: a submodule"));
            continue;
        }
        entries.push(Entry { path: path.to_string(), mode: mode.to_string(), blob: blob.to_string() });
    }
    Ok(entries)
}

/// Files the copy would hold (for the Share dialog's count and the
/// sensitive-file scan), without building it. Committed content only.
pub fn scope_files(exec: &dyn Exec, spec: &SnapshotSpec) -> Result<Vec<Entry>> {
    let (top, commit, _) = source(exec, spec)?;
    committed(exec, &top, &commit, spec, &mut Vec::new())
}

/// Build the copy in `work` (created; must not exist or be empty).
pub fn build(exec: &dyn Exec, spec: &SnapshotSpec, work: &str) -> Result<Snapshot> {
    exec::assert_off_ui("building a session's copy");
    if let Ok(entries) = exec.list_dir(work) {
        if !entries.is_empty() {
            return Err(PwError::new(crate::error::ErrorCode::Conflict, format!("{work} isn't empty")));
        }
    }
    let (top, source, label) = source(exec, spec)?;
    let mut skipped = Vec::new();
    let mut files = committed(exec, &top, &source, spec, &mut skipped)?;

    // `git init` makes missing parents; it runs from the top only because
    // it needs a folder that exists. The copy is a repository of its own.
    git(exec, &top, &["init", "-q", "--template=", "-b", "main", work], None)?;

    // The owner's blobs, exactly those, into the copy's object store.
    let mut ids: Vec<&str> = files.iter().map(|e| e.blob.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    if !ids.is_empty() {
        let list = ids.join("\n") + "\n";
        let pack = git(exec, &top, &["pack-objects", "--stdout", "-q"], Some(list.as_bytes()))?;
        git(exec, work, &["index-pack", "--stdin"], Some(&pack))?;
    }

    if spec.uncommitted {
        apply_uncommitted(exec, &top, &source, spec, work, &mut files, &mut skipped)?;
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut info = Vec::new();
    for e in &files {
        info.extend_from_slice(format!("{} {}\t{}", e.mode, e.blob, e.path).as_bytes());
        info.push(0);
    }
    git(exec, work, &["update-index", "--add", "-z", "--index-info"], Some(&info))?;
    let tree = text(git(exec, work, &["write-tree"], None)?);
    let short = &source[..source.len().min(7)];
    let message = format!("snapshot of {label} @ {short}");
    let commit = text(git(exec, work, &["commit-tree", "--no-gpg-sign", &tree, "-m", &message], None)?);
    git(exec, work, &["update-ref", "refs/heads/main", &commit], None)?;
    git(exec, work, &["checkout-index", "-a", "-f", "-u"], None)?;
    Ok(Snapshot { commit, source, message, files, skipped })
}

/// Replace committed entries with the owner's working tree for tracked
/// files that changed (deleted ones drop out). Untracked files stay out.
fn apply_uncommitted(
    exec: &dyn Exec,
    top: &str,
    source: &str,
    spec: &SnapshotSpec,
    work: &str,
    files: &mut Vec<Entry>,
    skipped: &mut Vec<String>,
) -> Result<()> {
    let specs = pathspecs(spec);
    let mut args = vec!["diff", "--raw", "-z", "--no-renames", "--no-ext-diff", source, "--"];
    args.extend(specs.iter().map(String::as_str));
    let out = git(exec, top, &args, None)?;
    let mut fields = out.split(|b| *b == 0).filter(|r| !r.is_empty());
    let mut changed = Vec::new();
    while let (Some(meta), Some(path)) = (fields.next(), fields.next()) {
        let meta = String::from_utf8_lossy(meta);
        let path = String::from_utf8_lossy(path).into_owned();
        let parts: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        let (new_mode, status) = (parts.get(1).copied().unwrap_or(""), parts.get(4).copied().unwrap_or(""));
        if excluded(&path, &spec.exclude) {
            continue;
        }
        files.retain(|e| e.path != path);
        if status.starts_with('D') {
            continue;
        }
        match new_mode {
            "100644" | "100755" => changed.push((path, new_mode.to_string())),
            "120000" => skipped.push(format!("{path}: a changed symlink (the committed one is kept out too)")),
            _ => skipped.push(format!("{path}: a submodule")),
        }
    }
    if changed.is_empty() {
        return Ok(());
    }
    let list: String = changed.iter().map(|(p, _)| exec::join(top, p) + "\n").collect();
    let ids = text(git(exec, work, &["hash-object", "-w", "--no-filters", "--stdin-paths"], Some(list.as_bytes()))?);
    for ((path, mode), blob) in changed.into_iter().zip(ids.lines()) {
        files.push(Entry { path, mode, blob: blob.to_string() });
    }
    Ok(())
}

/// Rebuild an idle session's copy at the scope's current state ("Update
/// copies", a scope change). Its own uncommitted changes are put back on
/// top when they still apply, else returned as a patch. The old copy's
/// objects go with it, so nothing excluded since survives.
pub fn refresh(exec: &dyn Exec, spec: &SnapshotSpec, work: &str) -> Result<Refreshed> {
    let git_dir = exec::join(work, ".git");
    let is_copy = exec.stat(&git_dir)?.is_some_and(|s| s.kind == exec::FileKind::Dir);
    if !is_copy {
        return Err(PwError::not_found(format!("{work} isn't a session's copy")));
    }
    git(exec, work, &["add", "-A"], None)?;
    let patch = git(exec, work, &["diff", "--cached", "--binary", "--no-ext-diff", "HEAD"], None)?;
    remove_tree(exec, work)?;
    let snapshot = build(exec, spec, work)?;
    let earlier = if patch.is_empty() {
        Earlier::None
    } else {
        let out = exec.run(&Cmd::new(&["git", "-C", work, "apply", "--whitespace=nowarn", "-"]).env(ENV).stdin(&patch))?;
        if out.ok() { Earlier::Reapplied } else { Earlier::KeptAsPatch(patch) }
    };
    Ok(Refreshed { snapshot, earlier })
}

/// Delete a session's copy (removing a session).
pub fn remove(exec: &dyn Exec, work: &str) -> Result<()> {
    if exec.stat(work)?.is_none() {
        return Ok(());
    }
    remove_tree(exec, work)
}

/// `dir` and everything in it; symlinks are removed, never followed.
fn remove_tree(exec: &dyn Exec, dir: &str) -> Result<()> {
    for entry in exec.list_dir(dir)? {
        let path = exec::join(dir, &entry.name);
        if entry.kind == exec::FileKind::Dir {
            remove_tree(exec, &path)?;
        } else {
            exec.remove_file(&path)?;
        }
    }
    exec.remove_dir(dir)
}

#[cfg(test)]
mod tests;
