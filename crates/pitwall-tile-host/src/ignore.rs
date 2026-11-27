//! What a project's `.gitignore` files say, for `files.list` and
//! `files.watch` (files.wit): the patterns of `.git/info/exclude` and of
//! each `.gitignore` from the project's folder down, with git's rules
//! (gitignore(5)): `#` comments, `!` to take a pattern back, a trailing
//! `/` for folders only, a `/` at the start or in the middle anchors the
//! pattern to its file's folder, `*`, `?`, `[…]` and `**`. A path in an
//! ignored folder is ignored, whatever its own patterns say, as in git.
//! The user's global excludes (`core.excludesFile`) aren't read.

use std::collections::BTreeMap;
use std::path::Path;

use regex::Regex;

/// One pattern line.
#[derive(Debug, Clone)]
struct Pattern {
    /// The folder of the file it came from, from the root ("" for the root
    /// and `.git/info/exclude`).
    base: String,
    negate: bool,
    dir_only: bool,
    re: Regex,
}

/// The patterns that apply in a project, loaded folder by folder as they
/// are asked about.
#[derive(Debug, Default)]
pub struct Ignore {
    root: std::path::PathBuf,
    /// Each folder's own `.gitignore` patterns, once read.
    by_dir: BTreeMap<String, Vec<Pattern>>,
    exclude: Vec<Pattern>,
}

impl Ignore {
    /// The rules of the project in `root` (its resolved folder).
    pub fn new(root: &Path) -> Ignore {
        let exclude = std::fs::read_to_string(root.join(".git").join("info").join("exclude")).map(|t| parse("", &t)).unwrap_or_default();
        Ignore { root: root.to_path_buf(), by_dir: BTreeMap::new(), exclude }
    }

    fn patterns_of(&mut self, dir: &str) -> &[Pattern] {
        if !self.by_dir.contains_key(dir) {
            let file = if dir.is_empty() { self.root.join(".gitignore") } else { self.root.join(dir).join(".gitignore") };
            let pats = std::fs::read_to_string(file).map(|t| parse(dir, &t)).unwrap_or_default();
            self.by_dir.insert(dir.to_string(), pats);
        }
        &self.by_dir[dir]
    }

    /// Whether `path` (from the root, `/` between parts) is ignored: it,
    /// or a folder it is in.
    pub fn ignored(&mut self, path: &str, is_dir: bool) -> bool {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        for i in 1..=parts.len() {
            let sub = parts[..i].join("/");
            let dir = i < parts.len() || is_dir;
            if self.matches(&sub, dir) {
                return true;
            }
        }
        false
    }

    /// Whether `path` itself is ignored by the patterns that apply to it
    /// (its folders not considered).
    pub fn matches(&mut self, path: &str, is_dir: bool) -> bool {
        let mut dirs = vec![String::new()];
        let parts: Vec<&str> = path.split('/').collect();
        for i in 1..parts.len() {
            dirs.push(parts[..i].join("/"));
        }
        let mut verdict = None;
        for p in self.exclude.clone().iter() {
            if let Some(v) = check(p, path, is_dir) {
                verdict = Some(v);
            }
        }
        // Deeper files come later and win.
        for d in dirs {
            for p in self.patterns_of(&d).to_vec() {
                if let Some(v) = check(&p, path, is_dir) {
                    verdict = Some(v);
                }
            }
        }
        verdict.unwrap_or(false)
    }
}

/// What pattern `p` says of `path`: `Some(true)` ignored, `Some(false)`
/// taken back, `None` it doesn't match.
fn check(p: &Pattern, path: &str, is_dir: bool) -> Option<bool> {
    if p.dir_only && !is_dir {
        return None;
    }
    let rel = if p.base.is_empty() {
        path
    } else {
        path.strip_prefix(p.base.as_str()).and_then(|r| r.strip_prefix('/'))?
    };
    p.re.is_match(rel).then_some(!p.negate)
}

/// The patterns of a `.gitignore` in folder `base`.
fn parse(base: &str, text: &str) -> Vec<Pattern> {
    text.lines().filter_map(|l| pattern(base, l)).collect()
}

fn pattern(base: &str, line: &str) -> Option<Pattern> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    // Trailing spaces go unless escaped.
    let mut line = line.to_string();
    while line.ends_with(' ') && !line.ends_with("\\ ") {
        line.pop();
    }
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (negate, rest) = match line.strip_prefix('!') {
        Some(r) => (true, r.to_string()),
        None => (false, line.strip_prefix('\\').filter(|r| r.starts_with(['#', '!'])).map(str::to_string).unwrap_or(line.clone())),
    };
    let (dir_only, rest) = match rest.strip_suffix('/') {
        Some(r) => (true, r.to_string()),
        None => (false, rest),
    };
    if rest.is_empty() {
        return None;
    }
    let anchored = rest.contains('/');
    let body = rest.strip_prefix('/').unwrap_or(&rest);
    let glob = glob_to_regex(body);
    let re = if anchored { format!("^{glob}$") } else { format!("^(?:.*/)?{glob}$") };
    Some(Pattern { base: base.to_string(), negate, dir_only, re: Regex::new(&re).ok()? })
}

/// A gitignore glob as a regex body (no anchors).
fn glob_to_regex(glob: &str) -> String {
    let c: Vec<char> = glob.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '*' if c.get(i + 1) == Some(&'*') => {
                let at_start = i == 0 || c[i - 1] == '/';
                let at_end = i + 2 == c.len();
                let slash_after = c.get(i + 2) == Some(&'/');
                if at_start && slash_after {
                    // `**/`: any folders, or none.
                    out.push_str("(?:.*/)?");
                    i += 3;
                } else if at_start && at_end {
                    // `/**` or a bare `**`: everything inside.
                    out.push_str(".*");
                    i += 2;
                } else {
                    out.push_str("[^/]*");
                    i += 2;
                }
            }
            '*' => {
                out.push_str("[^/]*");
                i += 1;
            }
            '?' => {
                out.push_str("[^/]");
                i += 1;
            }
            '[' => match c[i + 1..].iter().position(|&x| x == ']') {
                Some(len) if len > 0 => {
                    let inner: String = c[i + 1..i + 1 + len].iter().collect();
                    let inner = match inner.strip_prefix(['!', '^']) {
                        Some(r) => format!("^{}", r.replace('\\', "\\\\")),
                        None => inner.replace('\\', "\\\\"),
                    };
                    out.push('[');
                    out.push_str(&inner);
                    out.push(']');
                    i += len + 2;
                }
                _ => {
                    out.push_str("\\[");
                    i += 1;
                }
            },
            '\\' if i + 1 < c.len() => {
                out.push_str(&regex::escape(&c[i + 1].to_string()));
                i += 2;
            }
            ch => {
                out.push_str(&regex::escape(&ch.to_string()));
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (p, t) in files {
            let f = d.path().join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, t).unwrap();
        }
        d
    }

    #[test]
    fn git_rules() {
        let d = project(&[
            (".gitignore", "# build output\ntarget/\n*.log\n!keep.log\n/root-only.txt\ndocs/**/draft-*.md\nnode_modules\n"),
            ("sub/.gitignore", "secret.md\n/local/\n"),
            (".git/info/exclude", "scratch\n"),
        ]);
        let mut ig = Ignore::new(d.path());
        let cases = [
            ("target", true, true),
            ("target", false, false),
            ("crates/x/target", true, true),
            ("crates/x/target/debug/a.o", false, true),
            ("a.log", false, true),
            ("x/y/b.log", false, true),
            ("keep.log", false, false),
            ("root-only.txt", false, true),
            ("sub/root-only.txt", false, false),
            ("docs/draft-1.md", false, true),
            ("docs/a/b/draft-2.md", false, true),
            ("docs/final.md", false, false),
            ("web/node_modules/x/index.js", false, true),
            ("sub/secret.md", false, true),
            ("sub/deeper/secret.md", false, true),
            ("secret.md", false, false),
            ("sub/local/a", false, true),
            ("sub/x/local/a", false, false),
            ("scratch", false, true),
            ("docs/arch.md", false, false),
        ];
        for (path, dir, want) in cases {
            assert_eq!(ig.ignored(path, dir), want, "{path}");
        }
    }

    #[test]
    fn globs() {
        let re = |g: &str| Regex::new(&format!("^{}$", glob_to_regex(g))).unwrap();
        assert!(re("a?c").is_match("abc") && !re("a?c").is_match("a/c"));
        assert!(re("[ab]x").is_match("bx") && !re("[!ab]x").is_match("ax"));
        assert!(re("a/**/b").is_match("a/b") && re("a/**/b").is_match("a/x/y/b"));
        assert!(re("a/**").is_match("a/x/y"));
        assert!(re("*.md").is_match("x.md") && !re("*.md").is_match("d/x.md"));
        assert!(re("a+b(c)").is_match("a+b(c)"));
    }
}
