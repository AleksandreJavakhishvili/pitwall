//! Which of a project's files hold its docs diagrams (frd.md §3.1, FR-2):
//! the path list, its globs, the folders never looked in, and the limits.
//! Plain Rust, tested natively; the tile reads the files with `files`.

/// The path list when the user hasn't set one (frd.md §3.1).
pub const DEFAULT_PATHS: &str = "docs/**/*.md, **/hla.md, **/*-lld.md";
/// The most Markdown files read (FR-2, a proposal).
pub const MAX_FILES: usize = 2_000;
/// The largest Markdown file read (FR-2, a proposal).
pub const MAX_FILE_BYTES: u64 = 1 << 20;

/// Folders never looked in, besides hidden ones (FR-2).
const SKIPPED: &[&str] = &["node_modules", "target"];

/// The path list's globs, from the setting's text: comma or newline
/// separated, blanks dropped; the default when it is empty.
pub fn globs(setting: Option<&str>) -> Vec<String> {
    let text = setting.filter(|s| !s.trim().is_empty()).unwrap_or(DEFAULT_PATHS);
    text.split([',', '\n']).map(str::trim).filter(|s| !s.is_empty()).map(|s| s.trim_start_matches("./").to_string()).collect()
}

/// Whether `path` (relative, with `/`) is in a folder never looked in:
/// hidden, `node_modules` or `target` (FR-2; `.git` the host never
/// lists).
pub fn skipped(path: &str) -> bool {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    parts.iter().any(|p| p.starts_with('.') || SKIPPED.contains(p))
}

/// Whether Markdown file `path` is one the list names.
pub fn wanted(globs: &[String], path: &str) -> bool {
    path.ends_with(".md") && !skipped(path) && globs.iter().any(|g| glob(g, path))
}

/// Matches `path` against `pattern`: `**` any folders (none too), `*`
/// anything within a name, `?` one character.
pub fn glob(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let parts: Vec<&str> = path.split('/').collect();
    parts_match(&pat, &parts)
}

fn parts_match(pat: &[&str], parts: &[&str]) -> bool {
    match pat.split_first() {
        None => parts.is_empty(),
        Some((&"**", rest)) => (0..=parts.len()).any(|n| parts_match(rest, &parts[n..])),
        Some((p, rest)) => parts.split_first().is_some_and(|(first, others)| name_match(p.as_bytes(), first.as_bytes()) && parts_match(rest, others)),
    }
}

fn name_match(p: &[u8], s: &[u8]) -> bool {
    match p.split_first() {
        None => s.is_empty(),
        Some((b'*', rest)) => (0..=s.len()).any(|n| name_match(rest, &s[n..])),
        Some((b'?', rest)) => !s.is_empty() && name_match(rest, &s[1..]),
        Some((c, rest)) => s.first() == Some(c) && name_match(rest, &s[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_list_finds_the_docs() {
        let g = globs(None);
        for yes in ["docs/hla.md", "docs/spec/c4/frd.md", "crates/x/hla.md", "hla.md", "src/files-lld.md", "docs/a/b/c.md"] {
            assert!(wanted(&g, yes), "{yes}");
        }
        for no in ["README.md", "docs/a.txt", "node_modules/x/docs/hla.md", "target/docs/a.md", "docs/.drafts/a.md", ".github/hla.md", "src/lld.md"] {
            assert!(!wanted(&g, no), "{no}");
        }
    }

    #[test]
    fn a_set_list_replaces_the_default() {
        let g = globs(Some(" architecture/*.md ,\n\nREADME.md "));
        assert_eq!(g, ["architecture/*.md", "README.md"]);
        assert!(wanted(&g, "README.md") && wanted(&g, "architecture/c4.md"));
        assert!(!wanted(&g, "architecture/deep/c4.md") && !wanted(&g, "docs/hla.md"));
        assert_eq!(globs(Some("  ")), globs(None));
    }

    #[test]
    fn globs_match_names_and_folders() {
        assert!(glob("**/*-lld.md", "a/b/files-lld.md"));
        assert!(glob("docs/**", "docs/a/b"));
        assert!(glob("d?cs/*.md", "docs/a.md"));
        assert!(!glob("docs/*.md", "docs/a/b.md"));
        assert!(!glob("*.md", "a/b.md"));
    }
}
