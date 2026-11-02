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

fn spot(id: &str, folder: &str, created_at: u64) -> Spot {
    Spot { id: id.into(), folder: folder.into(), created_at, worked: vec![], tracked: vec![] }
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
        spot("main", "/r/src", 1),
        spot("own", "/r/.claude/worktrees/own/deep", 2),
        spot("shell", "/r", 3),
    ];
    let procs = vec![("shell".to_string(), "/r-proc/pkg".to_string())];
    let got = attribute(&entries, &spots, &procs);
    let who = |i: usize| (got[i].0.as_deref(), got[i].1);
    assert_eq!(who(0), (Some("main"), WorktreeVia::Own), "main checkout: the oldest agent in it");
    assert_eq!(who(1), (None, WorktreeVia::Other), "where it lives says nothing: no hooks, no process");
    assert_eq!(who(2), (Some("own"), WorktreeVia::Own), "an agent's own folder");
    assert_eq!(who(3), (Some("shell"), WorktreeVia::Process));
    assert_eq!(who(4), (None, WorktreeVia::Other));
    assert_eq!(who(5), (None, WorktreeVia::Other));

    // Without the process: nobody's.
    assert_eq!(attribute(&entries, &spots, &[])[3], (None, WorktreeVia::Other));
}

#[test]
fn a_shared_folder_s_worktrees_are_never_guessed() {
    let entries = vec![entry("/r"), entry("/r/.claude/worktrees/a"), entry("/r/.claude/worktrees/b")];
    let spots = vec![spot("old", "/r", 1), spot("new", "/r", 2)];
    let procs = vec![("new".to_string(), "/r/.claude/worktrees/b/x".to_string())];
    let got = attribute(&entries, &spots, &procs);
    assert_eq!(got[1], (None, WorktreeVia::Other), "either could have made it: not the oldest's");
    assert_eq!(got[2], (Some("new".into()), WorktreeVia::Process), "its process works (or worked) there");
    // Even one agent alone isn't given a worktree it never worked in.
    assert_eq!(attribute(&entries, &spots[..1], &[])[1], (None, WorktreeVia::Other));
}

#[test]
fn the_agent_whose_hooks_worked_in_a_worktree_owns_it() {
    let entries = vec![
        entry("/r"),
        entry("/r/.claude/worktrees/a"),
        entry("/r/.claude/worktrees/b"),
        entry("/r/.claude/worktrees/left"),
        entry("/home/u/.codex/worktrees/7/r"),
    ];
    let worked = |v: &[(&str, u32)]| v.iter().map(|(p, n)| (p.to_string(), f64::from(*n), 5)).collect::<Vec<_>>();
    let mut a = spot("a", "/r", 1);
    a.worked = worked(&[("/r", 40), ("/r/.claude/worktrees/a", 12), ("/r/.claude/worktrees/a/src", 3), ("/r/.claude/worktrees/b", 1)]);
    let mut b = spot("b", "/r", 2);
    b.worked = worked(&[("/r/.claude/worktrees/b", 9)]);
    let mut c = spot("c", "/r", 3);
    c.worked = worked(&[("/home/u/.codex/worktrees/7/r", 2)]);
    let got = attribute(&entries, &[a, b, c], &[("a".into(), "/r/.claude/worktrees/b".into())]);
    let who = |i: usize| (got[i].0.as_deref(), got[i].1);
    assert_eq!(who(0), (Some("a"), WorktreeVia::Own), "everyone works in the main checkout");
    assert_eq!(who(1), (Some("a"), WorktreeVia::WorkedIn), "subfolders count for their worktree");
    assert_eq!(who(2), (Some("b"), WorktreeVia::WorkedIn), "most steps there, over a visit or a process");
    assert_eq!(who(3), (None, WorktreeVia::Other), "made by someone else (a session outside Pitwall): nobody's here");
    assert_eq!(who(4), (Some("c"), WorktreeVia::WorkedIn), "worktrees outside the project (Codex's) too");
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

    // Claude worked in its sub-agent's worktree (its hooks say so).
    h.engine.ingest_hook(&claude.id, &serde_json::json!({"hook_event_name": "PostToolUse", "cwd": real(".claude/worktrees/sub")}));

    let projects = list(&h.engine);
    assert_eq!(projects.len(), 1, "{projects:?}");
    let p = &projects[0];
    assert_eq!((p.repo.as_str(), p.branch.as_deref()), (real("").trim_end_matches('/'), Some("main")));
    assert_eq!(p.agent_ids.len(), 2);
    assert_eq!(p.worktrees.len(), 4, "the main checkout is the project itself");
    let sub = by_name(p, "sub");
    assert_eq!((sub.agent_id.as_deref(), sub.via, sub.branch.as_deref()), (Some(claude.id.as_str()), WorktreeVia::WorkedIn, Some("worktree-sub")));
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
    assert_eq!(
        (p[0].worktrees[0].via, p[0].worktrees[0].agent_id.as_deref()),
        (WorktreeVia::Other, None),
        "either agent could have made it, and neither's conversation worked there"
    );
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

#[test]
fn hooks_say_whose_worktree_it_is_and_it_is_kept() {
    let x = fake();
    let h = Harness::with_exec(vec![claude_at("a", "/r"), claude_at("b", "/r")], x.clone());
    let hook = |id: &str, cwd: &str| h.engine.ingest_hook(id, &serde_json::json!({"hook_event_name": "PostToolUse", "cwd": cwd}));
    hook("a", "/r");
    hook("b", "/r/.claude/worktrees/w/src");
    hook("b", "/r/.claude/worktrees/w");
    let p = list(&h.engine);
    assert_eq!(
        (p[0].worktrees[0].via, p[0].worktrees[0].agent_id.as_deref()),
        (WorktreeVia::WorkedIn, Some("b")),
        "the younger agent worked there, whoever is older"
    );
    let b = h.engine.agents().iter().find(|a| a.rec.id == "b").unwrap().rec.worked_in.clone();
    let paths: Vec<&str> = b.iter().map(|w| w.path.as_str()).collect();
    assert_eq!(paths, ["/r/.claude/worktrees/w", "/r/.claude/worktrees/w/src"], "kept on the agent's record, most recent first");
    assert!(p[0].worktrees[0].active_at.is_some_and(|t| t >= b[0].last), "when it was last worked in");

    // A temporary worktree, worked in, then removed: forgotten.
    x.on(&LIST, "worktree /r\nHEAD a\nbranch refs/heads/main\n\nworktree /r/.claude/worktrees/w\nHEAD b\nbranch refs/heads/worktree-w\n\nworktree /r/.claude/worktrees/temp\nHEAD c\ndetached\n\n");
    h.clock.advance(PERIOD_MS);
    list(&h.engine);
    hook("b", "/r/.claude/worktrees/temp");
    hook("b", "/r/somewhere");
    x.on(&LIST, "worktree /r\nHEAD a\nbranch refs/heads/main\n\nworktree /r/.claude/worktrees/w\nHEAD b\nbranch refs/heads/worktree-w\n\n");
    h.clock.advance(PERIOD_MS);
    list(&h.engine);
    let b = h.engine.agents().iter().find(|a| a.rec.id == "b").unwrap().rec.worked_in.clone();
    assert!(b.iter().all(|w| w.path != "/r/.claude/worktrees/temp"), "{b:?}");
    assert!(b.iter().any(|w| w.path == "/r/.claude/worktrees/w"));
    assert!(b.iter().any(|w| w.path == "/r/somewhere"), "other folders age out on their own");
}

#[test]
fn worked_in_counts_moves_up_and_is_capped() {
    use crate::model::WorkedIn;
    let mut l = Vec::new();
    assert!(WorkedIn::note(&mut l, "/a/", 10), "a new folder: save now");
    assert!(!WorkedIn::note(&mut l, "/a", 10));
    assert!(WorkedIn::note(&mut l, "/b", 10));
    assert!(!WorkedIn::note(&mut l, "/a", 10));
    assert!(!WorkedIn::note(&mut l, "", 10));
    assert_eq!(l.iter().map(|w| (w.path.as_str(), w.score, w.last)).collect::<Vec<_>>(), [("/a", 3.0, 10), ("/b", 1.0, 10)]);
    for i in 0..WorkedIn::MAX + 5 {
        WorkedIn::note(&mut l, &format!("/n{i}"), 10);
    }
    assert_eq!(l.len(), WorkedIn::MAX);
    assert_eq!(l[0].path, format!("/n{}", WorkedIn::MAX + 4));
}

#[test]
fn old_work_counts_less_than_this_week_s() {
    use crate::model::WorkedIn;
    let day = 24 * 3_600_000u64;
    let mut l = Vec::new();
    for _ in 0..8 {
        WorkedIn::note(&mut l, "/w", 0);
    }
    assert!((l[0].score_at(2 * day) - 4.0).abs() < 1e-9, "halved after two days");
    WorkedIn::note(&mut l, "/w", 2 * day);
    assert!((l[0].score - 5.0).abs() < 1e-9 && l[0].last == 2 * day);

    // Heavy work a month ago loses to a few steps today.
    let entries = vec![entry("/r"), entry("/r/wt/x")];
    let mut old = spot("old", "/r", 1);
    let mut new = spot("new", "/r", 2);
    let now = 30 * day;
    old.worked = vec![("/r/wt/x".into(), WorkedIn { path: String::new(), score: 300.0, last: 0 }.score_at(now), 0)];
    new.worked = vec![("/r/wt/x".into(), 4.0, now)];
    assert_eq!(attribute(&entries, &[old.clone(), new.clone()], &[])[1].0.as_deref(), Some("new"));
    // A visit (close scores): whoever was there last.
    old.worked = vec![("/r/wt/x".into(), 1.2, now - day)];
    new.worked = vec![("/r/wt/x".into(), 1.0, now)];
    assert_eq!(attribute(&entries, &[old.clone(), new.clone()], &[])[1].0.as_deref(), Some("new"));
    // Real work beats a later visit.
    old.worked = vec![("/r/wt/x".into(), 40.0, now - day)];
    assert_eq!(attribute(&entries, &[old, new], &[])[1].0.as_deref(), Some("old"));
}

#[test]
fn worktrees_named_in_a_tool_s_input_count_where_the_cwd_stays() {
    let x = FakeExec::new();
    x.dir("/r").on(
        &LIST,
        "worktree /r\nHEAD a\nbranch refs/heads/main\n\nworktree /r-panel\nHEAD b\nbranch refs/heads/panel\n\nworktree /r-fair\nHEAD c\nbranch refs/heads/fair\n\n",
    );
    let h = Harness::with_exec(vec![claude_at("a", "/r"), claude_at("b", "/r")], x.clone());
    list(&h.engine);
    let hook = |id: &str, input: serde_json::Value| {
        h.engine.ingest_hook(id, &serde_json::json!({"hook_event_name": "PreToolUse", "cwd": "/r", "tool_input": input}))
    };
    // Claude's `cd` into a folder outside the project lasts one command.
    hook("a", serde_json::json!({"command": "cd ../r-panel && npm test"}));
    hook("a", serde_json::json!({"file_path": "/r-panel/src/app.ts", "old_string": "x"}));
    hook("b", serde_json::json!({"command": "git -C /r-fair/./pkg/.. status; git worktree list"}));
    // Just listing them names none.
    hook("b", serde_json::json!({"command": "git worktree list && ls .."}));
    h.clock.advance(PERIOD_MS);
    let p = list(&h.engine);
    let who = |name: &str| {
        let w = p[0].worktrees.iter().find(|w| w.name == name).unwrap();
        (w.agent_id.clone(), w.via)
    };
    assert_eq!(who("r-panel"), (Some("a".into()), WorktreeVia::WorkedIn));
    assert_eq!(who("r-fair"), (Some("b".into()), WorktreeVia::WorkedIn));
    let a = h.engine.agents().iter().find(|a| a.rec.id == "a").unwrap().rec.worked_in.clone();
    let panel = a.iter().find(|w| w.path == "/r-panel").unwrap();
    assert!((panel.score - 2.0).abs() < 1e-6, "counted once per hook, as the worktree: {a:?}");
    assert!(a.iter().all(|w| !w.path.contains("src")), "files count for their worktree, not themselves");
}

#[test]
fn paths_are_read_from_any_text() {
    assert_eq!(paths_in("cd ../x && cat './y/z' \"/abs/p\" ~/h", "/w/r"), vec!["/w/x".to_string(), "/w/r/y/z".into(), "/abs/p".into(), format!("{}/h", crate::paths::home().to_string_lossy())]);
    assert_eq!(tidy("/a/./b/../c/"), Some("/a/c".into()));
    assert_eq!(tidy("/.."), None);
    assert!(paths_in("git worktree list; ls", "/w").is_empty());
}

#[test]
fn an_agent_s_word_beats_every_guess() {
    let entries = vec![entry("/r"), entry("/r-bet"), entry("/r-own")];
    let mut said = spot("said", "/r", 1);
    said.tracked = vec!["/r-bet/src".into()];
    let mut busy = spot("busy", "/r", 2);
    busy.worked = vec![("/r-bet".into(), 50.0, 9)];
    let mut own = spot("own", "/r-own", 3);
    own.tracked = vec!["/r".into()];
    let procs = vec![("busy".to_string(), "/r-bet".to_string())];
    let got = attribute(&entries, &[said.clone(), busy.clone(), own.clone()], &procs);
    assert_eq!((got[1].0.as_deref(), got[1].1), (Some("said"), WorktreeVia::Tracked), "a folder inside it counts; hooks and processes don't win");
    assert_eq!((got[2].0.as_deref(), got[2].1), (Some("own"), WorktreeVia::Own), "an agent's own folder stays its");
    assert_eq!(got[0].1, WorktreeVia::Own, "tracking the main checkout changes nothing: it is everyone's");

    // Two said so: the newest word wins.
    busy.tracked = vec!["/r-bet".into()];
    said.tracked = vec!["/r-x".into(), "/r-bet".into()];
    let got = attribute(&entries, &[said, busy], &[]);
    assert_eq!((got[1].0.as_deref(), got[1].1), (Some("busy"), WorktreeVia::Tracked));
}

#[test]
fn tracking_keeps_the_newest_first_and_untracking_forgets() {
    let r = repo();
    let real = |rel: &str| LocalExec.real_path(r.0.join(rel).to_str().unwrap()).unwrap();
    let h = Harness::new(vec![]);
    let a = lifecycle::create(&h.engine, req("claude", r.path())).unwrap().id;
    assert!(track(&h.engine, &a, "relative/x", true).is_err(), "only full paths");
    track(&h.engine, &a, "/r-one/", true).unwrap();
    assert_eq!(track(&h.engine, &a, "/r-two", true).unwrap(), ["/r-two", "/r-one"]);
    assert_eq!(track(&h.engine, &a, "/r-one", true).unwrap(), ["/r-one", "/r-two"], "again: moves to the front");
    assert_eq!(track(&h.engine, &a, "/r-two", false).unwrap(), ["/r-one"]);
    assert_eq!(tracked(&h.engine, &a).unwrap(), ["/r-one"]);

    // Once listed, a folder inside a worktree counts as the worktree.
    let projects = list(&h.engine);
    assert_eq!(by_name(&projects[0], "proc").via, WorktreeVia::Other);
    assert_eq!(track(&h.engine, &a, &format!("{}/deep", real("wt/proc")), true).unwrap()[0], real("wt/proc"));
    let projects = list(&h.engine);
    let w = by_name(&projects[0], "proc");
    assert_eq!((w.agent_id.as_deref(), w.via), (Some(a.as_str()), WorktreeVia::Tracked));
}
