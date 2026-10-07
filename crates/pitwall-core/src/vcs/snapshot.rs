//! Snapshots of a working tree for per-task diffs (docs/spec/review.md):
//! a temporary index (`GIT_INDEX_FILE`) + `git add -A` + `write-tree`, so the
//! user's index and working tree are never touched. Trees are kept alive by
//! one ref per task, `refs/pitwall/<agent>/<task>`, pointing at a small tree
//! with `start` (and later `end`) entries.

use super::git::Git;
use crate::exec::{self, Exec};

/// A throwaway index file in the machine's temp dir, removed on drop.
struct TempIndex<'a> {
    exec: &'a dyn Exec,
    path: String,
}

impl<'a> TempIndex<'a> {
    fn new(exec: &'a dyn Exec) -> Result<TempIndex<'a>, String> {
        let path = exec::join(&exec.temp_dir()?, &format!("pitwall-index-{}", uuid::Uuid::new_v4()));
        Ok(TempIndex { exec, path })
    }
}

impl Drop for TempIndex<'_> {
    fn drop(&mut self) {
        let _ = self.exec.remove_file(&self.path);
        let _ = self.exec.remove_file(&format!("{}.lock", self.path));
    }
}

/// Tree id of the working tree as it is now, untracked (non-ignored) files
/// included. Uses a throwaway copy of the index; never writes the real one.
pub fn snapshot(git: &Git) -> Result<String, String> {
    let top = git.at(git.run(&["rev-parse", "--show-toplevel"])?.trim());
    let exec = git.exec();
    let tmp = TempIndex::new(exec)?;
    // Seed with a copy of the real index: keeps tracked-but-ignored files and
    // lets git reuse its stat cache. Fall back to HEAD's tree, then to empty.
    let real = top
        .run(&["rev-parse", "--path-format=absolute", "--git-path", "index"])
        .map(|s| s.trim().to_string())
        .ok();
    let seeded = real.as_ref().is_some_and(|p| exec.copy_file(p, &tmp.path).is_ok());
    let env = [("GIT_INDEX_FILE", tmp.path.as_str())];
    let with_index = |args: &[&str]| top.run_with(args, &env, None);
    if !seeded {
        let _ = with_index(&["read-tree", "HEAD"]);
    }
    if with_index(&["add", "-A"]).is_err() {
        // A seeded index we can't use (e.g. split index): start from HEAD.
        let _ = exec.remove_file(&tmp.path);
        let _ = with_index(&["read-tree", "HEAD"]);
        with_index(&["add", "-A"])?;
    }
    Ok(with_index(&["write-tree"])?.trim().to_string())
}

pub fn task_ref(agent: &str, task: &str) -> String {
    format!("refs/pitwall/{agent}/{task}")
}

/// Point the task's ref at a tree holding `start` / `end`, keeping both alive.
pub fn keep(git: &Git, agent: &str, task: &str, start: Option<&str>, end: Option<&str>) -> Result<(), String> {
    let mut listing = String::new();
    for (name, tree) in [("end", end), ("start", start)] {
        if let Some(t) = tree {
            listing.push_str(&format!("040000 tree {t}\t{name}\n"));
        }
    }
    if listing.is_empty() {
        return Ok(());
    }
    let holder = git.run_with(&["mktree"], &[], Some(listing.as_bytes()))?;
    git.run(&["update-ref", &task_ref(agent, task), holder.trim()]).map(|_| ())
}

/// Delete one task's ref (or every ref of the agent when `task` is `None`).
pub fn drop_refs(git: &Git, agent: &str, task: Option<&str>) -> Result<(), String> {
    let prefix = match task {
        Some(t) => task_ref(agent, t),
        None => format!("refs/pitwall/{agent}/"),
    };
    let refs = git.run(&["for-each-ref", "--format=%(refname)", &prefix])?;
    for r in refs.lines().filter(|r| !r.is_empty()) {
        git.run(&["update-ref", "-d", r])?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) use tests::TempRepo;

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::exec::LocalExec;
    use crate::testing::FakeExec;

    /// A throwaway repo under the system temp dir, deleted on drop.
    pub struct TempRepo(pub PathBuf);

    impl TempRepo {
        pub fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("pitwall-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let r = TempRepo(dir);
            r.git(&["init", "-q", "-b", "main"]);
            r.git(&["config", "user.name", "Test"]);
            r.git(&["config", "user.email", "test@example.com"]);
            r.git(&["config", "commit.gpgsign", "false"]);
            r
        }
        pub fn path(&self) -> &str {
            self.0.to_str().unwrap()
        }
        /// Git on this machine in the repo.
        pub fn g(&self) -> Git<'static> {
            Git::new(&LocalExec, self.path())
        }
        pub fn git(&self, args: &[&str]) -> String {
            self.g().run(args).unwrap_or_else(|e| panic!("git {args:?}: {e}"))
        }
        pub fn write(&self, rel: &str, content: &str) {
            let p = self.0.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
        pub fn commit_all(&self, msg: &str) {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "-m", msg]);
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn snapshot_includes_untracked_and_leaves_index_alone() {
        let r = TempRepo::new();
        r.write("a.txt", "one\n");
        r.write(".gitignore", "ignored/\n");
        r.commit_all("init");
        r.write("a.txt", "two\n");
        r.write("new.txt", "fresh\n");
        r.write("ignored/x.txt", "nope\n");
        let index = std::fs::read(r.0.join(".git/index")).unwrap();
        let status_before = r.git(&["status", "--porcelain"]);

        let tree = snapshot(&r.g()).unwrap();
        let files = r.git(&["ls-tree", "-r", "--name-only", &tree]);
        assert!(files.contains("a.txt") && files.contains("new.txt"));
        assert!(!files.contains("ignored/x.txt"));
        assert_eq!(r.git(&["cat-file", "blob", &format!("{tree}:a.txt")]), "two\n");
        // Real index and status untouched.
        assert_eq!(std::fs::read(r.0.join(".git/index")).unwrap(), index);
        assert_eq!(r.git(&["status", "--porcelain"]), status_before);
        assert!(status_before.contains("?? new.txt"));
    }

    #[test]
    fn snapshot_works_in_empty_repo_and_fails_outside_git() {
        let r = TempRepo::new();
        r.write("f", "x");
        assert!(snapshot(&r.g()).is_ok());
        let not_git = std::env::temp_dir().join(format!("pitwall-nogit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&not_git).unwrap();
        assert!(snapshot(&Git::new(&LocalExec, not_git.to_str().unwrap())).is_err());
        let _ = std::fs::remove_dir_all(&not_git);
    }

    #[test]
    fn refs_keep_trees_and_are_dropped() {
        let r = TempRepo::new();
        r.write("a", "1");
        r.commit_all("init");
        let t1 = snapshot(&r.g()).unwrap();
        r.write("a", "2");
        let t2 = snapshot(&r.g()).unwrap();
        keep(&r.g(), "ag", "t1", Some(&t1), Some(&t2)).unwrap();
        let held = r.git(&["rev-parse", &format!("{}:start", task_ref("ag", "t1"))]);
        assert_eq!(held.trim(), t1);
        let held = r.git(&["rev-parse", &format!("{}:end", task_ref("ag", "t1"))]);
        assert_eq!(held.trim(), t2);
        keep(&r.g(), "ag", "t2", Some(&t1), None).unwrap();
        keep(&r.g(), "other", "t9", Some(&t1), None).unwrap();
        drop_refs(&r.g(), "ag", Some("t1")).unwrap();
        assert!(!r.git(&["for-each-ref", "refs/pitwall/ag/"]).contains("/t1"));
        drop_refs(&r.g(), "ag", None).unwrap();
        assert!(r.git(&["for-each-ref", "refs/pitwall/ag/"]).trim().is_empty());
        assert!(!r.git(&["for-each-ref", "refs/pitwall/other/"]).trim().is_empty());
    }

    #[test]
    fn snapshot_uses_a_temp_index_on_the_agents_machine() {
        let x = FakeExec::new();
        let real = "/w/.git/index";
        x.file(real, b"INDEX");
        x.on(&["git", "-C", "/w/sub", "rev-parse", "--show-toplevel"], "/w\n")
            .on(&["git", "-C", "/w", "rev-parse", "--path-format=absolute", "--git-path", "index"], "/w/.git/index\n")
            .on(&["git", "-C", "/w", "add", "-A"], "")
            .on(&["git", "-C", "/w", "write-tree"], "tree123\n");
        assert_eq!(snapshot(&Git::new(&*x, "/w/sub")).unwrap(), "tree123");

        let calls = x.calls();
        let idx = calls[2].env_get("GIT_INDEX_FILE").expect("temp index").to_string();
        assert!(idx.starts_with(&format!("{}/pitwall-index-", crate::testing::FAKE_TEMP)), "{idx}");
        assert!(calls[2..].iter().all(|c| c.env_get("GIT_INDEX_FILE") == Some(idx.as_str())));
        assert!(calls.iter().all(|c| c.env_has("GIT_OPTIONAL_LOCKS", "0")));
        assert_eq!(x.ran(&["git", "-C", "/w", "read-tree", "HEAD"]), 0, "seeded from the real index");
        assert_eq!(x.contents(real).as_deref(), Some(&b"INDEX"[..]), "real index untouched");
        assert!(!x.paths().iter().any(|p| p.contains("pitwall-index-")), "temp index removed");
    }

    #[test]
    fn snapshot_falls_back_to_head_when_the_copy_cant_be_used() {
        let x = FakeExec::new();
        x.on(&["git", "-C", "/w", "rev-parse", "--show-toplevel"], "/w\n")
            // The index git names isn't there to copy.
            .on(&["git", "-C", "/w", "rev-parse", "--path-format=absolute", "--git-path", "index"], "/w/.git/index\n")
            .on(&["git", "-C", "/w", "read-tree", "HEAD"], "")
            .on(&["git", "-C", "/w", "add", "-A"], "")
            .on(&["git", "-C", "/w", "write-tree"], "t\n");
        assert_eq!(snapshot(&Git::new(&*x, "/w")).unwrap(), "t");
        assert_eq!(x.ran(&["git", "-C", "/w", "read-tree", "HEAD"]), 1);
    }

    #[test]
    fn keep_feeds_mktree_on_stdin() {
        let x = FakeExec::new();
        x.on(&["git", "-C", "/w", "mktree"], "holder\n").on(&["git", "-C", "/w", "update-ref", ".."], "");
        keep(&Git::new(&*x, "/w"), "ag", "t1", Some("s"), Some("e")).unwrap();
        let calls = x.calls();
        assert_eq!(calls[0].stdin.as_deref(), Some(&b"040000 tree e\tend\n040000 tree s\tstart\n"[..]));
        assert!(calls[1].matches(&["git", "-C", "/w", "update-ref", "refs/pitwall/ag/t1", "holder"]));
        keep(&Git::new(&*x, "/w"), "ag", "t2", None, None).unwrap();
        assert_eq!(x.calls().len(), 2, "nothing to keep: no git");
    }
}
