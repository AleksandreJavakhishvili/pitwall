//! Updates from a git source (docs/spec/extensions.md, "Updates from a
//! git source"): a manifest's `source` names a repository, a folder in it
//! and a tag pattern (`my-ext-v{version}`). [`tags`] lists the repository's
//! tags without cloning (`git ls-remote`), each tag that matches the
//! pattern giving a version and the commit it points at; [`fetch`] clones
//! one tag shallowly into a hidden folder, checks that it is the commit
//! the list named, and copies only the extension's folder out of it,
//! within limits, refusing links that point outside it.
//!
//! Nothing from the fetched files runs: git checks the files out (no
//! hooks of the repository's, which a clone never carries), and Pitwall
//! only reads its manifest. Git never asks for a password here
//! (`GIT_TERMINAL_PROMPT=0`, no askpass program): a private repository
//! works through the user's own credential helper, or fails in plain
//! words, as does a computer without git or offline.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use super::install::{compare_versions, SKIPPED};
use super::{parse, Manifest, MANIFEST, VERSION_PLACEHOLDER};
use crate::exec::{Cmd, Exec, LocalExec};

/// The most a fetched extension may hold, in bytes.
pub const MAX_BYTES: u64 = 50 * 1024 * 1024;
/// The most files a fetched extension may hold.
pub const MAX_FILES: usize = 5000;

/// How long listing the tags may take.
const LIST_TIMEOUT: Duration = Duration::from_secs(30);
/// How long fetching a tag may take.
const FETCH_TIMEOUT: Duration = Duration::from_secs(180);

/// A release in a source: a tag that matches its pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    /// The tag's name (`my-ext-v1.2.0`).
    pub name: String,
    /// The version it names (`1.2.0`).
    pub version: String,
    /// The commit it points at (an annotated tag's own commit, not the
    /// tag object).
    pub commit: String,
}

impl Tag {
    /// Its commit, short.
    pub fn short(&self) -> &str {
        short(&self.commit)
    }
}

/// The first 7 characters of commit `c`.
pub fn short(c: &str) -> &str {
    &c[..c.len().min(7)]
}

/// The version tag `tag` names under pattern `pattern` (`None` when it
/// doesn't match, or what is in place of `{version}` isn't a version).
pub fn version_of(tag: &str, pattern: &str) -> Option<String> {
    let (before, after) = pattern.split_once(VERSION_PLACEHOLDER)?;
    let v = tag.strip_prefix(before)?.strip_suffix(after)?;
    let ok = !v.is_empty() && v.len() <= 32 && v.starts_with(|c: char| c.is_ascii_digit()) && v.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c));
    ok.then(|| v.to_string())
}

/// The tags in `git ls-remote --tags` output `text` that match `pattern`,
/// newest version first (an annotated tag's `^{}` line gives its commit).
pub fn parse_tags(text: &str, pattern: &str) -> Vec<Tag> {
    let mut out: Vec<Tag> = Vec::new();
    for line in text.lines() {
        let Some((commit, r)) = line.trim().split_once(char::is_whitespace) else { continue };
        let Some(name) = r.trim().strip_prefix("refs/tags/") else { continue };
        let commit = commit.trim();
        if commit.len() < 7 || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let (name, peeled) = match name.strip_suffix("^{}") {
            Some(n) => (n, true),
            None => (name, false),
        };
        let Some(version) = version_of(name, pattern) else { continue };
        match out.iter_mut().find(|t| t.name == name) {
            // The commit an annotated tag points at wins over the tag
            // object's own id.
            Some(t) if peeled => t.commit = commit.to_string(),
            Some(_) => {}
            None => out.push(Tag { name: name.to_string(), version, commit: commit.to_string() }),
        }
    }
    out.sort_by(|a, b| compare_versions(&b.version, &a.version).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Whether version `v` is a pre-release (`1.2.0-beta`).
pub fn is_pre(v: &str) -> bool {
    v.split('+').next().unwrap_or_default().contains('-')
}

/// The newest of `tags` (newest first, as [`parse_tags`] gives them); a
/// pre-release only when `pre` (the installed version is one).
pub fn newest(tags: &[Tag], pre: bool) -> Option<&Tag> {
    tags.iter().find(|t| pre || !is_pre(&t.version))
}

/// Run git without a way to ask anything: no terminal prompt, no askpass
/// program, no credential manager window, no large-file download.
fn git(args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut argv = vec!["git", "-c", "core.askPass=", "-c", "credential.interactive=false", "-c", "advice.detachedHead=false"];
    argv.extend_from_slice(args);
    let env = [("GIT_TERMINAL_PROMPT", "0"), ("GIT_ASKPASS", ""), ("SSH_ASKPASS", ""), ("GCM_INTERACTIVE", "never"), ("GIT_LFS_SKIP_SMUDGE", "1")];
    let out = LocalExec.run(&Cmd::new(&argv).env(&env).timeout(timeout)).map_err(|e| {
        let e = e.to_string();
        if e.contains("not found") || e.contains("No such file") || e.contains("cannot find") {
            "Git isn't installed on this computer: Pitwall needs it to get extensions from a git source.".to_string()
        } else {
            format!("Git couldn't run: {e}")
        }
    })?;
    if out.ok() {
        Ok(out.stdout_text())
    } else {
        Err(plain(&out.stderr_text()))
    }
}

/// What git said, in a line: offline, a sign-in it doesn't have, or its
/// own last words.
fn plain(stderr: &str) -> String {
    let low = stderr.to_lowercase();
    if low.contains("could not resolve host") || low.contains("couldn't resolve host") || low.contains("failed to connect") || low.contains("network is unreachable") || low.contains("timed out") {
        return "Couldn't reach it: this computer seems to be offline, or the server is down.".into();
    }
    if low.contains("terminal prompts disabled") || low.contains("authentication failed") || low.contains("could not read username") || low.contains("repository not found") || low.contains("403") {
        return "Couldn't read it: the repository doesn't exist, or it is private and git has no sign-in for it.".into();
    }
    let last = stderr.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("git failed");
    let last = last.trim_start_matches("fatal: ").trim_start_matches("error: ");
    format!("Git: {}", last.chars().take(200).collect::<String>())
}

/// The releases of the repository at `url` under tag pattern `pattern`,
/// newest first (blocking: call it off the UI thread; nothing is written).
pub fn tags(url: &str, pattern: &str) -> Result<Vec<Tag>, String> {
    Ok(parse_tags(&list(url)?, pattern))
}

/// The repository's tags as `git ls-remote --tags` lists them, for
/// [`parse_tags`] (blocking: call it off the UI thread).
pub fn list(url: &str) -> Result<String, String> {
    if url.starts_with('-') {
        return Err("That isn't a repository address.".into());
    }
    git(&["ls-remote", "--tags", "--", url], LIST_TIMEOUT)
}

/// A tag fetched: the extension's folder (hidden, beside the installed
/// ones), its manifest, and the tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub folder: PathBuf,
    pub manifest: Manifest,
    pub tag: Tag,
}

/// A hidden folder in `work` (hidden folders aren't listed as
/// extensions).
pub(crate) fn hidden(work: &Path, what: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or_default();
    work.join(format!(".{what}-{}-{nanos}", std::process::id()))
}

/// Fetch release `tag` of the repository at `url`: cloned shallowly into a
/// hidden folder in `work`, checked to be the commit `tag` names, and
/// folder `dir` of it copied into another hidden folder in `work` (which
/// the caller installs from, then deletes). `id`: the extension it must be
/// (an update), or any (Add from Git). Refused: another commit than the
/// list named, a folder that isn't there, a link pointing outside it,
/// more than [`MAX_BYTES`] or [`MAX_FILES`], a manifest that can't be
/// used, of another extension or of another version than the tag's.
pub fn fetch(url: &str, tag: &Tag, dir: &str, id: Option<&str>, work: &Path) -> Result<Fetched, String> {
    if url.starts_with('-') || tag.name.starts_with('-') {
        return Err("That isn't a repository address or a tag.".into());
    }
    super::check_source_dir(dir)?;
    std::fs::create_dir_all(work).map_err(|e| format!("Can't make {}: {e}", work.display()))?;
    let clone = hidden(work, "clone");
    let clone_s = clone.to_string_lossy().into_owned();
    // A hooks folder that doesn't exist: not even the user's own hooks run.
    let hooks = format!("core.hooksPath={}", clone.join(".no-hooks").to_string_lossy());
    let res = (|| {
        git(&["-c", &hooks, "clone", "--quiet", "--depth", "1", "--no-tags", "--single-branch", "--branch", &tag.name, "--", url, &clone_s], FETCH_TIMEOUT)?;
        let head = git(&["-C", &clone_s, "rev-parse", "HEAD"], LIST_TIMEOUT)?;
        let head = head.trim();
        if !head.eq_ignore_ascii_case(&tag.commit) {
            return Err(format!("The {} tag changed while it was fetched: it pointed at {} and now at {}. Not installed.", tag.name, tag.short(), short(head)));
        }
        let src = if dir.is_empty() { clone.clone() } else { clone.join(dir) };
        if !std::fs::symlink_metadata(&src).is_ok_and(|m| m.is_dir()) {
            return Err(format!("The {} tag has no folder {dir}.", tag.name));
        }
        let staged = hidden(work, "fetched");
        let mut count = (0usize, 0u64);
        if let Err(e) = copy_checked(&src, &src, &staged, &mut count) {
            let _ = std::fs::remove_dir_all(&staged);
            return Err(e);
        }
        Ok(staged)
    })();
    let _ = std::fs::remove_dir_all(&clone); // our own fresh clone
    let staged = res?;
    let what = format!("The {} tag", tag.name);
    match fetched_manifest(&staged, &what, if dir.is_empty() { "its root" } else { dir }, id, &tag.version) {
        Ok(manifest) => Ok(Fetched { folder: staged, manifest, tag: tag.clone() }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staged);
            Err(e)
        }
    }
}

/// The manifest of fetched folder `staged` (`what`: the release, as a
/// sentence starts, "The x-v1.0.0 tag"; `place`: where in it the manifest
/// was looked for), which must be extension `id` (any when `None`) at
/// `version`.
pub(crate) fn fetched_manifest(staged: &Path, what: &str, place: &str, id: Option<&str>, version: &str) -> Result<Manifest, String> {
    let file = staged.join(MANIFEST);
    let meta = std::fs::symlink_metadata(&file).ok().filter(|m| m.is_file()).ok_or_else(|| format!("{what} has no {MANIFEST} in {place}."))?;
    if meta.len() > 64 * 1024 {
        return Err(format!("Its {MANIFEST} is bigger than 64 KiB."));
    }
    let text = std::fs::read_to_string(&file).map_err(|e| format!("Its {MANIFEST} can't be read: {e}"))?;
    let m = parse(&text).map_err(|e| format!("It can't be used: {e}."))?;
    if let Some(id) = id.filter(|id| *id != m.id) {
        return Err(format!("{what} holds extension “{}”, not “{id}”. Not installed.", m.id));
    }
    if m.version.as_deref().is_none_or(|v| compare_versions(v, version) != std::cmp::Ordering::Equal) {
        return Err(format!("{what}'s {MANIFEST} says version {}, not {version}. Not installed.", m.version.as_deref().unwrap_or("none")));
    }
    Ok(m)
}

/// Where link `link` (in `root`) points, `None` when it points outside
/// `root` or anywhere absolute; worked out from the names, not the disk.
fn inside(root: &Path, link: &Path, target: &Path) -> Option<PathBuf> {
    let rel = link.parent()?.strip_prefix(root).ok()?;
    let mut parts: Vec<std::ffi::OsString> = rel.components().map(|c| c.as_os_str().to_owned()).collect();
    for c in target.components() {
        match c {
            Component::Normal(p) => parts.push(p.to_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts.iter().fold(root.to_path_buf(), |p, c| p.join(c)))
}

/// Copy folder `from` (inside `root`) to `to`, counting files and bytes in
/// `count`: what [`SKIPPED`] names left out; a link to a file in `root`
/// copied as that file; a link outside `root`, or to a folder, refused.
fn copy_checked(root: &Path, from: &Path, to: &Path, count: &mut (usize, u64)) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let shown = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name();
        if SKIPPED.iter().any(|s| name == *s) || name == super::install::MARKER {
            continue;
        }
        let path = entry.path();
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let dest = to.join(&name);
        let file = if kind.is_symlink() {
            let target = std::fs::read_link(&path).map_err(|e| e.to_string())?;
            let Some(real) = inside(root, &path, &target) else {
                return Err(format!("It has a link that points outside its folder ({}). Not installed.", shown(&path)));
            };
            match std::fs::symlink_metadata(&real) {
                Ok(m) if m.is_file() => real,
                Ok(m) if m.is_dir() => return Err(format!("It has a link to a folder ({}), which Pitwall doesn't copy. Not installed.", shown(&path))),
                // A link to a link, or to nothing: left out.
                _ => continue,
            }
        } else if kind.is_dir() {
            copy_checked(root, &path, &dest, count)?;
            continue;
        } else if kind.is_file() {
            path
        } else {
            continue;
        };
        let len = std::fs::metadata(&file).map_err(|e| e.to_string())?.len();
        count.0 += 1;
        count.1 += len;
        if count.0 > MAX_FILES {
            return Err(format!("It has more than {MAX_FILES} files. Not installed."));
        }
        if count.1 > MAX_BYTES {
            return Err(format!("It is bigger than {} MiB. Not installed.", MAX_BYTES / 1024 / 1024));
        }
        std::fs::copy(&file, &dest).map_err(|e| format!("{}: {e}", shown(&file)))?;
    }
    Ok(())
}

/// Why offering release `tag` of `url` is refused, when its commit isn't
/// the one seen before for the same tag: `installed`, the tag and commit
/// the installed copy came from (same source); `seen`, the commit an
/// earlier check saw.
pub fn moved(tag: &Tag, installed: Option<(&str, &str)>, seen: Option<&str>) -> Option<String> {
    let before = installed.filter(|(t, _)| *t == tag.name).map(|(_, c)| c).or(seen)?;
    (!before.eq_ignore_ascii_case(&tag.commit)).then(|| format!("The {} tag moved: it pointed at {} and now points at {}. Not installed.", tag.name, short(before), tag.short()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{extension_release, git_in as sh_git, TempDir};

    fn release(dir: &Path, id: &str, version: &str, main: &str, annotated: bool) -> String {
        extension_release(dir, id, version, main, "[]", annotated)
    }

    #[test]
    fn tags_give_versions_by_the_pattern() {
        assert_eq!(version_of("my-ext-v1.2.0", "my-ext-v{version}").as_deref(), Some("1.2.0"));
        assert_eq!(version_of("1.2.0-beta.1", "{version}").as_deref(), Some("1.2.0-beta.1"));
        assert_eq!(version_of("rel/1.0/final", "rel/{version}/final").as_deref(), Some("1.0"));
        assert_eq!(version_of("other-v1.2.0", "my-ext-v{version}"), None);
        assert_eq!(version_of("my-ext-vlatest", "my-ext-v{version}"), None);
        assert_eq!(version_of("my-ext-v", "my-ext-v{version}"), None);
        let text = "aaaaaaa1111111111111111111111111111111111\trefs/tags/x-v1.0.0\n\
                    bbbbbbb2222222222222222222222222222222222\trefs/tags/x-v1.10.0\n\
                    ccccccc3333333333333333333333333333333333\trefs/tags/x-v1.10.0^{}\n\
                    ddddddd4444444444444444444444444444444444\trefs/tags/x-v2.0.0-beta\n\
                    eeeeeee5555555555555555555555555555555555\trefs/tags/other-v9.0.0\n\
                    fffffff6666666666666666666666666666666666\trefs/heads/main\n";
        let tags = parse_tags(text, "x-v{version}");
        let names: Vec<&str> = tags.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["x-v2.0.0-beta", "x-v1.10.0", "x-v1.0.0"], "newest first, others left out");
        assert!(tags[1].commit.starts_with("ccccccc"), "an annotated tag: the commit it points at");
        assert_eq!(newest(&tags, false).map(|t| t.version.as_str()), Some("1.10.0"), "no pre-release unless one is installed");
        assert_eq!(newest(&tags, true).map(|t| t.version.as_str()), Some("2.0.0-beta"));
        assert!(newest(&[], false).is_none());
    }

    /// A moved tag is refused, against the installed copy's record or an
    /// earlier check.
    #[test]
    fn a_moved_tag_is_refused() {
        let t = Tag { name: "x-v1.0.0".into(), version: "1.0.0".into(), commit: "bbbbbbbb".into() };
        assert_eq!(moved(&t, None, None), None);
        assert_eq!(moved(&t, Some(("x-v1.0.0", "bbbbbbbb")), None), None);
        assert_eq!(moved(&t, Some(("x-v0.9.0", "aaaaaaaa")), None), None, "another tag says nothing");
        assert!(moved(&t, Some(("x-v1.0.0", "aaaaaaaa")), None).unwrap().contains("moved: it pointed at aaaaaaa and now points at bbbbbbb"));
        assert!(moved(&t, None, Some("aaaaaaaa")).is_some());
    }

    /// Listing and fetching from a local repository: the newest tag, its
    /// commit checked, only the extension's folder copied, and nothing of
    /// the clone left behind.
    #[test]
    fn a_release_is_listed_and_fetched() {
        let t = TempDir::new("ext-source");
        let repo = t.path().join("repo");
        let c1 = release(&repo, "inbox", "1.0.0", "// 1", false);
        let c2 = release(&repo, "inbox", "1.1.0", "// 2", true);
        let url = repo.to_string_lossy().into_owned();
        let tags = tags(&url, "ext-v{version}").unwrap();
        assert_eq!(tags.iter().map(|t| (t.version.as_str(), t.commit.as_str())).collect::<Vec<_>>(), [("1.1.0", c2.as_str()), ("1.0.0", c1.as_str())]);
        let work = t.path().join("data/extensions");
        let f = fetch(&url, &tags[0], "ext", Some("inbox"), &work).unwrap();
        assert_eq!(f.manifest.version.as_deref(), Some("1.1.0"));
        assert_eq!(std::fs::read_to_string(f.folder.join("adapter/main.mjs")).unwrap(), "// 2");
        assert!(!f.folder.join("README.md").exists() && !f.folder.join(".git").exists(), "only its folder");
        let left: Vec<String> = std::fs::read_dir(&work).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left.len(), 1, "only the fetched folder: {left:?}");
        assert!(left[0].starts_with(".fetched-"));
        std::fs::remove_dir_all(&f.folder).unwrap();
        // The older one, by its own tag.
        let f = fetch(&url, &tags[1], "ext", None, &work).unwrap();
        assert_eq!(std::fs::read_to_string(f.folder.join("adapter/main.mjs")).unwrap(), "// 1");
        std::fs::remove_dir_all(&f.folder).unwrap();
    }

    /// What isn't what the list said, or what the caller asked for, is
    /// refused, and leaves nothing.
    #[test]
    fn a_fetch_refuses_what_doesnt_match() {
        let t = TempDir::new("ext-source-refuse");
        let repo = t.path().join("repo");
        release(&repo, "inbox", "1.0.0", "// 1", false);
        let url = repo.to_string_lossy().into_owned();
        let work = t.path().join("work");
        let tag = tags(&url, "ext-v{version}").unwrap().remove(0);
        let empty = |w: &Path| std::fs::read_dir(w).map(|d| d.count()).unwrap_or(0) == 0;
        // Another commit than listed: the tag moved since.
        let wrong = Tag { commit: "0".repeat(40), ..tag.clone() };
        assert!(fetch(&url, &wrong, "ext", None, &work).unwrap_err().contains("changed while it was fetched"));
        assert!(fetch(&url, &tag, "ext", Some("other"), &work).unwrap_err().contains("holds extension “inbox”, not “other”"));
        assert!(fetch(&url, &tag, "nope", None, &work).unwrap_err().contains("has no folder nope"));
        assert!(fetch(&url, &tag, "", None, &work).unwrap_err().contains("no extension.json in its root"));
        assert!(fetch(&url, &tag, "../x", None, &work).is_err());
        let liar = Tag { version: "2.0.0".into(), ..tag.clone() };
        assert!(fetch(&url, &liar, "ext", None, &work).unwrap_err().contains("says version 1.0.0, not 2.0.0"));
        assert!(empty(&work), "nothing left behind");
        // Offline or not a repository: a plain word, no prompt.
        let gone = t.path().join("gone").to_string_lossy().into_owned();
        assert!(tags(&gone, "ext-v{version}").is_err());
    }

    /// Links: one to a file inside the folder is copied as that file; one
    /// pointing outside it (or to a folder) refuses the whole release.
    #[test]
    fn links_outside_the_folder_are_refused() {
        let t = TempDir::new("ext-source-links");
        let root = t.path().join("src");
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/file.txt"), "x").unwrap();
        assert_eq!(inside(&root, &root.join("a/b/link"), Path::new("../file.txt")), Some(root.join("a/file.txt")));
        assert_eq!(inside(&root, &root.join("a/link"), Path::new("../../secret")), None);
        assert_eq!(inside(&root, &root.join("link"), Path::new("/etc/passwd")), None);
        assert_eq!(inside(&root, &root.join("link"), Path::new("./a/./file.txt")), Some(root.join("a/file.txt")));

        // A release with links, committed as links (git makes them, so
        // the test is the same everywhere; where the system has no links,
        // git checks them out as files and there is nothing to refuse).
        let repo = t.path().join("repo");
        release(&repo, "inbox", "1.0.0", "// 1", false);
        let link = |name: &str, target: &str| {
            std::fs::write(t.path().join("target"), target).unwrap();
            let blob = sh_git(&repo, &["hash-object", "-w", &t.path().join("target").to_string_lossy()]);
            sh_git(&repo, &["update-index", "--add", "--cacheinfo", &format!("120000,{blob},ext/{name}")]);
            sh_git(&repo, &["checkout", "--", &format!("ext/{name}")]);
            std::fs::symlink_metadata(repo.join("ext").join(name)).unwrap().file_type().is_symlink()
        };
        let links = link("same.mjs", "adapter/main.mjs");
        std::fs::write(repo.join("ext/extension.json"), std::fs::read_to_string(repo.join("ext/extension.json")).unwrap().replace("1.0.0", "1.0.1")).unwrap();
        sh_git(&repo, &["commit", "--quiet", "-am", "1.0.1"]);
        sh_git(&repo, &["tag", "ext-v1.0.1"]);
        let url = repo.to_string_lossy().into_owned();
        let work = t.path().join("work");
        let tag = tags(&url, "ext-v{version}").unwrap().remove(0);
        let f = fetch(&url, &tag, "ext", None, &work).unwrap();
        assert_eq!(std::fs::read_to_string(f.folder.join("same.mjs")).unwrap(), "// 1", "a link inside: copied as its file");
        assert!(!std::fs::symlink_metadata(f.folder.join("same.mjs")).unwrap().file_type().is_symlink());
        link("passwd", "../../outside");
        std::fs::write(repo.join("ext/extension.json"), std::fs::read_to_string(repo.join("ext/extension.json")).unwrap().replace("1.0.1", "1.0.2")).unwrap();
        sh_git(&repo, &["commit", "--quiet", "-am", "1.0.2"]);
        sh_git(&repo, &["tag", "ext-v1.0.2"]);
        let tag = tags(&url, "ext-v{version}").unwrap().remove(0);
        assert_eq!(tag.version, "1.0.2");
        let res = fetch(&url, &tag, "ext", None, &work);
        if links {
            assert!(res.unwrap_err().contains("points outside its folder (passwd)"));
        }
    }

    /// Too many files: refused.
    #[test]
    fn the_limits_hold() {
        let t = TempDir::new("ext-source-limits");
        let root = t.path().join("src");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a"), "12345").unwrap();
        let mut count = (MAX_FILES, 0);
        assert!(copy_checked(&root, &root, &t.path().join("out1"), &mut count).unwrap_err().contains("more than 5000 files"));
        let mut count = (0, MAX_BYTES - 2);
        assert!(copy_checked(&root, &root, &t.path().join("out2"), &mut count).unwrap_err().contains("bigger than 50 MiB"));
        let mut count = (0, 0);
        copy_checked(&root, &root, &t.path().join("out3"), &mut count).unwrap();
        assert_eq!(count, (1, 5));
    }
}
