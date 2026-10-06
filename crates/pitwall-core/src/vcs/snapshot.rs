//! Snapshots of a working tree for per-task diffs (docs/spec/review.md):
//! a temporary index (`GIT_INDEX_FILE`) + `git add -A` + `write-tree`, so the
//! user's index and working tree are never touched. Trees are kept alive by
//! one ref per task, `refs/pitwall/<agent>/<task>`, pointing at a small tree
//! with `start` (and later `end`) entries.

use std::path::PathBuf;
use std::process::Command;

/// git with `GIT_OPTIONAL_LOCKS=0`, like git.rs.
pub(crate) fn git(cwd: &str) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true");
    cmd
}

pub(crate) fn run(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn temp_index_path() -> PathBuf {
    std::env::temp_dir().join(format!("pitwall-index-{}", uuid::Uuid::new_v4()))
}

struct TempIndex(PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("lock"));
    }
}

/// Tree id of the working tree as it is now, untracked (non-ignored) files
/// included. Uses a throwaway copy of the index; never writes the real one.
pub fn snapshot(cwd: &str) -> Result<String, String> {
    let top = run(git(cwd).args(["rev-parse", "--show-toplevel"]))?.trim().to_string();
    let tmp = TempIndex(temp_index_path());
    // Seed with a copy of the real index: keeps tracked-but-ignored files and
    // lets git reuse its stat cache. Fall back to HEAD's tree, then to empty.
    let real = run(git(&top).args(["rev-parse", "--path-format=absolute", "--git-path", "index"]))
        .map(|s| PathBuf::from(s.trim()))
        .ok();
    let seeded = real.as_ref().is_some_and(|p| std::fs::copy(p, &tmp.0).is_ok());
    let with_index = |args: &[&str]| run(git(&top).env("GIT_INDEX_FILE", &tmp.0).args(args));
    if !seeded {
        let _ = with_index(&["read-tree", "HEAD"]);
    }
    if with_index(&["add", "-A"]).is_err() {
        // A seeded index we can't use (e.g. split index): start from HEAD.
        let _ = std::fs::remove_file(&tmp.0);
        let _ = with_index(&["read-tree", "HEAD"]);
        with_index(&["add", "-A"])?;
    }
    Ok(with_index(&["write-tree"])?.trim().to_string())
}

pub fn task_ref(agent: &str, task: &str) -> String {
    format!("refs/pitwall/{agent}/{task}")
}

/// Point the task's ref at a tree holding `start` / `end`, keeping both alive.
pub fn keep(cwd: &str, agent: &str, task: &str, start: Option<&str>, end: Option<&str>) -> Result<(), String> {
    let mut listing = String::new();
    for (name, tree) in [("end", end), ("start", start)] {
        if let Some(t) = tree {
            listing.push_str(&format!("040000 tree {t}\t{name}\n"));
        }
    }
    if listing.is_empty() {
        return Ok(());
    }
    let holder = run_with_stdin(git(cwd).arg("mktree"), &listing)?;
    run(git(cwd).args(["update-ref", &task_ref(agent, task), holder.trim()])).map(|_| ())
}

fn run_with_stdin(cmd: &mut Command, input: &str) -> Result<String, String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("git: no stdin")?
        .write_all(input.as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Delete one task's ref (or every ref of the agent when `task` is `None`).
pub fn drop_refs(cwd: &str, agent: &str, task: Option<&str>) -> Result<(), String> {
    let prefix = match task {
        Some(t) => task_ref(agent, t),
        None => format!("refs/pitwall/{agent}/"),
    };
    let refs = run(git(cwd).args(["for-each-ref", "--format=%(refname)", &prefix]))?;
    for r in refs.lines().filter(|r| !r.is_empty()) {
        run(git(cwd).args(["update-ref", "-d", r]))?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) use tests::TempRepo;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

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
        pub fn git(&self, args: &[&str]) -> String {
            run(git(self.path()).args(args)).unwrap_or_else(|e| panic!("git {args:?}: {e}"))
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

        let tree = snapshot(r.path()).unwrap();
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
        assert!(snapshot(r.path()).is_ok());
        let not_git = std::env::temp_dir().join(format!("pitwall-nogit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&not_git).unwrap();
        assert!(snapshot(not_git.to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&not_git);
    }

    #[test]
    fn refs_keep_trees_and_are_dropped() {
        let r = TempRepo::new();
        r.write("a", "1");
        r.commit_all("init");
        let t1 = snapshot(r.path()).unwrap();
        r.write("a", "2");
        let t2 = snapshot(r.path()).unwrap();
        keep(r.path(), "ag", "t1", Some(&t1), Some(&t2)).unwrap();
        let held = r.git(&["rev-parse", &format!("{}:start", task_ref("ag", "t1"))]);
        assert_eq!(held.trim(), t1);
        let held = r.git(&["rev-parse", &format!("{}:end", task_ref("ag", "t1"))]);
        assert_eq!(held.trim(), t2);
        keep(r.path(), "ag", "t2", Some(&t1), None).unwrap();
        keep(r.path(), "other", "t9", Some(&t1), None).unwrap();
        drop_refs(r.path(), "ag", Some("t1")).unwrap();
        assert!(!r.git(&["for-each-ref", "refs/pitwall/ag/"]).contains("/t1"));
        drop_refs(r.path(), "ag", None).unwrap();
        assert!(r.git(&["for-each-ref", "refs/pitwall/ag/"]).trim().is_empty());
        assert!(!r.git(&["for-each-ref", "refs/pitwall/other/"]).trim().is_empty());
    }
}
