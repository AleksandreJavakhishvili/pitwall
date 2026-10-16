//! The "Scope" tests (docs/spec/collaboration/plan.md §12): the copy holds
//! exactly the picked paths minus exclusions, has no link back, and its
//! blob ids equal the owner's.

use std::path::Path;

use super::*;
use crate::exec::LocalExec;
use crate::testing::TempDir;

fn sh(dir: &Path, argv: &[&str]) -> String {
    sh_in(dir, argv, b"")
}

fn sh_in(dir: &Path, argv: &[&str], stdin: &[u8]) -> String {
    let d = dir.to_string_lossy();
    let mut full = vec!["git", "-C", &d];
    full.extend_from_slice(argv);
    let out = LocalExec
        .run(&Cmd::new(&full).env(&[
            ("GIT_AUTHOR_NAME", "Owner"),
            ("GIT_AUTHOR_EMAIL", "owner@example.com"),
            ("GIT_COMMITTER_NAME", "Owner"),
            ("GIT_COMMITTER_EMAIL", "owner@example.com"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ]).stdin(stdin))
        .unwrap();
    assert!(out.ok(), "git {argv:?}: {}", out.stderr_text());
    out.stdout_text()
}

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// An owner's repository with history, a secret in an old commit, another
/// branch, an executable and a symlink.
fn owner_repo(dir: &Path) -> std::path::PathBuf {
    let repo = dir.join("api");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "commit.gpgsign", "false"]);
    write(&repo, "old-secret.txt", "CANARY-HISTORY");
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "first"]);
    sh(&repo, &["rm", "-q", "old-secret.txt"]);
    write(&repo, "src/main.rs", "fn main() {}\n");
    write(&repo, "src/lib.rs", "pub fn f() {}\n");
    write(&repo, "src/run.sh", "#!/bin/sh\necho hi\n");
    write(&repo, "docs/guide.md", "# Guide\n");
    write(&repo, ".env", "TOKEN=CANARY-ENV\n");
    write(&repo, "deploy/id_ed25519", "CANARY-KEY\n");
    write(&repo, "deploy/notes.md", "deploy notes\n");
    write(&repo, "infra/main.tf", "x\n");
    sh(&repo, &["add", "-A"]);
    // An executable and a symlink, made through git so it works everywhere.
    sh(&repo, &["update-index", "--chmod=+x", "src/run.sh"]);
    let target = sh_in(&repo, &["hash-object", "-w", "--stdin"], b"main.rs");
    sh(&repo, &["update-index", "--add", "--cacheinfo", &format!("120000,{},src/link.rs", target.trim())]);
    sh(&repo, &["commit", "-qm", "second"]);
    sh(&repo, &["checkout-index", "-f", "-a"]);
    sh(&repo, &["branch", "other"]);
    repo
}

fn spec(repo: &Path) -> SnapshotSpec {
    SnapshotSpec {
        repo: repo.to_string_lossy().into_owned(),
        git_ref: "HEAD".into(),
        paths: vec!["src".into(), "docs/".into(), "deploy".into(), ".env".into()],
        exclude: vec![".env".into(), "deploy/id_ed25519".into()],
        uncommitted: false,
    }
}

fn files_in(work: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            if rel == ".git" {
                continue;
            }
            if e.file_type().unwrap().is_dir() {
                walk(root, &p, out);
            } else {
                out.push(rel);
            }
        }
    }
    walk(work, work, &mut out);
    out.sort();
    out
}

#[test]
fn copy_holds_exactly_the_scope_minus_exclusions() {
    let dir = TempDir::new("snap-scope");
    let repo = owner_repo(dir.path());
    let work = dir.path().join("s/1/work");
    let snap = build(&LocalExec, &spec(&repo), &work.to_string_lossy()).unwrap();

    let expected = ["deploy/notes.md", "docs/guide.md", "src/lib.rs", "src/link.rs", "src/main.rs", "src/run.sh"];
    assert_eq!(files_in(&work), expected);
    let listed: Vec<&str> = snap.files.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(listed, expected);
    assert!(snap.skipped.is_empty());
    assert!(snap.message.starts_with("snapshot of main @ "), "{}", snap.message);
    assert_eq!(snap.source, sh(&repo, &["rev-parse", "HEAD"]).trim());

    // modes and symlinks as in the owner's tree
    let staged = sh(&work, &["ls-files", "-s", "src/run.sh", "src/link.rs"]);
    assert!(staged.contains("100755 ") && staged.contains("120000 "), "{staged}");
    // a clean working tree on `main`
    assert_eq!(sh(&work, &["status", "--porcelain"]), "");
    assert_eq!(sh(&work, &["symbolic-ref", "HEAD"]).trim(), "refs/heads/main");

    // what the scope lists without building
    let scoped: Vec<String> = scope_files(&LocalExec, &spec(&repo)).unwrap().into_iter().map(|e| e.path).collect();
    assert_eq!(scoped, expected);
}

#[test]
fn copy_has_no_link_back_and_no_history() {
    let dir = TempDir::new("snap-link");
    let repo = owner_repo(dir.path());
    let work = dir.path().join("work");
    build(&LocalExec, &spec(&repo), &work.to_string_lossy()).unwrap();

    assert!(work.join(".git").is_dir(), ".git is a folder of its own, not a pointer file");
    assert!(!work.join(".git/objects/info/alternates").exists());
    assert_eq!(sh(&work, &["remote"]), "");
    assert_eq!(sh(&work, &["for-each-ref", "--format=%(refname)"]).trim(), "refs/heads/main");
    assert_eq!(sh(&work, &["rev-list", "--all", "--count"]).trim(), "1");
    // Nothing of the owner's repository is reachable or even stored.
    let objects = sh(&work, &["cat-file", "--batch-all-objects", "--batch-check"]);
    let blobs: Vec<&str> = objects.lines().filter(|l| l.contains(" blob ")).collect();
    assert_eq!(blobs.len(), 6, "{objects}");
    let all = sh(&work, &["cat-file", "--batch-all-objects", "--batch"]);
    for canary in ["CANARY-HISTORY", "CANARY-ENV", "CANARY-KEY"] {
        assert!(!all.contains(canary), "{canary} in the copy's objects");
    }
    let repo_s = repo.to_string_lossy().into_owned();
    let config = std::fs::read_to_string(work.join(".git/config")).unwrap();
    assert!(!config.contains(&repo_s) && !config.contains("remote"), "{config}");
    // The owner's repository wasn't touched.
    assert_eq!(sh(&repo, &["status", "--porcelain"]), "");
}

#[test]
fn blob_ids_equal_the_owners() {
    let dir = TempDir::new("snap-blobs");
    let repo = owner_repo(dir.path());
    let work = dir.path().join("work");
    build(&LocalExec, &spec(&repo), &work.to_string_lossy()).unwrap();
    let owner = sh(&repo, &["ls-tree", "-r", "HEAD", "--", "src", "docs", "deploy/notes.md"]);
    let copy = sh(&work, &["ls-tree", "-r", "HEAD"]);
    assert_eq!(copy, owner);
}

#[test]
fn whole_repository_other_refs_and_uncommitted_changes() {
    let dir = TempDir::new("snap-uncommitted");
    let repo = owner_repo(dir.path());
    // the other branch, whole repository
    write(&repo, "src/main.rs", "fn main() { changed(); }\n");
    std::fs::remove_file(repo.join("docs/guide.md")).unwrap();
    write(&repo, "src/new.rs", "staged\n");
    write(&repo, "src/untracked.rs", "untracked\n");
    sh(&repo, &["add", "src/new.rs"]);

    let whole = SnapshotSpec { paths: vec![], exclude: vec![".env".into()], git_ref: "other".into(), ..spec(&repo) };
    let work = dir.path().join("w1");
    let snap = build(&LocalExec, &whole, &work.to_string_lossy()).unwrap();
    assert!(snap.message.starts_with("snapshot of other @ "));
    assert!(files_in(&work).contains(&"infra/main.tf".to_string()));
    assert!(!files_in(&work).contains(&".env".to_string()));
    assert_eq!(std::fs::read_to_string(work.join("src/main.rs")).unwrap(), "fn main() {}\n");

    let with = SnapshotSpec { uncommitted: true, ..spec(&repo) };
    let work = dir.path().join("w2");
    build(&LocalExec, &with, &work.to_string_lossy()).unwrap();
    let files = files_in(&work);
    assert_eq!(std::fs::read_to_string(work.join("src/main.rs")).unwrap(), "fn main() { changed(); }\n");
    assert!(files.contains(&"src/new.rs".to_string()), "staged new file is tracked");
    assert!(!files.contains(&"src/untracked.rs".to_string()));
    assert!(!files.contains(&"docs/guide.md".to_string()), "deleted in the working tree");
    assert_eq!(sh(&work, &["status", "--porcelain"]), "");
    // unchanged files keep the owner's blob ids
    assert_eq!(sh(&work, &["rev-parse", "HEAD:src/lib.rs"]), sh(&repo, &["rev-parse", "HEAD:src/lib.rs"]));
}

#[test]
fn refresh_rebuilds_and_keeps_earlier_changes() {
    let dir = TempDir::new("snap-refresh");
    let repo = owner_repo(dir.path());
    let work = dir.path().join("work");
    let w = work.to_string_lossy().into_owned();
    build(&LocalExec, &spec(&repo), &w).unwrap();
    assert!(build(&LocalExec, &spec(&repo), &w).is_err(), "won't build over an existing copy");

    // nothing changed by the session
    let r = refresh(&LocalExec, &spec(&repo), &w).unwrap();
    assert_eq!(r.earlier, Earlier::None);

    // the session changed a file; the owner moved on elsewhere
    write(&work, "src/lib.rs", "pub fn f() { 1 }\n");
    write(&repo, "docs/guide.md", "# Guide v2\n");
    sh(&repo, &["commit", "-qam", "docs"]);
    let r = refresh(&LocalExec, &spec(&repo), &w).unwrap();
    assert_eq!(r.earlier, Earlier::Reapplied);
    assert_eq!(std::fs::read_to_string(work.join("docs/guide.md")).unwrap(), "# Guide v2\n");
    assert_eq!(std::fs::read_to_string(work.join("src/lib.rs")).unwrap(), "pub fn f() { 1 }\n");
    assert_eq!(sh(&work, &["rev-list", "--all", "--count"]).trim(), "1");

    // the owner changed the same lines: kept as a patch
    write(&repo, "src/lib.rs", "pub fn f() { 2 }\n");
    sh(&repo, &["commit", "-qam", "lib"]);
    let r = refresh(&LocalExec, &spec(&repo), &w).unwrap();
    let Earlier::KeptAsPatch(patch) = r.earlier else { panic!("expected a patch") };
    assert!(String::from_utf8_lossy(&patch).contains("+pub fn f() { 1 }"));
    assert_eq!(std::fs::read_to_string(work.join("src/lib.rs")).unwrap(), "pub fn f() { 2 }\n");

    // narrowing the scope drops the files and their objects
    let narrow = SnapshotSpec { paths: vec!["docs".into()], ..spec(&repo) };
    refresh(&LocalExec, &narrow, &w).unwrap();
    assert_eq!(files_in(&work), ["docs/guide.md"]);
    let objects = sh(&work, &["cat-file", "--batch-all-objects", "--batch-check"]);
    assert_eq!(objects.lines().filter(|l| l.contains(" blob ")).count(), 1, "{objects}");

    remove(&LocalExec, &w).unwrap();
    assert!(!work.exists());
    remove(&LocalExec, &w).unwrap();
}
