//! Where an agent's viewer is rooted, end to end over a real repository
//! with linked worktrees side by side (as `git worktree add ../name`
//! makes them): the server's resolving (`pitwall_core::show`) on this
//! computer, then the app's [`plan`]. An agent may run in the
//! repository, in a folder above it (a projects folder, the home folder)
//! or in another worktree.

use std::path::{Path, PathBuf};
use std::process::Command;

use pitwall_core::exec::{Exec as _, LocalExec};
use pitwall_core::show::{names_for, resolve, Place};
use pitwall_proto::{AgentView, ViewerOpen};

use super::*;

/// `home/Projects/combatrash` (branch main, a file changed in it),
/// `combatrash-chicken` (feat/chicken-town) and `combatrash-other`
/// (feat/other) beside it, and `home/notes`, a plain folder. Paths are
/// real ones (the temporary folder may be behind a link). Deleted on drop.
pub(crate) struct Fixture {
    base: PathBuf,
    pub home: String,
    pub projects: String,
    pub repo: String,
    pub chicken: String,
    pub other: String,
    pub notes: String,
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn real(p: &Path) -> String {
    LocalExec.real_path(&p.to_string_lossy()).unwrap()
}

impl Fixture {
    pub(crate) fn new(tag: &str) -> Fixture {
        let base = std::env::temp_dir().join(format!("pw-roots-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let projects = base.join("home").join("Projects");
        let repo = projects.join("combatrash");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir_all(base.join("home").join("notes")).unwrap();
        git(&repo, &["init", "-q", "--template=", "-b", "main"]);
        std::fs::write(repo.join("src").join("game.rs"), "fn main() {}\n").unwrap();
        std::fs::write(repo.join("README.md"), "# combat\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        git(&repo, &["worktree", "add", "-q", "-b", "feat/chicken-town", "../combatrash-chicken"]);
        git(&repo, &["worktree", "add", "-q", "-b", "feat/other", "../combatrash-other"]);
        // A change only the chicken worktree has (its git status: M).
        std::fs::write(projects.join("combatrash-chicken").join("src").join("game.rs"), "fn main() { chicken() }\n").unwrap();
        Fixture {
            home: real(&base.join("home")),
            projects: real(&projects),
            repo: real(&repo),
            chicken: real(&projects.join("combatrash-chicken")),
            other: real(&projects.join("combatrash-other")),
            notes: real(&base.join("home").join("notes")),
            base,
        }
    }

    /// An agent on this computer whose files can be read, started in `cwd`.
    pub(crate) fn agent(&self, id: &str, cwd: &str) -> AgentView {
        let mut a = crate::agents::tests::agent(id, cwd, "idle", 0, true);
        a.caps.explorer = true;
        a
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// `req` from an agent started in `cwd` that works in `now` (its MCP
/// server, `req.cwd`, runs there too): the plan, or the error the agent
/// is told.
fn ask(fx: &Fixture, cwd: &str, now: Option<&str>, req: ViewerOpen) -> Result<Plan, String> {
    let req = ViewerOpen { cwd: Some(now.unwrap_or(cwd).to_string()), ..req };
    let x = LocalExec;
    let (worktrees, inside) = names_for(&x, cwd, now, &req);
    let place = Place { exec: &x, cwd, now, worktrees: &worktrees, inside: &inside, local: true };
    let (show, _) = resolve(&place, "a1", &req)?;
    let p = plan(&show, &[fx.agent("a1", cwd)]).map_err(|e| e.message)?;
    // (The server's note, as the answer carries it.)
    Ok(Plan { note: show.note.or(p.note.clone()), ..p })
}

fn root(r: &str) -> ViewerOpen {
    ViewerOpen { root: Some(r.into()), ..Default::default() }
}

fn folder(f: &str) -> ViewerOpen {
    ViewerOpen { folder: Some(f.into()), ..Default::default() }
}

/// Every way of asking for the chicken worktree, from every place an agent
/// may run: rooted exactly there, its git status its own.
#[test]
fn every_way_of_asking_for_a_worktree_roots_the_viewer_there() {
    let fx = Fixture::new("matrix");
    let chicken = PathBuf::from(&fx.chicken);
    let key = path_key(&fx.chicken);
    // (label, started in, works in now)
    let agents = [
        ("the repository", fx.repo.clone(), None),
        ("a parent of the repository and its worktrees", fx.projects.clone(), None),
        ("the home folder, working in the repository", fx.home.clone(), Some(fx.repo.clone())),
        ("another worktree", fx.other.clone(), None),
    ];
    for (who, cwd, now) in &agents {
        for (how, req) in [
            ("root: its path", root(&fx.chicken)),
            ("root: its folder name", root("combatrash-chicken")),
            ("root: its branch", root("feat/chicken-town")),
            ("folder: its path, no root", folder(&fx.chicken)),
        ] {
            let p = ask(&fx, cwd, now.as_deref(), req).unwrap_or_else(|e| panic!("{who}, {how}: {e}"));
            assert_eq!((&p.key, &p.root), (&key, &chicken), "{who}, {how}");
            assert!(p.folder.as_deref().is_none_or(str::is_empty), "{who}, {how}: {:?}", p.folder);
        }
    }
}

/// The four ways the agent in the report tried, started two levels above
/// the repository (its home folder, its viewer rooted there): each roots
/// the viewer at the worktree, not at a folder inside the home tree.
#[test]
fn from_the_home_folder_the_worktree_is_the_root_not_a_folder_in_it() {
    let fx = Fixture::new("home");
    let chicken = PathBuf::from(&fx.chicken);
    for now in [None, Some(fx.repo.as_str())] {
        // (1) its path, (2) its branch.
        let p = ask(&fx, &fx.home, now, root(&fx.chicken)).unwrap();
        assert_eq!((p.key.as_str(), &p.root), (path_key(&fx.chicken).as_str(), &chicken), "{now:?}");
        // A branch name: the repository the agent works in, or the one the
        // home folder doesn't hold directly: said plainly.
        match now {
            Some(_) => {
                let p = ask(&fx, &fx.home, now, root("feat/chicken-town")).unwrap();
                assert_eq!(p.root, chicken);
                let note = p.note.unwrap_or_default();
                assert!(note.contains("\"feat/chicken-town\" is the worktree") && note.contains(&fx.chicken), "{note}");
            }
            None => {
                let e = ask(&fx, &fx.home, now, root("feat/chicken-town")).unwrap_err();
                assert!(e.contains("give the worktree's path as root"), "{e}");
            }
        }
    }
    // (3) and (4), a tile and a window: `windows::tests`.
}

/// A folder by its path in the worktree shown stays a folder revealed in
/// it; in another worktree it roots the viewer there and is revealed.
#[test]
fn a_folder_roots_the_viewer_only_when_it_is_in_another_worktree() {
    let fx = Fixture::new("folder");
    let src = format!("{}{}src", fx.repo, std::path::MAIN_SEPARATOR);
    let p = ask(&fx, &fx.repo, None, folder(&src)).unwrap();
    assert_eq!((p.key.as_str(), p.folder.as_deref()), ("a1", Some("src")));
    let chicken_src = format!("{}{}src", fx.chicken, std::path::MAIN_SEPARATOR);
    let p = ask(&fx, &fx.repo, None, folder(&chicken_src)).unwrap();
    assert_eq!((p.root.to_string_lossy().as_ref(), p.folder.as_deref()), (fx.chicken.as_str(), Some("src")));
    assert!(p.note.unwrap_or_default().contains("is in the git worktree"));
    // From the home folder (no repository): the same.
    let p = ask(&fx, &fx.home, None, folder(&chicken_src)).unwrap();
    assert_eq!((p.root.to_string_lossy().as_ref(), p.folder.as_deref()), (fx.chicken.as_str(), Some("src")));
    // A plain folder inside the home folder: revealed there, as before.
    let p = ask(&fx, &fx.home, None, folder(&fx.notes)).unwrap();
    assert_eq!((p.key.as_str(), p.folder.as_deref()), ("a1", Some("notes")));
    // The repository as a folder, from a projects folder: its root.
    let p = ask(&fx, &fx.projects, None, folder(&fx.repo)).unwrap();
    assert_eq!(p.root.to_string_lossy(), fx.repo);
}

/// An agent working in a repository below the folder it started in (a
/// terminal opened in `~`, then `cd` and an agent): the viewer shows that
/// repository without being asked, its own worktree.
#[test]
fn an_agent_started_above_a_repository_is_shown_the_one_it_works_in() {
    let fx = Fixture::new("above");
    let file = format!("{}{}README.md", fx.repo, std::path::MAIN_SEPARATOR);
    let p = ask(&fx, &fx.home, Some(&fx.repo), ViewerOpen { files: vec![file], ..Default::default() }).unwrap();
    assert_eq!((p.root.to_string_lossy().as_ref(), p.files[0].0.as_str()), (fx.repo.as_str(), "README.md"));
    // Started in the repository, working in a folder of it: as before.
    let src = format!("{}{}src", fx.repo, std::path::MAIN_SEPARATOR);
    let p = ask(&fx, &fx.repo, Some(&src), ViewerOpen { files: vec!["game.rs".into()], ..Default::default() }).unwrap();
    assert_eq!((p.key.as_str(), p.files[0].0.as_str()), ("a1", "src/game.rs"));
}

/// A name no worktree has: the worktrees there are are listed, so the agent
/// can correct itself; one more than one repository has: their paths.
#[test]
fn unknown_and_ambiguous_worktree_names_say_what_there_is() {
    let fx = Fixture::new("names");
    let e = ask(&fx, &fx.repo, None, root("feat/nope")).unwrap_err();
    assert!(e.contains("no folder or worktree \"feat/nope\""), "{e}");
    for listed in ["combatrash (main)", "combatrash-chicken (feat/chicken-town)", "combatrash-other (feat/other)"] {
        assert!(e.contains(listed), "{listed}: {e}");
    }
    // Two repositories in a projects folder, both on main.
    let second = Path::new(&fx.projects).join("second");
    std::fs::create_dir_all(&second).unwrap();
    git(&second, &["init", "-q", "--template=", "-b", "main"]);
    git(&second, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let e = ask(&fx, &fx.projects, None, root("main")).unwrap_err();
    assert!(e.contains("2 worktrees match \"main\"") && e.contains(&fx.repo) && e.contains(&real(&second)), "{e}");
    // The branch of one of them only: found, and said where.
    let p = ask(&fx, &fx.projects, None, root("feat/other")).unwrap();
    assert_eq!(p.root.to_string_lossy(), fx.other);
    assert!(p.note.unwrap_or_default().contains(&format!("a repository in {}", fx.projects)));
}
