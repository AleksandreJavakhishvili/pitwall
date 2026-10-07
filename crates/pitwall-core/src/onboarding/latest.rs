//! The conversation an agent process is in when its arguments don't say:
//! the newest transcript of that kind for its folder (docs/spec/terminals.md).
//! Same transcript formats as the scan (`scan::parse_*`); reads only a few
//! files. Local machine only.

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::project_list::normalize;
use super::scan::{parse_claude_transcript, parse_codex_session};

/// Codex logs looked at, newest first, before giving up.
const MAX_CODEX_FILES: usize = 60;
const CODEX_DAYS: usize = 7;

#[derive(Debug, Clone, PartialEq)]
pub struct Latest {
    pub session_id: String,
    pub title: Option<String>,
}

fn mtime_ms(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

/// Entries of `dir`, newest name first (dated folders sort by name).
fn names_desc(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    v.sort_by(|a, b| b.cmp(a));
    v
}

fn jsonl_newest_first(dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut v: Vec<(PathBuf, u64)> = names_desc(dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .map(|p| {
            let t = mtime_ms(&p);
            (p, t)
        })
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v
}

/// Claude Code's folder for a cwd under `~/.claude/projects`: every
/// character other than ASCII letters and digits becomes `-`.
pub fn claude_project_dir(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn claude_dir(home: &Path, cwd: &str) -> PathBuf {
    home.join(".claude").join("projects").join(claude_project_dir(cwd))
}

fn claude_title(file: &Path) -> Option<String> {
    let f = std::fs::File::open(file).ok()?;
    parse_claude_transcript(BufReader::new(f), true).title
}

/// Recent Codex session logs (the last few dated folders), newest first.
fn codex_recent(home: &Path) -> Vec<(PathBuf, u64)> {
    let mut days = Vec::new();
    'outer: for year in names_desc(&home.join(".codex").join("sessions")) {
        for month in names_desc(&year) {
            for day in names_desc(&month) {
                days.push(day);
                if days.len() >= CODEX_DAYS {
                    break 'outer;
                }
            }
        }
    }
    let mut files: Vec<(PathBuf, u64)> = days.iter().flat_map(|d| jsonl_newest_first(d)).collect();
    files.sort_by(|a, b| b.1.cmp(&a.1));
    files.truncate(MAX_CODEX_FILES);
    files
}

fn codex_info(file: &Path) -> Option<super::scan::TranscriptInfo> {
    let f = std::fs::File::open(file).ok()?;
    Some(parse_codex_session(BufReader::new(f), true))
}

/// The newest conversation of `kind` that ran in `cwd`, written to at or
/// after `since` (unix ms; `None` = any time). Only kinds whose transcripts
/// Pitwall can read (Claude Code, Codex); others → `None`.
pub fn newest(home: &Path, kind: &str, cwd: &str, since: Option<u64>) -> Option<Latest> {
    let cwd = normalize(cwd);
    let fresh = |t: u64| since.is_none_or(|s| t >= s);
    match kind {
        "claude" => {
            let (file, _) = jsonl_newest_first(&claude_dir(home, &cwd)).into_iter().find(|(_, t)| fresh(*t))?;
            Some(Latest {
                session_id: file.file_stem()?.to_string_lossy().into_owned(),
                title: claude_title(&file),
            })
        }
        "codex" => codex_recent(home).into_iter().filter(|(_, t)| fresh(*t)).find_map(|(file, _)| {
            let info = codex_info(&file)?;
            (info.cwd.as_deref().map(normalize).as_deref() == Some(cwd.as_str())).then_some(())?;
            Some(Latest { session_id: info.session_id?, title: info.title })
        }),
        _ => None,
    }
}

/// Title of a known conversation (its first prompt), when it can be found cheaply.
pub fn title(home: &Path, kind: &str, cwd: &str, session_id: &str) -> Option<String> {
    match kind {
        "claude" => claude_title(&claude_dir(home, &normalize(cwd)).join(format!("{session_id}.jsonl"))),
        "codex" => codex_recent(home)
            .into_iter()
            .filter(|(f, _)| f.file_name().is_some_and(|n| n.to_string_lossy().contains(session_id)))
            .find_map(|(f, _)| codex_info(&f)?.title),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn age(path: &Path, ms_ago: u64) {
        let t = std::time::SystemTime::now() - std::time::Duration::from_millis(ms_ago);
        std::fs::File::options().write(true).open(path).unwrap().set_modified(t).unwrap();
    }

    fn now_ms() -> u64 {
        crate::clock::unix_ms()
    }

    #[test]
    fn claude_folder_names() {
        assert_eq!(claude_project_dir("/Users/dev/My Projects/web.app"), "-Users-dev-My-Projects-web-app");
    }

    #[test]
    fn newest_claude_conversation_for_a_folder() {
        let home = TempDir::new("latest-claude");
        let dir = home.path().join(".claude/projects").join(claude_project_dir("/w/app"));
        let line = |id: &str, prompt: &str| {
            format!("{{\"type\":\"user\",\"cwd\":\"/w/app\",\"sessionId\":\"{id}\",\"message\":{{\"content\":\"{prompt}\"}}}}\n")
        };
        write(&dir.join("old.jsonl"), &line("old", "first thing"));
        write(&dir.join("new.jsonl"), &line("new", "second thing"));
        age(&dir.join("old.jsonl"), 60_000);
        age(&dir.join("new.jsonl"), 1_000);
        let got = newest(home.path(), "claude", "/w/app/", None).unwrap();
        assert_eq!(got, Latest { session_id: "new".into(), title: Some("second thing".into()) });
        // Only conversations written since the agent started count.
        assert_eq!(newest(home.path(), "claude", "/w/app", Some(now_ms() + 60_000)), None);
        assert_eq!(newest(home.path(), "claude", "/w/other", None), None);
        assert_eq!(title(home.path(), "claude", "/w/app", "old").as_deref(), Some("first thing"));
        assert_eq!(newest(home.path(), "gemini", "/w/app", None), None);
    }

    #[test]
    fn newest_codex_conversation_for_a_folder() {
        let home = TempDir::new("latest-codex");
        let day = home.path().join(".codex/sessions/2026/10/06");
        let meta = |id: &str, cwd: &str| {
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\"}}}}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"hello {id}\"}}}}\n"
            )
        };
        write(&day.join("rollout-a-019a.jsonl"), &meta("019a", "/w/app"));
        write(&day.join("rollout-b-019b.jsonl"), &meta("019b", "/w/other"));
        write(&home.path().join(".codex/sessions/2026/10/05/rollout-c-019c.jsonl"), &meta("019c", "/w/app"));
        age(&day.join("rollout-a-019a.jsonl"), 5_000);
        age(&day.join("rollout-b-019b.jsonl"), 1_000);
        age(&home.path().join(".codex/sessions/2026/10/05/rollout-c-019c.jsonl"), 90_000_000);
        let got = newest(home.path(), "codex", "/w/app", None).unwrap();
        assert_eq!((got.session_id.as_str(), got.title.as_deref()), ("019a", Some("hello 019a")));
        assert_eq!(title(home.path(), "codex", "/w/app", "019c").as_deref(), Some("hello 019c"));
        assert_eq!(newest(home.path(), "codex", "/w/none", None), None);
    }
}
