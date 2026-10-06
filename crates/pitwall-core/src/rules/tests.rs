//! Store/set tests always run. The end-to-end tests run the real rulesync CLI
//! and only run when PITWALL_TEST_RULESYNC points at a rulesync binary, e.g.
//! `PITWALL_TEST_RULESYNC=/tmp/rs/node_modules/.bin/rulesync cargo test rules`.
//! They only ever touch fresh temp directories.

use std::path::{Path, PathBuf};

use super::runner::Runner;
use super::*;
use crate::testing::FakeExec;

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("pw-rules-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        // Canonical path: git prints /private/var/... on macOS.
        Tmp(p.canonicalize().unwrap())
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sh(cwd: &Path, args: &[&str]) -> String {
    let mut all = vec!["-c", "user.email=t@t", "-c", "user.name=t"];
    all.extend_from_slice(args);
    Git::new(&LocalExec, &cwd.to_string_lossy()).run(&all).unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}

fn real_runner() -> Option<Runner> {
    let bin = std::env::var("PITWALL_TEST_RULESYNC").ok()?;
    Some(Runner { argv: vec![bin], path_env: std::env::var("PATH").ok(), via: "rulesync" })
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

#[test]
fn rule_sets_save_rename_and_reject_duplicates() {
    let t = Tmp::new("sets");
    let dirs = Dirs { base: t.0.clone() };
    let a = save_set(&dirs, SaveRuleSet { id: None, name: "Web".into(), rule_ids: vec!["library:a.md".into(), "library:a.md".into()] }).unwrap();
    assert_eq!(a.rule_ids, vec!["library:a.md"]);
    assert!(save_set(&dirs, SaveRuleSet { id: None, name: "web".into(), rule_ids: vec![] }).is_err());
    let b = save_set(&dirs, SaveRuleSet { id: Some(a.id.clone()), name: "Web 2".into(), rule_ids: vec![] }).unwrap();
    assert_eq!(b.id, a.id);
    assert_eq!(read(&dirs).sets, vec![b]);
    assert!(save_set(&dirs, SaveRuleSet { id: Some("gone".into()), name: "x".into(), rule_ids: vec![] }).is_err());
}

#[test]
fn main_checkout_needs_confirmation() {
    let t = Tmp::new("confirm");
    let dirs = Dirs { base: t.0.join("pw") };
    let ctx = AgentCtx {
        id: "a".into(),
        cwd: t.0.to_string_lossy().into_owned(),
        project: t.0.to_string_lossy().into_owned(),
        worktree: false,
        kind_name: "Claude Code".into(),
        target: Some("claudecode".into()),
    };
    let err = apply_with(&dirs, None, &LocalExec, &ctx, false).unwrap_err();
    assert!(err.contains("main checkout"), "{err}");
    // Confirmed but nothing assigned: succeeds without running rulesync.
    let ok = apply_with(&dirs, None, &LocalExec, &ctx, true).unwrap();
    assert!(ok.generated.is_empty());
    assert!(read(&dirs).agents["a"].main_checkout_confirmed);
}

/// A repo with tracked CLAUDE.md + AGENTS.md, a worktree, and a library with
/// a shared non-root rule, a root rule, and a personal rule.
struct Fixture {
    _t: Tmp,
    dirs: Dirs,
    repo: PathBuf,
    wt: PathBuf,
}

fn fixture() -> Fixture {
    let t = Tmp::new("e2e");
    let repo = t.0.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q"]);
    write(&repo.join("CLAUDE.md"), "# project claude\n");
    write(&repo.join("AGENTS.md"), "# project agents\n");
    // A project config that would wipe things if Pitwall let it apply.
    write(&repo.join("rulesync.jsonc"), "{ \"delete\": true, \"targets\": [\"*\"] }\n");
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);
    let wt = t.0.join("wt");
    sh(&repo, &["worktree", "add", "-q", "-b", "worktree-x", wt.to_str().unwrap(), "HEAD"]);

    let dirs = Dirs { base: t.0.join("pitwall") };
    let lib = dirs.library_rules();
    write(&lib.join("style.md"), "---\nroot: false\ntargets: [\"*\"]\ndescription: \"Style\"\nglobs: [\"**/*\"]\n---\nUse tabs.\n");
    write(&lib.join("base.md"), "---\nroot: true\ntargets: [\"*\"]\ndescription: \"Base\"\n---\n# Base\n\nBe careful.\n");
    write(&lib.join("mine.md"), "---\nroot: false\ntargets: [\"*\"]\ndescription: \"Personal\"\n---\nPersonal note.\n");
    Fixture { dirs, repo, wt, _t: t }
}

fn ctx(f: &Fixture, target: &str) -> AgentCtx {
    AgentCtx {
        id: "agent-1".into(),
        cwd: f.wt.to_string_lossy().into_owned(),
        project: f.repo.to_string_lossy().into_owned(),
        worktree: true,
        kind_name: "Claude Code".into(),
        target: Some(target.into()),
    }
}

fn assign(f: &Fixture, shared: &[&str], extra: &[&str]) {
    let ids = |v: &[&str]| v.iter().map(|s| format!("library:{s}")).collect::<Vec<_>>();
    let s = save_set(&f.dirs, SaveRuleSet { id: None, name: format!("p{}", uuid::Uuid::new_v4()), rule_ids: ids(shared) }).unwrap();
    let e = save_set(&f.dirs, SaveRuleSet { id: None, name: format!("e{}", uuid::Uuid::new_v4()), rule_ids: ids(extra) }).unwrap();
    update(&f.dirs, |st| {
        st.project_defaults.insert(f.repo.to_string_lossy().into_owned(), s.id.clone());
        st.agents.entry("agent-1".into()).or_default().rule_set_id = Some(e.id.clone());
        Ok(())
    })
    .unwrap();
}

#[test]
fn e2e_claude_worktree() {
    let Some(r) = real_runner() else { return };
    let f = fixture();
    assign(&f, &["style.md", "base.md"], &["mine.md"]);
    let c = ctx(&f, "claudecode");
    let res = apply_with(&f.dirs, Some(&r), &LocalExec, &c, false).unwrap();
    eprintln!("{res:?}");
    // CLAUDE.md is tracked: never touched. Shared rule → .claude/rules, own rule → CLAUDE.local.md.
    assert_eq!(std::fs::read_to_string(f.wt.join("CLAUDE.md")).unwrap(), "# project claude\n");
    assert!(res.log.contains("kept the project's CLAUDE.md"));
    assert_eq!(res.generated, vec![".claude/rules/style.md", "CLAUDE.local.md"]);
    assert!(std::fs::read_to_string(f.wt.join(".claude/rules/style.md")).unwrap().contains("Use tabs."));
    assert!(std::fs::read_to_string(f.wt.join("CLAUDE.local.md")).unwrap().contains("Personal note."));
    // The project's rulesync.jsonc (delete: true) did not apply: AGENTS.md still there.
    assert!(f.wt.join("AGENTS.md").exists());
    // Nothing shows up in git.
    assert_eq!(sh(&f.wt, &["status", "--porcelain"]), "");
    assert!(!stale(&f.dirs, &read(&f.dirs), &c));

    // Editing a library rule marks the agent stale; re-apply updates in place.
    write(&f.dirs.library_rules().join("style.md"), "---\nroot: false\n---\nUse spaces.\n");
    assert!(stale(&f.dirs, &read(&f.dirs), &c));
    let res = apply_with(&f.dirs, Some(&r), &LocalExec, &c, false).unwrap();
    assert!(std::fs::read_to_string(f.wt.join(".claude/rules/style.md")).unwrap().contains("Use spaces."));
    assert_eq!(res.generated.len(), 2);
    assert!(!stale(&f.dirs, &read(&f.dirs), &c));

    // Worktree removal still works with the (ignored) generated files present.
    sh(&f.repo, &["worktree", "remove", f.wt.to_str().unwrap()]);
}

#[test]
fn e2e_codex_and_cleanup() {
    let Some(r) = real_runner() else { return };
    let f = fixture();
    // Codex folds everything into AGENTS.md, which the project tracks.
    assign(&f, &["style.md"], &[]);
    let c = ctx(&f, "codexcli");
    let res = apply_with(&f.dirs, Some(&r), &LocalExec, &c, false).unwrap();
    assert!(res.generated.is_empty(), "{res:?}");
    assert!(res.log.contains("kept the project's AGENTS.md"));
    assert_eq!(std::fs::read_to_string(f.wt.join("AGENTS.md")).unwrap(), "# project agents\n");

    // Without the tracked AGENTS.md, Pitwall writes and excludes it.
    sh(&f.wt, &["rm", "-q", "AGENTS.md"]);
    sh(&f.wt, &["commit", "-qm", "drop agents"]);
    let res = apply_with(&f.dirs, Some(&r), &LocalExec, &c, false).unwrap();
    assert_eq!(res.generated, vec!["AGENTS.md"]);
    assert_eq!(sh(&f.wt, &["status", "--porcelain"]), "");
    let exclude = PathBuf::from(read(&f.dirs).agents["agent-1"].exclude_file.clone().unwrap());
    assert!(std::fs::read_to_string(&exclude).unwrap().contains("/AGENTS.md"));

    // Unassigning removes what Pitwall wrote and its exclude block.
    update(&f.dirs, |s| {
        s.project_defaults.clear();
        Ok(())
    })
    .unwrap();
    assert!(stale(&f.dirs, &read(&f.dirs), &c));
    let res = apply_with(&f.dirs, Some(&r), &LocalExec, &c, false).unwrap();
    assert!(res.generated.is_empty());
    assert!(!f.wt.join("AGENTS.md").exists());
    assert!(!std::fs::read_to_string(&exclude).unwrap().contains("pitwall rules agent-1"));
}

#[test]
fn e2e_project_rulesync_is_an_input_root() {
    let Some(r) = real_runner() else { return };
    let f = fixture();
    // The project uses rulesync itself (generated files untracked).
    write(&f.wt.join(".rulesync/rules/project.md"), "---\nroot: false\ntargets: [\"*\"]\n---\nProject rule.\n");
    assign(&f, &["style.md"], &[]);
    let res = apply_with(&f.dirs, Some(&r), &LocalExec, &ctx(&f, "claudecode"), false).unwrap();
    assert!(res.generated.contains(&".claude/rules/project.md".to_string()), "{res:?}");
    assert!(res.generated.contains(&".claude/rules/style.md".to_string()));
}

#[test]
fn e2e_import_file() {
    let Some(r) = real_runner() else { return };
    let t = Tmp::new("import");
    let dirs = Dirs { base: t.0.join("pitwall") };
    let proj = t.0.join("myproj");
    write(&proj.join("CLAUDE.md"), "# Mine\n\nAlways test.\n");
    let (added, _log) = library::import_file(&dirs, &r, proj.join("CLAUDE.md").to_str().unwrap()).unwrap();
    assert_eq!(added, vec!["myproj.md"]);
    let lib = library::list(&dirs, &[]);
    assert_eq!(lib.len(), 1);
    assert!(lib[0].root);
    // The source project is untouched (no .rulesync created there).
    assert!(!proj.join(".rulesync").exists());
    assert!(!dirs.run().read_dir().map(|mut d| d.next().is_some()).unwrap_or(false));
}

/// Unassigning cleans up in the agent's folder through its machine's exec:
/// only files Pitwall wrote and nobody edited, plus its exclude block.
#[test]
fn cleanup_goes_through_the_agents_machine() {
    let t = Tmp::new("fake-cleanup");
    let dirs = Dirs { base: t.0.join("pw") };
    let x = FakeExec::new();
    let exclude = "/w/.git/info/exclude";
    x.dir("/w")
        .file("/w/.claude/rules/x.md", b"gen")
        .file("/w/edited.md", b"changed by the user")
        .file(exclude, b"*.log\n# >>> pitwall rules a (generated, do not edit)\n/x.md\n# <<< pitwall rules a\n")
        .on(&["git", "-C", "/w", "ls-files", "-z", "--", ".."], "");
    update(&dirs, |s| {
        let st = s.agents.entry("a".into()).or_default();
        st.files = vec![
            GenFile { path: ".claude/rules/x.md".into(), hash: apply::content_hash(b"gen") },
            GenFile { path: "edited.md".into(), hash: apply::content_hash(b"original") },
        ];
        st.exclude_file = Some(exclude.into());
        Ok(())
    })
    .unwrap();
    let ctx = AgentCtx {
        id: "a".into(),
        cwd: "/w".into(),
        project: "/w".into(),
        worktree: true,
        kind_name: "Claude Code".into(),
        target: Some("claudecode".into()),
    };
    let res = apply_with(&dirs, None, &*x, &ctx, false).unwrap();
    assert!(res.generated.is_empty());
    assert!(res.log.contains("kept edited.md"), "{}", res.log);
    assert_eq!(x.contents("/w/.claude/rules/x.md"), None);
    assert!(x.contents("/w/edited.md").is_some());
    assert_eq!(x.contents(exclude).as_deref(), Some(&b"*.log\n"[..]));
    // Gone folders are reported, not created.
    let gone = AgentCtx { cwd: "/nowhere".into(), ..ctx };
    assert!(apply_with(&dirs, None, &*x, &gone, false).unwrap_err().contains("no longer exists"));
}
