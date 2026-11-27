//! The test's project for `files` ([`crate::Harness::project`]): a folder
//! made for the test, a watcher that hears the changes the test makes
//! through the session (and nothing else, so a test never waits for the
//! system's events), and a Trash folder of the test's own.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pitwall_tile_host::files::{FakeWatcher, FolderTrash};
use pitwall_tile_host::{ui, TileHost, UiRequest};

use crate::Session;

pub(crate) struct Fs {
    pub project: Option<ui::Project>,
    watcher: Arc<FakeWatcher>,
    trash: Arc<FolderTrash>,
}

impl Fs {
    /// Watches and the Trash for `host`, under the test's folder `tmp`,
    /// and project `name` with `files` in it.
    pub fn new(host: &TileHost, tmp: &Path, name: Option<&str>, files: &[(String, Vec<u8>)]) -> Fs {
        let watcher = FakeWatcher::new();
        let trash = FolderTrash::new(tmp.join("trash"));
        host.set_watcher(watcher.clone());
        host.set_trash(trash.clone());
        let project = name.map(|name| {
            let folder = tmp.join("projects").join(name);
            std::fs::create_dir_all(&folder).expect("the project's folder is made");
            for (path, bytes) in files {
                let f = folder.join(path);
                std::fs::create_dir_all(f.parent().expect("a folder")).expect("its folder is made");
                std::fs::write(&f, bytes).expect("the file is written");
            }
            ui::Project { name: name.to_string(), folder: folder.to_string_lossy().into_owned() }
        });
        Fs { project, watcher, trash }
    }
}

/// What the tile asked for the extension's Activity: its changes to a
/// project's files and the refusals ([`Session::activity`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityLine {
    /// The call (`files.write`, `files.rename`…).
    pub method: String,
    /// The project and the path.
    pub detail: String,
    /// Why it was refused, when it was.
    pub refused: Option<String>,
}

impl Session {
    /// The test's project's folder ([`crate::Harness::project`]). Panics
    /// without one.
    pub fn project_dir(&self) -> PathBuf {
        PathBuf::from(&self.fs.project.as_ref().expect("the harness has no project: use .project(name)").folder)
    }

    fn project_path(&self, path: &str) -> PathBuf {
        self.project_dir().join(path)
    }

    /// The text of file `path` in the project, if it is there.
    pub fn project_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.project_path(path)).ok()
    }

    /// Something else (the user, an agent) writes `text` to file `path` in
    /// the project: the tile's watches hear it, then the session settles.
    pub fn write_project_file(&mut self, path: &str, text: impl AsRef<[u8]>) -> &mut Self {
        let f = self.project_path(path);
        std::fs::create_dir_all(f.parent().expect("a folder")).expect("its folder is made");
        std::fs::write(&f, text).expect("the file is written");
        self.fs.watcher.changed(&[f]);
        self.settle()
    }

    /// Something else removes file or folder `path` from the project.
    pub fn remove_project_file(&mut self, path: &str) -> &mut Self {
        let f = self.project_path(path);
        if f.is_dir() {
            std::fs::remove_dir_all(&f).expect("the folder is removed");
        } else {
            std::fs::remove_file(&f).expect("the file is removed");
        }
        self.fs.watcher.changed(&[f]);
        self.settle()
    }

    /// Something else renames `from` to `to` in the project.
    pub fn rename_project_file(&mut self, from: &str, to: &str) -> &mut Self {
        let (a, b) = (self.project_path(from), self.project_path(to));
        std::fs::create_dir_all(b.parent().expect("a folder")).expect("its folder is made");
        std::fs::rename(&a, &b).expect("the file is renamed");
        self.fs.watcher.changed(&[a, b]);
        self.settle()
    }

    /// The tile's changes to the project, as its watches would hear them
    /// from the system: what changed since the test last looked is told
    /// to them now. A tile hears its own changes too, as in Pitwall.
    pub fn changed_project_files(&mut self, paths: &[&str]) -> &mut Self {
        let list: Vec<PathBuf> = paths.iter().map(|p| self.project_path(p)).collect();
        self.fs.watcher.changed(&list);
        self.settle()
    }

    /// How many `files.watch` watches are on.
    pub fn watches(&self) -> usize {
        self.fs.watcher.watching()
    }

    /// What the tile moved to the Trash, by name, oldest first.
    pub fn trashed(&self) -> Vec<String> {
        self.fs.trash.items()
    }

    /// View `view` moves to project `name` (a new folder of the test,
    /// made when missing), or out of every project (`None`): the tile
    /// hears `project-changed`. Later views open there too.
    pub fn move_to_project(&mut self, view: &str, name: Option<&str>) -> &mut Self {
        let project = name.map(|name| {
            let base = match &self.fs.project {
                Some(p) => PathBuf::from(&p.folder).parent().expect("the projects' folder").to_path_buf(),
                None => self._data.path().join("projects"),
            };
            let folder = base.join(name);
            std::fs::create_dir_all(&folder).expect("the project's folder is made");
            ui::Project { name: name.to_string(), folder: folder.to_string_lossy().into_owned() }
        });
        self.fs.project = project.clone();
        self.check_running();
        assert!(self.handle.set_project(view, project));
        self.settle()
    }

    /// The lines the tile asked for in the extension's Activity: changes
    /// to a project's files, and refusals.
    pub fn activity(&self) -> Vec<ActivityLine> {
        self.ui
            .iter()
            .filter_map(|r| match r {
                UiRequest::Activity { method, detail, refused } => Some(ActivityLine { method: method.clone(), detail: detail.clone(), refused: refused.clone() }),
                _ => None,
            })
            .collect()
    }
}
