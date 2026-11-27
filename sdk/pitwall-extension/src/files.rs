//! Files in the project a view is open in, and in the extension's own
//! files folder.
//!
//! Every view is open somewhere in Pitwall's layout: in a project's
//! space, next to an agent working in a project, or opened from a
//! project's menu in the sidebar. That project is the view's
//! ([`project`]), and the only project folder the tile may reach: paths
//! are relative to a [`Root`], [`Root::Project`] with the view's id or
//! [`Root::Data`] for the extension's own folder.
//!
//! The manifest asks for `files.read` to read a project (on by default
//! when the user turns the extension on) and `files.write` to change one
//! (off until the user turns it on), each with its reason:
//!
//! ```json
//! "permissions": {
//!   "files.read": "To list the Markdown files with diagrams in your docs",
//!   "files.write": "To save a diagram into a Markdown file you pick"
//! }
//! ```
//!
//! ```ignore
//! use pitwall_extension::files::{self, Root};
//!
//! fn on_view_opened(&mut self, view: &ViewInfo, cx: &mut Cx<Self>) {
//!     let root = Root::project(&view.id);
//!     if let Ok(entries) = files::list(&root, "docs", true) {
//!         self.docs = entries.into_iter().filter(|e| e.path.ends_with(".md")).collect();
//!     }
//!     self.watch = files::watch(&root, "docs").ok();
//! }
//!
//! fn on_event(&mut self, event: &Event, cx: &mut Cx<Self>) {
//!     if let Event::FilesChanged { watch, paths } = event {
//!         // Read the changed files again.
//!         cx.notify();
//!     }
//! }
//! ```
//!
//! **What is never reached.** Absolute paths, `..` and links that lead out
//! of the root are refused ([`Error::NotAllowed`]); `.git` is never
//! listed, read or changed, and Pitwall's own files in a project
//! (`.pitwall.json`, `.pitwall/`) are never changed. [`list`] and
//! [`watch`] leave out what the project's `.gitignore` files ignore.
//! A file is at most 8 MiB, a list at most 10 000 entries, a tile has at
//! most 8 watches; the files folder holds at most 100 MiB. Changes to a
//! project, and refusals, are listed in the extension's Activity.
//!
//! **Agents' calls.** A `show`, `present` or `read` call runs without
//! the tile's acting permissions: it may read the project, but writing
//! needs an `act` call.

use crate::bindings::pitwall::tile::files as w;

/// A project, as Pitwall shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Its name, as the sidebar shows it (`checkout-web`).
    pub name: String,
    /// Its folder, absolute, as this computer writes it.
    pub folder: String,
}

/// Where a path starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Root {
    /// The project the view with this id is open in.
    Project(String),
    /// The extension's own files folder: no permission needed, kept across
    /// updates, at most 100 MiB.
    Data,
}

impl Root {
    /// The project view `view` is open in.
    pub fn project(view: &str) -> Root {
        Root::Project(view.to_string())
    }

    fn wit(&self) -> w::Root {
        match self {
            Root::Project(v) => w::Root::Project(v.clone()),
            Root::Data => w::Root::Data,
        }
    }
}

/// A file or a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
}

/// A file or folder in a [`list`] or from [`stat`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its path from the root, with `/` between parts (`docs/arch.md`).
    pub path: String,
    /// A file or a folder (a link is what it leads to).
    pub kind: Kind,
    /// Its size in bytes (0 for a folder).
    pub size: u64,
    /// When it last changed, in milliseconds since 1970 (UTC).
    pub modified: u64,
}

impl Entry {
    /// Whether it is a folder.
    pub fn is_folder(&self) -> bool {
        self.kind == Kind::Folder
    }

    /// Its last part (`arch.md` for `docs/arch.md`).
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

impl From<w::Entry> for Entry {
    fn from(e: w::Entry) -> Entry {
        Entry { path: e.path, kind: if e.kind == w::EntryKind::Folder { Kind::Folder } else { Kind::File }, size: e.size, modified: e.modified }
    }
}

/// Why a call didn't do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not granted `files.read` (to read) or `files.write` (to change), or
    /// an agent's call that may not change files.
    Denied,
    /// The view isn't open, or isn't in a project.
    NoProject,
    /// Nothing is at that path.
    NotFound,
    /// Something is already at that path.
    Exists,
    /// Outside the root, inside `.git`, or one of Pitwall's own files; the
    /// text says which.
    NotAllowed(String),
    /// Over a limit (8 MiB a file, 10 000 entries, the files folder's
    /// 100 MiB).
    TooLarge,
    /// Too many calls this second, or 8 watches already.
    Busy,
    /// Not a valid path, or a folder where a file is needed; the text says
    /// why.
    Invalid(String),
    /// The disk said no; the text says what happened.
    Failed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Denied => write!(f, "not allowed: the permission isn't granted"),
            Error::NoProject => write!(f, "the view isn't open in a project"),
            Error::NotFound => write!(f, "nothing is there"),
            Error::Exists => write!(f, "something is there already"),
            Error::NotAllowed(why) => write!(f, "not allowed: {why}"),
            Error::TooLarge => write!(f, "over a size limit"),
            Error::Busy => write!(f, "too many calls at once; try again later"),
            Error::Invalid(why) => write!(f, "not valid: {why}"),
            Error::Failed(why) => write!(f, "failed: {why}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<w::FileError> for Error {
    fn from(e: w::FileError) -> Error {
        match e {
            w::FileError::Denied => Error::Denied,
            w::FileError::NoProject => Error::NoProject,
            w::FileError::NotFound => Error::NotFound,
            w::FileError::Exists => Error::Exists,
            w::FileError::NotAllowed(s) => Error::NotAllowed(s),
            w::FileError::TooLarge => Error::TooLarge,
            w::FileError::Busy => Error::Busy,
            w::FileError::Invalid(s) => Error::Invalid(s),
            w::FileError::Failed(s) => Error::Failed(s),
        }
    }
}

/// A watch, as [`watch`] returns it and
/// [`Event::FilesChanged`](crate::Event::FilesChanged) names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WatchId(pub u32);

/// The project view `view` is open in, if any. Needs `files.read` (or
/// `projects.read`); without either, `None`. When the view moves to
/// another project the tile hears
/// [`Event::ProjectChanged`](crate::Event::ProjectChanged).
pub fn project(view: &str) -> Option<Project> {
    w::project(view).map(|p| Project { name: p.name, folder: p.folder })
}

/// A file's bytes (at most 8 MiB).
pub fn read(root: &Root, path: &str) -> Result<Vec<u8>, Error> {
    w::read(&root.wit(), path).map_err(Error::from)
}

/// A file's text (UTF-8; [`Error::Invalid`] when it isn't).
pub fn read_text(root: &Root, path: &str) -> Result<String, Error> {
    String::from_utf8(read(root, path)?).map_err(|_| Error::Invalid(format!("{path} isn't UTF-8 text")))
}

/// A file or folder (`""`: the root itself).
pub fn stat(root: &Root, path: &str) -> Result<Entry, Error> {
    w::stat(&root.wit(), path).map(Entry::from).map_err(Error::from)
}

/// What `folder` holds (`""`: the root), sorted by path; with `deep`,
/// everything under it too. Ignored entries and `.git` are left out. At
/// most 10 000 entries, else [`Error::TooLarge`].
pub fn list(root: &Root, folder: &str, deep: bool) -> Result<Vec<Entry>, Error> {
    w::list(&root.wit(), folder, deep).map(|v| v.into_iter().map(Entry::from).collect()).map_err(Error::from)
}

/// Follow changes under `folder` (`""`: the root): each arrives as
/// [`Event::FilesChanged`](crate::Event::FilesChanged) with this watch's
/// id, changes close together as one. At most 8 at once. A watch ends
/// with [`unwatch`], when its view closes or moves to another project,
/// or when the tile stops.
pub fn watch(root: &Root, folder: &str) -> Result<WatchId, Error> {
    w::watch(&root.wit(), folder).map(WatchId).map_err(Error::from)
}

/// Stop a watch.
pub fn unwatch(id: WatchId) {
    w::unwatch(id.0)
}

/// Write `contents` to `path`, replacing what is there, whole or not at
/// all; its folders are made when missing.
pub fn write(root: &Root, path: &str, contents: &[u8]) -> Result<(), Error> {
    w::write(&root.wit(), path, contents).map_err(Error::from)
}

/// [`write`] with text.
pub fn write_text(root: &Root, path: &str, text: &str) -> Result<(), Error> {
    write(root, path, text.as_bytes())
}

/// Make a new file `path` with `contents`; [`Error::Exists`] when
/// something is there.
pub fn create(root: &Root, path: &str, contents: &[u8]) -> Result<(), Error> {
    w::create(&root.wit(), path, contents).map_err(Error::from)
}

/// Make folder `path` (and those above it).
pub fn make_folder(root: &Root, path: &str) -> Result<(), Error> {
    w::make_folder(&root.wit(), path).map_err(Error::from)
}

/// Move or rename `from` to `to`, in the same root; [`Error::Exists`]
/// when something is at `to`.
pub fn rename(root: &Root, from: &str, to: &str) -> Result<(), Error> {
    w::rename(&root.wit(), from, to).map_err(Error::from)
}

/// Move `path` of a project to the system's Trash, where the user can put
/// it back; in the files folder it is deleted.
pub fn delete(root: &Root, path: &str) -> Result<(), Error> {
    w::delete(&root.wit(), path).map_err(Error::from)
}
