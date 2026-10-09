//! File references in terminal text (`src/a.rs:42:7`, `./README.md`,
//! `"docs/my notes.md"`, rustc's `--> src/main.rs:4:5`, tsc's
//! `file.ts(12,5)`, `lib.rs (line 10)`, Python's `File "x.py", line 3`),
//! resolved against a folder and checked on disk.
//!
//! Parsing is plain and cheap (one pass over one line's `char`s); it finds
//! candidates only. Whether a candidate is a link is decided by
//! [`existing_file`], which touches the disk and so runs off the UI thread.

use std::ops::Range;
use std::path::{Path, PathBuf};

/// A file reference found in a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathRef {
    /// The `char`s it covers in the line (the path and its `:line:col`).
    pub range: Range<usize>,
    /// The path as written (quotes and location removed).
    pub path: String,
    /// 1-based line, when given.
    pub line: Option<u32>,
    /// 1-based column, when given.
    pub column: Option<u32>,
}

/// Longest path considered.
const MAX_PATH: usize = 1024;

/// Characters that end an unquoted path.
fn breaks(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '|' | ',' | ';'
        )
}

fn is_quote(c: char) -> bool {
    matches!(c, '"' | '\'' | '`')
}

/// Every file reference candidate in `line`.
pub fn find_paths(line: &[char]) -> Vec<PathRef> {
    let n = line.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let c = line[i];
        if is_quote(c) {
            // A quoted path may hold spaces; the quote must close on this line.
            let close = line[i + 1..]
                .iter()
                .position(|&q| q == c)
                .map(|p| i + 1 + p);
            if let Some(j) = close {
                let inner: String = line[i + 1..j].iter().collect();
                if !inner.starts_with(' ') && !inner.ends_with(' ') && looks_like_path(&inner) {
                    let (path, mut line_no, mut col) = split_location(&inner);
                    let mut next = j + 1;
                    if line_no.is_none() {
                        if let Some((l, c2, len)) = location_after(line, j + 1) {
                            line_no = Some(l);
                            col = c2;
                            next = j + 1 + len;
                        }
                    }
                    // The underline stays inside the quotes.
                    out.push(PathRef {
                        range: i + 1..j,
                        path,
                        line: line_no,
                        column: col,
                    });
                    i = next;
                    continue;
                }
            }
            i += 1;
            continue;
        }
        if breaks(c) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n && !breaks(line[j]) {
            j += 1;
        }
        let token: String = line[i..j].iter().collect();
        if let Some(r) = token_ref(&token, i, line, j) {
            let next = r.range.end.max(j);
            out.push(r);
            i = next;
        } else {
            i = j;
        }
    }
    out
}

/// The reference in an unquoted `token` starting at `start` (`end`: where
/// it stopped in `line`, to read a location written after it).
fn token_ref(token: &str, start: usize, line: &[char], end: usize) -> Option<PathRef> {
    if token.contains("://") {
        return None;
    }
    // `--flag=path`: the part after the last `=`.
    let (skip, rest) = match token.rfind('=') {
        Some(k) => (token[..=k].chars().count(), &token[k + 1..]),
        None => (0, token),
    };
    // pytest's `tests/test_x.py::test_name`.
    let rest = rest.split("::").next().unwrap_or(rest);
    // Sentence punctuation after it.
    let trimmed = rest.trim_end_matches(['.', ',', ':', ';', '!', '?']);
    let (path, mut line_no, mut col) = split_location(trimmed);
    if !looks_like_path(&path) {
        return None;
    }
    let from = start + skip;
    let mut to = from + trimmed.chars().count();
    if line_no.is_none() && to == end {
        if let Some((l, c, len)) = location_after(line, end) {
            line_no = Some(l);
            col = c;
            to = end + len;
        }
    }
    Some(PathRef {
        range: from..to,
        path,
        line: line_no,
        column: col,
    })
}

/// `path:12`, `path:12:5` → the path, line and column.
fn split_location(s: &str) -> (String, Option<u32>, Option<u32>) {
    let Some((head, last)) = s.rsplit_once(':') else {
        return (s.to_string(), None, None);
    };
    let Some(n) = number(last) else {
        return (s.to_string(), None, None);
    };
    if let Some((path, mid)) = head.rsplit_once(':') {
        if let Some(line) = number(mid).filter(|_| !path.is_empty()) {
            return (path.to_string(), Some(line), Some(n));
        }
    }
    (head.to_string(), Some(n), None)
}

fn number(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 9 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// A location written right after a path: `(12,5)` / `(12)` (tsc, MSBuild),
/// ` (line 12)`, `, line 12` (Python), `:12` after a quoted path. Its
/// line, column and length.
fn location_after(line: &[char], at: usize) -> Option<(u32, Option<u32>, usize)> {
    let rest: String = line.get(at..)?.iter().take(24).collect();
    let digits = |s: &str| -> Option<(u32, usize)> {
        let d: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
        if d.is_empty() || d.len() > 9 {
            return None;
        }
        Some((d.parse().ok()?, d.len()))
    };
    if let Some(r) = rest.strip_prefix(':') {
        let (l, ln) = digits(r)?;
        let r2 = &r[ln..];
        if let Some((c, cn)) = r2.strip_prefix(':').and_then(digits) {
            return Some((l, Some(c), 1 + ln + 1 + cn));
        }
        return Some((l, None, 1 + ln));
    }
    if let Some(r) = rest.strip_prefix('(') {
        let (l, ln) = digits(r)?;
        let r2 = &r[ln..];
        if let Some(r3) = r2.strip_prefix(',') {
            let (c, cn) = digits(r3)?;
            if r3[cn..].starts_with(')') {
                return Some((l, Some(c), 1 + ln + 1 + cn + 1));
            }
        } else if r2.starts_with(')') {
            return Some((l, None, 1 + ln + 1));
        }
        return None;
    }
    for prefix in [" (line ", ", line "] {
        if let Some(r) = rest.strip_prefix(prefix) {
            let (l, ln) = digits(r)?;
            let close = usize::from(prefix.starts_with(" (") && r[ln..].starts_with(')'));
            if prefix.starts_with(" (") && close == 0 {
                return None;
            }
            return Some((l, None, prefix.len() + ln + close));
        }
    }
    None
}

/// Whether `s` could name a file: it has a `/`, starts with `~/`, or ends
/// in a `.ext`; and it has a letter (not `1.5`, not `...`).
pub fn looks_like_path(s: &str) -> bool {
    if s.is_empty() || s.len() > MAX_PATH || s.contains("://") || s.contains('\0') {
        return false;
    }
    if !s.chars().any(char::is_alphabetic) {
        return false;
    }
    if s.contains('/') {
        return !s.contains("//");
    }
    // A bare name: `name.ext` or `.dotfile`.
    let Some((stem, ext)) = s.rsplit_once('.') else {
        return false;
    };
    !ext.is_empty()
        && ext.chars().count() <= 12
        && ext.chars().all(char::is_alphanumeric)
        && !stem.ends_with('.')
}

/// `path` as a path on this machine: `~/…` under `home`, absolute as is,
/// else under `cwd`.
pub fn resolve(path: &str, cwd: &Path, home: Option<&Path>) -> Option<PathBuf> {
    if path == "~" {
        return home.map(Path::to_path_buf);
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home.map(|h| h.join(rest));
    }
    let p = Path::new(path);
    Some(if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    })
}

/// The file `path` names, with every link resolved, if it is a file (not a
/// folder). Touches the disk: call it off the UI thread.
pub fn existing_file(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    std::fs::canonicalize(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(s: &str) -> Vec<(String, String, Option<u32>, Option<u32>)> {
        let chars: Vec<char> = s.chars().collect();
        find_paths(&chars)
            .into_iter()
            .map(|r| {
                (
                    chars[r.range.clone()].iter().collect(),
                    r.path,
                    r.line,
                    r.column,
                )
            })
            .collect()
    }

    fn one(s: &str) -> (String, Option<u32>, Option<u32>) {
        let v = find(s);
        assert_eq!(v.len(), 1, "{s}: {v:?}");
        let (_, p, l, c) = v.into_iter().next().unwrap();
        (p, l, c)
    }

    #[test]
    fn line_and_column_suffixes() {
        assert_eq!(
            one("src/cart/cart.ts:42"),
            ("src/cart/cart.ts".into(), Some(42), None)
        );
        assert_eq!(
            one("at src/cart/cart.ts:42:7 here"),
            ("src/cart/cart.ts".into(), Some(42), Some(7))
        );
        assert_eq!(one("see ./README.md."), ("./README.md".into(), None, None));
        assert_eq!(
            one("/abs/path/file.rs:3"),
            ("/abs/path/file.rs".into(), Some(3), None)
        );
        assert_eq!(
            one("~/code/x/y.py:10"),
            ("~/code/x/y.py".into(), Some(10), None)
        );
        // The underline covers the location too.
        assert_eq!(find("x src/a.rs:4:5, y")[0].0, "src/a.rs:4:5");
    }

    #[test]
    fn compiler_and_test_formats() {
        assert_eq!(
            one("  --> src/main.rs:4:5"),
            ("src/main.rs".into(), Some(4), Some(5))
        );
        assert_eq!(
            one("src/app.ts(12,5): error TS2322"),
            ("src/app.ts".into(), Some(12), Some(5))
        );
        assert_eq!(find("src/app.ts(12,5): error")[0].0, "src/app.ts(12,5)");
        assert_eq!(
            one("tests/test_cart.py:10: AssertionError"),
            ("tests/test_cart.py".into(), Some(10), None)
        );
        assert_eq!(
            one("FAILED tests/test_cart.py::test_total"),
            ("tests/test_cart.py".into(), None, None)
        );
        assert_eq!(
            one("  File \"app/main.py\", line 7, in run"),
            ("app/main.py".into(), Some(7), None)
        );
        assert_eq!(
            one("crates/foo/src/lib.rs (line 10)"),
            ("crates/foo/src/lib.rs".into(), Some(10), None)
        );
        assert_eq!(
            one("--manifest-path=crates/x/Cargo.toml"),
            ("crates/x/Cargo.toml".into(), None, None)
        );
    }

    #[test]
    fn quoted_and_bracketed() {
        assert_eq!(
            one("open \"docs/my notes.md\" now"),
            ("docs/my notes.md".into(), None, None)
        );
        assert_eq!(
            one("wrote 'src/a b/c.rs':9"),
            ("src/a b/c.rs".into(), Some(9), None)
        );
        assert_eq!(one("(see src/lib.rs)"), ("src/lib.rs".into(), None, None));
        assert_eq!(one("[src/lib.rs:3]"), ("src/lib.rs".into(), Some(3), None));
        assert_eq!(one("`Cargo.toml`"), ("Cargo.toml".into(), None, None));
    }

    #[test]
    fn unicode_names() {
        assert_eq!(
            one("ფაილი/მთავარი.rs:12"),
            ("ფაილი/მთავარი.rs".into(), Some(12), None)
        );
        assert_eq!(
            one("\"დოკები/ჩემი ჩანაწერი.md\""),
            ("დოკები/ჩემი ჩანაწერი.md".into(), None, None)
        );
        // Ranges are in chars, so they map to cells.
        let chars: Vec<char> = "ok: ტესტი.py:3".chars().collect();
        let r = &find_paths(&chars)[0];
        assert_eq!(r.range, 4..chars.len());
    }

    #[test]
    fn not_paths() {
        for s in [
            "version 1.5 and 2.0.1",
            "https://example.com/a/b.html",
            "just some words here",
            "it's fine, isn't it",
            "...",
            "a // comment",
            "12:30:45",
        ] {
            assert!(find(s).is_empty(), "{s}: {:?}", find(s));
        }
    }

    #[test]
    fn resolves_against_the_folder() {
        let cwd = Path::new("/work/proj");
        let home = Path::new("/home/demo");
        assert_eq!(
            resolve("src/a.rs", cwd, Some(home)).unwrap(),
            Path::new("/work/proj/src/a.rs")
        );
        assert_eq!(
            resolve("./README.md", cwd, Some(home)).unwrap(),
            Path::new("/work/proj/./README.md")
        );
        assert_eq!(
            resolve("/etc/hosts", cwd, Some(home)).unwrap(),
            Path::new("/etc/hosts")
        );
        assert_eq!(
            resolve("~/x/y.py", cwd, Some(home)).unwrap(),
            Path::new("/home/demo/x/y.py")
        );
        assert_eq!(resolve("~/x/y.py", cwd, None), None);
    }

    #[test]
    fn only_existing_files() {
        let dir = std::env::temp_dir().join(format!("pw-paths-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join("ჩემი ფაილი.txt"), "x").unwrap();
        let found = existing_file(&resolve("src/a.rs", &dir, None).unwrap()).unwrap();
        assert!(found.is_absolute() && found.ends_with("src/a.rs"));
        assert!(existing_file(&resolve("ჩემი ფაილი.txt", &dir, None).unwrap()).is_some());
        assert!(
            existing_file(&resolve("src", &dir, None).unwrap()).is_none(),
            "folders are not links"
        );
        assert!(existing_file(&resolve("src/missing.rs", &dir, None).unwrap()).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
