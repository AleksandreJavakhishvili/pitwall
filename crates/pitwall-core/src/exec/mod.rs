//! Running programs and touching files on the machine where an agent works
//! (architecture.md §2.4). Git, task snapshots, Review, worktree discovery and
//! rules all go through an [`Exec`] instead of `std::process` / `std::fs`, so
//! a remote implementation (agw's `vm exec`) drops in without touching them.
//! Independent commands and file reads go through [`Exec::run_all`] /
//! [`Exec::read_files`] so a remote exec can answer them in one round trip.
//!
//! [`LocalExec`] is this machine. Paths are strings on the target machine:
//! join them with [`join`], never with `std::path` on the host.

mod local;

use std::time::Duration;

pub use local::LocalExec;
pub use crate::error::{PwError, Result};

/// Generous bound for commands that had none before (git, rulesync, login
/// shells): only a hung program ever hits it.
pub const LONG: Duration = Duration::from_secs(600);

/// One program to run.
#[derive(Debug, Clone, Copy)]
pub struct Cmd<'a> {
    /// Program and its arguments.
    pub argv: &'a [&'a str],
    /// Working directory on the target machine; `None` = the exec's default.
    pub cwd: Option<&'a str>,
    /// Added to (or overriding) the inherited environment.
    pub env: &'a [(&'a str, &'a str)],
    /// Bytes to feed on stdin; `None` = empty stdin.
    pub stdin: Option<&'a [u8]>,
    /// The program is killed after this long (`run` then fails).
    pub timeout: Duration,
}

impl<'a> Cmd<'a> {
    pub fn new(argv: &'a [&'a str]) -> Cmd<'a> {
        Cmd { argv, cwd: None, env: &[], stdin: None, timeout: LONG }
    }
    pub fn cwd(self, cwd: &'a str) -> Cmd<'a> {
        Cmd { cwd: Some(cwd), ..self }
    }
    pub fn env(self, env: &'a [(&'a str, &'a str)]) -> Cmd<'a> {
        Cmd { env, ..self }
    }
    pub fn stdin(self, stdin: &'a [u8]) -> Cmd<'a> {
        Cmd { stdin: Some(stdin), ..self }
    }
    pub fn timeout(self, timeout: Duration) -> Cmd<'a> {
        Cmd { timeout, ..self }
    }
}

/// What a finished program left behind.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Out {
    /// Exit code; -1 when it ended without one (a signal).
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.status == 0
    }
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// A path's entry itself (symlinks are not followed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub kind: FileKind,
    pub len: u64,
}

/// Run programs and read/write files on one machine. Errors carry a code
/// and a human-readable message. Blocking, like everything in the core.
pub trait Exec: Send + Sync {
    /// Run to completion. `Err` only when it couldn't run or timed out; a
    /// non-zero exit is an `Ok` with that `status`.
    fn run(&self, cmd: &Cmd) -> Result<Out>;
    /// At most `max` bytes of the file (a longer one comes back cut at
    /// `max`; ask for one byte more than you accept to spot that). `None`
    /// when it doesn't exist.
    fn read_file(&self, path: &str, max: u64) -> Result<Option<Vec<u8>>>;
    /// Create or replace a file, creating missing parent folders.
    fn write_file(&self, path: &str, bytes: &[u8]) -> Result<()>;
    fn remove_file(&self, path: &str) -> Result<()>;
    /// Remove an empty folder (fails when it isn't empty).
    fn remove_dir(&self, path: &str) -> Result<()>;
    fn copy_file(&self, from: &str, to: &str) -> Result<()>;
    /// `None` when nothing is at `path`.
    fn stat(&self, path: &str) -> Result<Option<Stat>>;
    /// Absolute path with every symlink resolved; fails when it doesn't exist.
    fn real_path(&self, path: &str) -> Result<String>;
    /// A scratch folder on the machine (temp index files for task snapshots).
    fn temp_dir(&self) -> Result<String>;
    /// The user's home folder there (for "~/…" display).
    fn home(&self) -> Result<String>;

    /// Several commands that don't depend on each other, each answered like
    /// [`run`](Self::run), in order. A remote exec runs them in one round
    /// trip; here they simply run one after another.
    fn run_all(&self, cmds: &[Cmd]) -> Vec<Result<Out>> {
        cmds.iter().map(|c| self.run(c)).collect()
    }

    /// [`read_file`](Self::read_file) for several files (one round trip
    /// for a remote exec).
    fn read_files(&self, paths: &[&str], max: u64) -> Vec<Result<Option<Vec<u8>>>> {
        paths.iter().map(|p| self.read_file(p, max)).collect()
    }
}

/// `rel` inside `dir` (target-machine path syntax).
pub fn join(dir: &str, rel: &str) -> String {
    if dir.is_empty() {
        return rel.to_string();
    }
    format!("{}/{}", dir.trim_end_matches('/'), rel.trim_start_matches('/'))
}

/// `path` is a folder (following symlinks).
pub fn is_dir(exec: &dyn Exec, path: &str) -> bool {
    exec.real_path(path)
        .ok()
        .and_then(|p| exec.stat(&p).ok().flatten())
        .is_some_and(|s| s.kind == FileKind::Dir)
}

/// Something exists at `path` (following symlinks, like `Path::exists`).
pub fn exists(exec: &dyn Exec, path: &str) -> bool {
    exec.real_path(path).is_ok()
}

/// stdout of a program on this machine if it finishes within `timeout`,
/// whatever its exit status (the onboarding scan and process listings).
pub(crate) fn local_stdout(argv: &[&str], timeout: Duration) -> Option<String> {
    LocalExec.run(&Cmd::new(argv).timeout(timeout)).ok().map(|o| o.stdout_text())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_target_paths() {
        assert_eq!(join("/r", "a/b"), "/r/a/b");
        assert_eq!(join("/tmp/", "x"), "/tmp/x");
        assert_eq!(join("", "x"), "x");
    }
}
