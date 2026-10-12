//! Repositories inside an agent's folder: an agent started in a folder that
//! holds other clones (`upstreams/foo`, with its own `.git`) changes files
//! git in the folder itself doesn't see. The Changes list reads them too
//! (git.rs [`Git::changes_nested`](super::git::Git::changes_nested)).

use std::collections::VecDeque;

use crate::exec::{self, Exec, FileKind, NOISE_DIRS};

/// Folders this deep under the agent's folder are looked in (`a/b/c`).
pub const MAX_DEPTH: usize = 3;
/// At most this many folders are listed per look (each is a round trip on
/// a remote machine).
pub const MAX_DIRS: usize = 300;
/// At most this many repositories are read.
pub const MAX_REPOS: usize = 32;
/// The folders are looked through again at most this often (ms).
pub const RESCAN_MS: u64 = 120_000;

/// The folders under `dir` (`a/b`, sorted) that hold a repository of their
/// own (a `.git` folder or file), not looking inside those, hidden folders
/// or [`NOISE_DIRS`]. Empty where the machine can't list folders. Blocking.
pub fn find(exec: &dyn Exec, dir: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::from([(String::new(), 0)]);
    let mut listed = 0;
    while let Some((rel, depth)) = queue.pop_front() {
        if listed >= MAX_DIRS || found.len() >= MAX_REPOS {
            break;
        }
        listed += 1;
        let path = if rel.is_empty() { dir.to_string() } else { exec::join(dir, &rel) };
        let Ok(entries) = exec.list_dir(&path) else { continue };
        if !rel.is_empty() && entries.iter().any(|e| e.name == ".git") {
            found.push(rel);
            continue;
        }
        if depth >= MAX_DEPTH {
            continue;
        }
        let mut subs: Vec<&str> = entries
            .iter()
            .filter(|e| e.kind == FileKind::Dir)
            .map(|e| e.name.as_str())
            .filter(|n| !n.starts_with('.') && !NOISE_DIRS.contains(n))
            .collect();
        subs.sort_unstable();
        for n in subs {
            let child = if rel.is_empty() { n.to_string() } else { format!("{rel}/{n}") };
            queue.push_back((child, depth + 1));
        }
    }
    found.sort();
    found
}

/// Which repository `path` (as the Changes list names it) is in: the nested
/// one's folder on the machine and the path inside it, or `None` for the
/// agent's own. `repos`: (the folder paths start with, its folder on the
/// machine).
pub fn route<'p>(repos: &[(String, String)], path: &'p str) -> Option<(String, &'p str)> {
    repos
        .iter()
        .filter_map(|(prefix, dir)| {
            let rest = path.strip_prefix(prefix.as_str())?.strip_prefix('/')?;
            Some((prefix.len(), dir.clone(), rest))
        })
        .max_by_key(|(len, ..)| *len)
        .map(|(_, dir, rest)| (dir, rest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::LocalExec;
    use crate::vcs::git::{Git, NOT_A_REPO};
    use crate::vcs::snapshot::TempRepo;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    fn repo_at(dir: &std::path::Path) {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q"]);
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", "one"]);
    }

    #[test]
    fn finds_clones_under_a_plain_folder_and_reads_their_changes() {
        // A plain folder (no repository of its own), removed on drop.
        let dir = std::env::temp_dir().join(format!("pitwall-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = TempRepo(dir);
        let top = root.path();
        repo_at(&std::path::Path::new(top).join("upstreams/foo"));
        repo_at(&std::path::Path::new(top).join("bar"));
        // Not looked in: hidden and noise folders.
        repo_at(&std::path::Path::new(top).join(".cache/x"));
        repo_at(&std::path::Path::new(top).join("node_modules/y"));
        std::fs::write(std::path::Path::new(top).join("upstreams/foo/a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(std::path::Path::new(top).join("bar/new.txt"), "n\n").unwrap();

        let found = find(&LocalExec, top);
        assert_eq!(found, vec!["bar".to_string(), "upstreams/foo".to_string()]);

        let g = Git::new(&LocalExec, top);
        assert_eq!(g.changes(None).unwrap_err(), NOT_A_REPO);
        let (files, _, prefix) = g.changes_nested(&found, false);
        let files = files.unwrap();
        assert_eq!(prefix, "");
        let got: Vec<(&str, Option<&str>, u32, bool)> =
            files.iter().map(|f| (f.path.as_str(), f.repo.as_deref(), f.added, f.untracked)).collect();
        assert_eq!(
            got,
            vec![("bar/new.txt", Some("bar"), 1, true), ("upstreams/foo/a.txt", Some("upstreams/foo"), 1, false)]
        );
        let repos: Vec<(String, String)> =
            found.iter().map(|r| (r.clone(), crate::exec::join(top, r))).collect();
        let (dir, rest) = route(&repos, "upstreams/foo/a.txt").unwrap();
        assert_eq!((dir.as_str(), rest), (crate::exec::join(top, "upstreams/foo").as_str(), "a.txt"));
        assert!(route(&repos, "upstreamsx/a.txt").is_none());
    }

    #[test]
    fn a_repository_with_a_clone_inside_lists_both() {
        let r = TempRepo::new();
        let top = r.path();
        r.write("main.txt", "m\n");
        r.commit_all("main");
        r.write("main.txt", "m2\n");
        repo_at(&std::path::Path::new(top).join("vendor/lib"));
        std::fs::write(std::path::Path::new(top).join("vendor/lib/a.txt"), "changed\n").unwrap();
        let found = find(&LocalExec, top);
        assert_eq!(found, vec!["vendor/lib".to_string()]);
        let (files, _, _) = Git::new(&LocalExec, top).changes_nested(&found, false);
        let paths: Vec<String> = files.unwrap().into_iter().map(|f| f.path).collect();
        assert!(paths.contains(&"main.txt".to_string()), "{paths:?}");
        assert!(paths.contains(&"vendor/lib/a.txt".to_string()), "{paths:?}");
        // Not the clone itself as one untracked entry of the outer repository.
        assert_eq!(paths.len(), 2, "{paths:?}");
    }
}
