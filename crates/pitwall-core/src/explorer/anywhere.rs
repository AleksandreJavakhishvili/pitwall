//! Files anywhere on this computer, by absolute path: the app's text
//! editor (docs/spec/editor.md). The user's own actions only; no client,
//! MCP tool or command line reaches these. Saving keeps the viewer's
//! rules (text up to its cap, never over a file that changed since it was
//! read unless the user said so, atomic); creating never replaces a file.

use std::fs;
use std::io::Write as _;
use std::path::Path;

use pitwall_proto::FileStatus;

use super::read::MAX_TEXT;
use super::write::{self, Saved};
use super::Res;
use crate::exec::LocalExec;
use crate::vcs::git::Git;

/// Save `text` over the file at absolute `path` (it must be there).
/// `base`: the file as it was read; when it changed since, nothing is
/// written ([`Saved::ChangedOnDisk`]); `None` writes over it.
pub fn write_local_file(path: &str, text: &str, base: Option<&str>) -> Res<Saved> {
    absolute(path)?;
    write::write(path, path, text, base)
}

/// A new file at absolute `path` with `text` (Save As, a new file's first
/// save). Its folder must be there; a file already there is refused
/// (the user is asked first, then [`write_local_file`] replaces it).
pub fn create_local_file(path: &str, text: &str) -> Res<()> {
    absolute(path)?;
    if let Some(why) = write::refused_by_name(path) {
        return Err(format!("{path}: {why}"));
    }
    if text.len() as u64 > MAX_TEXT {
        return Err(format!("{path}: too large to save here (over {} MiB)", MAX_TEXT / (1024 * 1024)));
    }
    let p = Path::new(path);
    if !p.parent().is_some_and(Path::is_dir) {
        return Err(format!("{path}: its folder isn't there"));
    }
    let mut f = fs::OpenOptions::new().write(true).create_new(true).open(p).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("{path}: a file is already there"),
        _ => format!("{path}: couldn't be created: {e}"),
    })?;
    let done = f.write_all(text.as_bytes()).and_then(|_| f.sync_all());
    if let Err(e) = done {
        drop(f);
        let _ = fs::remove_file(p);
        return Err(format!("{path}: couldn't be saved: {e}"));
    }
    Ok(())
}

/// What git says of a file on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitFile {
    /// Its change since HEAD (`None`: unchanged, or not in a repository).
    pub status: Option<FileStatus>,
    /// Its text at HEAD (`None`: not there, not text, or no repository).
    pub head: Option<String>,
    /// Whether it is in a git repository at all.
    pub in_repo: bool,
}

/// [`GitFile`] for the file at absolute `path` (blocking; git runs with
/// no locks taken, as every git Pitwall runs).
pub fn local_git_file(path: &str) -> GitFile {
    let p = Path::new(path);
    let (Some(dir), Some(name)) = (p.parent().and_then(Path::to_str), p.file_name().and_then(|n| n.to_str())) else {
        return GitFile::default();
    };
    if !Path::new(dir).is_dir() {
        return GitFile::default();
    }
    let exec = LocalExec;
    let git = Git::new(&exec, dir);
    if git.repo_root().is_none() {
        return GitFile::default();
    }
    let rel = format!("./{name}");
    let status = git
        .run(&["status", "--porcelain=v1", "--untracked-files=all", "--", &rel])
        .ok()
        .and_then(|out| out.lines().next().map(|l| l.to_string()))
        .and_then(|l| status_of(&l));
    let head = git
        .output(&["cat-file", "blob", &format!("HEAD:{rel}")], &[], None)
        .ok()
        .filter(|o| o.ok())
        .filter(|o| !super::read::looks_binary(&o.stdout) && o.stdout.len() as u64 <= MAX_TEXT)
        .and_then(|o| String::from_utf8(o.stdout).ok());
    GitFile { status, head, in_repo: true }
}

/// A `git status --porcelain` line's change (`!!`, ignored: none).
fn status_of(line: &str) -> Option<FileStatus> {
    let xy: Vec<char> = line.chars().take(2).collect();
    let [x, y] = xy[..] else { return None };
    Some(match (x, y) {
        ('?', '?') => FileStatus::U,
        ('!', '!') => return None,
        ('A', _) => FileStatus::A,
        ('R', _) | ('C', _) => FileStatus::R,
        ('D', _) | (_, 'D') => FileStatus::D,
        _ => FileStatus::M,
    })
}

fn absolute(path: &str) -> Res<()> {
    if Path::new(path).is_absolute() {
        Ok(())
    } else {
        Err(format!("{path}: not a full path"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("pw-anywhere-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn creates_new_files_and_never_replaces_one() {
        let d = tmp("create");
        let f = d.join("a.txt");
        let p = f.to_str().unwrap();
        create_local_file(p, "one\n").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "one\n");
        assert!(create_local_file(p, "two\n").unwrap_err().contains("already there"));
        assert_eq!(write_local_file(p, "two\n", Some("stale")).unwrap(), Saved::ChangedOnDisk);
        assert_eq!(write_local_file(p, "two\n", Some("one\n")).unwrap(), Saved::Written);
        assert_eq!(fs::read_to_string(&f).unwrap(), "two\n");
        assert!(create_local_file(d.join("no/b.txt").to_str().unwrap(), "").is_err());
        assert!(create_local_file("rel.txt", "").is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn git_says_what_changed_and_what_head_had() {
        let d = tmp("git");
        let run = |args: &[&str]| {
            let ok = std::process::Command::new("git").arg("-C").arg(&d).args(args).output().map(|o| o.status.success());
            ok.unwrap_or(false)
        };
        if !run(&["init", "-q"]) {
            return; // no git here
        }
        fs::write(d.join("a.txt"), "one\n").unwrap();
        run(&["add", "a.txt"]);
        if !run(&["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "commit", "-qm", "x"]) {
            return;
        }
        let a = d.join("a.txt");
        let g = local_git_file(a.to_str().unwrap());
        assert_eq!(g, GitFile { status: None, head: Some("one\n".into()), in_repo: true });
        fs::write(&a, "two\n").unwrap();
        assert_eq!(local_git_file(a.to_str().unwrap()).status, Some(FileStatus::M));
        let b = d.join("b.txt");
        fs::write(&b, "new\n").unwrap();
        let g = local_git_file(b.to_str().unwrap());
        assert_eq!((g.status, g.head), (Some(FileStatus::U), None));
        let _ = fs::remove_dir_all(&d);
        let outside = tmp("nogit");
        let c = outside.join("c.txt");
        fs::write(&c, "x").unwrap();
        // The temp folder may itself sit in a repository on some machines.
        let g = local_git_file(c.to_str().unwrap());
        assert!(!g.in_repo || g.status.is_some());
        let _ = fs::remove_dir_all(&outside);
    }
}
