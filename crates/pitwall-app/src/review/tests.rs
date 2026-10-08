//! Review end to end on a made-up git repository and a test engine.

use std::path::{Path, PathBuf};
use std::process::Command;

use gpui::{Entity, TestAppContext, VisualTestContext};
use pitwall_core::testing::{record, Harness};

use super::*;
use crate::theme::Theme;

/// A throwaway repository (deleted on drop).
struct Repo(PathBuf);

impl Repo {
    fn new() -> Repo {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pw-review-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let r = Repo(dir);
        r.git(&["init", "-q", "--template=", "-b", "main"]);
        // The app's own commits (the Commit dialog) need an identity too:
        // CI runners and fresh containers have no global one.
        r.git(&["config", "user.name", "Test"]);
        r.git(&["config", "user.email", "test@example.com"]);
        r.git(&["config", "commit.gpgsign", "false"]);
        r
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn write(&self, rel: &str, text: &str) {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn read(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(Path::new(&self.0).join(rel)).ok()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open<'a>(
    cx: &'a mut TestAppContext,
    h: &Harness,
) -> (Entity<ReviewView>, &'a mut VisualTestContext) {
    let engine = h.engine.clone();
    cx.update(|cx| {
        cx.set_global(Theme::light());
        cx.set_global(ReviewEngine {
            engine: engine.clone(),
            root: None,
        });
        register(cx);
    });
    let store = cx.new(|_| AgentStore::new(engine.views()));
    let (view, cx) =
        cx.add_window_view(|window, cx| ReviewView::new(store, Space::All, None, None, window, cx));
    cx.run_until_parked();
    (view, cx)
}

#[gpui::test]
fn lists_an_agents_changes_since_its_base_and_shows_the_diff(cx: &mut TestAppContext) {
    let r = Repo::new();
    r.write("src/a.rs", "fn a() {}\n");
    r.write("README.md", "# x\n");
    r.git(&["add", "-A"]);
    r.git(&["commit", "-q", "-m", "init"]);
    let mut rec = record("agent-1", r.path());
    rec.base_commit = Some(r.git(&["rev-parse", "HEAD"]));
    let h = Harness::new(vec![rec]);
    r.write("src/a.rs", "fn a() { გამარჯობა(); }\n");
    r.write("notes/new.md", "new\n");

    let (view, cx) = open(cx, &h);
    view.read_with(cx, |v, _| {
        assert_eq!(v.ordered.len(), 1);
        let files: Vec<&str> = v.files["agent-1"].iter().map(|f| f.path.as_str()).collect();
        assert_eq!(files.len(), 2);
        assert!(
            files.contains(&"src/a.rs") && files.contains(&"notes/new.md"),
            "{files:?}"
        );
        // The first file in tree order is selected and its diff shown.
        assert_eq!(
            v.sel,
            Sel::Agent {
                id: "agent-1".into(),
                path: Some("notes/new.md".into())
            }
        );
        assert_eq!(v.diff, DiffState::Shown);
    });
    view.read_with(cx, |v, cx| {
        assert_eq!(v.code.read(cx).path(), Some("notes/new.md"));
    });

    // Picking the other file loads its versions.
    view.update_in(cx, |v, window, cx| {
        v.select_file("agent-1", "src/a.rs", window, cx)
    });
    cx.run_until_parked();
    view.read_with(cx, |v, cx| {
        assert_eq!(v.code.read(cx).path(), Some("src/a.rs"));
        assert_eq!(v.code.read(cx).change_count(), 1);
    });

    // ↓ / ↑ move through the file rows.
    view.update(cx, |v, cx| v.move_cursor(-1, cx));
    assert_eq!(
        view.read_with(cx, |v, _| v.cursor.clone()),
        Some(("agent-1".into(), "notes/new.md".into()))
    );
}

#[gpui::test]
fn comments_are_collected_then_sent_as_one_prompt(cx: &mut TestAppContext) {
    let r = Repo::new();
    r.write("a.txt", "1\n2\n3\n");
    r.git(&["add", "-A"]);
    r.git(&["commit", "-q", "-m", "init"]);
    let mut rec = record("agent-2", r.path());
    rec.base_commit = Some(r.git(&["rev-parse", "HEAD"]));
    let h = Harness::new(vec![rec]);
    r.write("a.txt", "1\ntwo\n3\n");
    let (view, cx) = open(cx, &h);

    // A click in the margin opens the composer at that line.
    view.update_in(cx, |v, window, cx| v.start_comment(2, window, cx));
    view.update_in(cx, |v, window, cx| {
        let field = v.composer.as_ref().unwrap().field.clone();
        field.update(cx, |f, cx| f.set_text("use a digit", cx));
        v.add_comment(window, cx);
    });
    view.read_with(cx, |v, cx| {
        assert!(v.composer.is_none());
        let list = Comments::for_agent(cx, "agent-2");
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].path.as_str(), list[0].line), ("a.txt", 2));
    });
    // Send comments: the prompt is the visible template, editable.
    view.update_in(cx, |v, window, cx| v.open_send(window, cx));
    view.read_with(cx, |v, cx| match v.dialog.as_ref() {
        Some(Dialog::Prompt { field, title, .. }) => {
            assert_eq!(title, "Send comments · agent-2");
            assert_eq!(
                field.read(cx).text(),
                "Review comments:\n- a.txt:2 — use a digit"
            );
        }
        _ => panic!("the prompt dialog"),
    });
    view.update_in(cx, |v, window, cx| v.close_dialog(window, cx));
    assert!(view.read_with(cx, |v, _| v.dialog.is_none()));
}

#[gpui::test]
fn discarding_a_file_restores_it_and_the_list_follows(cx: &mut TestAppContext) {
    let r = Repo::new();
    r.write("keep.txt", "keep\n");
    r.write("b.txt", "before\n");
    r.git(&["add", "-A"]);
    r.git(&["commit", "-q", "-m", "init"]);
    let mut rec = record("agent-3", r.path());
    rec.base_commit = Some(r.git(&["rev-parse", "HEAD"]));
    let h = Harness::new(vec![rec]);
    r.write("b.txt", "after\n");
    r.write("keep.txt", "changed\n");
    let (view, cx) = open(cx, &h);
    view.update_in(cx, |v, window, cx| {
        v.select_file("agent-3", "b.txt", window, cx)
    });
    cx.run_until_parked();
    view.update_in(cx, |v, window, cx| v.open_discard(window, cx));
    view.update(cx, |v, cx| v.run_discard(cx));
    cx.run_until_parked();
    assert_eq!(r.read("b.txt").as_deref(), Some("before\n"));
    view.read_with(cx, |v, _| {
        assert!(v.dialog.is_none());
        assert_eq!(v.notice.as_deref(), Some("Discarded b.txt"));
        // The discarded file is gone: its neighbour is shown.
        assert_eq!(
            v.sel,
            Sel::Agent {
                id: "agent-3".into(),
                path: Some("keep.txt".into())
            }
        );
    });
}

#[gpui::test]
fn commit_dialog_reads_status_and_commits(cx: &mut TestAppContext) {
    let r = Repo::new();
    r.write("c.txt", "1\n");
    r.git(&["add", "-A"]);
    r.git(&["commit", "-q", "-m", "init"]);
    let mut rec = record("agent-4", r.path());
    rec.base_commit = Some(r.git(&["rev-parse", "HEAD"]));
    let h = Harness::new(vec![rec]);
    r.write("c.txt", "2\n");
    let (view, cx) = open(cx, &h);
    view.update_in(cx, |v, window, cx| v.open_commit_agent(window, cx));
    cx.run_until_parked();
    view.update_in(cx, |v, window, cx| {
        let Some(Dialog::Commit {
            status, message, ..
        }) = v.dialog.as_ref()
        else {
            panic!("commit dialog")
        };
        let st = status.clone().unwrap().unwrap();
        assert!(!st.worktree && st.uncommitted == 1);
        message.update(cx, |m, cx| m.set_text("agent work", cx));
        v.run_commit(window, cx);
    });
    cx.run_until_parked();
    assert_eq!(r.git(&["log", "-1", "--format=%s"]), "agent work");
    view.read_with(cx, |v, _| {
        assert!(v.dialog.is_none());
        assert!(
            v.notice
                .as_deref()
                .is_some_and(|n| n.starts_with("Committed ")),
            "{:?}",
            v.notice
        );
    });
}

#[gpui::test]
fn outside_git_agents_are_counted_not_listed(cx: &mut TestAppContext) {
    let h = Harness::new(vec![]);
    let (view, cx) = open(cx, &h);
    view.read_with(cx, |v, _| {
        assert!(v.ordered.is_empty());
        assert_eq!(v.sel, Sel::None);
    });
    // Esc leaves.
    let exited = std::rc::Rc::new(std::cell::Cell::new(false));
    let e = exited.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, ev: &ReviewEvent, _| {
            e.set(*ev == ReviewEvent::Exit)
        })
        .detach();
    });
    cx.simulate_keystrokes("escape");
    assert!(exited.get());
}
