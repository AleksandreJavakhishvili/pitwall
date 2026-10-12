//! An agent's uncommitted work as the sidebar shows it: what `git status`
//! shows (staged, unstaged, untracked), against HEAD. Review keeps its own
//! "since the agent started" baseline (`base_commit`, review.rs).

use super::Engine;
use crate::vcs::git::{FileChange, Git, NOT_A_REPO};

type Res<T> = Result<T, String>;

pub use crate::vcs::review::Stage;

/// Uncommitted changes against HEAD, like `git status`; also refreshes the
/// agent's +/- counts. Blocking (git).
///
/// With the repositories inside its folder (`vcs::nested`).
pub fn changes(engine: &Engine, agent_id: &str) -> Res<Vec<FileChange>> {
    let cwd = engine.with(agent_id, |a| a.rec.cwd.clone())?;
    let exec = engine.exec_for(agent_id);
    let nested = nested_repos(engine, agent_id, &cwd, &*exec);
    let (files, _, prefix) = Git::new(&*exec, &cwd).changes_nested(&nested, false);
    engine.with(agent_id, |a| a.nested = nested_named(&cwd, &nested, &prefix))?;
    let files = match files {
        Ok(files) => files,
        Err(e) => {
            // Outside a repository: no diffs for it (caps.diff) and no polling.
            if e == NOT_A_REPO && engine.with(agent_id, |a| a.git_repo.replace(false) != Some(false))? {
                engine.changed(false);
            }
            return Err(e);
        }
    };
    let counts = (
        files.iter().map(|f| f.added).sum(),
        files.iter().map(|f| f.removed).sum(),
        files.len() as u32,
    );
    let differs = engine.with(agent_id, |a| {
        let d = (a.added, a.removed, a.files_changed) != counts || a.git_repo != Some(true);
        (a.added, a.removed, a.files_changed) = counts;
        a.git_repo = Some(true);
        d
    })?;
    if differs {
        engine.changed(false);
    }
    Ok(files)
}

/// The repositories inside agent `id`'s folder `cwd` (`vcs::nested`),
/// relative to it: looked for again when `nested::RESCAN_MS` passed, else
/// as last found. Blocking (lists folders on the agent's machine).
pub(super) fn nested_repos(engine: &Engine, id: &str, cwd: &str, exec: &dyn crate::exec::Exec) -> Vec<String> {
    use crate::vcs::nested;
    let now = engine.now();
    let known = engine
        .with(id, |a| {
            let fresh = a.nested_at.is_some_and(|t| now.saturating_sub(t) < nested::RESCAN_MS);
            fresh.then(|| a.nested.iter().map(|(_, dir)| dir.clone()).collect::<Vec<_>>())
        })
        .ok()
        .flatten();
    if let Some(dirs) = known {
        // (`exec::join(cwd, rel)`, as they were made.)
        let base = crate::exec::join(cwd, "");
        return dirs.iter().filter_map(|d| d.strip_prefix(&base).map(String::from)).collect();
    }
    let found = nested::find(exec, cwd);
    let _ = engine.with(id, |a| a.nested_at = Some(now));
    found
}

/// What an agent keeps of them: (the folder their files' paths start with,
/// their folder on the machine). `prefix`: where `cwd` is in its repository.
pub(super) fn nested_named(cwd: &str, nested: &[String], prefix: &str) -> Vec<(String, String)> {
    nested.iter().map(|n| (format!("{prefix}{n}"), crate::exec::join(cwd, n))).collect()
}

/// Read agent `id`'s changes and branch now, bypassing the ticker's polling
/// pace (back-off, slow providers, idle agents): the user asked (opened the
/// Changes panel or Review, pressed refresh). A refresh already running for
/// the agent is awaited rather than repeated. Updates the agent's numbers
/// (`AgentsChanged` when they moved) and returns the changes. Blocking (git,
/// through the agent's machine).
pub fn refresh(engine: &Engine, agent_id: &str) -> Res<Vec<FileChange>> {
    super::ticker::refresh_git_now(engine, agent_id)
}

/// One file's uncommitted diff against HEAD. Blocking (git).
pub fn file_diff(engine: &Engine, agent_id: &str, path: &str, untracked: bool) -> Res<String> {
    let cwd = engine.with(agent_id, |a| a.rec.cwd.clone())?;
    if path.is_empty() || path.starts_with('/') || path.split('/').any(|p| p == "..") {
        return Err("invalid path".into());
    }
    Git::new(&*engine.exec_for(agent_id), &cwd).file_diff(None, path, untracked)
}

/// HEAD's and the working tree's text of `path` (relative to the
/// repository's top, as [`changes`] lists it): the Changes panel's diff of
/// one file. Blocking (git).
///
/// A file of a repository inside the agent's folder (`a/b/x.rs`, as the
/// list names it) is read from that one.
pub fn file_versions(engine: &Engine, agent_id: &str, path: &str) -> Res<crate::vcs::review::FileVersions> {
    file_part_versions(engine, agent_id, path, None)
}

/// [`file_versions`] of one part: what's staged, or what isn't yet
/// (`None`: both, against HEAD).
pub fn file_part_versions(
    engine: &Engine,
    agent_id: &str,
    path: &str,
    stage: Option<crate::vcs::review::Stage>,
) -> Res<crate::vcs::review::FileVersions> {
    use crate::vcs::review;
    let exec = engine.exec_for(agent_id);
    let (cwd, nested) = engine.with(agent_id, |a| (a.rec.cwd.clone(), a.nested.clone()))?;
    let (dir, path) = match crate::vcs::nested::route(&nested, path) {
        Some((dir, rest)) => (dir, rest),
        None => (cwd, path),
    };
    let top = review::toplevel(&Git::new(&*exec, &dir))?;
    match stage {
        Some(s) => review::stage_versions(&top, path, s),
        None => review::file_versions(&top, path, None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{record, Harness};
    use crate::vcs::snapshot::TempRepo;

    #[test]
    fn counts_follow_the_working_tree() {
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let h = Harness::new(vec![record("a", r.path())]);
        r.write("a.txt", "1\n2\n");
        r.write("new.txt", "x\n");
        let files = changes(&h.engine, "a").unwrap();
        assert_eq!(files.len(), 2);
        let v = &h.engine.views()[0];
        assert_eq!((v.added, v.removed, v.files_changed), (2, 0, 2));
        assert!(file_diff(&h.engine, "a", "a.txt", false).unwrap().contains("+2"));
        assert_eq!(file_diff(&h.engine, "a", "../x", false).unwrap_err(), "invalid path");
        let v = file_versions(&h.engine, "a", "a.txt").unwrap();
        assert_eq!((v.original.as_deref(), v.modified.as_deref()), (Some("1\n"), Some("1\n2\n")));
        let v = file_versions(&h.engine, "a", "new.txt").unwrap();
        assert_eq!((v.original, v.modified.as_deref()), (None, Some("x\n")));
    }

    /// What's staged and what isn't, as VS Code splits them: a file staged
    /// and then edited again is in both, each with its own counts and diff.
    #[test]
    fn staged_and_unstaged_parts() {
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.write("b.txt", "b\n");
        r.commit_all("init");
        let h = Harness::new(vec![record("a", r.path())]);
        r.write("a.txt", "1\n2\n");
        r.git(&["add", "a.txt"]);
        r.write("a.txt", "1\n2\n3\n4\n");
        r.write("b.txt", "b\nc\n");
        r.write("new.txt", "n\n");
        let files = refresh(&h.engine, "a").unwrap();
        let part = |p: Option<&crate::vcs::git::Part>| p.map(|p| (p.added, p.removed));
        type Row<'a> = (&'a str, u32, Option<(u32, u32)>, Option<(u32, u32)>);
        let got: Vec<Row> = files
            .iter()
            .map(|f| {
                let parts = f.parts.clone().unwrap();
                (f.path.as_str(), f.added, part(parts.staged.as_ref()), part(parts.unstaged.as_ref()))
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("a.txt", 3, Some((1, 0)), Some((2, 0))),
                ("b.txt", 1, None, Some((1, 0))),
                ("new.txt", 1, None, Some((1, 0))),
            ]
        );
        let v = file_part_versions(&h.engine, "a", "a.txt", Some(Stage::Staged)).unwrap();
        assert_eq!((v.original.as_deref(), v.modified.as_deref()), (Some("1\n"), Some("1\n2\n")));
        let v = file_part_versions(&h.engine, "a", "a.txt", Some(Stage::Unstaged)).unwrap();
        assert_eq!((v.original.as_deref(), v.modified.as_deref()), (Some("1\n2\n"), Some("1\n2\n3\n4\n")));
        let v = file_part_versions(&h.engine, "a", "new.txt", Some(Stage::Unstaged)).unwrap();
        assert_eq!((v.original, v.modified.as_deref()), (None, Some("n\n")));
    }

    /// An agent in a plain folder holding a clone: the clone's changes are
    /// its changes, and one file's diff is read from the clone.
    #[test]
    fn repositories_inside_the_folder_are_listed() {
        let outer = TempRepo(std::env::temp_dir().join(format!("pitwall-test-{}", uuid::Uuid::new_v4())));
        let inner = TempRepo::new();
        let dest = outer.0.join("upstreams/lib");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::rename(&inner.0, &dest).unwrap();
        let lib = TempRepo(dest);
        lib.write("a.txt", "1\n");
        lib.commit_all("init");
        lib.write("a.txt", "1\n2\n");
        let h = Harness::new(vec![record("a", outer.path())]);
        let files = refresh(&h.engine, "a").unwrap();
        let got: Vec<(&str, Option<&str>)> = files.iter().map(|f| (f.path.as_str(), f.repo.as_deref())).collect();
        assert_eq!(got, vec![("upstreams/lib/a.txt", Some("upstreams/lib"))]);
        let v = &h.engine.views()[0];
        assert_eq!((v.added, v.files_changed), (1, 1));
        assert!(v.caps.diff, "diffs, though the folder isn't a repository");
        // The panel's quiet polls read them too.
        let polled: Vec<String> = changes(&h.engine, "a").unwrap().into_iter().map(|f| f.path).collect();
        assert_eq!(polled, vec!["upstreams/lib/a.txt".to_string()]);
        let v = file_versions(&h.engine, "a", "upstreams/lib/a.txt").unwrap();
        assert_eq!((v.original.as_deref(), v.modified.as_deref()), (Some("1\n"), Some("1\n2\n")));
        drop(lib);
    }

    #[test]
    fn non_ascii_names_are_kept() {
        // git would print these as octal escapes ("\341\203\221…") by default.
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let h = Harness::new(vec![record("a", r.path())]);
        r.write("ფაილი.xlsx", "x\n");
        r.write("a.txt", "1\n2\n");
        r.commit_all("tracked");
        r.write("ანგარიში 2.txt", "y\n");
        let files = changes(&h.engine, "a").unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["ანგარიში 2.txt"]);
        assert_eq!((files[0].added, files[0].binary), (1, false));
        r.write("ფაილი.xlsx", "x\nz\n");
        let files = changes(&h.engine, "a").unwrap();
        assert!(files.iter().any(|f| f.path == "ფაილი.xlsx" && !f.untracked));
    }

    #[test]
    fn committed_work_is_not_listed() {
        // Matches `git status`: commits the agent made since it started are
        // Review's business, not the Changes list's.
        let r = TempRepo::new();
        r.write("a.txt", "1\n");
        r.commit_all("init");
        let mut rec = record("a", r.path());
        rec.base_commit = Some(r.git(&["rev-parse", "HEAD"]).trim().to_string());
        let h = Harness::new(vec![rec]);
        r.write("a.txt", "1\n2\n");
        r.commit_all("agent's commit");
        assert!(changes(&h.engine, "a").unwrap().is_empty());
        r.write("b.txt", "x\n");
        let files = changes(&h.engine, "a").unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(h.engine.views()[0].files_changed, 1);
    }

    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use crate::error::Result as PwResult;
    use crate::exec::{Cmd, Exec, Out, Stat};
    use crate::testing::FakeExec;

    /// The diff against HEAD (not the staged and unstaged ones that ride along).
    const DIFF: &[&str] = &["git", "-C", "/w", "diff", "--raw", "--numstat", "-z", "-M", "HEAD"];

    fn scripted(numstat: &str) -> Arc<FakeExec> {
        let x = FakeExec::new();
        x.on(&["git", "-C", "/w", "diff", "--raw", "--numstat", "-z", "-M", "HEAD"], numstat)
            .on(&["git", "-C", "/w", "ls-files", "-z", "--others", "--exclude-standard"], "")
            .on(&["git", "-C", "/w", "symbolic-ref", "--quiet", "--short", "HEAD"], "main\n");
        x
    }

    #[test]
    fn refresh_bypasses_the_polling_backoff() {
        let x = scripted("3\t1\tsrc/a.rs\0");
        let h = Harness::with_exec(vec![record("a", "/w")], x.clone());
        // A slow provider, idle and fully backed off: the ticker wouldn't look.
        h.engine
            .with("a", |a| {
                a.facts.provider.git_poll_ms = 15_000;
                a.git_wanted = false;
                a.git_every = 30_000;
                a.git_at = h.engine.now();
                a.status = crate::model::Status::Idle;
            })
            .unwrap();
        h.clock.advance(1_000);
        assert!(super::super::ticker::git_due_for_test(&h.engine).is_empty(), "not due");

        let files = refresh(&h.engine, "a").unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(x.ran(DIFF), 1);
        let v = &h.engine.views()[0];
        assert_eq!((v.added, v.removed, v.files_changed, v.branch.as_deref()), (3, 1, 1, Some("main")));
        assert_eq!(h.engine.wait_dirty(), (true, false), "agents-changed goes out");
        h.engine
            .with("a", |a| {
                assert_eq!(a.git_at, h.engine.now(), "counts as the latest look");
                assert_eq!(a.git_every, 15_000, "a change resets the back-off to the provider's pace");
                assert!(!a.git_inflight);
            })
            .unwrap();

        // Asked again right away: git runs again (no back-off for the user).
        refresh(&h.engine, "a").unwrap();
        assert_eq!(x.ran(DIFF), 2);
        assert!(refresh(&h.engine, "ghost").is_err());
    }

    #[test]
    fn refresh_refuses_where_git_cant_run() {
        let h = Harness::with_exec(vec![record("a", "/w")], scripted(""));
        h.engine.with("a", |a| a.facts.provider.exec = false).unwrap();
        assert!(refresh(&h.engine, "a").unwrap_err().contains("can't run git"));
    }

    /// A machine whose `git diff` waits until the test lets it go.
    struct Gated {
        inner: Arc<FakeExec>,
        entered: Mutex<Sender<()>>,
        release: Mutex<Receiver<()>>,
    }

    impl Exec for Gated {
        fn run(&self, cmd: &Cmd) -> PwResult<Out> {
            if cmd.argv.get(3) == Some(&"diff") {
                let _ = self.entered.lock().unwrap().send(());
                let _ = self.release.lock().unwrap().recv_timeout(Duration::from_secs(10));
            }
            self.inner.run(cmd)
        }
        fn read_file(&self, path: &str, max: u64) -> PwResult<Option<Vec<u8>>> {
            self.inner.read_file(path, max)
        }
        fn write_file(&self, path: &str, bytes: &[u8]) -> PwResult<()> {
            self.inner.write_file(path, bytes)
        }
        fn remove_file(&self, path: &str) -> PwResult<()> {
            self.inner.remove_file(path)
        }
        fn remove_dir(&self, path: &str) -> PwResult<()> {
            self.inner.remove_dir(path)
        }
        fn copy_file(&self, from: &str, to: &str) -> PwResult<()> {
            self.inner.copy_file(from, to)
        }
        fn stat(&self, path: &str) -> PwResult<Option<Stat>> {
            self.inner.stat(path)
        }
        fn real_path(&self, path: &str) -> PwResult<String> {
            self.inner.real_path(path)
        }
        fn temp_dir(&self) -> PwResult<String> {
            self.inner.temp_dir()
        }
        fn home(&self) -> PwResult<String> {
            self.inner.home()
        }
    }

    #[test]
    fn concurrent_refreshes_share_one_git_run() {
        let x = scripted("2\t0\tb.txt\0");
        let (entered_tx, entered) = channel();
        let (release, release_rx) = channel();
        let gated = Arc::new(Gated { inner: x.clone(), entered: Mutex::new(entered_tx), release: Mutex::new(release_rx) });
        let h = Harness::with_exec(vec![record("a", "/w")], gated);
        let first = {
            let e = h.engine.clone();
            std::thread::spawn(move || refresh(&e, "a"))
        };
        entered.recv_timeout(Duration::from_secs(10)).expect("git started");
        let second = {
            let e = h.engine.clone();
            std::thread::spawn(move || refresh(&e, "a"))
        };
        while h.engine.git_flights.waiters("a") == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
        release.send(()).unwrap();
        let (a, b) = (first.join().unwrap().unwrap(), second.join().unwrap().unwrap());
        assert_eq!(a, b, "the second caller gets the first one's result");
        assert_eq!(x.ran(DIFF), 1, "git ran once");
        assert!(!h.engine.with("a", |a| a.git_inflight).unwrap());
    }

    #[test]
    fn git_runs_on_the_agents_machine() {
        let x = crate::testing::FakeExec::new();
        x.on(&["git", "-C", "/remote/w", "diff", "--raw", "--numstat", "-z", "-M", "HEAD"], "4\t2\tsrc/a.rs\0")
            .on(&["git", "-C", "/remote/w", "ls-files", "-z", "--others", "--exclude-standard"], "");
        let mut rec = record("a", "/remote/w");
        rec.base_commit = Some("b0".into());
        let h = Harness::with_exec(vec![rec], x.clone());
        assert_eq!(changes(&h.engine, "a").unwrap().len(), 1);
        let v = &h.engine.views()[0];
        assert_eq!((v.added, v.removed, v.files_changed), (4, 2, 1));
        // The diff, untracked files, and what's staged and what isn't.
        assert_eq!(x.ran(&["git", "-C", "/remote/w", ".."]), 4);
    }
}
