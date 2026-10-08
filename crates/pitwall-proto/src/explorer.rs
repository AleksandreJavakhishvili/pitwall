//! The read-only code explorer (docs/spec/explorer.md): a lazy file tree of
//! the folder an agent works in, file contents for the viewer, a quick-open
//! index, search results and the "Open in editor" setting. Every path here
//! is relative to that folder and uses `/`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Git's letter for a changed file, as VS Code's SCM view shows it:
/// Modified, Added, Deleted, Renamed, Untracked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum FileStatus {
    M,
    A,
    D,
    R,
    U,
}

impl FileStatus {
    /// From a `git diff --raw` / `--name-status` letter (copies count as
    /// added, type changes and unmerged paths as modified).
    pub fn from_git(letter: &str) -> FileStatus {
        match letter.chars().next() {
            Some('A') | Some('C') => FileStatus::A,
            Some('D') => FileStatus::D,
            Some('R') => FileStatus::R,
            _ => FileStatus::M,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    File,
    /// Expandable (`list_files` with its path). Submodules and nested
    /// repositories are folders too.
    Dir,
    /// Shown, never expanded; reading it reads its target only when that
    /// stays inside the agent's folder.
    Symlink,
}

/// One child of a listed folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    /// From the agent's folder, `/`-separated.
    pub path: String,
    pub kind: EntryKind,
    /// The Changes panel's letter for it (same base); `U` for an untracked
    /// folder.
    pub status: Option<FileStatus>,
    /// Folders: changed paths below it (0 for files).
    pub changes: u32,
}

/// One folder's children (`list_files`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DirListing {
    /// The folder listed ("" = the agent's folder itself).
    pub dir: String,
    /// Folders first, then files, by name (case-insensitive).
    pub entries: Vec<FileEntry>,
    /// More entries than the cap were there; the rest are left out.
    pub truncated: bool,
    /// Listed by git, so `.gitignore` applies (false: a plain listing).
    pub git: bool,
}

/// Every file of the agent's folder, for quick open (`list_all_files`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileIndex {
    pub files: Vec<String>,
    pub truncated: bool,
    /// Ignore files were respected (git or ripgrep listed them).
    pub git: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ContentKind {
    /// `text` holds the whole file.
    Text,
    /// Not transferred.
    Binary,
    /// Over the size cap; not transferred.
    TooLarge,
}

/// One file for the viewer (`read_file`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileView {
    pub path: String,
    /// Bytes.
    #[ts(type = "number")]
    pub size: u64,
    pub kind: ContentKind,
    /// The file as UTF-8 (invalid bytes replaced), only for `text`.
    pub text: Option<String>,
    /// A language hint from its name ("rust", "typescript", …).
    pub lang: Option<String>,
}

/// What to search for (`search_files`). Defaults match VS Code's: a plain,
/// case-insensitive string.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub query: String,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub whole_word: bool,
    /// Globs a file must match (`*.rs`, `src/**`); none = every file.
    #[serde(default)]
    pub include: Vec<String>,
    /// Globs to leave out.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Stop after this many matches (default 2 000, at most 10 000).
    #[serde(default)]
    pub max_results: Option<u32>,
    /// Search hidden files (dotfiles) too (default yes); ignore files still
    /// apply.
    #[serde(default)]
    pub hidden: Option<bool>,
}

/// Start and end of a highlighted part, in UTF-16 code units of
/// [`SearchMatch::text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MatchRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub path: String,
    /// 1-based.
    pub line: u32,
    /// 1-based character column of the first match in the whole line.
    pub column: u32,
    /// The line, cut to a window around the first match when long
    /// (`text_offset` characters were cut from its start).
    pub text: String,
    pub text_offset: u32,
    pub ranges: Vec<MatchRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SearchEngine {
    Ripgrep,
    GitGrep,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub matches: Vec<SearchMatch>,
    /// Files with at least one match (among `matches`).
    pub files: u32,
    /// Stopped at the cap; there are more.
    pub truncated: bool,
    pub engine: SearchEngine,
}

/// An editor found on this computer (Settings → Editor).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EditorChoice {
    pub id: String,
    pub label: String,
    /// Its command line, with `{path}`, `{line}`, `{column}`.
    pub command: String,
}

/// The "Open in editor" setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EditorSettings {
    /// What the user set; `None`: the first detected one.
    pub command: Option<String>,
    /// Editors found on the login PATH, then the system opener.
    pub detected: Vec<EditorChoice>,
    /// What "Open in editor" runs now.
    pub effective: Option<String>,
}
