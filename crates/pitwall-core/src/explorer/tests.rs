//! The explorer against real temp repositories (this machine) and scripted
//! remote machines ([`FakeExec`]).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pitwall_proto::{ContentKind, EntryKind, FileStatus, SearchEngine, SearchQuery};

use super::*;
use crate::error::Result as PwResult;
use crate::exec::{Cmd, DirEntry, Exec, LocalExec, Out, Stat};
use crate::testing::{record, FakeExec, Harness, TempDir};
use crate::vcs::snapshot::TempRepo;

fn names(l: &DirListing) -> Vec<(&str, EntryKind, Option<FileStatus>, u32)> {
    l.entries
        .iter()
        .map(|e| (e.name.as_str(), e.kind, e.status, e.changes))
        .collect()
}

fn q(text: &str) -> SearchQuery {
    SearchQuery {
        query: text.into(),
        ..Default::default()
    }
}

#[test]
fn paths_must_stay_relative() {
    for bad in [
        "/etc/passwd",
        "\\x",
        "C:\\Windows",
        "c:x",
        "..",
        "../x",
        "a/../../b",
        "a\\..\\b",
        "a/..",
        "x\0y",
    ] {
        assert!(clean(bad).is_err(), "{bad:?} accepted");
    }
    assert!(clean(&"a/".repeat(3000)).is_err(), "too long");
    assert_eq!(clean("").unwrap(), "");
    assert_eq!(clean("./src//a.rs/").unwrap(), "src/a.rs");
    assert_eq!(
        clean("a..b/.c").unwrap(),
        "a..b/.c",
        "dots in names are fine"
    );
    assert!(
        inside("/r", "/r")
            && inside("/r", "/r/a")
            && inside("/r/", "/r/a")
            && inside("C:\\r", "C:\\r\\a")
    );
    assert!(!inside("/r", "/rx") && !inside("/r", "/") && !inside("/r/a", "/r"));
    assert!(inside("/", "/anything"));
}

#[test]
fn gitignore_and_change_letters_in_a_repo() {
    let r = TempRepo::new();
    r.write(".gitignore", "target/\n*.log\n");
    r.write("README.md", "hi\n");
    r.write("src/a.rs", "fn a() {}\n");
    r.write("src/deep/b.rs", "fn b() {}\n");
    r.write("old.txt", "old\n");
    r.commit_all("init");
    let base = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    r.write("README.md", "hi there\n");
    r.write("target/debug/out.bin", "x");
    r.write("app.log", "noise");
    r.write("new.txt", "n\n");
    r.write("fresh/x.rs", "x\n");
    r.write("src/deep/c.rs", "c\n");
    std::fs::remove_file(r.0.join("old.txt")).unwrap();
    let mut rec = record("a", r.path());
    rec.base_commit = Some(base);
    let h = Harness::new(vec![rec]);

    let root = list_files(&h.engine, "a", "").unwrap();
    assert!(root.git && !root.truncated && root.dir.is_empty());
    assert_eq!(
        names(&root),
        [
            ("fresh", EntryKind::Dir, Some(FileStatus::U), 1),
            ("src", EntryKind::Dir, None, 1),
            (".gitignore", EntryKind::File, None, 0),
            ("new.txt", EntryKind::File, Some(FileStatus::U), 0),
            ("README.md", EntryKind::File, Some(FileStatus::M), 0),
        ],
        "ignored target/ and *.log are hidden, the deleted old.txt is gone"
    );
    let src = list_files(&h.engine, "a", "src").unwrap();
    assert_eq!(
        names(&src),
        [
            ("deep", EntryKind::Dir, None, 1),
            ("a.rs", EntryKind::File, None, 0)
        ]
    );
    let deep = list_files(&h.engine, "a", "src/deep/").unwrap();
    assert_eq!(
        deep.entries
            .iter()
            .map(|e| (e.path.as_str(), e.status))
            .collect::<Vec<_>>(),
        [
            ("src/deep/b.rs", None),
            ("src/deep/c.rs", Some(FileStatus::U))
        ]
    );
    let fresh = list_files(&h.engine, "a", "fresh").unwrap();
    assert_eq!(fresh.entries[0].path, "fresh/x.rs");
    assert!(list_files(&h.engine, "a", "nope")
        .unwrap_err()
        .contains("not found"));

    let index = list_all_files(&h.engine, "a").unwrap();
    assert!(index.git);
    assert_eq!(
        index.files,
        [
            ".gitignore",
            "README.md",
            "fresh/x.rs",
            "new.txt",
            "src/a.rs",
            "src/deep/b.rs",
            "src/deep/c.rs"
        ]
    );

    let f = read_file(&h.engine, "a", "README.md").unwrap();
    assert_eq!(
        (f.kind, f.text.as_deref(), f.lang.as_deref()),
        (ContentKind::Text, Some("hi there\n"), Some("markdown"))
    );
    // Ignored files aren't listed, but asking for one by name still reads it.
    assert_eq!(
        read_file(&h.engine, "a", "app.log")
            .unwrap()
            .text
            .as_deref(),
        Some("noise")
    );
    assert!(read_file(&h.engine, "a", "").is_err());
    assert!(read_file(&h.engine, "a", "src")
        .unwrap_err()
        .contains("is a folder"));
}

#[test]
fn traversal_and_symlink_escapes_are_refused() {
    let outside = TempDir::new("outside");
    std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
    let r = TempRepo::new();
    r.write("in.txt", "in\n");
    r.write("dir/x.txt", "x\n");
    let linked = crate::platform::symlink(outside.path(), &r.0.join("escape")).is_ok()
        && crate::platform::symlink(&outside.path().join("secret.txt"), &r.0.join("secret-link"))
            .is_ok()
        && crate::platform::symlink(&r.0.join("in.txt"), &r.0.join("ok-link")).is_ok();
    r.commit_all("init");
    let h = Harness::new(vec![record("a", r.path())]);

    for bad in ["../", "..", "/etc", "dir/../../x"] {
        assert!(
            list_files(&h.engine, "a", bad)
                .unwrap_err()
                .starts_with("invalid path"),
            "{bad}"
        );
        assert!(
            read_file(&h.engine, "a", bad)
                .unwrap_err()
                .starts_with("invalid path"),
            "{bad}"
        );
        assert!(
            open_in_editor(&h.engine, "a", bad, None, None).is_err(),
            "{bad}"
        );
    }
    let real_secret = outside
        .path()
        .join("secret.txt")
        .to_string_lossy()
        .into_owned();
    assert!(
        read_file(&h.engine, "a", &real_secret).is_err(),
        "absolute paths are refused"
    );
    if !linked {
        eprintln!("symlinks unavailable here: skipping the symlink cases");
        return;
    }
    let root = list_files(&h.engine, "a", "").unwrap();
    let kinds: Vec<_> = root
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.kind))
        .collect();
    assert!(
        kinds.contains(&("escape", EntryKind::Symlink))
            && kinds.contains(&("secret-link", EntryKind::Symlink))
    );
    assert!(list_files(&h.engine, "a", "escape")
        .unwrap_err()
        .contains("outside the agent's folder"));
    assert!(read_file(&h.engine, "a", "escape/secret.txt")
        .unwrap_err()
        .contains("outside the agent's folder"));
    assert!(read_file(&h.engine, "a", "secret-link")
        .unwrap_err()
        .contains("outside the agent's folder"));
    assert_eq!(
        read_file(&h.engine, "a", "ok-link")
            .unwrap()
            .text
            .as_deref(),
        Some("in\n"),
        "a link inside the folder is followed"
    );
    h.engine
        .with("a", |a| a.facts.provider.local_files = true)
        .unwrap();
    assert!(open_in_editor(&h.engine, "a", "secret-link", None, None)
        .unwrap_err()
        .contains("outside the agent's folder"));
}

#[test]
fn a_folder_outside_git_is_listed_plainly() {
    let dir = TempDir::new("plain");
    let root = dir.path().to_string_lossy().into_owned();
    for (p, body) in [
        ("b.txt", "hello plain\n"),
        ("A/c.txt", "c"),
        (".DS_Store", "x"),
        (".hidden", "h"),
        ("node_modules/m/i.js", "m"),
    ] {
        LocalExec
            .write_file(&exec::join(&root, p), body.as_bytes())
            .unwrap();
    }
    let h = Harness::new(vec![record("a", &root)]);
    let l = list_files(&h.engine, "a", "").unwrap();
    assert!(!l.git);
    assert_eq!(
        names(&l),
        [
            ("A", EntryKind::Dir, None, 0),
            ("node_modules", EntryKind::Dir, None, 0),
            (".hidden", EntryKind::File, None, 0),
            ("b.txt", EntryKind::File, None, 0),
        ],
        ".DS_Store is never shown"
    );
    let index = list_all_files(&h.engine, "a").unwrap();
    assert!(
        index.files.contains(&"A/c.txt".to_string()) && index.files.contains(&"b.txt".to_string())
    );
    assert!(
        !index.files.iter().any(|f| f.starts_with("node_modules/")),
        "not descended into"
    );
    // Search works only with ripgrep here (no repository).
    match search(&h.engine, "a", &q("plain")) {
        Ok(r) => assert_eq!(
            (r.engine, r.matches[0].path.as_str()),
            (SearchEngine::Ripgrep, "b.txt")
        ),
        Err(e) => assert!(e.contains("needs ripgrep"), "{e}"),
    }
}

#[test]
fn searching_a_real_repo() {
    let r = TempRepo::new();
    r.write("a.txt", "one Needle\ntwo\nneedle three\n");
    r.write("src/b.rs", "// NEEDLE\n");
    r.write("node_modules/x.js", "needle\n");
    r.commit_all("init");
    r.write("untracked.md", "needle here\n");
    let h = Harness::new(vec![record("a", r.path())]);
    let res = search(&h.engine, "a", &q("needle")).unwrap();
    let mut hits: Vec<_> = res
        .matches
        .iter()
        .map(|m| (m.path.as_str(), m.line))
        .collect();
    hits.sort();
    assert_eq!(
        hits,
        [
            ("a.txt", 1),
            ("a.txt", 3),
            ("src/b.rs", 1),
            ("untracked.md", 1)
        ],
        "{:?}",
        res.engine
    );
    let exact = search(
        &h.engine,
        "a",
        &SearchQuery {
            case_sensitive: true,
            include: vec!["*.txt".into()],
            ..q("needle")
        },
    )
    .unwrap();
    assert_eq!(
        exact
            .matches
            .iter()
            .map(|m| (m.path.as_str(), m.line))
            .collect::<Vec<_>>(),
        [("a.txt", 3)]
    );
    let m = &res.matches.iter().find(|m| m.path == "a.txt").unwrap();
    assert_eq!((m.column, m.ranges[0].start, m.ranges[0].end), (5, 4, 10));
}

/// A remote machine: git answers are scripted, and each `run_all` (one
/// round trip on agw) is counted.
struct Remote {
    x: Arc<FakeExec>,
    trips: AtomicUsize,
}

impl Exec for Remote {
    fn run(&self, cmd: &Cmd) -> PwResult<Out> {
        self.trips.fetch_add(1, Ordering::Relaxed);
        self.x.run(cmd)
    }
    fn run_all(&self, cmds: &[Cmd]) -> Vec<PwResult<Out>> {
        self.trips.fetch_add(1, Ordering::Relaxed);
        cmds.iter().map(|c| self.x.run(c)).collect()
    }
    fn read_file(&self, path: &str, max: u64) -> PwResult<Option<Vec<u8>>> {
        self.trips.fetch_add(1, Ordering::Relaxed);
        self.x.read_file(path, max)
    }
    fn write_file(&self, path: &str, bytes: &[u8]) -> PwResult<()> {
        self.x.write_file(path, bytes)
    }
    fn remove_file(&self, path: &str) -> PwResult<()> {
        self.x.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> PwResult<()> {
        self.x.remove_dir(path)
    }
    fn copy_file(&self, from: &str, to: &str) -> PwResult<()> {
        self.x.copy_file(from, to)
    }
    fn stat(&self, path: &str) -> PwResult<Option<Stat>> {
        self.trips.fetch_add(1, Ordering::Relaxed);
        self.x.stat(path)
    }
    fn real_path(&self, path: &str) -> PwResult<String> {
        self.trips.fetch_add(1, Ordering::Relaxed);
        self.x.real_path(path)
    }
    fn temp_dir(&self) -> PwResult<String> {
        self.x.temp_dir()
    }
    fn home(&self) -> PwResult<String> {
        self.x.home()
    }
    fn list_dir(&self, path: &str) -> PwResult<Vec<DirEntry>> {
        self.trips.fetch_add(1, Ordering::Relaxed);
        self.x.list_dir(path)
    }
}

fn remote() -> (Arc<FakeExec>, Arc<Remote>, Harness) {
    let x = FakeExec::new();
    x.dir("/remote/w/src")
        .file("/remote/w/src/a.rs", b"fn a() {}\n")
        .file("/remote/w/README.md", b"hi\n")
        .link("/remote/w/out", "/etc");
    x.file("/etc/passwd", b"root");
    let r = Arc::new(Remote {
        x: x.clone(),
        trips: AtomicUsize::new(0),
    });
    let mut rec = record("a", "/remote/w");
    rec.base_commit = Some("b0".into());
    let h = Harness::with_exec(vec![rec], r.clone());
    (x, r, h)
}

#[test]
fn a_remote_folder_in_one_round_trip() {
    let (x, r, h) = remote();
    x.on(
        &["git", "-C", "/remote/w", "ls-files", "-z", "--stage"],
        "100644 aa 0\tREADME.md\x00100644 bb 0\tsrc/a.rs\0",
    )
    .on(
        &["git", "-C", "/remote/w", "ls-files", "-z", "--others", ".."],
        "notes.txt\0",
    )
    .on(
        &[
            "git",
            "-C",
            "/remote/w",
            "diff",
            "--raw",
            "--numstat",
            "-z",
            "-M",
            "--relative",
            "b0",
        ],
        ":100644 100644 a b M\0README.md\x001\t0\tREADME.md\0",
    );
    let l = list_files(&h.engine, "a", "").unwrap();
    assert_eq!(
        names(&l),
        [
            ("src", EntryKind::Dir, None, 0),
            ("notes.txt", EntryKind::File, Some(FileStatus::U), 0),
            ("README.md", EntryKind::File, Some(FileStatus::M), 0),
        ]
    );
    // The root resolved once (and remembered), then one batch.
    assert_eq!(r.trips.load(Ordering::Relaxed), 2);
    assert!(x
        .calls()
        .iter()
        .all(|c| c.env_has("GIT_OPTIONAL_LOCKS", "0")));
    list_files(&h.engine, "a", "").unwrap();
    assert_eq!(
        r.trips.load(Ordering::Relaxed),
        3,
        "the root isn't resolved again"
    );

    x.on(
        &["git", "-C", "/remote/w/src", "ls-files", "-z", "--stage"],
        "100644 bb 0\ta.rs\0",
    )
    .on(
        &[
            "git",
            "-C",
            "/remote/w/src",
            "ls-files",
            "-z",
            "--others",
            "..",
        ],
        "",
    )
    .on(&["git", "-C", "/remote/w/src", "diff", ".."], "");
    let src = list_files(&h.engine, "a", "src").unwrap();
    assert_eq!(src.entries[0].path, "src/a.rs");
    assert_eq!(
        r.trips.load(Ordering::Relaxed),
        5,
        "resolve the folder, then one batch"
    );

    let before = r.trips.load(Ordering::Relaxed);
    assert_eq!(
        read_file(&h.engine, "a", "src/a.rs")
            .unwrap()
            .text
            .as_deref(),
        Some("fn a() {}\n")
    );
    assert_eq!(
        r.trips.load(Ordering::Relaxed) - before,
        2,
        "resolve + read"
    );
    assert!(list_files(&h.engine, "a", "out")
        .unwrap_err()
        .contains("outside"));
    assert!(read_file(&h.engine, "a", "out/passwd")
        .unwrap_err()
        .contains("outside"));
    // Remote files can't go to a local editor.
    assert!(open_in_editor(&h.engine, "a", "README.md", None, None)
        .unwrap_err()
        .contains("another machine"));
    let v = &h.engine.views()[0];
    assert!(v.caps.explorer && !v.caps.open_in_editor);
}

#[test]
fn a_remote_folder_outside_git() {
    let (x, _r, h) = remote();
    x.on_exit(
        &["git", ".."],
        128,
        "",
        "fatal: not a git repository (or any of the parent directories): .git",
    );
    let l = list_files(&h.engine, "a", "").unwrap();
    assert!(!l.git);
    assert_eq!(
        names(&l),
        [
            ("src", EntryKind::Dir, None, 0),
            ("out", EntryKind::Symlink, None, 0),
            ("README.md", EntryKind::File, None, 0),
        ]
    );
}

#[test]
fn search_prefers_ripgrep_and_remembers_when_it_is_missing() {
    let (x, r, h) = remote();
    let rg_out = r#"{"type":"match","data":{"path":{"text":"./src/a.rs"},"lines":{"text":"fn a() {}\n"},"line_number":1,"submatches":[{"match":{"text":"a"},"start":3,"end":4}]}}"#;
    x.on(&["rg", ".."], rg_out);
    let res = search(&h.engine, "a", &q("a")).unwrap();
    assert_eq!(
        (
            res.engine,
            res.matches[0].path.as_str(),
            res.matches[0].column
        ),
        (SearchEngine::Ripgrep, "src/a.rs", 4)
    );
    let call = x.calls().into_iter().find(|c| c.argv[0] == "rg").unwrap();
    assert_eq!(call.cwd.as_deref(), Some("/remote/w"));
    assert_eq!(&call.argv[call.argv.len() - 4..], ["-e", "a", "--", "."]);

    // Another machine without ripgrep: git grep, and rg isn't tried again.
    let (x, r2, h) = remote();
    x.on_error(&["rg", ".."], "rg: No such file or directory");
    x.on(
        &["git", "-C", "/remote/w", "grep", ".."],
        "src/a.rs\x001\x004\x00fn a() {}\n",
    );
    let res = search(&h.engine, "a", &q("a")).unwrap();
    assert_eq!((res.engine, res.matches.len()), (SearchEngine::GitGrep, 1));
    search(&h.engine, "a", &q("a")).unwrap();
    assert_eq!(x.ran(&["rg", ".."]), 1, "probed once per machine");
    assert_eq!(x.ran(&["git", "-C", "/remote/w", "grep", ".."]), 2);
    assert!(r2.trips.load(Ordering::Relaxed) >= 3);
    let _ = r;

    // Neither ripgrep nor a repository.
    let (x, _, h) = remote();
    x.on_error(&["rg", ".."], "rg: No such file or directory");
    h.engine.with("a", |a| a.git_repo = Some(false)).unwrap();
    assert!(search(&h.engine, "a", &q("a"))
        .unwrap_err()
        .starts_with("Search needs ripgrep (rg) or a git repository"));
    assert!(
        search(&h.engine, "a", &q("")).unwrap().matches.is_empty(),
        "nothing to look for"
    );
}

/// A machine whose ripgrep runs until it is cancelled.
struct Slow;

impl Exec for Slow {
    fn run(&self, cmd: &Cmd) -> PwResult<Out> {
        for _ in 0..1000 {
            if cmd.cancelled() {
                return Err(crate::error::PwError::other("rg: cancelled"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(Out::default())
    }
    fn read_file(&self, _: &str, _: u64) -> PwResult<Option<Vec<u8>>> {
        Ok(None)
    }
    fn write_file(&self, _: &str, _: &[u8]) -> PwResult<()> {
        Ok(())
    }
    fn remove_file(&self, _: &str) -> PwResult<()> {
        Ok(())
    }
    fn remove_dir(&self, _: &str) -> PwResult<()> {
        Ok(())
    }
    fn copy_file(&self, _: &str, _: &str) -> PwResult<()> {
        Ok(())
    }
    fn stat(&self, _: &str) -> PwResult<Option<Stat>> {
        Ok(None)
    }
    fn real_path(&self, path: &str) -> PwResult<String> {
        Ok(path.to_string())
    }
    fn temp_dir(&self) -> PwResult<String> {
        Ok("/tmp".into())
    }
    fn home(&self) -> PwResult<String> {
        Ok("/home".into())
    }
}

#[test]
fn a_search_can_be_cancelled_or_replaced() {
    let h = Harness::with_exec(vec![record("a", "/w")], Arc::new(Slow));
    let started = std::time::Instant::now();
    let first = {
        let e = h.engine.clone();
        std::thread::spawn(move || search(&e, "a", &q("x")))
    };
    while lock(&h.engine.explorer.searches).is_empty() {
        std::thread::sleep(Duration::from_millis(1));
    }
    cancel_search(&h.engine, "a");
    assert_eq!(first.join().unwrap().unwrap_err(), "cancelled");
    assert!(started.elapsed() < Duration::from_secs(4));
    assert!(
        lock(&h.engine.explorer.rg).is_empty(),
        "a cancelled probe teaches nothing"
    );

    // A new search replaces the running one.
    let first = {
        let e = h.engine.clone();
        std::thread::spawn(move || search(&e, "a", &q("x")))
    };
    while lock(&h.engine.explorer.searches).is_empty() {
        std::thread::sleep(Duration::from_millis(1));
    }
    let second = {
        let e = h.engine.clone();
        std::thread::spawn(move || search(&e, "a", &q("y")))
    };
    assert_eq!(first.join().unwrap().unwrap_err(), "cancelled");
    cancel_search(&h.engine, "a");
    assert_eq!(second.join().unwrap().unwrap_err(), "cancelled");
    cancel_search(&h.engine, "nobody");
}

#[test]
fn caps_follow_the_provider() {
    let h = Harness::with_exec(vec![record("a", "/w")], FakeExec::new());
    let v = &h.engine.views()[0];
    assert!(
        v.caps.explorer && !v.caps.open_in_editor,
        "fake files aren't on this computer"
    );
    h.engine
        .with("a", |a| a.facts.provider.local_files = true)
        .unwrap();
    assert!(h.engine.views()[0].caps.open_in_editor);
    h.engine
        .with("a", |a| a.facts.provider.exec = false)
        .unwrap();
    let v = &h.engine.views()[0];
    assert!(!v.caps.explorer && !v.caps.open_in_editor);
    assert!(list_files(&h.engine, "a", "")
        .unwrap_err()
        .contains("can't read files"));
    assert!(list_files(&h.engine, "ghost", "").is_err());
}

#[test]
fn open_in_editor_runs_the_users_command_detached() {
    if crate::platform::which("touch").is_none() {
        eprintln!("no touch here: skipping");
        return;
    }
    let r = TempRepo::new();
    r.write("a b.txt", "x\n");
    let h = Harness::new(vec![record("a", r.path())]);
    h.engine
        .with("a", |a| a.facts.provider.local_files = true)
        .unwrap();
    set_editor(&h.engine, Some("touch {path}.opened-{line}")).unwrap();
    open_in_editor(&h.engine, "a", "a b.txt", Some(7), None).unwrap();
    let made = r.0.join("a b.txt.opened-7");
    for _ in 0..500 {
        if made.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        made.exists(),
        "the editor got the file's path as one argument"
    );
    set_editor(&h.engine, Some("no-such-editor-pitwall {path}")).unwrap();
    assert!(open_in_editor(&h.engine, "a", "a b.txt", None, None)
        .unwrap_err()
        .contains("not found"));
    assert_eq!(
        editor_settings(&h.engine).command.as_deref(),
        Some("no-such-editor-pitwall {path}")
    );
}
