//! [`AgwExec`] against the fake agw in `tests/fake_agw/` (which, like agw,
//! runs the remote command with its output, errors and exit status passed
//! through): the same checks as `LocalExec`'s, and git, changes and task
//! snapshots through it give exactly what they give on this Mac — in as few
//! agw calls as promised.
//!
//! Safety: only `/bin/sh`, git and coreutils run, in temp folders. The real
//! agw is never run here (see `live_agw_exec.rs`).

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pitwall_core::exec::{self, Cmd, Exec, FileKind, LocalExec, Stat};
use pitwall_core::testing::TempDir;
use pitwall_core::vcs::git::Git;
use pitwall_core::vcs::snapshot;
use pitwall_providers::agw::{AgwExec, User};

/// A fake agw with one VM (`fakevm`) and one workspace (`ws`).
struct Fake {
    dir: TempDir,
    agw: String,
    ws: String,
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

impl Fake {
    fn new() -> Fake {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("agw-exec");
        let state = dir.path().join("agw");
        std::fs::create_dir_all(&state).unwrap();
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let ws = LocalExec.real_path(&ws.to_string_lossy()).unwrap();
        let body = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_agw/agw")).unwrap();
        let agw = state.join("agw");
        std::fs::write(&agw, body.replace("__DIR__", &state.to_string_lossy())).unwrap();
        std::fs::set_permissions(&agw, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(state.join("ws.ws"), &ws).unwrap();
        Fake { agw: agw.to_string_lossy().into_owned(), ws, dir }
    }

    fn exec(&self, workspace: Option<&str>) -> AgwExec {
        AgwExec::new(&self.agw, Arc::new(LocalExec), User::Admin { vm: "fakevm".into() }, workspace)
    }

    /// The agw exec calls so far: `<vm|agent> <name> <workspace|-> <command…>`.
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("agw/exec.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with("vm ") || l.starts_with("agent "))
            .map(String::from)
            .collect()
    }

    /// A repo in the workspace: committed, modified, deleted, untracked
    /// (text, binary, odd names), ignored.
    fn repo(&self) {
        let p = Path::new(&self.ws);
        git_in(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(p.join("gone.txt"), "x\n").unwrap();
        std::fs::write(p.join(".gitignore"), "ignored.log\n").unwrap();
        git_in(p, &["add", "."]);
        git_in(p, &["-c", "user.name=pw", "-c", "user.email=pw@example.invalid", "commit", "-q", "-m", "init"]);
        std::fs::write(p.join("a.txt"), "one\n2\nthree\n").unwrap();
        std::fs::remove_file(p.join("gone.txt")).unwrap();
        std::fs::write(p.join("new file's.txt"), "a\nb\nc").unwrap();
        std::fs::write(p.join("blob.bin"), [0u8, 1, 2, 255]).unwrap();
        std::fs::write(p.join("ignored.log"), "noise\n").unwrap();
    }
}

#[test]
fn runs_with_args_cwd_env_and_stdin_like_local_exec() {
    let f = Fake::new();
    let x = f.exec(None);
    let out = x
        .run(&Cmd::new(&["sh", "-c", "pwd; printf %s \"$PW_X\"; cat; echo oops >&2; exit 3"])
            .cwd(&f.ws)
            .env(&[("PW_X", "x=1 'q' $HOME")])
            .stdin(b"in\n"))
        .unwrap();
    assert_eq!(out.status, 3);
    assert_eq!(out.stdout_text(), format!("{}\nx=1 'q' $HOMEin\n", f.ws));
    assert_eq!(out.stderr_text(), "oops", "the command's own stderr, nothing of agw's");
    // Arguments arrive exactly, whatever is in them.
    let odd = ["a b", "it's", "$HOME", "", "*", "x;y", "-n", "\"q\"", "tab\there", "nl\nx"];
    let mut argv = vec!["printf", "[%s]"];
    argv.extend(odd);
    let out = x.run(&Cmd::new(&argv)).unwrap();
    assert_eq!(out.stdout_text(), odd.iter().map(|a| format!("[{a}]")).collect::<String>());
    // Couldn't run: an error, like LocalExec's.
    assert!(x.run(&Cmd::new(&[])).is_err());
    let e = x.run(&Cmd::new(&["/no/such/program"])).unwrap_err();
    assert!(e.message.starts_with("/no/such/program: "), "{e:?}");
    assert!(x.run(&Cmd::new(&["true"]).cwd("/no/such/dir")).unwrap_err().message.starts_with("/no/such/dir: "));
    // A signal: no exit code of its own (LocalExec: -1; through agw's ssh: 255 or 128+n).
    assert_ne!(x.run(&Cmd::new(&["sh", "-c", "kill -9 $$"])).unwrap().status, 0);
    // Options before the VM name, `--` before the command, one call each.
    let calls = f.calls();
    assert!(calls.iter().all(|c| c.starts_with("vm fakevm - sh -c ")), "{calls:#?}");
}

#[test]
fn the_workspace_is_where_commands_start() {
    let f = Fake::new();
    let out = f.exec(Some("ws")).run(&Cmd::new(&["pwd"])).unwrap();
    assert_eq!(out.stdout_text().trim(), f.ws);
    assert!(f.calls()[0].starts_with("vm fakevm ws sh -c "));
    let e = f.exec(Some("nope")).run(&Cmd::new(&["pwd"])).unwrap_err();
    assert_eq!(e.message, "agw exec on fakevm: workspace 'nope' not found", "agw's own error, not the command's");
    let e = AgwExec::new(&f.agw, Arc::new(LocalExec), User::Admin { vm: "other".into() }, None).home().unwrap_err();
    assert!(e.is(pitwall_core::provider::ErrorCode::Unreachable) && e.message.contains("VM 'other' not found"), "{e:?}");
}

#[test]
fn a_command_that_hangs_times_out() {
    let f = Fake::new();
    let x = f.exec(None).with_slack(Duration::ZERO);
    let started = Instant::now();
    let e = x.run(&Cmd::new(&["sleep", "5"]).timeout(Duration::from_millis(300))).unwrap_err();
    assert!(e.message.contains("timed out"), "{e:?}");
    assert!(started.elapsed() < Duration::from_secs(4));
}

#[test]
fn files_round_trip_like_local_exec() {
    let f = Fake::new();
    let x = f.exec(None);
    let root = f.ws.clone();
    let p = exec::join(&root, "a/b/c.txt");
    assert_eq!(x.read_file(&p, 10).unwrap(), None);
    assert_eq!(x.stat(&p).unwrap(), None);
    x.write_file(&p, b"hello world").unwrap();
    assert_eq!(x.read_file(&p, 100).unwrap().as_deref(), Some(&b"hello world"[..]));
    assert_eq!(x.read_file(&p, 5).unwrap().as_deref(), Some(&b"hello"[..]), "cut at max");
    assert_eq!(x.stat(&p).unwrap(), Some(Stat { kind: FileKind::File, len: 11 }));
    assert!(exec::is_dir(&x, &exec::join(&root, "a/b")) && !exec::is_dir(&x, &p));
    let g = exec::join(&root, "copy it's.txt");
    x.copy_file(&p, &g).unwrap();
    assert!(exec::exists(&x, &g));
    assert!(x.remove_dir(&exec::join(&root, "a/b")).is_err(), "not empty");
    assert!(x.remove_file(&exec::join(&root, "a")).is_err(), "a folder is not a file");
    x.remove_file(&p).unwrap();
    assert!(x.remove_file(&p).is_err(), "already gone");
    x.remove_dir(&exec::join(&root, "a/b")).unwrap();
    assert!(!exec::exists(&x, &exec::join(&root, "a/b")));
    assert!(x.real_path(&exec::join(&root, "gone")).is_err());
    assert_eq!(x.real_path(&format!("{root}/./a/..")).unwrap(), root);
    std::os::unix::fs::symlink(&g, Path::new(&root).join("link")).unwrap();
    assert_eq!(x.stat(&exec::join(&root, "link")).unwrap().map(|s| s.kind), Some(FileKind::Symlink));
    assert_eq!(x.real_path(&exec::join(&root, "link")).unwrap(), g);
    assert!(x.read_file(&root, 10).is_err(), "a folder can't be read");
    // Home and temp are asked once.
    let before = f.calls().len();
    let (home, tmp) = (x.home().unwrap(), x.temp_dir().unwrap());
    assert!(!home.is_empty() && !tmp.is_empty() && !tmp.ends_with('/'));
    assert_eq!((x.home().unwrap(), x.temp_dir().unwrap()), (home, tmp));
    assert_eq!(f.calls().len(), before + 1);
}

#[test]
fn binary_files_and_batches_survive_byte_for_byte() {
    let f = Fake::new();
    let x = f.exec(None);
    let bin: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
    let p = exec::join(&f.ws, "bin.dat");
    x.write_file(&p, &bin).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), bin, "written exactly");
    assert_eq!(x.read_file(&p, 1_000_000).unwrap().unwrap(), bin, "read back exactly");
    let before = f.calls().len();
    let missing = exec::join(&f.ws, "missing");
    let got = x.read_files(&[&p, &missing, &p], 3);
    assert_eq!(f.calls().len(), before + 1, "one agw call for every file");
    assert_eq!(got[0].as_ref().unwrap().as_deref(), Some(&bin[..3]));
    assert_eq!(got[1].as_ref().unwrap(), &None);
    assert_eq!(got[2].as_ref().unwrap().as_deref(), Some(&bin[..3]));
    let cmds = [
        Cmd::new(&["printf", "\\000\\377"]),
        Cmd::new(&["sh", "-c", "echo e >&2; exit 4"]),
        Cmd::new(&["/no/such"]),
        Cmd::new(&["cat"]).stdin(b"fed"),
    ];
    let before = f.calls().len();
    let outs = x.run_all(&cmds);
    assert_eq!(f.calls().len(), before + 1, "one agw call for the batch");
    assert_eq!(outs[0].as_ref().unwrap().stdout, [0u8, 255]);
    let o = outs[1].as_ref().unwrap();
    assert_eq!((o.status, o.stderr_text()), (4, "e".to_string()));
    assert!(outs[2].is_err());
    assert_eq!(outs[3].as_ref().unwrap().stdout_text(), "fed");
}

#[test]
fn changes_and_snapshots_match_this_mac_in_two_calls() {
    let f = Fake::new();
    f.repo();
    let x = f.exec(Some("ws"));
    let remote = Git::new(&x, &f.ws);
    let local = Git::new(&LocalExec, &f.ws);
    let before = f.calls().len();
    let (changes, branch) = remote.changes_and_branch(None);
    assert_eq!(f.calls().len() - before, 2, "git commands in one call, the untracked files in one more");
    let changes = changes.unwrap();
    assert_eq!(changes, local.changes(None).unwrap());
    assert_eq!(branch.as_deref(), Some("main"));
    let names: Vec<_> = changes.iter().map(|c| (c.path.as_str(), c.added, c.removed, c.untracked, c.binary)).collect();
    assert_eq!(
        names,
        [
            ("a.txt", 2, 1, false, false),
            ("blob.bin", 0, 0, true, true),
            ("gone.txt", 0, 1, false, false),
            ("new file's.txt", 3, 0, true, false),
        ]
    );
    // Not a repository: the same calm answer.
    let outside = Git::new(&x, "/");
    assert_eq!(outside.changes(None).unwrap_err(), pitwall_core::vcs::git::NOT_A_REPO);

    // A task snapshot through agw: the same tree as here, the real index
    // and the working tree untouched.
    let status = || git_in(Path::new(&f.ws), &["status", "--porcelain=v1", "-uall"]);
    let index = || std::fs::read(Path::new(&f.ws).join(".git/index")).unwrap();
    let (s0, i0) = (status(), index());
    let tree = snapshot::snapshot(&remote).unwrap();
    assert_eq!(tree, snapshot::snapshot(&local).unwrap());
    assert_eq!((status(), index()), (s0, i0), "nothing touched");
    // Kept alive by a ref, and dropped again.
    snapshot::keep(&remote, "agent", "task", Some(&tree), None).unwrap();
    assert!(git_in(Path::new(&f.ws), &["for-each-ref", "refs/pitwall/"]).contains("refs/pitwall/agent/task"));
    snapshot::drop_refs(&remote, "agent", None).unwrap();
    assert!(git_in(Path::new(&f.ws), &["for-each-ref", "refs/pitwall/"]).is_empty());
}
