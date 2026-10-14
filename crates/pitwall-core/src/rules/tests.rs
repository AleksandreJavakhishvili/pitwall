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
        fallback: None,
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
        fallback: None,
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
    assign(&f, &["style.md"], &["mine.md"]);
    let res = apply_with(&f.dirs, Some(&r), &LocalExec, &ctx(&f, "claudecode"), false).unwrap();
    assert!(res.generated.contains(&".claude/rules/project.md".to_string()), "{res:?}");
    assert!(res.generated.contains(&".claude/rules/mine.md".to_string()), "the agent's own set");
    // The project has rules of its own: the old default set is only a fallback.
    assert!(!res.generated.contains(&".claude/rules/style.md".to_string()));
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
        fallback: None,
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

/// A built-in kind's fallback (so the shipped definitions are what's tested).
fn builtin_fallback(id: &str) -> crate::kind::RulesFallback {
    let kinds = crate::kind::load_kinds(Path::new("/nonexistent"));
    let kind = kinds.iter().find(|k| k.id == id).unwrap();
    assert!(target_for(kind).is_none(), "{id} has no rulesync target");
    fallback_for(kind).unwrap()
}

fn fallback_ctx(f: &Fixture, id: &str, name: &str) -> AgentCtx {
    AgentCtx { kind_name: name.into(), target: None, fallback: Some(builtin_fallback(id)), ..ctx(f, "unused") }
}

/// Gemini CLI: no rulesync needed. Its rules go into GEMINI.md, filtered by
/// `targets`, frontmatter dropped, kept out of git, and refreshed when stale.
#[test]
fn gemini_gets_a_gemini_md_written_by_pitwall() {
    let f = fixture();
    write(&f.dirs.library_rules().join("claude-only.md"), "---\ntargets: [\"claudecode\"]\n---\nOnly for Claude.\n");
    write(&f.dirs.library_rules().join("gem.md"), "---\ntargets: [\"geminicli\"]\n---\nFor Gemini.\n");
    assign(&f, &["style.md", "claude-only.md", "gem.md"], &["mine.md"]);
    let c = fallback_ctx(&f, "gemini", "Gemini CLI");
    let res = apply_with(&f.dirs, None, &LocalExec, &c, false).unwrap();
    assert_eq!(res.generated, vec!["GEMINI.md"], "{}", res.log);
    let text = std::fs::read_to_string(f.wt.join("GEMINI.md")).unwrap();
    assert!(text.starts_with(apply::FALLBACK_HEADER), "{text}");
    assert!(text.contains("Use tabs.\n\nFor Gemini.\n\nPersonal note.\n"), "{text}");
    assert!(!text.contains("Only for Claude") && !text.contains("root: false"), "{text}");
    assert_eq!(sh(&f.wt, &["status", "--porcelain"]), "");
    assert!(read(&f.dirs).agents["agent-1"].launch_args.is_empty(), "Gemini reads GEMINI.md by itself");
    assert!(!stale(&f.dirs, &read(&f.dirs), &c));

    write(&f.dirs.library_rules().join("style.md"), "---\ntargets: [\"*\"]\n---\nUse spaces.\n");
    assert!(stale(&f.dirs, &read(&f.dirs), &c));
    apply_with(&f.dirs, None, &LocalExec, &c, false).unwrap();
    assert!(std::fs::read_to_string(f.wt.join("GEMINI.md")).unwrap().contains("Use spaces."));
    assert!(!stale(&f.dirs, &read(&f.dirs), &c));
}

/// A project that tracks its own GEMINI.md keeps it: the rules go into
/// Pitwall's file, and fresh starts ask Gemini to read it.
#[test]
fn gemini_with_a_tracked_gemini_md_is_pointed_at_pitwalls_file() {
    let f = fixture();
    write(&f.wt.join("GEMINI.md"), "# project gemini\n");
    sh(&f.wt, &["add", "GEMINI.md"]);
    sh(&f.wt, &["commit", "-qm", "gemini"]);
    assign(&f, &["style.md"], &[]);
    let c = fallback_ctx(&f, "gemini", "Gemini CLI");
    let res = apply_with(&f.dirs, None, &LocalExec, &c, false).unwrap();
    assert_eq!(res.generated, vec![".gemini/pitwall-rules.md"]);
    assert!(res.log.contains("kept the project's GEMINI.md"), "{}", res.log);
    assert_eq!(std::fs::read_to_string(f.wt.join("GEMINI.md")).unwrap(), "# project gemini\n");
    let file = f.wt.join(".gemini/pitwall-rules.md").to_string_lossy().into_owned();
    let args = launch_args(&f.dirs, "agent-1");
    assert_eq!(args[0], "--prompt-interactive");
    assert!(args[1].starts_with(&format!("Read {file} first")), "{args:?}");
    assert_eq!(sh(&f.wt, &["status", "--porcelain"]), "");
}

/// Aider: a file of Pitwall's, passed with --read; unassigning removes it
/// and the flag.
#[test]
fn aider_reads_pitwalls_file_until_rules_are_unassigned() {
    let f = fixture();
    assign(&f, &["style.md"], &[]);
    let c = fallback_ctx(&f, "aider", "Aider");
    let res = apply_with(&f.dirs, None, &LocalExec, &c, false).unwrap();
    assert_eq!(res.generated, vec![".aider.pitwall-rules.md"]);
    let file = f.wt.join(".aider.pitwall-rules.md");
    assert_eq!(launch_args(&f.dirs, "agent-1"), vec!["--read".to_string(), file.to_string_lossy().into_owned()]);
    assert_eq!(sh(&f.wt, &["status", "--porcelain"]), "");

    update(&f.dirs, |s| {
        s.project_defaults.clear();
        s.agents.get_mut("agent-1").unwrap().rule_set_id = None;
        Ok(())
    })
    .unwrap();
    assert!(stale(&f.dirs, &read(&f.dirs), &c));
    apply_with(&f.dirs, None, &LocalExec, &c, false).unwrap();
    assert!(!file.exists());
    assert!(launch_args(&f.dirs, "agent-1").is_empty());
}

/// The main checkout still needs the user's OK, fallback or not.
#[test]
fn fallback_respects_the_main_checkout_confirmation() {
    let f = fixture();
    assign(&f, &["style.md"], &[]);
    let c = AgentCtx { worktree: false, cwd: f.repo.to_string_lossy().into_owned(), ..fallback_ctx(&f, "gemini", "Gemini CLI") };
    assert!(apply_with(&f.dirs, None, &LocalExec, &c, false).unwrap_err().contains("main checkout"));
    assert!(!f.repo.join("GEMINI.md").exists());
}

/// Bulk re-apply: only agents whose rules changed, scoped to a project or
/// a set; a main checkout nobody confirmed is left alone.
#[test]
fn stale_agents_are_reapplied_in_bulk() {
    use crate::engine::lifecycle::create;
    use crate::model::CreateAgentRequest;
    let h = crate::testing::Harness::new(vec![]);
    let d = Dirs::of(h.engine.paths());
    write(&d.library_rules().join("a.md"), "Rule A.\n");
    write(&d.library_rules().join("b.md"), "Rule B.\n");
    let set = |name: &str, rule: &str| save_set(&d, SaveRuleSet { id: None, name: name.into(), rule_ids: vec![format!("library:{rule}")] }).unwrap().id;
    let (sa, sb) = (set("A", "a.md"), set("B", "b.md"));
    let proj = |name: &str, set: &str| {
        let p = h.dir.path().join(name);
        std::fs::create_dir_all(&p).unwrap();
        let p = p.to_string_lossy().into_owned();
        set_project_rules(&d, &p, Some(set.into())).unwrap();
        p
    };
    let (pa, pb) = (proj("pa", &sa), proj("pb", &sb));
    let new = |p: &str, confirm: bool| {
        let req = CreateAgentRequest { kind: "aider".into(), project_path: p.into(), apply_to_main_checkout: confirm, ..Default::default() };
        create(&h.engine, req).unwrap().id
    };
    let (a, b, unconfirmed) = (new(&pa, true), new(&pb, true), new(&pb, false));
    let stale_ids = || {
        let mut v: Vec<String> = agent_rules(&h.engine).into_iter().filter(|v| v.needs_reapply()).map(|v| v.agent_id).collect();
        v.sort();
        v
    };
    assert!(stale_ids().is_empty(), "{:?}", agent_rules(&h.engine));
    assert!(reapply_stale(&h.engine, None, None).is_empty());

    write(&d.library_rules().join("a.md"), "Rule A, changed.\n");
    write(&d.library_rules().join("b.md"), "Rule B, changed.\n");
    let mut both = vec![a.clone(), b.clone()];
    both.sort();
    assert_eq!(stale_ids(), both, "not the unconfirmed main checkout");
    let done = reapply_stale(&h.engine, Some(&pa), None);
    assert_eq!(done.iter().map(|r| r.agent_id.as_str()).collect::<Vec<_>>(), vec![a.as_str()]);
    assert_eq!(done[0].error, None);
    assert!(std::fs::read_to_string(format!("{pa}/.aider.pitwall-rules.md")).unwrap().contains("Rule A, changed."));
    assert_eq!(stale_ids(), vec![b.clone()]);
    assert!(reapply_stale(&h.engine, None, Some(&sa)).is_empty(), "nothing of set A left");
    let done = reapply_stale(&h.engine, None, Some(&sb));
    assert_eq!(done.iter().map(|r| r.agent_id.as_str()).collect::<Vec<_>>(), vec![b.as_str()]);
    assert!(stale_ids().is_empty());
    assert!(std::fs::read_to_string(format!("{pb}/.aider.pitwall-rules.md")).unwrap().contains("Rule B, changed."));
    assert!(read(&d).agents.get(&unconfirmed).is_none_or(|s| s.files.is_empty()));
}

// ------------------------------------------------------------------ project rules

/// A stand-in for rulesync: `generate` writes CLAUDE.md from the input
/// roots' rules (CLAUDE.local.md for `localRoot` ones) into the output root;
/// `import` turns the CLAUDE.md / AGENTS.md in its folder into
/// `.rulesync/rules/overview.md`.
fn fake_rulesync(dir: &Path) -> Option<Runner> {
    // A shell script: not on Windows.
    if std::path::MAIN_SEPARATOR == '\\' {
        return None;
    }
    let script = dir.join("fake-rulesync");
    write(
        &script,
        r#"#!/bin/sh
cmd=""; out=""; ins=""; mode=""
for a in "$@"; do
  case "$a" in
    --json) ;;
    generate|import) cmd="$a" ;;
    --output-roots) mode=out ;;
    --input-roots) mode=in ;;
    --targets|--features) mode=skip ;;
    *) case "$mode" in out) out="$a"; mode="" ;; in) ins="$ins $a" ;; skip) mode="" ;; esac ;;
  esac
done
if [ "$cmd" = import ]; then
  mkdir -p .rulesync/rules
  for f in CLAUDE.md AGENTS.md; do [ -f "$f" ] && { printf -- '---\nroot: true\n---\n'; cat "$f"; } > .rulesync/rules/overview.md; done
  echo '{"success":true}'; exit 0
fi
: > "$out/CLAUDE.md"; local=""
for r in $ins; do for f in "$r"/rules/*.md; do [ -f "$f" ] || continue
  if grep -q 'localRoot: true' "$f"; then cat "$f" >> "$out/CLAUDE.local.md"; local=1; else cat "$f" >> "$out/CLAUDE.md"; fi
done; done
ops='{"action":"write","kind":"file","path":"CLAUDE.md"}'
[ -n "$local" ] && ops="$ops,{\"action\":\"write\",\"kind\":\"file\",\"path\":\"CLAUDE.local.md\"}"
echo "{\"success\":true,\"data\":{\"plan\":{\"operations\":[$ops]}}}"
"#,
    );
    crate::platform::make_executable(&script).unwrap();
    Some(Runner { argv: vec![script.to_string_lossy().into_owned()], path_env: std::env::var("PATH").ok(), via: "rulesync" })
}

/// A git repo (`repo`, one commit) and Pitwall's folder with a library.
struct Proj {
    _t: Tmp,
    dirs: Dirs,
    repo: PathBuf,
}

impl Proj {
    fn new(tag: &str) -> Proj {
        let t = Tmp::new(tag);
        let repo = t.0.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        sh(&repo, &["init", "-q"]);
        write(&repo.join("README.md"), "# repo\n");
        sh(&repo, &["add", "-A"]);
        sh(&repo, &["commit", "-qm", "init"]);
        let dirs = Dirs { base: t.0.join("pitwall") };
        write(&dirs.library_rules().join("style.md"), "---\ndescription: Style\n---\nUse tabs.\n");
        write(&dirs.library_rules().join("testing.md"), "Run the tests.\n");
        Proj { dirs, repo, _t: t }
    }
    fn path(&self) -> String {
        self.repo.to_string_lossy().into_owned()
    }
    fn rule(&self, name: &str) -> PathBuf {
        self.repo.join(".rulesync/rules").join(name)
    }
    fn lib(&self, name: &str) -> PathBuf {
        self.dirs.library_rules().join(name)
    }
    fn status(&self) -> ProjectRulesStatus {
        project_status(&self.dirs, &self.path()).unwrap()
    }
    fn state(&self, name: &str) -> pitwall_proto::ProjectRuleState {
        self.status().rules.into_iter().find(|r| r.name == name).unwrap().state
    }
    fn set(&self, name: &str, rules: &[&str]) -> String {
        let ids = rules.iter().map(|r| format!("library:{r}")).collect();
        save_set(&self.dirs, SaveRuleSet { id: None, name: name.into(), rule_ids: ids }).unwrap().id
    }
}

/// Assigning copies clean rulesync files into the repo, records where they
/// came from, and leaves them for the user to commit (not excluded).
#[test]
fn assigning_copies_rules_into_the_project() {
    let p = Proj::new("assign");
    let web = p.set("Web", &["style.md", "testing.md"]);
    let r = project_assign(&p.dirs, &p.path(), Some(&web), &[], &[]).unwrap();
    assert_eq!(r.copied, vec!["style.md", "testing.md"]);
    assert_eq!(std::fs::read(p.rule("style.md")).unwrap(), std::fs::read(p.lib("style.md")).unwrap());
    let m = project::load(&p.repo);
    assert_eq!(m.rules["style.md"].source, "library:style.md");
    assert_eq!(m.rules["style.md"].hash, apply::content_hash(b"---\ndescription: Style\n---\nUse tabs.\n"));
    // Shows up as a change to commit, nothing excluded.
    assert_eq!(sh(&p.repo, &["status", "--porcelain"]), "?? .rulesync/\n");
    assert!(!std::fs::read_to_string(p.repo.join(".git/info/exclude")).unwrap_or_default().contains(".rulesync"));
    let s = p.status();
    assert!(s.rulesync && s.updates == 0);
    assert_eq!(s.rules.iter().map(|r| (r.name.as_str(), r.state)).collect::<Vec<_>>(), vec![
        ("style.md", pitwall_proto::ProjectRuleState::Current),
        ("testing.md", pitwall_proto::ProjectRuleState::Current)
    ]);
    assert_eq!(s.rules[0].description.as_deref(), Some("Style"));
    // Again: nothing to do. A single rule by id works too.
    let r = project_assign(&p.dirs, &p.path(), None, &["library:style.md".into(), "library:gone.md".into()], &[]).unwrap();
    assert_eq!((r.copied.len(), r.unchanged.clone(), r.skipped.clone()), (0, vec!["style.md".to_string()], vec!["library:gone.md: not in the library".to_string()]));
}

/// A file already in `.rulesync/rules/` with other content is never replaced
/// unless the caller names it.
#[test]
fn assigning_never_overwrites_a_different_file_unasked() {
    let p = Proj::new("no-overwrite");
    write(&p.rule("style.md"), "Our own style.\n");
    let r = project_assign(&p.dirs, &p.path(), None, &["library:style.md".into()], &[]).unwrap();
    assert_eq!(r.conflicts, vec!["style.md"]);
    assert_eq!(std::fs::read_to_string(p.rule("style.md")).unwrap(), "Our own style.\n");
    assert!(project::load(&p.repo).rules.is_empty());
    assert_eq!(p.state("style.md"), pitwall_proto::ProjectRuleState::Own);
    let r = project_assign(&p.dirs, &p.path(), None, &["library:style.md".into()], &["style.md".into()]).unwrap();
    assert_eq!(r.copied, vec!["style.md"]);
    assert!(std::fs::read_to_string(p.rule("style.md")).unwrap().contains("Use tabs."));
}

/// Two sources with a rule of the same name: the second goes under a
/// folder named for its source.
#[test]
fn same_named_rules_from_two_sources_both_fit() {
    let p = Proj::new("names");
    let team = p._t.0.join("team.md");
    std::fs::write(&team, "Team style.\n").unwrap();
    let mut lib = library::list(&p.dirs, &[]);
    lib.push(RuleFile { id: "team:style.md".into(), path: team.to_string_lossy().into_owned(), description: None, targets: vec![], root: false, local_root: false, source: "team".into() });
    let r = project::assign(&p.repo, &lib, &["library:style.md".into(), "team:style.md".into()], &[]).unwrap();
    assert_eq!(r.copied, vec!["style.md", "team/style.md"]);
    assert_eq!(std::fs::read_to_string(p.rule("team/style.md")).unwrap(), "Team style.\n");
}

/// Updates are opt-in: library changes show as updates with a diff, local
/// changes are never overwritten, and a conflict needs an explicit choice.
#[test]
fn updates_modifications_and_conflicts() {
    use pitwall_proto::ProjectRuleState::*;
    let p = Proj::new("updates");
    project_assign(&p.dirs, &p.path(), None, &["library:style.md".into(), "library:testing.md".into()], &[]).unwrap();
    // The library changes: an update, nothing written yet.
    write(&p.lib("style.md"), "---\ndescription: Style\n---\nUse spaces.\n");
    assert_eq!(p.state("style.md"), Update);
    assert!(std::fs::read_to_string(p.rule("style.md")).unwrap().contains("Use tabs."));
    // The project's copy changes: modified.
    write(&p.rule("testing.md"), "Run the tests twice.\n");
    assert_eq!(p.state("testing.md"), Modified);
    assert_eq!(p.status().updates, 1);
    let u = project_updates(&p.dirs, &p.path()).unwrap();
    assert_eq!(u.iter().map(|x| (x.name.as_str(), x.state)).collect::<Vec<_>>(), vec![("style.md", Update), ("testing.md", Modified)]);
    assert!(u[0].diff.contains("-Use tabs.") && u[0].diff.contains("+Use spaces."), "{}", u[0].diff);
    // Both change testing.md: a conflict.
    write(&p.lib("testing.md"), "Run the tests in CI.\n");
    assert_eq!(p.state("testing.md"), Conflict);
    // Updating everything takes style.md, leaves the conflict alone.
    let r = project_update(&p.dirs, &p.path(), &[], None).unwrap();
    assert_eq!((r.updated.clone(), r.conflicts.clone()), (vec!["style.md".to_string()], vec!["testing.md".to_string()]));
    assert!(std::fs::read_to_string(p.rule("style.md")).unwrap().contains("Use spaces."));
    assert_eq!(std::fs::read_to_string(p.rule("testing.md")).unwrap(), "Run the tests twice.\n");
    assert_eq!(p.state("style.md"), Current);
    // Keep the project's: it stays, now just modified.
    let r = project_update(&p.dirs, &p.path(), &["testing.md".into()], Some("project")).unwrap();
    assert_eq!(r.kept, vec!["testing.md"]);
    assert_eq!(p.state("testing.md"), Modified);
    // The library changes again; take the library's this time.
    write(&p.lib("testing.md"), "Run the tests in CI, always.\n");
    assert_eq!(p.state("testing.md"), Conflict);
    let r = project_update(&p.dirs, &p.path(), &["library:testing.md".into()], Some("library")).unwrap();
    assert_eq!(r.updated, vec!["testing.md"]);
    assert_eq!(std::fs::read_to_string(p.rule("testing.md")).unwrap(), "Run the tests in CI, always.\n");
    assert_eq!(p.state("testing.md"), Current);
    assert!(project_update(&p.dirs, &p.path(), &[], Some("both")).is_err());
    assert!(project_update(&p.dirs, &p.path(), &["nope.md".into()], None).is_err());
}

/// Unassigning removes only what is as copied, unless forced.
#[test]
fn unassigning_keeps_local_changes_unless_forced() {
    let p = Proj::new("unassign");
    project_assign(&p.dirs, &p.path(), None, &["library:style.md".into(), "library:testing.md".into()], &[]).unwrap();
    write(&p.rule("testing.md"), "changed here\n");
    let r = project_unassign(&p.path(), &["library:style.md".into(), "testing.md".into(), "other.md".into()], false).unwrap();
    assert_eq!((r.removed.clone(), r.modified.clone(), r.unknown.clone()), (vec!["style.md".to_string()], vec!["testing.md".to_string()], vec!["other.md".to_string()]));
    assert!(!p.rule("style.md").exists() && p.rule("testing.md").exists());
    let r = project_unassign(&p.path(), &["testing.md".into()], true).unwrap();
    assert_eq!(r.removed, vec!["testing.md"]);
    assert!(!p.rule("testing.md").exists());
    assert!(project::load(&p.repo).rules.is_empty());
}

/// A repo with a hand-written, tracked CLAUDE.md: it's offered for import
/// first, generation won't replace it until it's imported, and afterwards
/// the generated CLAUDE.md carries its text.
#[test]
fn existing_instructions_are_imported_before_generating() {
    let p = Proj::new("import-existing");
    let Some(runner) = fake_rulesync(&p._t.0) else { return };
    write(&p.repo.join("CLAUDE.md"), "# Ours\n\nAlways test.\n");
    sh(&p.repo, &["add", "CLAUDE.md"]);
    sh(&p.repo, &["commit", "-qm", "claude"]);
    assert_eq!(p.status().existing, vec!["CLAUDE.md"]);
    project_assign(&p.dirs, &p.path(), None, &["library:style.md".into()], &[]).unwrap();
    assert_eq!(p.status().existing, vec!["CLAUDE.md"], "still offered");
    // Generating now would lose the hand-written text: kept, out of date.
    let g = project_generate_with(&p.dirs, &runner, &p.repo, &["claudecode".into()], &[], Vec::new).unwrap();
    assert_eq!(g.needs_confirm, vec!["CLAUDE.md"]);
    assert_eq!(std::fs::read_to_string(p.repo.join("CLAUDE.md")).unwrap(), "# Ours\n\nAlways test.\n");
    assert!(p.status().generated.out_of_date);

    let stage = p.dirs.run().join("import");
    let r = project::import_existing(&p.repo, &runner, &stage).unwrap();
    assert_eq!((r.files.clone(), r.added.clone()), (vec!["CLAUDE.md".to_string()], vec!["claude-overview.md".to_string()]));
    assert!(std::fs::read_to_string(p.rule("claude-overview.md")).unwrap().contains("Always test."));
    let s = p.status();
    assert!(s.existing.is_empty());
    assert!(s.rules.iter().any(|r| r.name == "claude-overview.md" && r.state == pitwall_proto::ProjectRuleState::Own));
    let g = project_generate_with(&p.dirs, &runner, &p.repo, &["claudecode".into()], &[], Vec::new).unwrap();
    assert_eq!(g.written, vec!["CLAUDE.md"]);
    let text = std::fs::read_to_string(p.repo.join("CLAUDE.md")).unwrap();
    assert!(text.contains("Always test.") && text.contains("Use tabs."), "{text}");
    assert!(!p.status().generated.out_of_date);
}

/// Generated files are tracked against `.rulesync/`: out of date when the
/// rules change or the outputs are edited; an edited output is never
/// replaced unasked.
#[test]
fn generated_files_go_out_of_date() {
    let p = Proj::new("generate");
    let Some(runner) = fake_rulesync(&p._t.0) else { return };
    project_assign(&p.dirs, &p.path(), None, &["library:style.md".into()], &[]).unwrap();
    let s = p.status();
    assert!(!s.generated.generated && s.generated.out_of_date, "copied, never generated");
    let g = project_generate_with(&p.dirs, &runner, &p.repo, &[], &[], || vec!["claudecode".into()]).unwrap();
    assert_eq!((g.written.clone(), g.targets.clone()), (vec!["CLAUDE.md".to_string()], vec!["claudecode".to_string()]));
    let s = p.status().generated;
    assert!(s.generated && !s.out_of_date && s.targets == vec!["claudecode"]);
    // Committed like any change.
    assert!(sh(&p.repo, &["status", "--porcelain"]).contains("?? CLAUDE.md"));

    project_assign(&p.dirs, &p.path(), None, &["library:testing.md".into()], &[]).unwrap();
    assert!(p.status().generated.out_of_date, "rules changed");
    let g = project_generate_with(&p.dirs, &runner, &p.repo, &[], &[], Vec::new).unwrap();
    assert_eq!(g.written, vec!["CLAUDE.md"], "Pitwall's own output is replaced");
    assert!(!p.status().generated.out_of_date);

    write(&p.repo.join("CLAUDE.md"), "edited by hand\n");
    let s = p.status().generated;
    assert!(s.out_of_date && s.changed == vec!["CLAUDE.md"]);
    let g = project_generate_with(&p.dirs, &runner, &p.repo, &[], &[], Vec::new).unwrap();
    assert_eq!(g.needs_confirm, vec!["CLAUDE.md"]);
    assert_eq!(std::fs::read_to_string(p.repo.join("CLAUDE.md")).unwrap(), "edited by hand\n");
    let g = project_generate_with(&p.dirs, &runner, &p.repo, &[], &["CLAUDE.md".into()], Vec::new).unwrap();
    assert_eq!(g.written, vec!["CLAUDE.md"]);
    assert!(!p.status().generated.out_of_date);
}

fn main_ctx(p: &Proj) -> AgentCtx {
    AgentCtx {
        id: "agent-1".into(),
        cwd: p.path(),
        project: p.path(),
        worktree: false,
        kind_name: "Claude Code".into(),
        target: Some("claudecode".into()),
        fallback: None,
    }
}

/// Agent scope never clobbers shared files: a tracked CLAUDE.md is kept and
/// not excluded; an agent's own CLAUDE.md that the project then generates
/// is handed over (no longer excluded, shows in Review).
#[test]
fn agent_rules_and_project_files_dont_collide() {
    let p = Proj::new("collide");
    let Some(runner) = fake_rulesync(&p._t.0) else { return };
    write(&p.lib("mine.md"), "Personal note.\n");
    let mine = p.set("Mine", &["mine.md"]);
    update(&p.dirs, |s| {
        s.agents.entry("agent-1".into()).or_default().rule_set_id = Some(mine.clone());
        Ok(())
    })
    .unwrap();
    let c = main_ctx(&p);
    // Its own rules land in an (untracked, excluded) CLAUDE.md.
    let res = apply_with(&p.dirs, Some(&runner), &LocalExec, &c, true).unwrap();
    assert_eq!(res.generated, vec!["CLAUDE.md"]);
    let exclude = p.repo.join(".git/info/exclude");
    assert!(std::fs::read_to_string(&exclude).unwrap().contains("/CLAUDE.md"));
    assert_eq!(sh(&p.repo, &["status", "--porcelain"]), "");

    // The project takes CLAUDE.md over: generated, released from the agent.
    project_assign(&p.dirs, &p.path(), None, &["library:style.md".into()], &[]).unwrap();
    let g = project_generate_with(&p.dirs, &runner, &p.repo, &["claudecode".into()], &[], Vec::new).unwrap();
    assert_eq!(g.written, vec!["CLAUDE.md"]);
    assert!(!std::fs::read_to_string(&exclude).unwrap().contains("/CLAUDE.md"));
    assert!(read(&p.dirs).agents["agent-1"].files.is_empty());
    assert!(sh(&p.repo, &["status", "--porcelain"]).contains("?? CLAUDE.md"));
    // Once committed (tracked), the agent's apply keeps it and excludes nothing.
    sh(&p.repo, &["add", "-A"]);
    sh(&p.repo, &["commit", "-qm", "rules"]);
    let res = apply_with(&p.dirs, Some(&runner), &LocalExec, &c, true).unwrap();
    assert!(res.generated.is_empty(), "{res:?}");
    assert!(res.log.contains("kept the project's CLAUDE.md"), "{}", res.log);
    assert!(!std::fs::read_to_string(&exclude).unwrap().contains("/CLAUDE.md"));
    assert_eq!(sh(&p.repo, &["status", "--porcelain"]), "");
    // The agent cleaning up never deletes the project's file.
    forget_agent(&p.dirs, &LocalExec, "agent-1");
    assert!(p.repo.join("CLAUDE.md").exists());
}

/// The old default set is a fallback while the project has no `.rulesync/`;
/// "move to project" copies its rules in and drops it.
#[test]
fn the_old_default_set_moves_into_the_project() {
    let p = Proj::new("migrate");
    let web = p.set("Web", &["style.md"]);
    // rules.json as an older Pitwall wrote it still loads.
    let mut store: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p.dirs.base.join("rules.json")).unwrap()).unwrap();
    store["projectDefaults"] = serde_json::json!({ p.path(): web });
    std::fs::write(p.dirs.base.join("rules.json"), store.to_string()).unwrap();
    assert_eq!(p.status().fallback_set.as_deref(), Some(web.as_str()));
    let c = main_ctx(&p);
    assert_eq!(project_set(&read(&p.dirs), &c).as_deref(), Some(web.as_str()), "used while there's no .rulesync/");

    let r = project_move_default(&p.dirs, &p.path()).unwrap();
    assert_eq!(r.copied, vec!["style.md"]);
    assert!(read(&p.dirs).project_defaults.is_empty());
    let s = p.status();
    assert_eq!(s.fallback_set, None);
    assert_eq!(project_set(&read(&p.dirs), &c), None, "the project's own rules now");
    assert!(project_move_default(&p.dirs, &p.path()).is_err(), "nothing left to move");
}
