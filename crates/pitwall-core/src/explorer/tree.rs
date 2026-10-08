//! The file tree, one folder at a time, and the quick-open index.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

use pitwall_proto::{DirListing, EntryKind, FileEntry, FileIndex, FileStatus};

use super::{Ctx, Res};
use crate::exec::{self, Cmd, Exec, FileKind, Out};
use crate::vcs::git::{parse_raw_numstat, GIT_ENV};

/// Entries per folder; the rest are left out (`truncated`).
pub const MAX_ENTRIES: usize = 5_000;
/// Paths in the quick-open index.
pub const MAX_INDEX: usize = 100_000;
/// Time a plain (no git, no ripgrep) index may take.
const INDEX_BUDGET: Duration = Duration::from_secs(3);
/// Listings are quick reads; only a hung machine hits this.
const LIST_TIMEOUT: Duration = Duration::from_secs(60);

/// Never shown in a plain listing (VS Code's default `files.exclude`).
pub const HIDDEN: [&str; 6] = [".git", ".svn", ".hg", "CVS", ".DS_Store", "Thumbs.db"];
/// Not descended into for quick open and search (VS Code's `search.exclude`).
pub const NOT_SEARCHED: [&str; 2] = ["node_modules", "bower_components"];

/// stdout of a finished, successful command.
fn stdout(r: Option<crate::error::Result<Out>>) -> Option<String> {
    match r? {
        Ok(o) if o.ok() => Some(o.stdout_text()),
        _ => None,
    }
}

fn git<'a>(dir: &'a str, args: &[&'a str]) -> Vec<&'a str> {
    [&["git", "-C", dir][..], args].concat()
}

/// Folder `rel` (resolved: `real`) of the agent's folder; `ignored`: list
/// what git ignores too (marked).
pub(super) fn list(c: &Ctx, rel: &str, real: &str, ignored: bool) -> Res<DirListing> {
    if c.git_repo != Some(false) {
        match git_list(c, rel, real, ignored) {
            Some(Listed::Git(listing)) => return Ok(listing),
            Some(Listed::Ignored) => {
                // An ignored folder, opened: everything in it is ignored.
                let mut l = plain_list(&*c.exec, rel, real)?;
                l.entries.iter_mut().for_each(|e| e.ignored = true);
                return Ok(l);
            }
            None => {}
        }
    }
    plain_list(&*c.exec, rel, real)
}

enum Listed {
    Git(DirListing),
    /// The folder itself is ignored (asked for with `ignored`).
    Ignored,
}

/// git's view of the folder: tracked files (with their modes), untracked
/// ones that aren't ignored (with `ignored`: ignored ones too), and the
/// changes since the agent's base, in one round trip. `None` when git
/// can't list it (not a repository).
fn git_list(c: &Ctx, rel: &str, real: &str, ignored: bool) -> Option<Listed> {
    let base = c.base.as_deref().unwrap_or("HEAD");
    let cached = git(real, &["ls-files", "-z", "--stage"]);
    let others = git(
        real,
        &[
            "ls-files",
            "-z",
            "--others",
            "--exclude-standard",
            "--directory",
            "--no-empty-directory",
        ],
    );
    let diff = git(
        real,
        &["diff", "--raw", "--numstat", "-z", "-M", "--relative", base],
    );
    let ignored_cmd = git(
        real,
        &[
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ],
    );
    let mut list = vec![&cached, &others, &diff];
    if ignored {
        list.push(&ignored_cmd);
    }
    let cmds: Vec<Cmd> = list
        .iter()
        .map(|a| Cmd::new(a).env(&GIT_ENV).timeout(LIST_TIMEOUT))
        .collect();
    let mut outs = c.exec.run_all(&cmds).into_iter();
    let cached = stdout(outs.next())?;
    let mut others = stdout(outs.next()).unwrap_or_default();
    let changes: HashMap<String, FileStatus> = stdout(outs.next())
        .map(|d| {
            parse_raw_numstat(&d)
                .into_iter()
                .filter_map(|f| Some((f.path, f.status?)))
                .collect()
        })
        .unwrap_or_default();
    let ignored_out = if ignored {
        stdout(outs.next()).unwrap_or_default()
    } else {
        String::new()
    };
    if ignored_out.split('\0').any(|p| p == "./") {
        return Some(Listed::Ignored);
    }
    if others.split('\0').any(|p| p == "./") {
        // The folder itself is untracked (`--directory` names only it): its
        // files one by one, in a second round trip.
        let all = git(real, &["ls-files", "-z", "--others", "--exclude-standard"]);
        others = stdout(Some(
            c.exec
                .run(&Cmd::new(&all).env(&GIT_ENV).timeout(LIST_TIMEOUT)),
        ))
        .unwrap_or_default();
    }
    Some(Listed::Git(children(
        rel,
        &cached,
        &others,
        &ignored_out,
        &changes,
    )))
}

#[derive(Default)]
struct Child {
    kind: Option<EntryKind>,
    status: Option<FileStatus>,
    changes: u32,
    ignored: bool,
}

/// One folder's entries from `ls-files --stage` / `--others --directory`
/// / `--others --ignored --directory` output and the changes (paths
/// relative to the folder).
fn children(
    rel: &str,
    cached: &str,
    others: &str,
    ignored: &str,
    changes: &HashMap<String, FileStatus>,
) -> DirListing {
    let mut kids: BTreeMap<String, Child> = BTreeMap::new();
    for rec in cached.split('\0').filter(|r| !r.is_empty()) {
        // `<mode> <object> <stage>\t<path>`
        let Some((meta, path)) = rec.split_once('\t') else {
            continue;
        };
        if changes.get(path) == Some(&FileStatus::D) {
            continue; // gone from the working tree
        }
        let (name, kind) = match path.split_once('/') {
            Some((dir, _)) => (dir, EntryKind::Dir),
            None => match meta.split(' ').next() {
                Some("120000") => (path, EntryKind::Symlink),
                Some("160000") => (path, EntryKind::Dir), // a submodule
                _ => (path, EntryKind::File),
            },
        };
        kids.entry(name.to_string()).or_default().kind = Some(kind);
    }
    for path in others.split('\0').filter(|r| !r.is_empty()) {
        let whole_dir = path.ends_with('/');
        let path = path.trim_end_matches('/');
        let kid = match path.split_once('/') {
            Some((dir, _)) => {
                let k = kids.entry(dir.to_string()).or_default();
                k.kind = Some(EntryKind::Dir);
                k
            }
            None => {
                let k = kids.entry(path.to_string()).or_default();
                k.kind = Some(if whole_dir {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                });
                k.status = Some(FileStatus::U);
                k
            }
        };
        if kid.kind == Some(EntryKind::Dir) {
            kid.changes += 1;
        }
    }
    // Ignored entries of this folder itself (deeper ones sit in folders
    // listed already, or in an ignored folder named here).
    for path in ignored.split('\0').filter(|r| !r.is_empty()) {
        let whole_dir = path.ends_with('/');
        let path = path.trim_end_matches('/');
        if path.contains('/') || kids.contains_key(path) {
            continue;
        }
        kids.insert(
            path.to_string(),
            Child {
                kind: Some(if whole_dir {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                }),
                ignored: true,
                ..Default::default()
            },
        );
    }
    for (path, status) in changes {
        let (name, nested) = match path.split_once('/') {
            Some((dir, _)) => (dir, true),
            None => (path.as_str(), false),
        };
        let Some(kid) = kids.get_mut(name) else {
            continue;
        };
        if nested {
            kid.changes += 1;
        } else if kid.status.is_none() {
            kid.status = Some(*status);
        }
    }
    let entries = kids
        .into_iter()
        .filter_map(|(name, k)| {
            let kind = k.kind?;
            let changes = if kind == EntryKind::Dir { k.changes } else { 0 };
            Some(FileEntry {
                path: exec::join(rel, &name),
                name,
                kind,
                status: k.status,
                changes,
                ignored: k.ignored,
            })
        })
        .collect();
    finish(rel, entries, true)
}

/// A folder outside git: every entry but [`HIDDEN`] ones.
fn plain_list(exec: &dyn Exec, rel: &str, real: &str) -> Res<DirListing> {
    let entries = exec
        .list_dir(real)
        .map_err(|e| {
            format!(
                "{}: {e}",
                if rel.is_empty() {
                    "the agent's folder"
                } else {
                    rel
                }
            )
        })?
        .into_iter()
        .filter(|e| !HIDDEN.contains(&e.name.as_str()))
        .map(|e| FileEntry {
            path: exec::join(rel, &e.name),
            kind: match e.kind {
                FileKind::Dir => EntryKind::Dir,
                FileKind::Symlink => EntryKind::Symlink,
                FileKind::File | FileKind::Other => EntryKind::File,
            },
            name: e.name,
            status: None,
            changes: 0,
            ignored: false,
        })
        .collect();
    Ok(finish(rel, entries, false))
}

/// Folders first, then by name (case-insensitive), capped.
fn finish(rel: &str, mut entries: Vec<FileEntry>, git: bool) -> DirListing {
    entries.sort_by(|a, b| {
        (a.kind != EntryKind::Dir)
            .cmp(&(b.kind != EntryKind::Dir))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let truncated = entries.len() > MAX_ENTRIES;
    entries.truncate(MAX_ENTRIES);
    DirListing {
        dir: rel.to_string(),
        entries,
        truncated,
        git,
    }
}

// ---------------------------------------------------------------- quick open

/// Every file below `root` (resolved). `rg`: whether ripgrep runs there, if
/// known; the second value is what was learnt about it.
pub(super) fn index(c: &Ctx, root: &str, rg: Option<bool>) -> (FileIndex, Option<bool>) {
    if c.git_repo != Some(false) {
        if let Some(index) = git_index(&*c.exec, root) {
            return (index, None);
        }
    }
    let mut learnt = None;
    if rg != Some(false) {
        match rg_index(&*c.exec, root) {
            Some(index) => return (index, Some(true)),
            None => learnt = rg.is_none().then_some(false),
        }
    }
    (walk(&*c.exec, root), learnt)
}

fn capped(mut files: Vec<String>, git: bool) -> FileIndex {
    files.sort();
    files.dedup();
    let truncated = files.len() > MAX_INDEX;
    files.truncate(MAX_INDEX);
    FileIndex {
        files,
        truncated,
        git,
    }
}

fn git_index(exec: &dyn Exec, root: &str) -> Option<FileIndex> {
    let cached = git(root, &["ls-files", "-z", "--cached"]);
    let others = git(root, &["ls-files", "-z", "--others", "--exclude-standard"]);
    let deleted = git(root, &["ls-files", "-z", "--deleted"]);
    let cmds: Vec<Cmd> = [&cached, &others, &deleted]
        .iter()
        .map(|a| Cmd::new(a).env(&GIT_ENV).timeout(LIST_TIMEOUT))
        .collect();
    let mut outs = exec.run_all(&cmds).into_iter();
    let cached = stdout(outs.next())?;
    let others = stdout(outs.next()).unwrap_or_default();
    let deleted = stdout(outs.next()).unwrap_or_default();
    let gone: std::collections::HashSet<&str> = deleted.split('\0').collect();
    let files = cached
        .split('\0')
        .chain(others.split('\0'))
        .filter(|p| !p.is_empty() && !gone.contains(p))
        .map(String::from)
        .collect();
    Some(capped(files, true))
}

/// `rg --files` (ignore files respected); `None` when ripgrep can't run.
fn rg_index(exec: &dyn Exec, root: &str) -> Option<FileIndex> {
    let mut argv = vec![
        "rg",
        "--files",
        "--no-config",
        "--hidden",
        "--null",
        "-g",
        "!.git",
    ];
    let globs: Vec<String> = NOT_SEARCHED.iter().map(|d| format!("!{d}")).collect();
    for g in &globs {
        argv.extend(["-g", g]);
    }
    argv.extend(["--", "."]);
    let out = exec
        .run(&Cmd::new(&argv).cwd(root).timeout(LIST_TIMEOUT))
        .ok()?;
    // 1: nothing found; 2: some folders couldn't be read.
    if !matches!(out.status, 0..=2) {
        return None;
    }
    let text = out.stdout_text();
    let files = text
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(super::search::tidy_path)
        .collect();
    Some(capped(files, true))
}

/// Breadth-first, without [`HIDDEN`] and [`NOT_SEARCHED`] folders, within
/// [`INDEX_BUDGET`].
fn walk(exec: &dyn Exec, root: &str) -> FileIndex {
    let started = Instant::now();
    let mut files = Vec::new();
    let mut todo = VecDeque::from([String::new()]);
    let mut truncated = false;
    while let Some(rel) = todo.pop_front() {
        if started.elapsed() > INDEX_BUDGET || files.len() > MAX_INDEX {
            truncated = true;
            break;
        }
        let Ok(entries) = exec.list_dir(&exec::join(root, &rel)) else {
            continue;
        };
        for e in entries {
            if HIDDEN.contains(&e.name.as_str()) {
                continue;
            }
            let path = exec::join(&rel, &e.name);
            match e.kind {
                FileKind::Dir if !NOT_SEARCHED.contains(&e.name.as_str()) => todo.push_back(path),
                FileKind::Dir => {}
                _ => files.push(path),
            }
        }
    }
    let mut index = capped(files, false);
    index.truncated |= truncated;
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_from_git_output() {
        let cached = "100644 aa 0\tREADME.md\x00100644 bb 0\tsrc/a.rs\x00100644 cc 0\tsrc/b/c.rs\0\
                      120000 dd 0\tlink\x00160000 ee 0\tvendor/sub\x00100644 ff 0\tgone.txt\x00100644 11 0\told/x.txt\0";
        let others = "new.txt\0newdir/\0src/fresh.rs\0";
        let changes: HashMap<String, FileStatus> = [
            ("README.md", FileStatus::M),
            ("gone.txt", FileStatus::D),
            ("old/x.txt", FileStatus::D),
            ("src/a.rs", FileStatus::M),
        ]
        .into_iter()
        .map(|(p, s)| (p.to_string(), s))
        .collect();
        let ignored = "target/\0debug.log\0src/gen/\0";
        let l = children("", cached, others, ignored, &changes);
        let got: Vec<_> = l
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.kind, e.status, e.changes, e.ignored))
            .collect();
        assert_eq!(
            got,
            [
                ("newdir", EntryKind::Dir, Some(FileStatus::U), 1, false),
                ("src", EntryKind::Dir, None, 2, false),
                ("target", EntryKind::Dir, None, 0, true),
                ("vendor", EntryKind::Dir, None, 0, false),
                ("debug.log", EntryKind::File, None, 0, true),
                ("link", EntryKind::Symlink, None, 0, false),
                ("new.txt", EntryKind::File, Some(FileStatus::U), 0, false),
                ("README.md", EntryKind::File, Some(FileStatus::M), 0, false),
            ]
        );
        assert!(l.git && !l.truncated);
        // Inside a folder: paths are the folder's.
        let sub = children(
            "src",
            "100644 bb 0\ta.rs\x00160000 cc 0\tmod\0",
            "",
            "",
            &HashMap::new(),
        );
        let got: Vec<_> = sub
            .entries
            .iter()
            .map(|e| (e.path.as_str(), e.kind))
            .collect();
        assert_eq!(
            got,
            [("src/mod", EntryKind::Dir), ("src/a.rs", EntryKind::File)]
        );
    }

    #[test]
    fn folders_first_then_names_ignoring_case_capped() {
        let entry = |name: &str, kind| FileEntry {
            name: name.into(),
            path: name.into(),
            kind,
            status: None,
            changes: 0,
            ignored: false,
        };
        let l = finish(
            "",
            vec![
                entry("b", EntryKind::File),
                entry("Z", EntryKind::Dir),
                entry("A", EntryKind::File),
                entry("a", EntryKind::Dir),
            ],
            false,
        );
        assert_eq!(
            l.entries
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            ["a", "Z", "A", "b"]
        );
        let many = (0..MAX_ENTRIES + 3)
            .map(|i| entry(&format!("f{i}"), EntryKind::File))
            .collect();
        let l = finish("", many, false);
        assert!(l.truncated && l.entries.len() == MAX_ENTRIES);
    }
}
