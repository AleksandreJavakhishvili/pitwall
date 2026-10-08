//! Search in the agent's folder: ripgrep (`rg --json`) when it runs on that
//! machine, else `git grep`. Both are given every argument as argv (never a
//! shell), stop at a time limit, and can be cancelled.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use pitwall_proto::{MatchRange, SearchEngine, SearchMatch, SearchQuery, SearchResult};
use serde_json::Value;

use super::tree::NOT_SEARCHED;
use super::{Ctx, Res};
use crate::exec::{Cmd, Out};
use crate::vcs::git::GIT_ENV;

pub const CANCELLED: &str = "cancelled";
/// Matches returned when the query names no number.
pub const DEFAULT_RESULTS: u32 = 2_000;
pub const MAX_RESULTS: u32 = 10_000;
/// Matches per file.
pub const PER_FILE: usize = 100;
/// Longest line sent (characters); longer ones are cut around the match.
pub const MAX_LINE: usize = 400;
/// Characters kept before the first match when a line is cut.
const BEFORE: usize = 100;
const TIMEOUT: Duration = Duration::from_secs(30);

/// A search's answer, and what was learnt about ripgrep on the machine
/// (`Some(false)`: it isn't there).
pub(super) fn run(
    c: &Ctx,
    root: &str,
    q: &SearchQuery,
    cancel: &AtomicBool,
    rg: Option<bool>,
) -> (Res<SearchResult>, Option<bool>) {
    let max = q
        .max_results
        .unwrap_or(DEFAULT_RESULTS)
        .clamp(1, MAX_RESULTS) as usize;
    if q.query.is_empty() {
        let empty = SearchResult {
            matches: Vec::new(),
            files: 0,
            truncated: false,
            engine: SearchEngine::Ripgrep,
        };
        return (Ok(empty), None);
    }
    if q.query.contains('\0') || q.query.len() > 10_000 {
        return (Err("invalid search".into()), None);
    }
    let mut learnt = None;
    if rg != Some(false) {
        let argv = rg_argv(q);
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        match c
            .exec
            .run(&Cmd::new(&argv).cwd(root).timeout(TIMEOUT).cancel(cancel))
        {
            Ok(out) => return (rg_result(&out, max), Some(true)),
            Err(e) if e.contains(CANCELLED) || e.contains("timed out") => {
                return (Err(e.into()), None)
            }
            // Not installed there (or it can't start): git grep instead.
            Err(_) => learnt = Some(false),
        }
    }
    if c.git_repo == Some(false) {
        return (Err(no_tool(c)), learnt);
    }
    let argv = git_argv(root, q);
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let res = match c.exec.run(
        &Cmd::new(&argv)
            .env(&GIT_ENV)
            .timeout(TIMEOUT)
            .cancel(cancel),
    ) {
        Ok(out) if out.status <= 1 => Ok(git_result(&out, q, max)),
        Ok(out)
            if out
                .stderr_text()
                .to_lowercase()
                .contains("not a git repository") =>
        {
            Err(no_tool(c))
        }
        Ok(out) => Err(out.stderr_text()),
        Err(e) if e.contains(CANCELLED) || e.contains("timed out") => Err(e.into()),
        Err(_) => Err(no_tool(c)),
    };
    (res, learnt)
}

fn no_tool(c: &Ctx) -> String {
    let at = if c.machine_label.is_empty() {
        "that machine".to_string()
    } else {
        c.machine_label.clone()
    };
    format!("Search needs ripgrep (rg) or a git repository on {at}.")
}

/// A glob as given, or for any depth when it names no folder (`*.rs`).
fn deep(glob: &str) -> String {
    if glob.contains('/') {
        glob.trim_start_matches('/').to_string()
    } else {
        format!("**/{glob}")
    }
}

fn rg_argv(q: &SearchQuery) -> Vec<String> {
    let mut a: Vec<String> = [
        "rg",
        "--json",
        "--no-config",
        "--max-count",
        "100",
        "--max-filesize",
        "2M",
    ]
    .map(String::from)
    .to_vec();
    if q.hidden.unwrap_or(true) {
        a.push("--hidden".into());
    }
    a.extend(["-g".into(), "!.git".into()]);
    if q.default_excludes.unwrap_or(true) {
        for d in NOT_SEARCHED {
            a.extend(["-g".into(), format!("!{d}")]);
        }
    }
    for g in q.include.iter().filter(|g| !g.trim().is_empty()) {
        a.extend(["-g".into(), g.trim().to_string()]);
    }
    for g in q.exclude.iter().filter(|g| !g.trim().is_empty()) {
        a.extend(["-g".into(), format!("!{}", g.trim())]);
    }
    if !q.regex {
        a.push("-F".into());
    }
    a.push(if q.case_sensitive { "-s" } else { "-i" }.into());
    if q.whole_word {
        a.push("-w".into());
    }
    a.extend(["-e".into(), q.query.clone(), "--".into(), ".".into()]);
    a
}

fn git_argv(root: &str, q: &SearchQuery) -> Vec<String> {
    let mut a: Vec<String> = [
        "git",
        "-C",
        root,
        "grep",
        "-n",
        "-z",
        "--column",
        "-I",
        "--untracked",
        "--no-color",
    ]
    .map(String::from)
    .to_vec();
    a.push(if q.regex { "-E" } else { "-F" }.into());
    if !q.case_sensitive {
        a.push("-i".into());
    }
    if q.whole_word {
        a.push("-w".into());
    }
    a.extend(["-e".into(), q.query.clone(), "--".into()]);
    let include: Vec<&String> = q.include.iter().filter(|g| !g.trim().is_empty()).collect();
    if include.is_empty() {
        a.push(".".into());
    }
    for g in include {
        a.push(format!(":(glob){}", deep(g.trim())));
    }
    if q.default_excludes.unwrap_or(true) {
        for d in NOT_SEARCHED {
            a.push(format!(":(exclude,glob)**/{d}/**"));
        }
    }
    if !q.hidden.unwrap_or(true) {
        a.extend([
            ":(exclude,glob)**/.*".into(),
            ":(exclude,glob)**/.*/**".into(),
        ]);
    }
    for g in q.exclude.iter().filter(|g| !g.trim().is_empty()) {
        a.push(format!(":(exclude,glob){}", deep(g.trim())));
    }
    a
}

/// A path as the tools print it, relative and `/`-separated (ripgrep on
/// Windows prints `.\a\b`).
pub fn tidy_path(p: &str) -> String {
    match p.strip_prefix(".\\") {
        Some(rest) => rest.replace('\\', "/"),
        None => p.strip_prefix("./").unwrap_or(p).to_string(),
    }
}

/// Collects matches up to the caps.
struct Collect {
    max: usize,
    matches: Vec<SearchMatch>,
    files: u32,
    last_file: Option<String>,
    in_file: usize,
    truncated: bool,
}

impl Collect {
    fn new(max: usize) -> Collect {
        Collect {
            max,
            matches: Vec::new(),
            files: 0,
            last_file: None,
            in_file: 0,
            truncated: false,
        }
    }

    /// `false` once full.
    fn push(&mut self, path: String, line: u32, text: &str, bytes: &[(usize, usize)]) -> bool {
        if self.matches.len() >= self.max {
            self.truncated = true;
            return false;
        }
        if self.last_file.as_deref() != Some(&path) {
            self.files += 1;
            self.in_file = 0;
            self.last_file = Some(path.clone());
        }
        self.in_file += 1;
        if self.in_file > PER_FILE {
            return true;
        }
        let (text, text_offset, column, ranges) = preview(text, bytes);
        self.matches.push(SearchMatch {
            path,
            line,
            column,
            text,
            text_offset,
            ranges,
        });
        true
    }

    fn done(self, engine: SearchEngine) -> SearchResult {
        SearchResult {
            matches: self.matches,
            files: self.files,
            truncated: self.truncated,
            engine,
        }
    }
}

/// `rg --json` output: one JSON message per line, `match` ones counted.
fn rg_result(out: &Out, max: usize) -> Res<SearchResult> {
    let mut all = Collect::new(max);
    let text = out.stdout_text();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["type"] != "match" {
            continue;
        }
        let d = &v["data"];
        // Paths and lines that aren't UTF-8 come as base64 `bytes`: skipped.
        let (Some(path), Some(text), Some(n)) = (
            d["path"]["text"].as_str(),
            d["lines"]["text"].as_str(),
            d["line_number"].as_u64(),
        ) else {
            continue;
        };
        let ranges: Vec<(usize, usize)> = d["submatches"]
            .as_array()
            .map(|subs| {
                subs.iter()
                    .filter_map(|s| {
                        Some((s["start"].as_u64()? as usize, s["end"].as_u64()? as usize))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !all.push(tidy_path(path), n as u32, text, &ranges) {
            break;
        }
    }
    // 1: no match; 2: an error (a bad pattern, or files it couldn't read).
    if out.status >= 2 && all.matches.is_empty() {
        let e = out.stderr_text();
        if !e.is_empty() {
            return Err(e);
        }
    }
    Ok(all.done(SearchEngine::Ripgrep))
}

/// The query as a Rust regex, to find match ranges in git grep's lines.
fn matcher(q: &SearchQuery) -> Option<regex::Regex> {
    let mut p = if q.regex {
        q.query.clone()
    } else {
        regex::escape(&q.query)
    };
    if q.whole_word {
        p = format!(r"\b(?:{p})\b");
    }
    regex::RegexBuilder::new(&p)
        .case_insensitive(!q.case_sensitive)
        .size_limit(1 << 20)
        .build()
        .ok()
}

/// `git grep -n -z --column` output: `path\0line\0column\0text\n`.
fn git_result(out: &Out, q: &SearchQuery, max: usize) -> SearchResult {
    let re = matcher(q);
    let mut all = Collect::new(max);
    let text = out.stdout_text();
    for rec in text.split('\n').filter(|r| !r.is_empty()) {
        let mut f = rec.splitn(4, '\0');
        let (Some(path), Some(n), Some(col), Some(line)) = (f.next(), f.next(), f.next(), f.next())
        else {
            continue;
        };
        let Ok(n) = n.parse::<u32>() else { continue };
        let ranges: Vec<(usize, usize)> = match &re {
            Some(re) => re
                .find_iter(line)
                .map(|m| (m.start(), m.end()))
                .filter(|(s, e)| s < e)
                .collect(),
            None => Vec::new(),
        };
        let ranges = if ranges.is_empty() {
            // git found it where the regex didn't (dialects differ): its column.
            let at = col
                .parse::<usize>()
                .unwrap_or(1)
                .saturating_sub(1)
                .min(line.len());
            vec![(at, at)]
        } else {
            ranges
        };
        if !all.push(tidy_path(path), n, line, &ranges) {
            break;
        }
    }
    all.done(SearchEngine::GitGrep)
}

/// UTF-16 length of `s` (what JS string offsets count).
fn u16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

/// Largest char boundary of `s` at or before `i`.
fn floor(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The line to show (cut around the first match when long), how many
/// characters were cut from its start, the 1-based column of the first
/// match, and the match ranges in UTF-16 units of the shown text. `bytes`
/// are byte ranges in `line`.
fn preview(line: &str, bytes: &[(usize, usize)]) -> (String, u32, u32, Vec<MatchRange>) {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    let first = bytes.first().map_or(0, |r| floor(line, r.0));
    let column = line[..first].chars().count() as u32 + 1;
    let (start, end) = if line.chars().count() <= MAX_LINE {
        (0, line.len())
    } else {
        let skip = (column as usize - 1).saturating_sub(BEFORE);
        let start = line.char_indices().nth(skip).map_or(line.len(), |(i, _)| i);
        let end = line[start..]
            .char_indices()
            .nth(MAX_LINE)
            .map_or(line.len(), |(i, _)| start + i);
        (start, end)
    };
    let shown = &line[start..end];
    let ranges = bytes
        .iter()
        .map(|&(s, e)| (floor(line, s), floor(line, e)))
        .filter(|&(s, e)| s >= start && e <= end && s <= e)
        .map(|(s, e)| MatchRange {
            start: u16_len(&line[start..s]),
            end: u16_len(&line[start..e]),
        })
        .collect();
    (
        shown.to_string(),
        line[..start].chars().count() as u32,
        column,
        ranges,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ripgrep_json_matches() {
        let json = [
            r#"{"type":"begin","data":{"path":{"text":"./src/a.rs"}}}"#,
            r#"{"type":"match","data":{"path":{"text":"./src/a.rs"},"lines":{"text":"let héllo = hello;\n"},"line_number":3,"absolute_offset":0,"submatches":[{"match":{"text":"héllo"},"start":4,"end":10},{"match":{"text":"hello"},"start":13,"end":18}]}}"#,
            r#"{"type":"match","data":{"path":{"bytes":"/w8="},"lines":{"text":"x\n"},"line_number":1,"submatches":[]}}"#,
            r#"{"type":"match","data":{"path":{"text":"b.md"},"lines":{"text":"hello\r\n"},"line_number":9,"submatches":[{"match":{"text":"hello"},"start":0,"end":5}]}}"#,
            r#"{"type":"summary","data":{}}"#,
        ]
        .join("\n");
        let out = Out {
            status: 0,
            stdout: json.into_bytes(),
            stderr: Vec::new(),
        };
        let r = rg_result(&out, 10).unwrap();
        assert_eq!(
            (r.files, r.truncated, r.engine),
            (2, false, SearchEngine::Ripgrep)
        );
        let m = &r.matches[0];
        assert_eq!(
            (m.path.as_str(), m.line, m.column, m.text.as_str()),
            ("src/a.rs", 3, 5, "let héllo = hello;")
        );
        assert_eq!(
            m.ranges,
            [
                MatchRange { start: 4, end: 9 },
                MatchRange { start: 12, end: 17 }
            ],
            "UTF-16 units"
        );
        assert_eq!(
            (r.matches[1].path.as_str(), r.matches[1].text.as_str()),
            ("b.md", "hello")
        );
        let capped = rg_result(&out, 1).unwrap();
        assert!(capped.truncated && capped.matches.len() == 1);
        let bad = Out {
            status: 2,
            stdout: Vec::new(),
            stderr: b"regex parse error".to_vec(),
        };
        assert_eq!(rg_result(&bad, 10).unwrap_err(), "regex parse error");
    }

    #[test]
    fn git_grep_matches() {
        let out = Out {
            status: 0,
            stdout: b"a.txt\x002\x007\x00foo hello Hello\nsrc/b.rs\x0010\x001\x00HELLO\n".to_vec(),
            stderr: Vec::new(),
        };
        let q = SearchQuery {
            query: "hello".into(),
            ..Default::default()
        };
        let r = git_result(&out, &q, 10);
        assert_eq!((r.files, r.engine), (2, SearchEngine::GitGrep));
        assert_eq!((r.matches[0].line, r.matches[0].column), (2, 5));
        assert_eq!(
            r.matches[0].ranges,
            [
                MatchRange { start: 4, end: 9 },
                MatchRange { start: 10, end: 15 }
            ]
        );
        assert_eq!(
            (r.matches[1].path.as_str(), r.matches[1].line),
            ("src/b.rs", 10)
        );
        // A pattern the Rust regex can't read: git's column marks it.
        let q = SearchQuery {
            query: "[[:alpha:]".into(),
            regex: true,
            ..Default::default()
        };
        assert_eq!(
            git_result(&out, &q, 10).matches[0].ranges,
            [MatchRange { start: 6, end: 6 }]
        );
    }

    #[test]
    fn long_lines_are_cut_around_the_match() {
        let line = format!("{}needle{}", "a".repeat(1000), "b".repeat(1000));
        let (text, offset, column, ranges) = preview(&line, &[(1000, 1006)]);
        assert_eq!((offset, column), (900, 1001));
        assert_eq!(text.chars().count(), MAX_LINE);
        assert_eq!(
            ranges,
            [MatchRange {
                start: 100,
                end: 106
            }]
        );
        assert_eq!(&text[100..106], "needle");
        let (short, offset, _, _) = preview("x needle\n", &[(2, 8)]);
        assert_eq!((short.as_str(), offset), ("x needle", 0));
    }

    #[test]
    fn per_file_cap() {
        let mut c = Collect::new(1_000);
        for n in 0..(PER_FILE as u32 + 5) {
            assert!(c.push("a".into(), n + 1, "x", &[(0, 1)]));
        }
        c.push("b".into(), 1, "x", &[(0, 1)]);
        let r = c.done(SearchEngine::Ripgrep);
        assert_eq!((r.matches.len(), r.files), (PER_FILE + 1, 2));
    }

    #[test]
    fn arguments_never_need_a_shell() {
        let q = SearchQuery {
            query: "-rf $(x)".into(),
            whole_word: true,
            include: vec!["*.rs".into(), "src/**".into(), " ".into()],
            exclude: vec!["gen".into()],
            hidden: Some(false),
            ..Default::default()
        };
        let rg = rg_argv(&q);
        assert!(!rg.contains(&"--hidden".to_string()));
        assert_eq!(&rg[rg.len() - 4..], ["-e", "-rf $(x)", "--", "."]);
        for w in ["-F", "-i", "-w", "*.rs", "src/**", "!gen", "!node_modules"] {
            assert!(rg.contains(&w.to_string()), "{w} in {rg:?}");
        }
        let g = git_argv("/r", &q);
        assert_eq!(&g[..4], ["git", "-C", "/r", "grep"]);
        for w in [
            "-e",
            "-rf $(x)",
            "--",
            ":(glob)**/*.rs",
            ":(glob)src/**",
            ":(exclude,glob)**/gen",
            ":(exclude,glob)**/.*",
            "--untracked",
        ] {
            assert!(g.contains(&w.to_string()), "{w} in {g:?}");
        }
        assert!(!g.contains(&".".to_string()), "includes given: no '.'");
        let all = SearchQuery {
            query: "x".into(),
            default_excludes: Some(false),
            ..Default::default()
        };
        assert!(!rg_argv(&all).contains(&"!node_modules".to_string()));
        assert!(!git_argv("/r", &all).iter().any(|a| a.contains("node_modules")));
        assert!(git_argv("/r", &q).iter().any(|a| a.contains("node_modules")));
        assert_eq!(tidy_path(".\\src\\a.rs"), "src/a.rs");
        assert_eq!(tidy_path("./a b.rs"), "a b.rs");
    }
}
