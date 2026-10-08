//! Git plumbing, run through an [`Exec`] on the machine where the agent works.
//! Every command runs with GIT_OPTIONAL_LOCKS=0, so polling never fights with
//! the agents for the index lock, and never prompts or opens an editor.

use serde::Serialize;

use crate::exec::{self, Cmd, Exec, Out};

/// Every git Pitwall runs: no lock files, no prompts, and paths printed as
/// they are (`core.quotePath=false`) so non-ASCII names (Georgian, emoji, …)
/// aren't shown as octal escapes.
pub(crate) const GIT_ENV: [(&str, &str); 6] = [
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_EDITOR", "true"),
    ("GIT_CONFIG_COUNT", "1"),
    ("GIT_CONFIG_KEY_0", "core.quotePath"),
    ("GIT_CONFIG_VALUE_0", "false"),
];

/// Untracked files larger than this count as binary in [`Git::changes`].
const MAX_COUNTED_BYTES: u64 = 1_000_000;

/// Git in one folder (`git -C <dir>`) of the machine `exec` reaches.
#[derive(Clone)]
pub struct Git<'a> {
    exec: &'a dyn Exec,
    dir: String,
}

impl<'a> Git<'a> {
    pub fn new(exec: &'a dyn Exec, dir: &str) -> Git<'a> {
        Git { exec, dir: dir.to_string() }
    }

    /// The same machine, another folder.
    pub fn at(&self, dir: &str) -> Git<'a> {
        Git::new(self.exec, dir)
    }

    pub fn exec(&self) -> &'a dyn Exec {
        self.exec
    }

    pub fn dir(&self) -> &str {
        &self.dir
    }

    /// `git -C <dir> <args>` with `env` on top of the defaults; any exit status.
    pub(crate) fn output(&self, args: &[&str], env: &[(&str, &str)], stdin: Option<&[u8]>) -> Result<Out, String> {
        let mut argv = vec!["git", "-C", &self.dir];
        argv.extend_from_slice(args);
        let mut all_env = GIT_ENV.to_vec();
        all_env.extend_from_slice(env);
        let mut cmd = Cmd::new(&argv).env(&all_env);
        if let Some(input) = stdin {
            cmd = cmd.stdin(input);
        }
        Ok(self.exec.run(&cmd)?)
    }

    /// stdout on success, else git's trimmed stderr.
    pub(crate) fn run_with(&self, args: &[&str], env: &[(&str, &str)], stdin: Option<&[u8]>) -> Result<String, String> {
        let out = self.output(args, env, stdin)?;
        if out.ok() {
            Ok(out.stdout_text())
        } else {
            Err(out.stderr_text())
        }
    }

    pub(crate) fn run(&self, args: &[&str]) -> Result<String, String> {
        self.run_with(args, &[], None)
    }

    fn line(&self, args: &[&str]) -> Option<String> {
        self.run(args).ok().map(|s| s.trim().to_string())
    }

    pub fn repo_root(&self) -> Option<String> {
        self.line(&["rev-parse", "--show-toplevel"])
    }

    pub fn head(&self) -> Option<String> {
        self.line(&["rev-parse", "HEAD"])
    }

    /// Current branch name, or `None` when detached or not a repo.
    pub fn current_branch(&self) -> Option<String> {
        self.line(&["symbolic-ref", "--quiet", "--short", "HEAD"]).filter(|s| !s.is_empty())
    }

    /// `git worktree remove` (never `--force`: refuses on uncommitted changes).
    pub fn worktree_remove(&self, path: &str) -> Result<(), String> {
        self.run(&["worktree", "remove", path]).map(|_| ())
    }

    /// Every worktree of the repo (the main checkout first).
    pub fn worktree_list(&self) -> Result<Vec<WorktreeEntry>, String> {
        Ok(parse_worktree_list(&self.run(&["worktree", "list", "--porcelain"])?))
    }

    pub fn merge_base(&self, a: &str, b: &str) -> Option<String> {
        self.line(&["merge-base", a, b]).filter(|s| !s.is_empty())
    }

    /// Everything that differs from `base` in the working tree, including
    /// commits the agent made and files it created but never added.
    pub fn changes(&self, base: Option<&str>) -> Result<Vec<FileChange>, String> {
        self.changes_with(base, false).0
    }

    /// [`changes`](Self::changes) and [`current_branch`](Self::current_branch)
    /// at once (the poller's refresh): the git commands go in one round trip
    /// ([`Exec::run_all`]), the untracked files' contents in one more, only
    /// when there are any.
    pub fn changes_and_branch(&self, base: Option<&str>) -> (Result<Vec<FileChange>, String>, Option<String>) {
        self.changes_with(base, true)
    }

    fn changes_with(&self, base: Option<&str>, branch: bool) -> (Result<Vec<FileChange>, String>, Option<String>) {
        let base = base.unwrap_or("HEAD");
        // Status letters (`--raw`) and line counts (`--numstat`) in one
        // diff, NUL-separated so odd paths survive; renames detected (`-M`).
        let diff = git_argv(&self.dir, &["diff", "--raw", "--numstat", "-z", "-M", base]);
        let others = git_argv(&self.dir, &["ls-files", "-z", "--others", "--exclude-standard"]);
        let head = git_argv(&self.dir, &["symbolic-ref", "--quiet", "--short", "HEAD"]);
        let mut cmds = vec![Cmd::new(&diff).env(&GIT_ENV), Cmd::new(&others).env(&GIT_ENV)];
        if branch {
            cmds.push(Cmd::new(&head).env(&GIT_ENV));
        }
        let mut outs = self.exec.run_all(&cmds).into_iter().map(|r| match r {
            Ok(o) if o.ok() => Ok(o.stdout_text()),
            Ok(o) => Err(o.stderr_text()),
            Err(e) => Err(String::from(e)),
        });
        let (diff, others) = (outs.next().unwrap_or(Err(String::new())), outs.next().unwrap_or(Err(String::new())));
        let branch = outs.next().and_then(|r| r.ok()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        (self.collect_changes(diff, others), branch)
    }

    fn collect_changes(&self, diff: Result<String, String>, others: Result<String, String>) -> Result<Vec<FileChange>, String> {
        let mut files = Vec::new();
        let out = diff.map_err(|e| {
            if e.contains("Not a git repository") || e.contains("not a git repository") {
                NOT_A_REPO.to_string()
            } else {
                e
            }
        })?;
        files.extend(parse_raw_numstat(&out));
        let untracked = others?;
        // NUL-separated (`-z`): names with quotes, tabs or newlines survive.
        let paths: Vec<&str> = untracked.split('\0').filter(|l| !l.is_empty()).collect();
        let full: Vec<String> = paths.iter().map(|p| exec::join(&self.dir, p)).collect();
        let full: Vec<&str> = full.iter().map(String::as_str).collect();
        let contents = if full.is_empty() { Vec::new() } else { self.exec.read_files(&full, MAX_COUNTED_BYTES + 1) };
        for (path, read) in paths.into_iter().zip(contents) {
            let (added, binary) = match read {
                Ok(Some(bytes)) if bytes.len() as u64 <= MAX_COUNTED_BYTES && !bytes.contains(&0) => {
                    let lines = bytes.iter().filter(|&&b| b == b'\n').count();
                    let trailing = usize::from(!bytes.is_empty() && !bytes.ends_with(b"\n"));
                    ((lines + trailing) as u32, false)
                }
                _ => (0, true),
            };
            files.push(FileChange { path: path.to_string(), added, removed: 0, untracked: true, binary, status: Some(FileStatus::U) });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    pub fn file_diff(&self, base: Option<&str>, path: &str, untracked: bool) -> Result<String, String> {
        if untracked {
            // `--no-index` exits 1 when files differ, which is the expected case.
            let out = self.output(&["diff", "--no-index", "--", "/dev/null", path], &[], None)?;
            return Ok(out.stdout_text());
        }
        self.run(&["diff", base.unwrap_or("HEAD"), "--", path])
    }
}

fn git_argv<'s>(dir: &'s str, args: &[&'s str]) -> Vec<&'s str> {
    [&["git", "-C", dir][..], args].concat()
}

/// One record of `git worktree list --porcelain`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorktreeEntry {
    pub path: String,
    /// Commit checked out (absent for a bare repository).
    pub head: Option<String>,
    /// Short branch name; `None` when detached (or bare).
    pub branch: Option<String>,
    /// The repository itself, without a working tree.
    pub bare: bool,
    pub detached: bool,
    /// `git worktree lock`ed: git refuses to remove or prune it.
    pub locked: bool,
    pub lock_reason: Option<String>,
    /// Its folder is gone; `git worktree prune` would drop it.
    pub prunable: bool,
}

/// Lock and prune reasons may be C-quoted when they contain odd characters.
fn unquote(s: &str) -> String {
    let Some(inner) = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else { return s.to_string() };
    let mut out = String::new();
    let mut it = inner.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(o) => out.push(o),
            None => break,
        }
    }
    out
}

/// Records of `git worktree list --porcelain` (main worktree first): one
/// attribute per line, a blank line between records.
pub fn parse_worktree_list(out: &str) -> Vec<WorktreeEntry> {
    let mut res: Vec<WorktreeEntry> = Vec::new();
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            res.push(WorktreeEntry { path: p.to_string(), ..Default::default() });
            continue;
        }
        let Some(last) = res.last_mut() else { continue };
        let (key, value) = line.split_once(' ').map_or((line, None), |(k, v)| (k, Some(v)));
        match key {
            "HEAD" => last.head = value.map(String::from),
            "branch" => last.branch = value.map(|b| b.strip_prefix("refs/heads/").unwrap_or(b).to_string()),
            "bare" => last.bare = true,
            "detached" => last.detached = true,
            "locked" => {
                last.locked = true;
                last.lock_reason = value.map(unquote).filter(|r| !r.is_empty());
            }
            "prunable" => last.prunable = true,
            _ => {}
        }
    }
    res
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct FileChange {
    /// For a rename: the new path.
    pub path: String,
    pub added: u32,
    pub removed: u32,
    pub untracked: bool,
    pub binary: bool,
    /// Git's letter for it, as VS Code's SCM view shows it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<FileStatus>,
}

/// Modified, Added, Deleted, Renamed, Untracked (shared with the explorer).
pub use pitwall_proto::FileStatus;

/// Output of `git diff --raw --numstat -z -M`: raw records (`:<modes> <shas>
/// <letter>`, then one path, two for renames/copies), then numstat records
/// (`<added>\t<removed>\t<path>`, or an empty path followed by old and new
/// for renames). Joined by (new) path.
pub fn parse_raw_numstat(out: &str) -> Vec<FileChange> {
    let mut status: std::collections::HashMap<String, FileStatus> = Default::default();
    let mut files = Vec::new();
    let mut it = out.split('\0').filter(|t| !t.is_empty()).peekable();
    while let Some(tok) = it.next() {
        if let Some(meta) = tok.strip_prefix(':') {
            let letter = meta.rsplit(' ').next().unwrap_or("M");
            let st = FileStatus::from_git(letter);
            let mut path = it.next().unwrap_or_default();
            if matches!(letter.chars().next(), Some('R') | Some('C')) {
                path = it.next().unwrap_or(path);
            }
            status.insert(path.to_string(), st);
            continue;
        }
        let mut parts = tok.splitn(3, '\t');
        let (Some(a), Some(r), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let path = if path.is_empty() {
            // Rename: old path, then new path.
            let _old = it.next();
            it.next().unwrap_or_default()
        } else {
            path
        };
        files.push(FileChange {
            path: path.to_string(),
            added: a.parse().unwrap_or(0),
            removed: r.parse().unwrap_or(0),
            untracked: false,
            binary: a == "-",
            status: Some(status.get(path).copied().unwrap_or(FileStatus::M)),
        });
    }
    files
}

/// Error returned by [`Git::changes`] when the agent works outside a git
/// repository; the UI shows a calm note instead of git's usage text.
pub const NOT_A_REPO: &str = "not-a-git-repo";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeExec;

    #[test]
    fn every_call_is_lock_free_and_targets_the_folder() {
        let x = FakeExec::new();
        x.on(&["git", "-C", "/r", "rev-parse", "--show-toplevel"], "/r\n");
        x.on(&["git", "-C", "/r", "symbolic-ref", "--quiet", "--short", "HEAD"], "\n");
        let g = Git::new(&*x, "/r");
        assert_eq!(g.repo_root().as_deref(), Some("/r"));
        assert_eq!(g.current_branch(), None, "empty output = detached");
        assert_eq!(g.head(), None, "unscripted command fails");
        let calls = x.calls();
        assert_eq!(calls.len(), 3);
        assert!(calls.iter().all(|c| c.env_has("GIT_OPTIONAL_LOCKS", "0") && c.env_has("GIT_TERMINAL_PROMPT", "0")));
    }

    #[test]
    fn changes_count_tracked_and_untracked_files() {
        let x = FakeExec::new();
        x.on(
            &["git", "-C", "/r", "diff", "--raw", "--numstat", "-z", "-M", "abc"],
            ":100644 100644 aa bb M\0src/a.rs\0:100644 100644 cc dd M\0img.png\0\
             3\t1\tsrc/a.rs\0-\t-\timg.png\0",
        );
        x.on(
            &["git", "-C", "/r", "ls-files", "-z", "--others", "--exclude-standard"],
            "new.txt\0no-newline.txt\0bin.dat\0big.log\0vanished\0",
        );
        x.file("/r/new.txt", b"a\nb\n")
            .file("/r/no-newline.txt", b"a\nb")
            .file("/r/bin.dat", b"a\0b")
            .file("/r/big.log", &vec![b'x'; MAX_COUNTED_BYTES as usize + 1]);
        let files = Git::new(&*x, "/r").changes(Some("abc")).unwrap();
        let by = |p: &str| files.iter().find(|f| f.path == p).cloned().unwrap();
        assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), [
            "big.log", "bin.dat", "img.png", "new.txt", "no-newline.txt", "src/a.rs", "vanished"
        ]);
        assert_eq!((by("src/a.rs").added, by("src/a.rs").removed, by("src/a.rs").untracked), (3, 1, false));
        assert!(by("img.png").binary);
        assert_eq!((by("new.txt").added, by("new.txt").untracked, by("new.txt").binary), (2, true, false));
        assert_eq!(by("no-newline.txt").added, 2);
        assert!(by("bin.dat").binary && by("big.log").binary && by("vanished").binary);
    }

    #[test]
    fn outside_a_repo_changes_say_so() {
        let x = FakeExec::new();
        x.on_exit(&["git", "-C", "/tmp", "diff", ".."], 128, "", "fatal: not a git repository (or any parent)");
        assert_eq!(Git::new(&*x, "/tmp").changes(None).unwrap_err(), NOT_A_REPO);
        x.on_exit(&["git", "-C", "/tmp", "diff", ".."], 128, "", "fatal: bad revision");
        assert_eq!(Git::new(&*x, "/tmp").changes(None).unwrap_err(), "fatal: bad revision");
    }

    #[test]
    fn status_letters_include_deletes_and_renames() {
        let out = ":100644 000000 aa 00 D\0gone.txt\0\
                   :100644 100644 bb bb R087\0old name.rs\0new name.rs\0\
                   :000000 100644 00 cc A\0added.md\0\
                   :100644 100644 dd ee M\0m.rs\0\
                   0\t3\tgone.txt\0\
                   1\t1\t\0old name.rs\0new name.rs\0\
                   2\t0\tadded.md\0\
                   1\t1\tm.rs\0";
        let files = parse_raw_numstat(out);
        let st: Vec<_> = files.iter().map(|f| (f.path.as_str(), f.status.unwrap(), f.added, f.removed)).collect();
        assert_eq!(st, [
            ("gone.txt", FileStatus::D, 0, 3),
            ("new name.rs", FileStatus::R, 1, 1),
            ("added.md", FileStatus::A, 2, 0),
            ("m.rs", FileStatus::M, 1, 1),
        ]);
        assert_eq!(serde_json::to_value(FileStatus::R).unwrap(), "R");
    }

    #[test]
    fn real_git_reports_deletes_renames_and_untracked() {
        let r = crate::vcs::snapshot::TempRepo::new();
        r.write("a.txt", "one\ntwo\nthree\nfour\n");
        r.write("b.txt", "b\n");
        r.commit_all("init");
        r.git(&["mv", "a.txt", "moved.txt"]);
        std::fs::remove_file(r.0.join("b.txt")).unwrap();
        r.write("new.txt", "n\n");
        let files = Git::new(&crate::exec::LocalExec, r.path()).changes(None).unwrap();
        let st: Vec<_> = files.iter().map(|f| (f.path.as_str(), f.status.unwrap())).collect();
        assert_eq!(st, [("b.txt", FileStatus::D), ("moved.txt", FileStatus::R), ("new.txt", FileStatus::U)]);
    }

    #[test]
    fn untracked_diff_accepts_exit_one() {
        let x = FakeExec::new();
        x.on_exit(&["git", "-C", "/r", "diff", "--no-index", "--", "/dev/null", "n.txt"], 1, "+hi\n", "");
        assert_eq!(Git::new(&*x, "/r").file_diff(None, "n.txt", true).unwrap(), "+hi\n");
        x.on(&["git", "-C", "/r", "diff", "HEAD", "--", "a.txt"], "-a\n+b\n");
        assert_eq!(Git::new(&*x, "/r").file_diff(None, "a.txt", false).unwrap(), "-a\n+b\n");
    }
}
