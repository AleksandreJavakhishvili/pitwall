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

/// A stand-in for rulesync, shaped like the real one (checked against
/// rulesync 33): `generate` writes each target's file from the input
/// roots' rules that apply to it (`targets` with the target or `*`, or
/// none) — bodies only, joined by a blank line: claudecode → CLAUDE.md
/// (`localRoot` rules → CLAUDE.local.md), codexcli → AGENTS.md — with the
/// `rules` feature, and copies each input root's `skills/<name>/` to
/// `.claude/skills/<name>/` with `skills`; `import` turns the CLAUDE.md or
/// AGENTS.md in its folder into `.rulesync/rules/<file>` as a root rule for
/// every tool, as `rulesync import` does.
fn fake_rulesync(dir: &Path) -> Option<Runner> {
    // A shell script: not on Windows.
    if std::path::MAIN_SEPARATOR == '\\' {
        return None;
    }
    let script = dir.join("fake-rulesync");
    write(
        &script,
        r#"#!/bin/sh
cmd=""; out=""; ins=""; mode=""; feats=""; tgts=""
for a in "$@"; do
  case "$a" in
    --json) ;;
    generate|import) cmd="$a" ;;
    --output-roots) mode=out ;;
    --input-roots) mode=in ;;
    --features) mode=feat ;;
    --targets) mode=tgt ;;
    *) case "$mode" in out) out="$a"; mode="" ;; in) ins="$ins $a" ;; feat) feats="$a"; mode="" ;; tgt) tgts="$a"; mode="" ;; esac ;;
  esac
done
if [ "$cmd" = import ]; then
  mkdir -p .rulesync/rules
  for f in CLAUDE.md AGENTS.md; do [ -f "$f" ] && { printf -- "---\nroot: true\ntargets:\n  - '*'\nglobs:\n  - '**/*'\n---\n"; cat "$f"; } > ".rulesync/rules/$f"; done
  echo '{"success":true}'; exit 0
fi
targets_of() { awk 'NR==1&&/^---/{fm=1;next} fm&&/^---/{exit} fm&&/^targets:/{t=1; sub(/^targets:[ ]*/,""); print; next} fm&&t&&/^[ \t]/{print; next} fm{t=0}' "$1"; }
applies() { tl=$(targets_of "$1"); [ -z "$(echo "$tl" | tr -d ' []')" ] && return 0; echo "$tl" | grep -q -e "\*" -e "$2"; }
body() { awk 'NR==1&&/^---/{fm=1;next} fm&&/^---/{fm=0;next} !fm{print}' "$1"; }
emit() { # target file local?
  : > "$out/$2.tmp"; first=1
  for r in $ins; do for f in "$r"/rules/*.md; do [ -f "$f" ] || continue
    applies "$f" "$1" || continue
    if grep -q 'localRoot: true' "$f"; then [ "$3" = local ] || continue; else [ "$3" = local ] && continue; fi
    [ $first = 1 ] || echo >> "$out/$2.tmp"; first=0; body "$f" >> "$out/$2.tmp"
  done; done
  if [ $first = 0 ]; then mv "$out/$2.tmp" "$out/$2"; printf '{"action":"write","kind":"file","path":"%s"},' "$2" >> "$out/ops"; else rm "$out/$2.tmp"; fi
}
: > "$out/ops"
case ",$feats," in *,rules,*)
  case ",$tgts," in *,claudecode,*) emit claudecode CLAUDE.md shared; emit claudecode CLAUDE.local.md local ;; esac
  case ",$tgts," in *,codexcli,*) emit codexcli AGENTS.md shared ;; esac
;; esac
case ",$feats," in *,skills,*)
  for r in $ins; do for d in "$r"/skills/*/; do [ -f "$d/SKILL.md" ] || continue
    n=$(basename "$d"); mkdir -p "$out/.claude/skills/$n"; cp -R "$d". "$out/.claude/skills/$n/"
    find "$out/.claude/skills/$n" -type f -exec chmod a-x {} + # rulesync drops the bit
    (cd "$out/.claude/skills/$n" && find . -type f | sed 's|^\./||' | sort) | while read -r f; do
      printf '{"action":"write","kind":"file","path":".claude/skills/%s/%s"},' "$n" "$f" >> "$out/ops"; done
  done; done
;; esac
ops=$(sed 's/,$//' "$out/ops"); rm "$out/ops"
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
    lib.push(RuleFile { id: "team:style.md".into(), path: team.to_string_lossy().into_owned(), description: None, targets: vec![], root: false, local_root: false, skill: false, source: "team".into() });
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
    assert!(p.status().existing.is_empty(), "not offered before rules are assigned");
    project_assign(&p.dirs, &p.path(), None, &["library:style.md".into()], &[]).unwrap();
    assert_eq!(p.status().existing, vec!["CLAUDE.md"], "offered once rules are assigned");
    // Generating now would lose the hand-written text: kept, out of date.
    let g = project_generate_with(&p.dirs, &runner, &p.repo, &["claudecode".into()], &[], Vec::new).unwrap();
    assert_eq!(g.needs_confirm, vec!["CLAUDE.md"]);
    assert_eq!(std::fs::read_to_string(p.repo.join("CLAUDE.md")).unwrap(), "# Ours\n\nAlways test.\n");
    assert!(p.status().generated.out_of_date);

    let stage = p.dirs.run().join("import");
    let r = project::import_existing(&p.repo, &runner, &stage).unwrap();
    assert_eq!((r.files.clone(), r.added.clone()), (vec!["CLAUDE.md".to_string()], vec!["claude.md".to_string()]));
    assert!(std::fs::read_to_string(p.rule("claude.md")).unwrap().contains("Always test."));
    let s = p.status();
    assert!(s.existing.is_empty());
    assert!(s.rules.iter().any(|r| r.name == "claude.md" && r.state == pitwall_proto::ProjectRuleState::Own));
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

// ------------------------------------------------------------------ skills

fn skill(dirs: &Dirs, name: &str, front: &str, body: &str) -> PathBuf {
    let dir = dirs.library().join("skills").join(name);
    write(&dir.join("SKILL.md"), &format!("---\n{front}---\n{body}"));
    dir
}

/// The library lists skills (folders with SKILL.md) before rules, with the
/// name and description from their frontmatter.
#[test]
fn the_library_lists_skills_first() {
    let t = Tmp::new("skills-list");
    let dirs = Dirs { base: t.0.clone() };
    write(&dirs.library_rules().join("style.md"), "Use tabs.\n");
    let review = skill(&dirs, "review", "name: review\ndescription: >-\n  Review a diff\n  carefully\ntargets: [\"claudecode\"]\n", "Look closely.\n");
    write(&review.join("scripts/check.sh"), "#!/bin/sh\n");
    std::fs::create_dir_all(dirs.library().join("skills/empty")).unwrap();
    let lib = library::list(&dirs, &[]);
    assert_eq!(lib.iter().map(|f| (f.id.as_str(), f.skill)).collect::<Vec<_>>(), vec![("library:skills/review", true), ("library:style.md", false)]);
    assert_eq!(lib[0].description.as_deref(), Some("Review a diff carefully"));
    assert_eq!(lib[0].targets, vec!["claudecode"]);
    assert_eq!(library::skill_name(&lib[0].id), Some("review"));
    assert_eq!(library::skill_name("library:style.md"), None);
    assert_eq!(library::files_under(&review), vec!["SKILL.md", "scripts/check.sh"]);
}

/// `rules new` / `rules skill new`: proper rulesync files, from scratch, a
/// file, a folder or a library entry; the other kind only when asked.
#[test]
fn new_rules_and_skills_in_the_library() {
    let t = Tmp::new("skills-new");
    let dirs = Dirs { base: t.0.join("pw") };
    let new = |e: NewEntry| {
        let lib = library::list(&dirs, &[]);
        library::create(&dirs, &lib, &e)
    };
    let (id, _) = new(NewEntry { name: "tabs".into(), description: Some("Tabs, not spaces".into()), ..Default::default() }).unwrap();
    assert_eq!(id, "library:tabs.md");
    let text = std::fs::read_to_string(dirs.library_rules().join("tabs.md")).unwrap();
    let f = library::parse_front(&text);
    assert_eq!((f.description.as_deref(), f.targets.clone(), f.root), (Some("Tabs, not spaces"), vec!["*".to_string()], false));
    assert!(new(NewEntry { name: "tabs".into(), ..Default::default() }).unwrap_err().contains("already has a rule"));
    assert!(new(NewEntry { name: "../x".into(), ..Default::default() }).is_err());

    // A skill needs a description; from a folder it keeps its files and other keys.
    assert!(new(NewEntry { skill: true, name: "bare".into(), ..Default::default() }).unwrap_err().contains("description"));
    let src = t.0.join("deploy");
    write(&src.join("SKILL.md"), "---\nname: old\ndescription: Ship it\nclaudecode:\n  model: opus\n---\nRun the deploy.\n");
    write(&src.join("run.sh"), "#!/bin/sh\necho deploy\n");
    let (id, _) = new(NewEntry { skill: true, name: "deploy".into(), from: Some(src.to_string_lossy().into()), ..Default::default() }).unwrap();
    assert_eq!(id, "library:skills/deploy");
    let dir = dirs.library().join("skills/deploy");
    let md = std::fs::read_to_string(dir.join("SKILL.md")).unwrap();
    let f = library::parse_front(&md);
    assert_eq!((f.name.as_deref(), f.description.as_deref()), (Some("deploy"), Some("Ship it")));
    assert!(md.contains("claudecode:\n  model: opus\n") && md.ends_with("Run the deploy.\n"), "{md}");
    assert!(dir.join("run.sh").is_file());

    // Rule ↔ skill only when asked.
    let e = new(NewEntry { skill: true, name: "tabs-skill".into(), from: Some("library:tabs.md".into()), ..Default::default() }).unwrap_err();
    assert!(e.contains("--as-skill"), "{e}");
    let (id, notes) = new(NewEntry { skill: true, name: "tabs-skill".into(), from: Some("library:tabs.md".into()), convert: true, ..Default::default() }).unwrap();
    assert_eq!(id, "library:skills/tabs-skill");
    assert!(notes[0].contains("only when an agent decides"));
    let e = new(NewEntry { name: "deploy-rule".into(), from: Some("library:skills/deploy".into()), ..Default::default() }).unwrap_err();
    assert!(e.contains("--as-rule"), "{e}");
    let (_, notes) = new(NewEntry { name: "deploy-rule".into(), from: Some("library:skills/deploy".into()), convert: true, ..Default::default() }).unwrap();
    assert!(notes.iter().any(|n| n.contains("run.sh")) && notes.iter().any(|n| n.contains("every session")), "{notes:?}");
    assert!(std::fs::read_to_string(dirs.library_rules().join("deploy-rule.md")).unwrap().contains("Run the deploy."));
    // From text (the app's form): its frontmatter is read, the name is the given one.
    let (id, _) = new(NewEntry { skill: true, name: "notes".into(), text: Some("---\nname: x\ndescription: Take notes\n---\nWrite them down.\n".into()), ..Default::default() }).unwrap();
    assert_eq!(id, "library:skills/notes");
    let md = std::fs::read_to_string(dirs.library().join("skills/notes/SKILL.md")).unwrap();
    assert!(md.starts_with("---\nname: \"notes\"\ndescription: \"Take notes\"\n") && md.ends_with("---\nWrite them down.\n"), "{md}");
    // Overwriting is allowed when asked for.
    new(NewEntry { skill: true, name: "deploy".into(), description: Some("Ship it now".into()), overwrite: true, ..Default::default() }).unwrap();
    assert!(!dir.join("run.sh").exists(), "replaced as a whole");
}

/// A skill assigned to a project: its whole folder is copied and hashed;
/// updates, local changes and conflicts as for rules, with a diff per file.
#[test]
fn skills_are_copied_into_projects_as_folders() {
    use pitwall_proto::ProjectRuleState::*;
    let p = Proj::new("skill-assign");
    let lib_dir = skill(&p.dirs, "review", "name: review\ndescription: Review\n", "Read it.\n");
    write(&lib_dir.join("checklist.md"), "- tests\n");
    let r = project_assign(&p.dirs, &p.path(), None, &["library:skills/review".into()], &[]).unwrap();
    assert_eq!(r.copied, vec!["skills/review"]);
    let here = p.repo.join(".rulesync/skills/review");
    assert_eq!(std::fs::read_to_string(here.join("checklist.md")).unwrap(), "- tests\n");
    let m = project::load(&p.repo);
    assert_eq!(m.skills["review"].hash, project::dir_hash(&lib_dir).unwrap());
    let st = p.status();
    assert_eq!((st.rules[0].name.as_str(), st.rules[0].skill, st.rules[0].state), ("review", true, Current));
    assert!(st.existing.is_empty());

    // A different folder already there: kept unless named.
    let p2 = Proj::new("skill-conflict");
    let d2 = skill(&p2.dirs, "review", "name: review\ndescription: Review\n", "Read it.\n");
    let _ = d2;
    write(&p2.repo.join(".rulesync/skills/review/SKILL.md"), "---\nname: review\ndescription: ours\n---\nOurs.\n");
    let r = project_assign(&p2.dirs, &p2.path(), None, &["library:skills/review".into()], &[]).unwrap();
    assert_eq!(r.conflicts, vec!["skills/review"]);
    assert!(std::fs::read_to_string(p2.repo.join(".rulesync/skills/review/SKILL.md")).unwrap().contains("Ours."));

    // Library change: an update with a per-file diff.
    write(&lib_dir.join("checklist.md"), "- tests\n- docs\n");
    let st = p.status();
    assert_eq!((st.rules[0].state, st.updates), (Update, 1));
    let u = project_updates(&p.dirs, &p.path()).unwrap();
    assert!(u[0].skill && u[0].diff.contains("+++ library/checklist.md") && u[0].diff.contains("+- docs"), "{}", u[0].diff);
    project_update(&p.dirs, &p.path(), &["skills/review".into()], None).unwrap();
    assert_eq!(std::fs::read_to_string(here.join("checklist.md")).unwrap(), "- tests\n- docs\n");
    assert_eq!(p.state("review"), Current);
    // Both change: a conflict until someone chooses.
    write(&here.join("checklist.md"), "- ours\n");
    assert_eq!(p.state("review"), Modified);
    write(&lib_dir.join("checklist.md"), "- theirs\n");
    assert_eq!(p.state("review"), Conflict);
    assert_eq!(project_update(&p.dirs, &p.path(), &[], None).unwrap().conflicts, vec!["skills/review"]);
    project_update(&p.dirs, &p.path(), &["library:skills/review".into()], Some("library")).unwrap();
    assert_eq!(std::fs::read_to_string(here.join("checklist.md")).unwrap(), "- theirs\n");
    // Unassign: only unchanged, unless forced.
    write(&here.join("extra.md"), "mine\n");
    assert_eq!(project_unassign(&p.path(), &["review".into()], false).unwrap().modified, vec!["skills/review"]);
    assert_eq!(project_unassign(&p.path(), &["review".into()], true).unwrap().removed, vec!["skills/review"]);
    assert!(!here.exists());
}

/// A skills-only project keeps its own CLAUDE.md / AGENTS.md: generation
/// writes the skills' outputs only, and import isn't offered.
#[test]
fn skills_only_generation_leaves_the_main_files_alone() {
    let p = Proj::new("skills-only");
    let Some(runner) = fake_rulesync(&p._t.0) else { return };
    write(&p.repo.join("CLAUDE.md"), "# Ours\n");
    write(&p.repo.join("AGENTS.md"), "# Ours too\n");
    sh(&p.repo, &["add", "-A"]);
    sh(&p.repo, &["commit", "-qm", "instructions"]);
    let lib_dir = skill(&p.dirs, "review", "name: review\ndescription: Review\n", "Read it.\n");
    write(&lib_dir.join("run.sh"), "#!/bin/sh\n");
    crate::platform::make_executable(&lib_dir.join("run.sh")).unwrap();
    project_assign(&p.dirs, &p.path(), None, &["library:skills/review".into()], &[]).unwrap();
    let st = p.status();
    assert!(st.existing.is_empty(), "no import offer for skills");
    assert!(st.generated.out_of_date);
    let g = project_generate_with(&p.dirs, &runner, &p.repo, &["claudecode".into()], &[], Vec::new).unwrap();
    assert_eq!(g.features, vec!["skills"]);
    assert_eq!(g.written, vec![".claude/skills/review/SKILL.md", ".claude/skills/review/run.sh"], "{}", g.log);
    assert!(crate::platform::is_executable(&p.repo.join(".claude/skills/review/run.sh")), "the script stays executable");
    assert!(!crate::platform::is_executable(&p.repo.join(".claude/skills/review/SKILL.md")));
    assert_eq!(std::fs::read_to_string(p.repo.join("CLAUDE.md")).unwrap(), "# Ours\n");
    assert_eq!(std::fs::read_to_string(p.repo.join("AGENTS.md")).unwrap(), "# Ours too\n");
    let st = p.status().generated;
    assert!(!st.out_of_date && st.features == vec!["skills"]);
    assert_eq!(project::load(&p.repo).features, vec!["skills"]);
    let status = sh(&p.repo, &["status", "--porcelain"]);
    assert!(status.contains("?? .claude/") && !status.contains("CLAUDE.md"), "{status}");
    // Rules assigned too: now rules are generated (and CLAUDE.md asks first).
    project_assign(&p.dirs, &p.path(), None, &["library:style.md".into()], &[]).unwrap();
    assert_eq!(p.status().existing, vec!["CLAUDE.md", "AGENTS.md"], "now the hand-written files are offered for import");
    let g = project_generate_with(&p.dirs, &runner, &p.repo, &["claudecode".into()], &[], Vec::new).unwrap();
    assert_eq!(g.features, vec!["rules", "skills"]);
    assert_eq!(g.needs_confirm, vec!["CLAUDE.md"]);
    assert_eq!(std::fs::read_to_string(p.repo.join("CLAUDE.md")).unwrap(), "# Ours\n");
}

/// An agent's own skills are personal: generated where it works, kept out
/// of git, never over the project's committed skills.
#[test]
fn agents_get_personal_skills_kept_out_of_git() {
    let p = Proj::new("agent-skills");
    let Some(runner) = fake_rulesync(&p._t.0) else { return };
    let mine = skill(&p.dirs, "mine", "name: mine\ndescription: Mine\n", "Personal.\n");
    write(&mine.join("run.sh"), "#!/bin/sh\n");
    crate::platform::make_executable(&mine.join("run.sh")).unwrap();
    skill(&p.dirs, "shared", "name: shared\ndescription: Shared\n", "Shared, from the library.\n");
    write(&p.repo.join(".claude/skills/shared/SKILL.md"), "the project's committed skill\n");
    sh(&p.repo, &["add", "-A"]);
    sh(&p.repo, &["commit", "-qm", "skill"]);
    let set = save_set(&p.dirs, SaveRuleSet { id: None, name: "Me".into(), rule_ids: vec!["library:skills/mine".into(), "library:skills/shared".into()] }).unwrap();
    update(&p.dirs, |s| {
        s.agents.entry("agent-1".into()).or_default().rule_set_id = Some(set.id.clone());
        Ok(())
    })
    .unwrap();
    let c = main_ctx(&p);
    let res = apply_with(&p.dirs, Some(&runner), &LocalExec, &c, true).unwrap();
    assert!(res.generated.contains(&".claude/skills/mine/SKILL.md".to_string()), "{res:?}");
    assert!(crate::platform::is_executable(&p.repo.join(".claude/skills/mine/run.sh")), "the script stays executable");
    assert!(res.log.contains("kept the project's .claude/skills/shared/SKILL.md"), "{}", res.log);
    assert_eq!(std::fs::read_to_string(p.repo.join(".claude/skills/shared/SKILL.md")).unwrap(), "the project's committed skill\n");
    let exclude = std::fs::read_to_string(p.repo.join(".git/info/exclude")).unwrap();
    assert!(exclude.contains("/.claude/skills/mine/SKILL.md") && !exclude.contains("shared"), "{exclude}");
    assert_eq!(sh(&p.repo, &["status", "--porcelain"]), "");
    // Gemini CLI / Aider (Pitwall's own file) take no skills.
    let gem = AgentCtx { target: None, fallback: Some(builtin_fallback("gemini")), ..main_ctx(&p) };
    let res = apply_with(&p.dirs, None, &LocalExec, &gem, true).unwrap();
    assert!(res.log.contains("takes no skills"), "{}", res.log);
}

/// Importing a project brings in Claude Code's own skills too.
#[test]
fn importing_a_project_copies_its_claude_skills() {
    let p = Proj::new("import-skills");
    write(&p.repo.join(".claude/skills/lint/SKILL.md"), "---\nname: lint\ndescription: Lint it\n---\nRun the linter.\n");
    write(&p.repo.join(".claude/skills/lint/lint.sh"), "#!/bin/sh\n");
    let r = import(&p.dirs, ImportRequest { kind: "project".into(), source: p.path() }).unwrap();
    assert_eq!(r.added, vec!["library:skills/lint"]);
    assert!(p.dirs.library().join("skills/lint/lint.sh").is_file());
    let lib = library_files(&p.dirs);
    assert!(lib.iter().any(|f| f.id == "library:skills/lint" && f.skill && f.description.as_deref() == Some("Lint it")));
    // Again: the library's own is kept.
    let r = import(&p.dirs, ImportRequest { kind: "project".into(), source: p.path() }).unwrap();
    assert!(r.added.is_empty() && r.log.contains("kept the library's own lint"), "{}", r.log);
}

/// A git repository with only Claude Code's skills (no rulesync setup):
/// they are copied into the library; no source or clone is kept.
#[test]
fn importing_a_git_repo_copies_its_bare_claude_skills() {
    let p = Proj::new("import-git-skills");
    write(&p.repo.join(".claude/skills/lint/SKILL.md"), "---\nname: lint\ndescription: Lint it\n---\nRun the linter.\n");
    write(&p.repo.join(".claude/skills/lint/lint.sh"), "#!/bin/sh\n");
    crate::platform::make_executable(&p.repo.join(".claude/skills/lint/lint.sh")).unwrap();
    sh(&p.repo, &["add", "-A"]);
    sh(&p.repo, &["commit", "-qm", "skills"]);
    let r = import(&p.dirs, ImportRequest { kind: "git".into(), source: p.path() }).unwrap();
    assert_eq!(r.added, vec!["library:skills/lint"], "{}", r.log);
    assert!(crate::platform::is_executable(&p.dirs.library().join("skills/lint/lint.sh")));
    assert!(sources(&p.dirs).is_empty());
    assert!(std::fs::read_dir(p.dirs.sources()).unwrap().next().is_none(), "the clone is gone");
    // Nothing to import at all: an error.
    let empty = Proj::new("import-git-empty");
    let e = import(&p.dirs, ImportRequest { kind: "git".into(), source: empty.path() }).unwrap_err();
    assert!(e.contains(".claude/skills"), "{e}");
}

/// Both CLAUDE.md and AGENTS.md imported (rulesync makes each a root rule
/// for every tool): Pitwall gives each its own tools, so generating
/// reproduces each file as it was instead of merging them.
#[test]
fn importing_both_instruction_files_keeps_them_apart() {
    let p = Proj::new("import-both");
    let Some(runner) = fake_rulesync(&p._t.0) else { return };
    write(&p.repo.join("CLAUDE.md"), "FROM CLAUDE\n");
    write(&p.repo.join("AGENTS.md"), "FROM AGENTS\n");
    sh(&p.repo, &["add", "-A"]);
    sh(&p.repo, &["commit", "-qm", "both"]);
    project_assign(&p.dirs, &p.path(), None, &["library:testing.md".into()], &[]).unwrap();
    let r = project::import_existing(&p.repo, &runner, &p.dirs.run().join("i")).unwrap();
    assert_eq!(r.added, vec!["claude.md", "agents.md"]);
    let claude = std::fs::read_to_string(p.rule("claude.md")).unwrap();
    // Only `targets` changed: rulesync's other lines and the body stay.
    assert_eq!(claude, "---\nroot: true\ntargets: [\"claudecode\"]\nglobs:\n  - '**/*'\n---\nFROM CLAUDE\n");
    let agents = library::parse_front(&std::fs::read_to_string(p.rule("agents.md")).unwrap());
    assert!(agents.root && !agents.targets.contains(&"claudecode".to_string()) && agents.targets.contains(&"codexcli".to_string()), "{:?}", agents.targets);
    // Without the copied testing.md rule (it applies everywhere), each file comes back as it was.
    project_unassign(&p.path(), &["testing.md".into()], false).unwrap();
    let g = project_generate_with(&p.dirs, &runner, &p.repo, &["claudecode".into(), "codexcli".into()], &[], Vec::new).unwrap();
    assert_eq!(g.needs_confirm, Vec::<String>::new(), "{}", g.log);
    assert_eq!(std::fs::read_to_string(p.repo.join("CLAUDE.md")).unwrap(), "FROM CLAUDE\n");
    assert_eq!(std::fs::read_to_string(p.repo.join("AGENTS.md")).unwrap(), "FROM AGENTS\n");
}

/// Only AGENTS.md: its rule still reaches Claude Code.
#[test]
fn an_imported_agents_md_alone_still_reaches_claude() {
    let p = Proj::new("import-agents");
    let Some(runner) = fake_rulesync(&p._t.0) else { return };
    write(&p.repo.join("AGENTS.md"), "FROM AGENTS\n");
    sh(&p.repo, &["add", "-A"]);
    sh(&p.repo, &["commit", "-qm", "agents"]);
    project_assign(&p.dirs, &p.path(), None, &["library:testing.md".into()], &[]).unwrap();
    project::import_existing(&p.repo, &runner, &p.dirs.run().join("i")).unwrap();
    assert_eq!(library::parse_front(&std::fs::read_to_string(p.rule("agents.md")).unwrap()).targets, vec!["*"]);
    project_unassign(&p.path(), &["testing.md".into()], false).unwrap();
    project_generate_with(&p.dirs, &runner, &p.repo, &["claudecode".into(), "codexcli".into()], &[], Vec::new).unwrap();
    assert_eq!(std::fs::read_to_string(p.repo.join("CLAUDE.md")).unwrap(), "FROM AGENTS\n");
    assert_eq!(std::fs::read_to_string(p.repo.join("AGENTS.md")).unwrap(), "FROM AGENTS\n");
}

/// Imported in two runs (AGENTS.md first, CLAUDE.md later): the earlier
/// AGENTS.md rule stops going to Claude Code, if it's still as Pitwall
/// left it; changed since, it is kept and reported.
#[test]
fn importing_in_two_runs_narrows_the_earlier_rule() {
    for edited in [false, true] {
        let p = Proj::new("import-two-runs");
        let Some(runner) = fake_rulesync(&p._t.0) else { return };
        write(&p.repo.join("AGENTS.md"), "FROM AGENTS\n");
        sh(&p.repo, &["add", "-A"]);
        sh(&p.repo, &["commit", "-qm", "agents"]);
        project_assign(&p.dirs, &p.path(), None, &["library:testing.md".into()], &[]).unwrap();
        project::import_existing(&p.repo, &runner, &p.dirs.run().join("i")).unwrap();
        if edited {
            let path = p.rule("agents.md");
            let text = std::fs::read_to_string(&path).unwrap() + "Edited.\n";
            write(&path, &text);
        }
        write(&p.repo.join("CLAUDE.md"), "FROM CLAUDE\n");
        sh(&p.repo, &["add", "-A"]);
        sh(&p.repo, &["commit", "-qm", "claude"]);
        let r = project::import_existing(&p.repo, &runner, &p.dirs.run().join("j")).unwrap();
        assert_eq!(r.files, vec!["CLAUDE.md"]);
        let agents = library::parse_front(&std::fs::read_to_string(p.rule("agents.md")).unwrap()).targets;
        if edited {
            assert_eq!(agents, vec!["*"], "changed since the import: left alone");
            assert!(r.log.contains("kept agents.md: changed since it was imported"), "{}", r.log);
        } else {
            assert!(!agents.contains(&"claudecode".to_string()) && !agents.contains(&"*".to_string()), "{agents:?}");
            project_unassign(&p.path(), &["testing.md".into()], false).unwrap();
            let g = project_generate_with(&p.dirs, &runner, &p.repo, &["claudecode".into(), "codexcli".into()], &[], Vec::new).unwrap();
            assert!(g.needs_confirm.is_empty(), "{}", g.log);
            assert_eq!(std::fs::read_to_string(p.repo.join("CLAUDE.md")).unwrap(), "FROM CLAUDE\n");
            assert_eq!(std::fs::read_to_string(p.repo.join("AGENTS.md")).unwrap(), "FROM AGENTS\n");
        }
    }
}

#[test]
fn targets_are_set_without_touching_the_rest() {
    let s = |t: &[&str]| t.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(project::set_targets("---\nroot: true\ntargets:\n  - '*'\nglobs: []\n---\nbody\n", &s(&["claudecode"])), "---\nroot: true\ntargets: [\"claudecode\"]\nglobs: []\n---\nbody\n");
    assert_eq!(project::set_targets("---\nroot: true\n---\nbody", &s(&["a", "b"])), "---\nroot: true\ntargets: [\"a\", \"b\"]\n---\nbody");
    assert_eq!(project::set_targets("plain\n", &s(&["*"])), "---\ntargets: [\"*\"]\n---\nplain\n");
    let in_use = s(&["claudecode", "cursor"]);
    assert_eq!(project::imported_targets("CLAUDE.md", true, &in_use), s(&["claudecode"]));
    assert_eq!(project::imported_targets("AGENTS.md", false, &in_use), s(&["*"]));
    let a = project::imported_targets("AGENTS.md", true, &in_use);
    assert_eq!(a[0], "cursor");
    assert!(a.contains(&"codexcli".to_string()) && !a.contains(&"claudecode".to_string()));
    assert_eq!(project::imported_targets("GEMINI.md", true, &in_use), s(&["geminicli"]));
}

// ------------------------------------------------------------------ real rulesync
// `cargo test -p pitwall-core real_rulesync -- --ignored`: runs the
// `rulesync` on your PATH (never installs it) in fresh temp folders.

fn installed_rulesync() -> Runner {
    super::runner::runner(false).expect("these tests need `rulesync` on your PATH (npm install -g rulesync)")
}

/// Skills only: rulesync writes each target's skill folders (scripts stay
/// executable) and leaves the project's CLAUDE.md / AGENTS.md alone.
#[test]
#[ignore]
fn real_rulesync_skills_only() {
    let r = installed_rulesync();
    let p = Proj::new("real-skills");
    write(&p.repo.join("CLAUDE.md"), "# Ours\n");
    write(&p.repo.join("AGENTS.md"), "# Ours too\n");
    sh(&p.repo, &["add", "-A"]);
    sh(&p.repo, &["commit", "-qm", "instructions"]);
    let dir = skill(&p.dirs, "review", "name: review\ndescription: Review a diff\ntargets: [\"*\"]\n", "Read it.\n");
    write(&dir.join("run.sh"), "#!/bin/sh\necho ok\n");
    crate::platform::make_executable(&dir.join("run.sh")).unwrap();
    project_assign(&p.dirs, &p.path(), None, &["library:skills/review".into()], &[]).unwrap();
    let g = project_generate_with(&p.dirs, &r, &p.repo, &["claudecode".into(), "codexcli".into()], &[], Vec::new).unwrap();
    eprintln!("{g:?}");
    assert_eq!(g.features, vec!["skills"]);
    for f in [".claude/skills/review/SKILL.md", ".claude/skills/review/run.sh", ".agents/skills/review/SKILL.md", ".agents/skills/review/run.sh"] {
        assert!(g.written.contains(&f.to_string()), "{f} not in {:?}", g.written);
    }
    assert!(crate::platform::is_executable(&p.repo.join(".claude/skills/review/run.sh")), "the script stays executable");
    assert_eq!(std::fs::read_to_string(p.repo.join("CLAUDE.md")).unwrap(), "# Ours\n");
    assert_eq!(std::fs::read_to_string(p.repo.join("AGENTS.md")).unwrap(), "# Ours too\n");
    assert!(!p.status().generated.out_of_date);
}

/// Agent scope: shared rules to .claude/rules, its own to CLAUDE.local.md,
/// the tracked CLAUDE.md kept, nothing in git status.
#[test]
#[ignore]
fn real_rulesync_agent_local_split() {
    let r = installed_rulesync();
    let f = fixture();
    assign(&f, &["style.md", "base.md"], &["mine.md"]);
    let res = apply_with(&f.dirs, Some(&r), &LocalExec, &ctx(&f, "claudecode"), false).unwrap();
    eprintln!("{res:?}");
    assert_eq!(res.generated, vec![".claude/rules/style.md", "CLAUDE.local.md"]);
    assert!(std::fs::read_to_string(f.wt.join("CLAUDE.local.md")).unwrap().contains("Personal note."));
    assert_eq!(std::fs::read_to_string(f.wt.join("CLAUDE.md")).unwrap(), "# project claude\n");
    assert_eq!(sh(&f.wt, &["status", "--porcelain"]), "");
}

/// Hand-written CLAUDE.md and AGENTS.md imported, then generated: each
/// comes back as it was (not merged).
#[test]
#[ignore]
fn real_rulesync_two_file_import_roundtrip() {
    let r = installed_rulesync();
    let p = Proj::new("real-import");
    let claude = "# Claude\n\nFROM CLAUDE\n";
    let agents = "# Agents\n\nFROM AGENTS\n";
    write(&p.repo.join("CLAUDE.md"), claude);
    write(&p.repo.join("AGENTS.md"), agents);
    sh(&p.repo, &["add", "-A"]);
    sh(&p.repo, &["commit", "-qm", "both"]);
    project_assign(&p.dirs, &p.path(), None, &["library:testing.md".into()], &[]).unwrap();
    let i = project::import_existing(&p.repo, &r, &p.dirs.run().join("i")).unwrap();
    eprintln!("{i:?}");
    project_unassign(&p.path(), &["testing.md".into()], false).unwrap();
    let g = project_generate_with(&p.dirs, &r, &p.repo, &["claudecode".into(), "codexcli".into()], &[], Vec::new).unwrap();
    eprintln!("{g:?}");
    assert!(g.needs_confirm.is_empty(), "{}", g.log);
    assert_eq!(std::fs::read_to_string(p.repo.join("CLAUDE.md")).unwrap(), claude);
    assert_eq!(std::fs::read_to_string(p.repo.join("AGENTS.md")).unwrap(), agents);
}
