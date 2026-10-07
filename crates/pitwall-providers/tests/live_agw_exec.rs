//! Diffs and Review through agw's exec, end to end and strictly read-only:
//! find the session, list its changes (numstat + status letters + untracked
//! files, the poller's refresh), read one tracked file, and take a task
//! snapshot tree with a temporary index — then check that `git status`, the
//! real index, HEAD and the refs are exactly as before.
//!
//! The snapshot writes its objects into a throwaway object folder under the
//! VM's temp dir (`GIT_OBJECT_DIRECTORY`, the repo's own objects as an
//! alternate), and no ref is created (`snapshot::keep` is not called): the
//! repository itself is never written. Every git command goes through a
//! guard that refuses anything but reads and the temp-index snapshot, and
//! every agw call is checked against agw's read-only commands and `exec`.
//!
//! Runs against the fake agw always; against the real one only when asked:
//! `PITWALL_AGW_LIVE_EXEC=<session> cargo test -p pitwall-providers --test
//! live_agw_exec -- --ignored --nocapture` (read-only commands only).

// The fake agw (and agw itself) are shell scripts / Unix tools.
#![cfg(unix)]

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use pitwall_core::exec::{self, Cmd, Exec, LocalExec, Out, Result, Stat};
use pitwall_core::provider::{Locator, Provider};
use pitwall_core::testing::TempDir;
use pitwall_core::vcs::git::Git;
use pitwall_core::vcs::snapshot;
use pitwall_providers::agw::{AgwConfig, AgwProvider};

/// Runs agw on this Mac and remembers every command line.
#[derive(Default)]
struct Recorder(Mutex<Vec<Vec<String>>>);

impl Recorder {
    fn calls(&self) -> Vec<Vec<String>> {
        self.0.lock().unwrap().clone()
    }
}

impl Exec for Recorder {
    fn run(&self, cmd: &Cmd) -> Result<Out> {
        self.0.lock().unwrap().push(cmd.argv.iter().map(|s| s.to_string()).collect());
        LocalExec.run(cmd)
    }
    fn read_file(&self, path: &str, max: u64) -> Result<Option<Vec<u8>>> {
        LocalExec.read_file(path, max)
    }
    fn write_file(&self, _: &str, _: &[u8]) -> Result<()> {
        panic!("never writes on this Mac")
    }
    fn remove_file(&self, _: &str) -> Result<()> {
        panic!("never removes on this Mac")
    }
    fn remove_dir(&self, _: &str) -> Result<()> {
        panic!("never removes on this Mac")
    }
    fn copy_file(&self, _: &str, _: &str) -> Result<()> {
        panic!("never copies on this Mac")
    }
    fn stat(&self, path: &str) -> Result<Option<Stat>> {
        LocalExec.stat(path)
    }
    fn real_path(&self, path: &str) -> Result<String> {
        LocalExec.real_path(path)
    }
    fn temp_dir(&self) -> Result<String> {
        LocalExec.temp_dir()
    }
    fn home(&self) -> Result<String> {
        LocalExec.home()
    }
}

/// The VM, but only reads: git commands that read, the snapshot's
/// temp-index commands (objects into `scratch`), and file writes only under
/// `scratch`. Anything else panics before it reaches the VM.
struct ReadOnly<'a> {
    inner: &'a dyn Exec,
    /// `<temp>/pitwall-live-…`: the throwaway objects and temp files.
    scratch: String,
    /// GIT_OBJECT_DIRECTORY / GIT_ALTERNATE_OBJECT_DIRECTORIES for git.
    objects: Option<(String, String)>,
}

const READS: &[&str] = &["rev-parse", "status", "diff", "ls-files", "symbolic-ref", "cat-file", "for-each-ref", "ls-tree"];
const TEMP_INDEX: &[&str] = &["add", "write-tree", "read-tree"];

impl ReadOnly<'_> {
    fn mine(&self, path: &str) {
        assert!(path.starts_with(&format!("{}/", self.scratch)) || path.starts_with(&exec::join(&self.temp(), "pitwall-index-")), "refused: write to {path}");
    }
    fn temp(&self) -> String {
        self.inner.temp_dir().unwrap()
    }
}

impl<'a> ReadOnly<'a> {
    /// The environment `cmd` runs with, after checking that it only reads.
    fn checked<'c>(&'c self, cmd: &Cmd<'c>) -> Vec<(&'c str, &'c str)> {
        let argv = cmd.argv;
        let mut env = cmd.env.to_vec();
        if argv.first() == Some(&"git") {
            assert_eq!(argv.get(1), Some(&"-C"), "git always runs with -C");
            let sub = argv.get(3).copied().unwrap_or_default();
            let index = cmd.env.iter().find(|(k, _)| *k == "GIT_INDEX_FILE").map(|(_, v)| *v);
            let allowed = READS.contains(&sub)
                || (TEMP_INDEX.contains(&sub)
                    && index.is_some_and(|i| {
                        self.mine(i);
                        self.objects.is_some()
                    }));
            assert!(allowed, "refused: {argv:?} (env {:?})", cmd.env);
            if let Some((objects, alternates)) = &self.objects {
                env.push(("GIT_OBJECT_DIRECTORY", objects));
                env.push(("GIT_ALTERNATE_OBJECT_DIRECTORIES", alternates));
            }
        } else {
            assert!(argv == ["mkdir", "-p", exec::join(&self.scratch, "objects").as_str()] || argv == ["rm", "-rf", self.scratch.as_str()], "refused: {argv:?}");
        }
        env
    }
}

impl Exec for ReadOnly<'_> {
    fn run(&self, cmd: &Cmd) -> Result<Out> {
        let env = self.checked(cmd);
        self.inner.run(&Cmd { env: &env, ..*cmd })
    }
    fn run_all(&self, cmds: &[Cmd]) -> Vec<Result<Out>> {
        let envs: Vec<_> = cmds.iter().map(|c| self.checked(c)).collect();
        let cmds: Vec<Cmd> = cmds.iter().zip(&envs).map(|(c, env)| Cmd { env, ..*c }).collect();
        self.inner.run_all(&cmds)
    }
    fn read_file(&self, path: &str, max: u64) -> Result<Option<Vec<u8>>> {
        self.inner.read_file(path, max)
    }
    fn read_files(&self, paths: &[&str], max: u64) -> Vec<Result<Option<Vec<u8>>>> {
        self.inner.read_files(paths, max)
    }
    fn write_file(&self, path: &str, bytes: &[u8]) -> Result<()> {
        self.mine(path);
        self.inner.write_file(path, bytes)
    }
    fn remove_file(&self, path: &str) -> Result<()> {
        self.mine(path);
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> Result<()> {
        self.mine(path);
        self.inner.remove_dir(path)
    }
    fn copy_file(&self, from: &str, to: &str) -> Result<()> {
        self.mine(to);
        self.inner.copy_file(from, to)
    }
    fn stat(&self, path: &str) -> Result<Option<Stat>> {
        self.inner.stat(path)
    }
    fn real_path(&self, path: &str) -> Result<String> {
        self.inner.real_path(path)
    }
    fn temp_dir(&self) -> Result<String> {
        self.inner.temp_dir()
    }
    fn home(&self) -> Result<String> {
        self.inner.home()
    }
}

/// `git status`, the real index's checksum, HEAD and every ref: one batch.
fn fingerprint(git: &Git, index: &str) -> Vec<String> {
    let x = git.exec();
    let dir = git.dir();
    let env = [("GIT_OPTIONAL_LOCKS", "0")];
    let argvs = [
        vec!["git", "-C", dir, "status", "--porcelain=v1", "-uall", "-z"],
        vec!["git", "-C", dir, "rev-parse", "HEAD"],
        vec!["git", "-C", dir, "for-each-ref", "--format=%(refname) %(objectname)"],
    ];
    let cmds: Vec<Cmd> = argvs.iter().map(|a| Cmd::new(a).env(&env)).collect();
    let mut res: Vec<String> = x.run_all(&cmds).into_iter().map(|r| r.map(|o| o.stdout_text()).unwrap_or_else(|e| format!("error: {e}"))).collect();
    // The real index, byte for byte (its length and an FNV hash).
    let index = x.read_file(index, 64 << 20).ok().flatten();
    res.push(format!("{:?}", index.map(|b| (b.len(), b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &c| (h ^ c as u64).wrapping_mul(0x100_0000_01b3))))));
    res
}

/// The read-only walk; returns what it saw for the log.
fn walk(p: &AgwProvider, rec: &Recorder, name: &str) -> String {
    let mut log = String::new();
    let started = Instant::now();
    // Which VM: agw's listings (read-only).
    let vm = p
        .machines()
        .expect("agw vm list")
        .into_iter()
        .find(|m| p.sessions(m.id.as_str(), false).is_ok_and(|s| s.iter().any(|s| s.name == name)))
        .unwrap_or_else(|| panic!("no agw session {name}"));
    let loc = Locator::new(p.id(), &vm.id, name);
    let x = p.session_exec(&loc).expect("session exec");
    log += &format!("{loc}: {:?} in workspace {:?}\n", x.user(), x.workspace());

    // The workspace's repository.
    let top = x.run(&Cmd::new(&["git", "rev-parse", "--show-toplevel"])).expect("rev-parse");
    assert!(top.ok(), "not a git repo: {}", top.stderr_text());
    let top = top.stdout_text().trim().to_string();
    let scratch = exec::join(&x.temp_dir().unwrap(), &format!("pitwall-live-{}", std::process::id()));
    let guard = ReadOnly { inner: &*x, scratch: scratch.clone(), objects: None };
    let git = Git::new(&guard, &top);
    let index = git_line(&git, &["rev-parse", "--path-format=absolute", "--git-path", "index"]);
    let objects = git_line(&git, &["rev-parse", "--path-format=absolute", "--git-path", "objects"]);
    let before = fingerprint(&git, &index);
    log += &format!("repo {top}, branch {:?}\n", git.current_branch());

    // The poller's refresh: one agw call for git, one more for untracked files.
    let n = rec.calls().len();
    let t = Instant::now();
    let (changes, branch) = git.changes_and_branch(None);
    let changes = changes.expect("changes");
    let calls = rec.calls().len() - n;
    let untracked = changes.iter().filter(|c| c.untracked).count();
    assert_eq!(calls, 1 + usize::from(untracked > 0), "refresh = one agw call (+1 for untracked files)");
    log += &format!(
        "changes: {} files (+{} -{}), {untracked} untracked, branch {branch:?}: {calls} agw call(s) in {:.1}s\n",
        changes.len(),
        changes.iter().map(|c| c.added).sum::<u32>(),
        changes.iter().map(|c| c.removed).sum::<u32>(),
        t.elapsed().as_secs_f32()
    );

    // One tracked file, as Review's editor reads it.
    let files = git_line(&git, &["ls-files"]);
    let first = files.lines().next().expect("a tracked file");
    let path = exec::join(&top, first);
    let t = Instant::now();
    let bytes = x.read_file(&path, 1_000_001).expect("read").expect("exists");
    let stat = x.stat(&path).unwrap().expect("stat");
    assert!(bytes.len() as u64 == stat.len.min(1_000_001) || stat.kind == pitwall_core::exec::FileKind::Symlink);
    log += &format!("read {first}: {} bytes in {:.1}s\n", bytes.len(), t.elapsed().as_secs_f32());

    // A task snapshot: temp index, objects into scratch, no ref.
    // GIT_OBJECT_DIRECTORY must exist for git to accept the repository.
    x.run(&Cmd::new(&["mkdir", "-p", &exec::join(&scratch, "objects")])).unwrap();
    let snap = ReadOnly { inner: &*x, scratch: scratch.clone(), objects: Some((exec::join(&scratch, "objects"), objects)) };
    let t = Instant::now();
    let tree = snapshot::snapshot(&Git::new(&snap, &top));
    let kind = tree.as_ref().ok().map(|t| git_line(&Git::new(&snap, &top), &["cat-file", "-t", t]));
    let cleaned = x.run(&Cmd::new(&["rm", "-rf", &scratch]));
    let tree = tree.expect("snapshot");
    assert_eq!(kind.as_deref(), Some("tree"));
    assert!(cleaned.is_ok_and(|o| o.ok()), "scratch removed");
    log += &format!("snapshot tree {tree} in {:.1}s (no ref created)\n", t.elapsed().as_secs_f32());

    let after = fingerprint(&git, &index);
    assert_eq!(before, after, "git status, index, HEAD and refs unchanged");
    log += &format!("unchanged: status, index, HEAD, refs; total {:.1}s\n", started.elapsed().as_secs_f32());

    // Only agw's read-only listings and exec ever ran.
    for c in rec.calls() {
        let what = c.get(2..4).map(|w| w.join(" ")).unwrap_or_default();
        assert!(["vm list", "session list", "session describe", "vm exec", "agent exec"].contains(&what.as_str()), "refused agw call: {c:?}");
    }
    log += &format!("{} agw calls, all read-only listings or exec\n", rec.calls().len());
    log
}

fn git_line(git: &Git, args: &[&str]) -> String {
    let mut argv = vec!["git", "-C", git.dir()];
    argv.extend_from_slice(args);
    let out = git.exec().run(&Cmd::new(&argv).env(&[("GIT_OPTIONAL_LOCKS", "0")])).expect("git");
    assert!(out.ok(), "git {args:?}: {}", out.stderr_text());
    out.stdout_text().trim().to_string()
}

#[test]
fn read_only_walk_against_the_fake_agw() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("agw-live-fake");
    let state = dir.path().join("agw");
    std::fs::create_dir_all(&state).unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let git = |args: &[&str]| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(&ws)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap()
            .success();
        assert!(ok);
    };
    git(&["init", "-q"]);
    std::fs::write(ws.join("README"), "hi\n").unwrap();
    git(&["add", "."]);
    git(&["-c", "user.name=pw", "-c", "user.email=pw@example.invalid", "commit", "-q", "-m", "init"]);
    std::fs::write(ws.join("README"), "hi\nthere\n").unwrap();
    std::fs::write(ws.join("new.txt"), "new\n").unwrap();
    let body = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_agw/agw")).unwrap();
    let agw = state.join("agw");
    std::fs::write(&agw, body.replace("__DIR__", &state.to_string_lossy())).unwrap();
    std::fs::set_permissions(&agw, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(state.join("s.state"), "stopped\n").unwrap();
    std::fs::write(state.join("s.ws"), ws.to_string_lossy().as_bytes()).unwrap();
    let rec = Arc::new(Recorder::default());
    let mut cfg = AgwConfig::new(dir.path().join("hold"), Err("no attach".into()));
    cfg.agw = Some(agw);
    cfg.exec = rec.clone();
    let p = AgwProvider::new(cfg);
    let objects = || count_files(&ws.join(".git"));
    let before = objects();
    let log = walk(&p, &rec, "s");
    eprintln!("{log}");
    assert!(log.contains("Admin { vm: \"fakevm\" } in workspace Some(\"s\")") && log.contains("2 files"), "{log}");
    assert_eq!(objects(), before, "nothing written into the repository");
    assert!(Command::new("git").args(["for-each-ref", "refs/pitwall/"]).current_dir(&ws).output().unwrap().stdout.is_empty());
}

#[test]
#[ignore = "live: read-only commands in the real agw session named by PITWALL_AGW_LIVE_EXEC (read-only)"]
fn live_agw_exec_read_only() {
    let Ok(name) = std::env::var("PITWALL_AGW_LIVE_EXEC") else { return };
    let rec = Arc::new(Recorder::default());
    let mut cfg = AgwConfig::new(std::env::temp_dir().join("pw-agw-live-exec-unused"), Err("no attach here".into()));
    cfg.exec = rec.clone();
    let p = AgwProvider::new(cfg);
    let log = walk(&p, &rec, &name);
    eprintln!("{log}");
    assert!(log.contains(&format!("/{name}: ")) && log.contains(" in workspace "), "{log}");
}

/// Files under `dir` (recursively), with their sizes.
fn count_files(dir: &Path) -> Vec<(String, u64)> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            v.extend(count_files(&p));
        } else {
            v.push((p.to_string_lossy().into_owned(), e.metadata().unwrap().len()));
        }
    }
    v.sort();
    v
}
