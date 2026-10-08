//! The explorer's UI logic, free of GPUI so it can be tested (a port of
//! `src/lib/explorer.ts`): quick-open matching, the lazy tree's rows, search
//! result grouping, and small path and size helpers.
//!
//! Text positions here are `char` indexes unless named `byte`: paths and
//! lines may hold any script (Georgian file names, for one), and GPUI's
//! highlight ranges are byte ranges, so conversions are explicit.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use pitwall_core::explorer::{DirListing, EntryKind, FileEntry, SearchMatch};

// ── quick open (⌘P) ──────────────────────────────────────────────────────

/// One quick-open match.
#[derive(Debug, Clone, PartialEq)]
pub struct QuickHit {
    pub path: String,
    pub score: f64,
    /// `char` indexes into `path` of the matched characters (sorted).
    pub positions: Vec<usize>,
}

fn is_sep(c: char) -> bool {
    matches!(c, '/' | '\\' | '_' | '-' | '.') || c.is_whitespace()
}

/// One lower-case `char` per `char` (keeps indexes aligned; a few scripts
/// lower-case to several chars, which only the first of is kept).
fn lower_chars(s: &str) -> Vec<char> {
    s.chars()
        .map(|c| c.to_lowercase().next().unwrap_or(c))
        .collect()
}

/// VS Code-style fuzzy match of `query` against `path`: every query
/// character in order, case-insensitive; spaces separate parts that may
/// match anywhere. Matches in the file name, at word starts and in a row
/// score higher. `None`: no match.
pub fn fuzzy_match(path: &str, query: &str) -> Option<QuickHit> {
    let parts: Vec<Vec<char>> = query
        .split_whitespace()
        .map(lower_chars)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Some(QuickHit {
            path: path.to_string(),
            score: 0.0,
            positions: Vec::new(),
        });
    }
    let orig: Vec<char> = path.chars().collect();
    let lower = lower_chars(path);
    let base_start = orig
        .iter()
        .rposition(|c| *c == '/')
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut score = 0.0;
    let mut positions = Vec::new();
    for part in &parts {
        let (s, p) = match_part(&orig, &lower, part, base_start)?;
        score += s;
        positions.extend(p);
    }
    // Shorter paths first among equals (less to wade through).
    score -= orig.len() as f64 * 0.01;
    positions.sort_unstable();
    positions.dedup();
    Some(QuickHit {
        path: path.to_string(),
        score,
        positions,
    })
}

fn find_from(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

fn match_part(
    orig: &[char],
    lower: &[char],
    q: &[char],
    base_start: usize,
) -> Option<(f64, Vec<usize>)> {
    // A plain substring of the file name beats any scattered match.
    if let Some(at) = find_from(lower, q, base_start) {
        let positions = (at..at + q.len()).collect();
        let starts = at == base_start;
        let exact = starts && lower.len() - base_start == q.len();
        let stem =
            starts && (lower.len() == base_start + q.len() || lower[base_start + q.len()] == '.');
        let score = 100.0
            + q.len() as f64 * 10.0
            + if starts { 40.0 } else { 0.0 }
            + if stem { 30.0 } else { 0.0 }
            + if exact { 30.0 } else { 0.0 };
        return Some((score, positions));
    }
    // Match from the file name backwards: the query's tail lands as far
    // right as possible.
    let mut positions = Vec::with_capacity(q.len());
    let mut qi = q.len();
    for i in (0..lower.len()).rev() {
        if qi == 0 {
            break;
        }
        if lower[i] == q[qi - 1] {
            positions.push(i);
            qi -= 1;
        }
    }
    if qi > 0 {
        return None;
    }
    positions.reverse();
    let mut score = 0.0;
    for (k, &p) in positions.iter().enumerate() {
        if k > 0 && p == positions[k - 1] + 1 {
            score += 8.0;
        }
        let word_start =
            p == 0 || is_sep(orig[p - 1]) || (orig[p] != lower[p] && orig[p - 1] == lower[p - 1]);
        if word_start {
            score += 6.0;
        }
        if p >= base_start {
            score += 4.0;
        }
        score += 1.0;
    }
    Some((score, positions))
}

/// The best `limit` matches, best first; an empty query lists `recent`
/// first (those still there), then the rest in order.
pub fn quick_open(files: &[String], query: &str, limit: usize, recent: &[String]) -> Vec<QuickHit> {
    let plain = |path: &String| QuickHit {
        path: path.clone(),
        score: 0.0,
        positions: Vec::new(),
    };
    if query.trim().is_empty() {
        let all: HashSet<&str> = files.iter().map(String::as_str).collect();
        let recent: Vec<&String> = recent.iter().filter(|p| all.contains(p.as_str())).collect();
        let seen: HashSet<&str> = recent.iter().map(|p| p.as_str()).collect();
        return recent
            .into_iter()
            .chain(files.iter().filter(|f| !seen.contains(f.as_str())))
            .take(limit)
            .map(plain)
            .collect();
    }
    let mut hits: Vec<QuickHit> = files.iter().filter_map(|f| fuzzy_match(f, query)).collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });
    hits.truncate(limit);
    hits
}

/// Byte ranges in `text` of the `char` `positions` (indexes into a longer
/// string `text` starts at char `offset` of), merged into runs.
pub fn char_positions_to_byte_ranges(
    text: &str,
    positions: &[usize],
    offset: usize,
) -> Vec<Range<usize>> {
    let set: HashSet<usize> = positions
        .iter()
        .filter_map(|p| p.checked_sub(offset))
        .collect();
    let mut out: Vec<Range<usize>> = Vec::new();
    for (ci, (bi, c)) in text.char_indices().enumerate() {
        if !set.contains(&ci) {
            continue;
        }
        let end = bi + c.len_utf8();
        match out.last_mut() {
            Some(last) if last.end == bi => last.end = end,
            _ => out.push(bi..end),
        }
    }
    out
}

// ── paths ────────────────────────────────────────────────────────────────

/// "src/a/b.ts" → ["src", "src/a"]: the folders to open to show it.
pub fn parent_dirs(path: &str) -> Vec<String> {
    let parts: Vec<&str> = path.split('/').collect();
    (1..parts.len()).map(|i| parts[..i].join("/")).collect()
}

/// A relative path's folder ("" at the top) and file name.
pub fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("", path),
    }
}

/// The agent's folder joined with a relative path, with its own separator
/// (what "Copy path" copies).
pub fn absolute_path(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        return root.to_string();
    }
    let b = root.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\';
    let win = drive || (root.contains('\\') && !root.contains('/'));
    let trimmed = root.trim_end_matches(['/', '\\']);
    if win {
        format!("{trimmed}\\{}", rel.replace('/', "\\"))
    } else {
        format!("{trimmed}/{rel}")
    }
}

/// "12 KB", "3.4 MB" (binary units, as VS Code shows them).
pub fn size_label(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 {
        return if kb < 10.0 {
            format!("{kb:.1} KB")
        } else {
            format!("{} KB", kb.round())
        };
    }
    let mb = kb / 1024.0;
    if mb < 10.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{} MB", mb.round())
    }
}

/// Include / exclude fields: comma-separated globs ("*.ts, src/**").
pub fn glob_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// The word for a change letter (tooltips).
pub use crate::kit::status_title;

pub use crate::kit::status_letter_text as status_letter;

// ── the lazy tree ────────────────────────────────────────────────────────

/// One agent's tree: what is listed, open, loading or failed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TreeState {
    /// Listed folders by path ("" = the agent's folder).
    pub listings: HashMap<String, DirListing>,
    /// Folders that couldn't be listed, with why.
    pub errors: HashMap<String, String>,
    /// Open folders, in the order they were opened.
    pub expanded: Vec<String>,
    /// Folders being listed for the first time.
    pub loading: Vec<String>,
    /// A refresh of everything shown is running.
    pub refreshing: bool,
    /// When the last refresh ended (ms since the epoch).
    pub updated_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TreeRow {
    Entry {
        entry: FileEntry,
        depth: usize,
        open: bool,
    },
    Loading {
        dir: String,
        depth: usize,
    },
    Error {
        dir: String,
        depth: usize,
        error: String,
    },
    Truncated {
        dir: String,
        depth: usize,
    },
}

impl TreeRow {
    pub fn depth(&self) -> usize {
        match self {
            TreeRow::Entry { depth, .. }
            | TreeRow::Loading { depth, .. }
            | TreeRow::Error { depth, .. }
            | TreeRow::Truncated { depth, .. } => *depth,
        }
    }

    pub fn entry(&self) -> Option<&FileEntry> {
        match self {
            TreeRow::Entry { entry, .. } => Some(entry),
            _ => None,
        }
    }
}

impl TreeState {
    pub fn is_open(&self, dir: &str) -> bool {
        self.expanded.iter().any(|d| d == dir)
    }

    /// What the tree shows, top to bottom: entries of open folders nested
    /// under them.
    pub fn rows(&self) -> Vec<TreeRow> {
        let open: HashSet<&str> = self.expanded.iter().map(String::as_str).collect();
        let mut out = Vec::new();
        self.walk("", 0, &open, &mut out);
        out
    }

    fn walk(&self, dir: &str, depth: usize, open: &HashSet<&str>, out: &mut Vec<TreeRow>) {
        let Some(l) = self.listings.get(dir) else {
            match self.errors.get(dir) {
                Some(e) => out.push(TreeRow::Error {
                    dir: dir.into(),
                    depth,
                    error: e.clone(),
                }),
                None => out.push(TreeRow::Loading {
                    dir: dir.into(),
                    depth,
                }),
            }
            return;
        };
        for entry in &l.entries {
            let is_open = entry.kind == EntryKind::Dir && open.contains(entry.path.as_str());
            out.push(TreeRow::Entry {
                entry: entry.clone(),
                depth,
                open: is_open,
            });
            if is_open {
                self.walk(&entry.path, depth + 1, open, out);
            }
        }
        if l.truncated {
            out.push(TreeRow::Truncated {
                dir: dir.into(),
                depth,
            });
        }
        if let Some(e) = self.errors.get(dir) {
            out.push(TreeRow::Error {
                dir: dir.into(),
                depth,
                error: e.clone(),
            });
        }
    }

    /// The entry for `path` among the listed folders, if listed.
    pub fn find_entry(&self, path: &str) -> Option<&FileEntry> {
        let (dir, _) = split_path(path);
        self.listings
            .get(dir)?
            .entries
            .iter()
            .find(|e| e.path == path)
    }
}

// ── search results ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct MatchGroup {
    pub path: String,
    pub matches: Vec<SearchMatch>,
}

/// Matches grouped by file, in the order the files first appear.
pub fn group_matches(matches: &[SearchMatch]) -> Vec<MatchGroup> {
    let mut order: Vec<MatchGroup> = Vec::new();
    let mut at: HashMap<&str, usize> = HashMap::new();
    for m in matches {
        match at.get(m.path.as_str()) {
            Some(&i) => order[i].matches.push(m.clone()),
            None => {
                at.insert(&m.path, order.len());
                order.push(MatchGroup {
                    path: m.path.clone(),
                    matches: vec![m.clone()],
                });
            }
        }
    }
    order
}

/// The byte offset in `text` of UTF-16 offset `u16` (clamped to the end).
pub fn utf16_to_byte(text: &str, u16: usize) -> usize {
    let mut n = 0;
    for (bi, c) in text.char_indices() {
        if n >= u16 {
            return bi;
        }
        n += c.len_utf16();
    }
    text.len()
}

/// The `char` index in `text` of UTF-16 offset `u16`.
pub fn utf16_to_char(text: &str, u16: usize) -> usize {
    let mut n = 0;
    for (ci, c) in text.chars().enumerate() {
        if n >= u16 {
            return ci;
        }
        n += c.len_utf16();
    }
    text.chars().count()
}

/// A match's highlighted parts as byte ranges of its `text` (the core sends
/// UTF-16 ranges, for the web UI), sorted, never overlapping.
pub fn match_byte_ranges(m: &SearchMatch) -> Vec<Range<usize>> {
    let mut ranges: Vec<_> = m.ranges.iter().map(|r| (r.start, r.end)).collect();
    ranges.sort_unstable();
    let mut out: Vec<Range<usize>> = Vec::new();
    for (s, e) in ranges {
        let s = utf16_to_byte(&m.text, s as usize);
        let e = utf16_to_byte(&m.text, e as usize);
        if s >= e || out.last().is_some_and(|l| s < l.end) {
            continue;
        }
        out.push(s..e);
    }
    out
}

/// Where a click on a match opens the file: its line and the first match's
/// `char` columns in the whole line (0-based).
pub fn match_target(m: &SearchMatch) -> (u32, usize, usize) {
    let off = m.text_offset as usize;
    match m.ranges.iter().min_by_key(|r| r.start) {
        Some(r) => (
            m.line,
            off + utf16_to_char(&m.text, r.start as usize),
            off + utf16_to_char(&m.text, r.end as usize),
        ),
        None => {
            let c = m.column.saturating_sub(1) as usize;
            (m.line, c, c)
        }
    }
}

/// A short summary of a search's answer ("12 results in 3 files").
pub fn search_summary(total: usize, files: u32, truncated: bool) -> String {
    if total == 0 {
        return "No results.".into();
    }
    let r = if total == 1 { "" } else { "s" };
    let f = if files == 1 { "" } else { "s" };
    let more = if truncated {
        " — stopped there, narrow the search"
    } else {
        ""
    };
    format!("{total} result{r} in {files} file{f}{more}")
}

/// Shown like "12 s ago" next to ↻ ("updated … ago").
pub fn ago(now_ms: u64, then_ms: u64) -> String {
    let s = now_ms.saturating_sub(then_ms) / 1000;
    if s < 5 {
        "just now".into()
    } else if s < 60 {
        format!("{s} s ago")
    } else if s < 3600 {
        format!("{} min ago", s / 60)
    } else {
        format!("{} h ago", s / 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::explorer::MatchRange;

    fn files() -> Vec<String> {
        [
            "README.md",
            "package.json",
            "src/app.ts",
            "src/orders/handler.ts",
            "src/orders/pricing.ts",
            "src/orders/validation/cart.ts",
            "test/orders/pricing.test.ts",
            "docs/api.md",
        ]
        .map(String::from)
        .to_vec()
    }

    fn pairs(r: Vec<Range<usize>>) -> Vec<(usize, usize)> {
        r.into_iter().map(|r| (r.start, r.end)).collect()
    }

    fn paths(h: Vec<QuickHit>) -> Vec<String> {
        h.into_iter().map(|h| h.path).collect()
    }

    #[test]
    fn quick_open_needs_every_character_in_order() {
        assert!(fuzzy_match("src/orders/handler.ts", "SOH").is_some());
        assert!(fuzzy_match("src/orders/handler.ts", "hos").is_none());
    }

    #[test]
    fn quick_open_ranks_file_names_first_then_shorter_paths() {
        assert_eq!(
            paths(quick_open(&files(), "pric", 60, &[])),
            ["src/orders/pricing.ts", "test/orders/pricing.test.ts"]
        );
        assert_eq!(
            quick_open(&files(), "handler", 60, &[])[0].path,
            "src/orders/handler.ts"
        );
        assert!(paths(quick_open(&files(), "ovc", 60, &[]))
            .contains(&"src/orders/validation/cart.ts".to_string()));
    }

    #[test]
    fn quick_open_matches_parts_anywhere() {
        let hits = paths(quick_open(&files(), "orders cart", 60, &[]));
        assert_eq!(hits[0], "src/orders/validation/cart.ts");
        assert!(!hits.contains(&"README.md".to_string()));
    }

    #[test]
    fn quick_open_lists_recent_first_when_empty() {
        let recent = ["docs/api.md", "gone.ts"].map(String::from);
        assert_eq!(
            paths(quick_open(&files(), "  ", 3, &recent)),
            ["docs/api.md", "README.md", "package.json"]
        );
    }

    #[test]
    fn quick_open_marks_matched_characters() {
        let h = fuzzy_match("src/app.ts", "app").unwrap();
        assert_eq!(h.positions, [4, 5, 6]);
        assert_eq!(
            pairs(char_positions_to_byte_ranges("app.ts", &h.positions, 4)),
            [(0, 3)]
        );
    }

    #[test]
    fn quick_open_handles_georgian_names() {
        // Georgian letters are 3 bytes in UTF-8: positions stay char-based.
        let files = ["docs/ანგარიში.md", "src/main.rs"].map(String::from);
        let hits = quick_open(&files, "ანგ", 60, &[]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].positions, [5, 6, 7]);
        let (dir, base) = split_path(&hits[0].path);
        assert_eq!((dir, base), ("docs", "ანგარიში.md"));
        assert_eq!(
            pairs(char_positions_to_byte_ranges(base, &hits[0].positions, 5)),
            [(0, 9)]
        );
        // Mtavruli capitals match their lower-case letters.
        assert!(fuzzy_match("docs/ანგარიში.md", "ᲐᲜᲒ").is_some());
    }

    #[test]
    fn paths_and_labels() {
        assert_eq!(parent_dirs("src/a/b.ts"), ["src", "src/a"]);
        assert!(parent_dirs("b.ts").is_empty());
        assert_eq!(
            absolute_path("/srv/work/w/", "src/a.ts"),
            "/srv/work/w/src/a.ts"
        );
        assert_eq!(
            absolute_path("C:\\code\\app", "src/a.ts"),
            "C:\\code\\app\\src\\a.ts"
        );
        assert_eq!(size_label(512), "512 B");
        assert_eq!(size_label(3_480_000), "3.3 MB");
        assert_eq!(size_label(20 * 1024), "20 KB");
        assert_eq!(glob_list(" *.ts, src/** ,, "), ["*.ts", "src/**"]);
    }

    fn e(path: &str, kind: EntryKind) -> FileEntry {
        FileEntry {
            name: split_path(path).1.into(),
            path: path.into(),
            kind,
            status: None,
            changes: 0,
            ignored: false,
        }
    }

    fn listing(dir: &str, entries: Vec<FileEntry>, truncated: bool) -> DirListing {
        DirListing {
            dir: dir.into(),
            entries,
            truncated,
            git: true,
        }
    }

    #[test]
    fn tree_rows_nest_open_folders_and_notes() {
        let mut s = TreeState::default();
        assert!(matches!(s.rows()[..], [TreeRow::Loading { .. }]));
        s.listings.insert(
            "".into(),
            listing(
                "",
                vec![e("src", EntryKind::Dir), e("README.md", EntryKind::File)],
                false,
            ),
        );
        s.expanded.push("src".into());
        let rows = s.rows();
        assert!(matches!(&rows[1], TreeRow::Loading { dir, depth: 1 } if dir == "src"));
        s.listings.insert(
            "src".into(),
            listing("src", vec![e("src/a.ts", EntryKind::File)], true),
        );
        s.errors.insert("src".into(), "boom".into());
        let rows = s.rows();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[1].entry().unwrap().path, "src/a.ts");
        assert_eq!(rows[1].depth(), 1);
        assert!(matches!(rows[2], TreeRow::Truncated { .. }));
        assert!(matches!(rows[3], TreeRow::Error { .. }));
        assert_eq!(s.find_entry("src/a.ts").unwrap().name, "a.ts");
        assert!(s.find_entry("nope").is_none());
    }

    fn m(path: &str, line: u32, text: &str, ranges: &[(u32, u32)]) -> SearchMatch {
        SearchMatch {
            path: path.into(),
            line,
            column: ranges[0].0 + 1,
            text: text.into(),
            text_offset: 0,
            ranges: ranges
                .iter()
                .map(|&(start, end)| MatchRange { start, end })
                .collect(),
        }
    }

    #[test]
    fn search_results_group_by_file_and_mark_utf16_ranges() {
        let g = group_matches(&[
            m("b.ts", 1, "x", &[(0, 1)]),
            m("a.ts", 2, "x", &[(0, 1)]),
            m("b.ts", 5, "x", &[(0, 1)]),
        ]);
        let got: Vec<_> = g
            .iter()
            .map(|x| {
                (
                    x.path.as_str(),
                    x.matches.iter().map(|y| y.line).collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(got, [("b.ts", vec![1, 5]), ("a.ts", vec![2])]);
        assert_eq!(
            match_byte_ranges(&m(
                "a.ts",
                1,
                "a currency and currency",
                &[(15, 23), (2, 10)]
            )),
            [2..10, 15..23]
        );
        // "გამარჯობა" after an emoji: UTF-16 offsets differ from bytes and chars.
        let hit = m("a.ts", 7, "😀 გამარჯობა", &[(3, 12)]);
        assert_eq!(pairs(match_byte_ranges(&hit)), [(5, 32)]);
        assert_eq!(&hit.text[5..32], "გამარჯობა");
        assert_eq!(match_target(&hit), (7, 2, 11));
    }

    #[test]
    fn summaries_and_ages() {
        assert_eq!(search_summary(0, 0, false), "No results.");
        assert_eq!(search_summary(1, 1, false), "1 result in 1 file");
        assert_eq!(
            search_summary(2000, 30, true),
            "2000 results in 30 files — stopped there, narrow the search"
        );
        assert_eq!(ago(10_000, 9_000), "just now");
        assert_eq!(ago(100_000, 10_000), "1 min ago");
    }
}
