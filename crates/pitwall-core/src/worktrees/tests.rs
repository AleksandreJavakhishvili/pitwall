use super::*;
use crate::engine::lifecycle;
use crate::exec::LocalExec;
use crate::model::CreateAgentRequest;
use crate::testing::{record, FakeExec, Harness};
use crate::vcs::git::parse_worktree_list;
use crate::vcs::snapshot::TempRepo;

fn entry(path: &str) -> WorktreeEntry {
    WorktreeEntry { path: path.into(), head: Some("h".into()), branch: Some("b".into()), ..Default::default() }
}

fn spot(id: &str, folder: &str, dirs: &[&str], created_at: u64) -> Spot {
    Spot { id: id.into(), folder: folder.into(), tool_dirs: dirs.iter().map(|d| d.to_string()).collect(), created_at }
}

#[test]
fn porcelain_records_with_every_flag() {
    let out = "worktree /r\nHEAD aaa\nbranch refs/heads/main\n\n\
               worktree /r/.claude/worktrees/x\nHEAD bbb\ndetached\n\n\
               worktree /r/wt/locked\nHEAD ccc\nbranch refs/heads/feature/a\nlocked\n\n\
               worktree /r/wt/why\nHEAD ddd\nbranch refs/heads/b\nlocked \"on a\\ndisk\"\n\n\
               worktree /gone\nHEAD eee\nbranch refs/heads/g\nprunable gitdir file points to non-existent location\n\n";
    let got = parse_worktree_list(out);
    assert_eq!(got.len(), 5);
    assert_eq!((got[0].branch.as_deref(), got[0].head.as_deref(), got[0].detached), (Some("main"), Some("aaa"), false));
    assert_eq!((got[1].branch.as_deref(), got[1].detached), (None, true));
    assert_eq!((got[2].branch.as_deref(), got[2].locked, got[2].lock_reason.as_deref()), (Some("feature/a"), true, None));
    assert_eq!(got[3].lock_reason.as_deref(), Some("on a\ndisk"));
    assert!(got[4].prunable && !got[4].locked);

    let bare = parse_worktree_list("worktree /srv/r.git\nbare\n\nworktree /srv/w\nHEAD f\nbranch refs/heads/w\n");
    assert!(bare[0].bare && bare[0].head.is_none() && bare[0].branch.is_none());
    assert_eq!(bare[1].branch.as_deref(), Some("w"));
    assert!(parse_worktree_list("").is_empty());
    assert!(parse_worktree_list("HEAD orphan\n").is_empty(), "attributes before a record are ignored");
}

#[test]
fn attribution_follows_the_rules_in_order() {
    let entries = vec![
        entry("/r"),
        entry("/r/.claude/worktrees/sub"),
        entry("/r/.claude/worktrees/own"),
        entry("/r-proc"),
        entry("/r-other"),
        entry("/r/.claudeX/worktrees/no"),
    ];
    let spots = vec![
        spot("main", "/r/src", &[".claude/worktrees"], 1),
        spot("own", "/r/.claude/worktrees/own/deep", &[".claude/worktrees"], 2),
        spot("shell", "/r", &[], 3),
    ];
    let procs = vec![("shell".to_string(), "/r-proc/pkg".to_string())];
    let got = attribute(&entries, &spots, &procs);
    let who = |i: usize| (got[i].0.as_deref(), got[i].1);
    assert_eq!(who(0), (Some("main"), WorktreeVia::Own), "main checkout: the oldest agent in it");
    assert_eq!(who(1), (Some("main"), WorktreeVia::ToolDir), "under the main agent's .claude/worktrees");
    assert_eq!(who(2), (Some("own"), WorktreeVia::Own), "an agent's own folder wins over a tool dir");
    assert_eq!(who(3), (Some("shell"), WorktreeVia::Process));
    assert_eq!(who(4), (None, WorktreeVia::Other));
    assert_eq!(who(5), (None, WorktreeVia::Other), "a lookalike folder is not the tool dir");

    // Without the process: nobody's.
    assert_eq!(attribute(&entries, &spots, &[])[3], (None, WorktreeVia::Other));
}

#[test]
fn ties_in_a_tool_dir_go_to_the_agent_working_there() {
    let entries = vec![entry("/r"), entry("/r/.claude/worktrees/a"), entry("/r/.claude/worktrees/b")];
    let spots = vec![spot("old", "/r", &[".claude/worktrees"], 1), spot("new", "/r", &[".claude/worktrees"], 2)];
    let procs = vec![("new".to_string(), "/r/.claude/worktrees/b/x".to_string())];
    let got = attribute(&entries, &spots, &procs);
    assert_eq!(got[1], (Some("old".into()), WorktreeVia::ToolDir), "no process there: the oldest agent");
    assert_eq!(got[2], (Some("new".into()), WorktreeVia::ToolDir), "its process works there");
    // Tool dirs never escape the checkout.
    let sneaky = vec![spot("s", "/r", &["../elsewhere", "/"], 1)];
    let got = attribute(&[entry("/r"), entry("/elsewhere/w")], &sneaky, &[]);
    assert_eq!(got[1], (None, WorktreeVia::Other));
}

/// A temp repo with: `.claude/worktrees/sub` (branch worktree-sub), `wt/proc`
/// (branch proc), `wt/locked` (locked, branch locked), `wt/detached`.
fn repo() -> TempRepo {
    let r = TempRepo::new();
    r.write("a.txt", "1\n");
    r.commit_all("init");
    for (dir, branch) in [(".claude/worktrees/sub", "worktree-sub"), ("wt/proc", "proc"), ("wt/locked", "locked")] {
        r.git(&["worktree", "add", "-q", "-b", branch, r.0.join(dir).to_str().unwrap(), "HEAD"]);
    }
    r.git(&["worktree", "add", "-q", "--detach", r.0.join("wt/detached").to_str().unwrap(), "HEAD"]);
    r.git(&["worktree", "lock", "--reason", "on a stick", r.0.join("wt/locked").to_str().unwrap()]);
    r.write(".gitignore", ".claude/\nwt/\n");
    r.commit_all("ignore worktrees");
    r
}

fn req(kind: &str, path: &str) -> CreateAgentRequest {
    CreateAgentRequest { name: kind.into(), kind: kind.into(), project_path: path.into(), ..Default::default() }
}

fn by_name<'a>(p: &'a ProjectWorktrees, name: &str) -> &'a WorktreeView {
    p.worktrees.iter().find(|w| w.name == name).unwrap_or_else(|| panic!("no worktree {name}: {:?}", p.worktrees))
}

#[test]
fn a_real_repo_is_listed_attributed_diffed_merged_and_cleaned_up() {
    let r = repo();
    let real = |rel: &str| LocalExec.real_path(r.0.join(rel).to_str().unwrap()).unwrap();
    let h = Harness::new(vec![]);
    let claude = lifecycle::create(&h.engine, req("claude", r.path())).unwrap();
    let shell = lifecycle::create(&h.engine, req("shell", r.path())).unwrap();
    // The shell's process went into a worktree (what `process_cwd` sees).
    h.provider.set_cwd(&shell.id, &format!("{}/sub", real("wt/proc")));
    std::fs::create_dir_all(r.0.join("wt/proc/sub")).unwrap();

    let projects = list(&h.engine);
    assert_eq!(projects.len(), 1, "{projects:?}");
    let p = &projects[0];
    assert_eq!((p.repo.as_str(), p.branch.as_deref()), (real("").trim_end_matches('/'), Some("main")));
    assert_eq!(p.agent_ids.len(), 2);
    assert_eq!(p.worktrees.len(), 4, "the main checkout is the project itself");
    let sub = by_name(p, "sub");
    assert_eq!((sub.agent_id.as_deref(), sub.via, sub.branch.as_deref()), (Some(claude.id.as_str()), WorktreeVia::ToolDir, Some("worktree-sub")));
    assert!(sub.caps.diff && sub.caps.commit && sub.caps.merge && sub.caps.remove && sub.caps.terminal);
    let pr = by_name(p, "proc");
    assert_eq!((pr.agent_id.as_deref(), pr.via), (Some(shell.id.as_str()), WorktreeVia::Process));
    let locked = by_name(p, "locked");
    assert_eq!((locked.via, locked.locked, locked.lock_reason.as_deref()), (WorktreeVia::Other, true, Some("on a stick")));
    assert!(!locked.caps.remove && locked.caps.merge);
    let det = by_name(p, "detached");
    assert!(det.branch.is_none() && !det.caps.merge && det.caps.remove);

    // Changes: against the merge-base with main, committed + uncommitted +
    // untracked, while main moved on — and no index is touched to read them.
    let sub_path = sub.path.clone();
    std::fs::write(r.0.join(".claude/worktrees/sub/b.txt"), "agent\n").unwrap();
    r.git(&["-C", &sub_path, "add", "b.txt"]);
    r.git(&["-C", &sub_path, "commit", "-q", "-m", "agent"]);
    std::fs::write(r.0.join(".claude/worktrees/sub/a.txt"), "1\n2\n").unwrap();
    std::fs::write(r.0.join(".claude/worktrees/sub/new.txt"), "n\n").unwrap();
    r.write("main-only.txt", "main\n");
    r.commit_all("main moves on");
    let index = r.0.join(".git/worktrees/sub/index");
    let before = (std::fs::read(&index).unwrap(), std::fs::metadata(&index).unwrap().modified().unwrap());
    let files = changes(&h.engine, &p.id, &sub_path).unwrap();
    let names: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(names, ["a.txt", "b.txt", "new.txt"]);
    assert!(files[2].untracked);
    assert_eq!((std::fs::read(&index).unwrap(), std::fs::metadata(&index).unwrap().modified().unwrap()), before);
    let v = file_versions(&h.engine, &p.id, &sub_path, "a.txt").unwrap();
    assert_eq!((v.original.as_deref(), v.modified.as_deref()), (Some("1\n"), Some("1\n2\n")));
    assert!(changes(&h.engine, &p.id, "/not/listed").unwrap_err().contains("isn't listed"));
    assert!(changes(&h.engine, "nope", &sub_path).is_err());

    // Commit, then merge (refused while the main checkout is dirty).
    let st = merge_status(&h.engine, &p.id, &sub_path).unwrap();
    assert_eq!((st.worktree, st.uncommitted, st.ahead, st.target.as_deref()), (true, 2, 1, Some("main")));
    commit(&h.engine, &p.id, &sub_path, "more agent work").unwrap();
    r.write("a.txt", "dirty\n");
    let res = merge(&h.engine, &p.id, &sub_path).unwrap();
    assert!(!res.merged && res.message.contains("uncommitted"), "{res:?}");
    r.git(&["checkout", "--", "a.txt"]);
    let res = merge(&h.engine, &p.id, &sub_path).unwrap();
    assert!(res.merged, "{res:?}");
    assert!(r.0.join("new.txt").exists());
    assert!(merge(&h.engine, &p.id, &by_name(p, "detached").path).unwrap_err().contains("detached"));

    // Remove: never a locked one, never with uncommitted changes (no --force).
    assert!(remove(&h.engine, &p.id, &locked.path).unwrap_err().contains("locked"));
    assert!(r.0.join("wt/locked").exists());
    std::fs::write(r.0.join("wt/proc/dirty.txt"), "x\n").unwrap();
    assert!(remove(&h.engine, &p.id, &pr.path).is_err(), "git refuses without --force");
    assert!(r.0.join("wt/proc/dirty.txt").exists());
    remove(&h.engine, &p.id, &sub_path).unwrap();
    assert!(!r.0.join(".claude/worktrees/sub").exists());
    assert!(r.git(&["branch", "--list", "worktree-sub"]).contains("worktree-sub"), "the branch is kept");

    // It leaves the list.
    let p = list(&h.engine).remove(0);
    assert!(p.worktrees.iter().all(|w| w.name != "sub"));
    assert_eq!(p.worktrees.len(), 3);
}

#[test]
fn an_agents_own_worktree_is_its_folder_and_not_removed_here() {
    let r = repo();
    let wt = LocalExec.real_path(r.0.join("wt/proc").to_str().unwrap()).unwrap();
    let mut rec = record("a", &wt);
    rec.project = r.path().into();
    let h = Harness::new(vec![rec]);
    let p = list(&h.engine).remove(0);
    let own = by_name(&p, "proc");
    assert_eq!((own.agent_id.as_deref(), own.via), (Some("a"), WorktreeVia::Own));
    assert!(!own.caps.remove && own.caps.commit);
    assert!(remove(&h.engine, &p.id, &own.path).unwrap_err().contains("remove the agent"));
    assert!(r.0.join("wt/proc").exists());
    // A stopped shell has no process to ask: the rest are other worktrees.
    assert!(p.worktrees.iter().filter(|w| w.name != "proc").all(|w| w.via == WorktreeVia::Other));
}

const LIST: [&str; 6] = ["git", "-C", "/r", "worktree", "list", "--porcelain"];

fn fake() -> std::sync::Arc<FakeExec> {
    let x = FakeExec::new();
    x.dir("/r").on(&LIST, "worktree /r\nHEAD a\nbranch refs/heads/main\n\nworktree /r/.claude/worktrees/w\nHEAD b\nbranch refs/heads/worktree-w\n\n");
    x
}

fn claude_at(id: &str, cwd: &str) -> crate::model::AgentRecord {
    let mut rec = record(id, cwd);
    rec.kind = "claude".into();
    rec
}

#[test]
fn one_list_per_project_per_refresh_then_only_when_due() {
    let x = fake();
    let h = Harness::with_exec(vec![claude_at("a", "/r"), claude_at("b", "/r")], x.clone());
    let lists = || x.ran(&LIST);
    let p = list(&h.engine);
    assert_eq!((p.len(), lists()), (1, 1), "two agents, one project, one list");
    assert_eq!(p[0].worktrees[0].via, WorktreeVia::ToolDir);
    assert_eq!(p[0].worktrees[0].agent_id.as_deref(), Some("a"), "the older agent");
    list(&h.engine);
    assert_eq!(lists(), 1, "nothing changed");
    // Something changed in the project: listed again, but not more than every MIN_GAP_MS.
    h.engine.with("a", |a| a.added = 5).unwrap();
    list(&h.engine);
    assert_eq!(lists(), 1, "too soon after the last list");
    h.clock.advance(MIN_GAP_MS);
    list(&h.engine);
    assert_eq!(lists(), 2);
    // And every ~30 s while asked.
    h.clock.advance(PERIOD_MS - PERIOD_SLACK_MS);
    list(&h.engine);
    assert_eq!(lists(), 3);
    assert!(x.calls().iter().all(|c| c.env_has("GIT_OPTIONAL_LOCKS", "0")));
    // Nothing else was run: no diffs until a worktree is shown.
    assert_eq!(x.ran(&["git", ".."]), 3);
}

#[test]
fn slow_machines_are_listed_at_their_pace() {
    let x = fake();
    let h = Harness::with_exec(vec![claude_at("a", "/r")], x.clone());
    let mut caps = crate::testing::FakeProvider::local_like();
    caps.git_poll_ms = 15_000;
    h.provider.set_caps(caps);
    h.engine.with("a", |a| a.facts.provider = caps).unwrap();
    list(&h.engine);
    h.engine.with("a", |a| a.added = 1).unwrap();
    h.clock.advance(MIN_GAP_MS);
    list(&h.engine);
    assert_eq!(x.ran(&LIST), 1, "a change waits for git_poll_ms");
    h.clock.advance(15_000);
    list(&h.engine);
    assert_eq!(x.ran(&LIST), 2);
}

#[test]
fn no_listing_without_exec_or_outside_a_repo() {
    let x = fake();
    let h = Harness::with_exec(vec![claude_at("a", "/r"), claude_at("b", "/elsewhere")], x.clone());
    h.engine.with("b", |a| a.git_repo = Some(false)).unwrap();
    let mut caps = crate::testing::FakeProvider::local_like();
    caps.exec = false;
    h.engine.with("a", |a| a.facts.provider = caps).unwrap();
    assert!(list(&h.engine).is_empty());
    assert_eq!(x.ran(&["git", ".."]), 0);
    let v = h.engine.views();
    assert!(!v[0].caps.worktrees && !v[1].caps.worktrees);
}

#[test]
fn a_failed_list_keeps_the_last_one_and_says_why() {
    let x = fake();
    let h = Harness::with_exec(vec![claude_at("a", "/r")], x.clone());
    assert_eq!(list(&h.engine)[0].worktrees.len(), 1);
    x.on_exit(&LIST, 128, "", "fatal: machine went away");
    h.clock.advance(PERIOD_MS);
    let p = list(&h.engine).remove(0);
    assert_eq!(p.worktrees.len(), 1);
    assert!(p.error.as_deref().unwrap().contains("went away"));
}

#[test]
fn a_forced_refresh_lists_again_whatever_the_pace() {
    let x = fake();
    let h = Harness::with_exec(vec![claude_at("a", "/r")], x.clone());
    let mut caps = crate::testing::FakeProvider::local_like();
    caps.git_poll_ms = 15_000;
    h.provider.set_caps(caps);
    h.engine.with("a", |a| a.facts.provider = caps).unwrap();
    let id = list(&h.engine)[0].id.clone();
    list(&h.engine);
    assert_eq!(x.ran(&LIST), 1, "not due yet");
    // The user asks: listed now, even on a slow machine just listed.
    let p = refresh_now(&h.engine, Some(&id));
    assert_eq!((p.len(), x.ran(&LIST)), (1, 2));
    refresh_now(&h.engine, None);
    assert_eq!(x.ran(&LIST), 3, "every project");
    // Another project's id forces nothing here.
    refresh_now(&h.engine, Some("local:this-mac:/elsewhere"));
    assert_eq!(x.ran(&LIST), 3);
    // A plain list afterwards is still paced.
    list(&h.engine);
    assert_eq!(x.ran(&LIST), 3);
}

#[test]
fn a_forced_refresh_asked_twice_lists_once() {
    let x = fake();
    let h = Harness::with_exec(vec![claude_at("a", "/r")], x.clone());
    list(&h.engine);
    // Another forced refresh of every project is running: the caller waits for it.
    let (go, wait) = std::sync::mpsc::channel::<()>();
    let leader = {
        let e = h.engine.clone();
        std::thread::spawn(move || {
            e.worktrees.forced.run("*", || {
                wait.recv().unwrap();
                refresh(&e, Force::All)
            })
        })
    };
    while !h.engine.worktrees.forced.running("*") {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let follower = {
        let e = h.engine.clone();
        std::thread::spawn(move || refresh_now(&e, None))
    };
    while h.engine.worktrees.forced.waiters("*") == 0 {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    go.send(()).unwrap();
    assert_eq!(leader.join().unwrap(), follower.join().unwrap());
    assert_eq!(x.ran(&LIST), 2, "one forced list for both");
}
