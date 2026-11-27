//! The harness on the tile host's committed files tile: `files` in the
//! view's project (a folder of the test's) and in the extension's own
//! files folder, its permissions, the paths it never reaches, watches,
//! the Trash, the Activity, and a view moved to another project.

use std::path::{Path, PathBuf};

use pitwall_extension_test::{Harness, Session};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pitwall-tile-host/tests/fixtures/files")
}

fn key(t: &Session, k: &str) -> String {
    t.find_key(k).and_then(|f| f.text().map(String::from)).unwrap_or_default()
}

fn status(t: &Session) -> String {
    key(t, "status")
}

fn shop() -> Harness {
    Harness::load(fixture())
        .project("shop")
        .project_file("docs/a.md", "# A\n")
        .project_file("docs/arch/b.md", "# B\n")
        .project_file("docs/draft.tmp", "scratch")
        .project_file(".gitignore", "*.tmp\nbuild/\n")
        .project_file("build/out.md", "built")
        .project_file(".git/config", "[core]")
        .project_file("README.md", "# Shop")
}

#[test]
fn reads_need_files_read_and_know_their_project() {
    let mut t = shop().open("main");
    // Without the grant: no project, every call refused, in the Activity.
    assert_eq!(key(&t, "project"), "in no project");
    t.click_key("do/read");
    assert_eq!(status(&t), "read: not allowed: the permission isn't granted");
    assert!(t.activity().iter().any(|l| l.method == "files.read" && l.refused.as_deref() == Some("not granted files.read")), "{:?}", t.activity());
    t.grant("files.read");
    assert_eq!(key(&t, "project"), format!("in shop at {}", t.project_dir().display()));
    t.click_key("do/read");
    assert_eq!(status(&t), "read: # A\n");
    t.click_key("do/stat");
    assert_eq!(status(&t), "stat: docs/a.md 4 bytes");
    // Ignored files and .git are left out of lists.
    t.click_key("do/list");
    assert_eq!(status(&t), "list: docs/a.md, docs/arch/, docs/arch/b.md");
    t.click_key("do/list-root");
    assert_eq!(status(&t), "list: .gitignore, README.md, docs/");
    // Reading changes nothing in the Activity beyond the first refusal.
    assert_eq!(t.activity().len(), 1);
}

#[test]
fn paths_never_leave_the_project() {
    let mut t = shop().grant("files.read").grant("files.write").open("main");
    std::fs::write(t.project_dir().parent().unwrap().join("secret.txt"), "secret").unwrap();
    for (button, said) in [
        ("up", "up: not allowed: “../secret.txt” goes up with ..: paths stay inside the root"),
        ("abs", "abs: not allowed: “/etc/hosts” isn't a path from the root: write it like docs/notes.md"),
        ("git", "git: not allowed: .git is never reached"),
        ("own", "own: not allowed: Pitwall's own files in a project are never changed"),
        ("big", "big: over a size limit"),
    ] {
        t.click_key(&format!("do/{button}"));
        assert_eq!(status(&t), said);
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(t.project_dir().parent().unwrap(), t.project_dir().join("out")).unwrap();
        t.click_key("do/link");
        assert_eq!(status(&t), "link: not allowed: a link there leads out of the root");
    }
    assert!(t.project_file(".pitwall.json").is_none() && t.project_file("big.bin").is_none());
    let refused: Vec<_> = t.activity().into_iter().filter(|l| l.refused.is_some()).map(|l| l.method).collect();
    assert!(refused.contains(&"files.read".to_string()) && refused.contains(&"files.write".to_string()), "{refused:?}");
}

#[test]
fn writes_need_files_write_and_go_to_the_trash() {
    let mut t = shop().grant("files.read").open("main");
    t.click_key("do/write");
    assert_eq!(status(&t), "write: not allowed: the permission isn't granted");
    assert!(t.project_file("docs/new.md").is_none());
    t.grant("files.write").click_key("do/write");
    assert_eq!(status(&t), "write: ok");
    assert_eq!(t.project_file("docs/new.md").as_deref(), Some("# New\n"));
    t.click_key("do/create");
    assert_eq!(status(&t), "create: something is there already");
    t.click_key("do/mkdir");
    assert_eq!(status(&t), "mkdir: ok");
    assert!(t.project_dir().join("notes/2026").is_dir());
    t.click_key("do/rename");
    assert_eq!(status(&t), "rename: ok");
    assert_eq!(t.project_file("docs/renamed.md").as_deref(), Some("# New\n"));
    t.click_key("do/delete");
    assert_eq!(status(&t), "delete: ok");
    assert!(t.project_file("docs/renamed.md").is_none());
    assert_eq!(t.trashed(), ["renamed.md"]);
    let done: Vec<(String, String)> = t.activity().into_iter().filter(|l| l.refused.is_none()).map(|l| (l.method, l.detail)).collect();
    assert_eq!(
        done,
        [
            ("files.write".to_string(), "shop: docs/new.md".to_string()),
            ("files.make-folder".into(), "shop: notes/2026".into()),
            ("files.rename".into(), "shop: docs/new.md → docs/renamed.md".into()),
            ("files.delete".into(), "shop: docs/renamed.md (to the Trash)".into()),
        ]
    );
}

#[test]
fn its_own_files_folder_needs_no_grant() {
    let mut t = Harness::load(fixture()).open("main");
    t.click_key("data/write");
    assert_eq!(status(&t), "data write: ok");
    t.click_key("data/read");
    assert_eq!(status(&t), "data read: kept");
    // Kept across a restart; deleted for good, not to the Trash.
    t.restart().click_key("data/read");
    assert_eq!(status(&t), "data read: kept");
    t.click_key("data/delete").click_key("data/read");
    assert_eq!(status(&t), "data read: nothing is there");
    assert!(t.activity().is_empty(), "its own folder isn't in the Activity");
}

#[test]
fn watches_hear_changes_but_not_ignored_ones() {
    let mut t = shop().grant("files.read").open("main");
    t.click_key("do/watch");
    assert_eq!((status(&t), t.watches()), ("watching".to_string(), 1));
    t.write_project_file("docs/a.md", "# A, again\n");
    assert_eq!(key(&t, "changed"), "changed: docs/a.md");
    t.write_project_file("docs/c.tmp", "ignored");
    assert_eq!(key(&t, "changed"), "changed: docs/a.md", "an ignored file isn't told");
    t.rename_project_file("docs/arch/b.md", "docs/b.md");
    assert_eq!(key(&t, "changed"), "changed: docs/arch/b.md, docs/b.md");
    t.remove_project_file("README.md");
    assert_eq!(key(&t, "changed"), "changed: README.md");
    t.click_key("do/unwatch");
    assert_eq!(t.watches(), 0);
    t.write_project_file("docs/a.md", "# A, third\n");
    assert_eq!(key(&t, "changed"), "changed: README.md");
}

#[test]
fn a_view_moved_to_another_project_reaches_only_that_one() {
    let mut t = shop().grant("files.read").open("main");
    t.click_key("do/watch");
    t.move_to_project("main", Some("api"));
    assert_eq!(status(&t), format!("moved in api at {}", t.project_dir().display()));
    assert_eq!(t.watches(), 0, "the watch on the old project ended");
    t.click_key("do/read");
    assert_eq!(status(&t), "read: nothing is there");
    t.move_to_project("main", None);
    assert_eq!(status(&t), "moved in no project");
    t.click_key("do/read");
    assert_eq!(status(&t), "read: the view isn't open in a project");
}

#[test]
fn taking_files_write_back_refuses_the_next_write() {
    let mut t = shop().grant("files.read").grant("files.write").open("main");
    t.click_key("do/write");
    assert_eq!(status(&t), "write: ok");
    t.revoke("files.write").click_key("do/write");
    assert_eq!(status(&t), "write: not allowed: the permission isn't granted");
}

#[test]
fn a_revoke_ends_the_project_watches() {
    let mut t = shop().grant("files.read").open("main");
    t.click_key("do/watch");
    assert_eq!(t.watches(), 1);
    t.revoke("files.read");
    assert_eq!(t.watches(), 0, "the watch ended at once");
    t.write_project_file("docs/a.md", "# A, again\n");
    assert_eq!(key(&t, "changed"), "", "no change heard");
    t.click_key("do/read");
    assert_eq!(status(&t), "read: not allowed: the permission isn't granted");
}
