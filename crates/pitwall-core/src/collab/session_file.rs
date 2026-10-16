//! Reply capture tier (a) from a kind's own session files (§7.2): the
//! agent writes its conversation as JSON Lines under its account folder
//! (Codex: `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-…-<id>.jsonl`), and a
//! turn's reply is a field of the line that ends the turn
//! (`event_msg` / `task_complete` / `last_agent_message`). What to look for
//! is the kind's data (`[share.session_files]`); this module only follows
//! a file.
//!
//! A [`SessionFileReader`] finds this session's file (the one whose meta
//! line names the session's working folder, written since the launch) and
//! reads what was appended since the last look, so a long conversation is
//! never read twice. Codex's format was checked against codex-cli 0.160.1
//! (`codex exec` and the interactive TUI, recorded against a local mock
//! model API: `fixtures/codex-rollout.jsonl`).

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::kind::SessionFiles;

/// What a look at the file found, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// The kind's id for this conversation (its resume needs it).
    SessionId(String),
    /// A turn ended; its reply when the line has one.
    TurnEnded(Option<String>),
}

/// Follows one session's file.
#[derive(Debug)]
pub struct SessionFileReader {
    spec: SessionFiles,
    /// The account folder the files are under.
    root: PathBuf,
    /// The session's working folder, as the agent records it.
    cwd: PathBuf,
    /// Files older than this aren't this session's.
    since: SystemTime,
    file: Option<PathBuf>,
    offset: u64,
    partial: Vec<u8>,
}

/// Files modified this long before the launch still count (clocks, and a
/// resumed conversation's file written just before).
const SLACK: Duration = Duration::from_secs(5);

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

fn matches(line: &Value, want: &std::collections::BTreeMap<String, String>) -> bool {
    !want.is_empty() && want.iter().all(|(ptr, v)| line.pointer(ptr).and_then(Value::as_str) == Some(v.as_str()))
}

impl SessionFileReader {
    /// `root`: the account folder (`$CODEX_HOME`); `cwd`: where the session
    /// runs; `since`: when it was launched. `resume`: the conversation id a
    /// resumed launch continues (its file is older than the launch).
    pub fn new(spec: SessionFiles, root: &Path, cwd: &Path, since: SystemTime) -> SessionFileReader {
        SessionFileReader { spec, root: root.to_path_buf(), cwd: canon(cwd), since, file: None, offset: 0, partial: Vec::new() }
    }

    /// The file being followed, once found.
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Skip everything written so far (a message is about to be sent: only
    /// what comes after it is this turn's).
    pub fn catch_up(&mut self) {
        let _ = self.poll();
    }

    /// The file whose meta line names this session's folder: the newest
    /// such file modified since the launch.
    fn find(&self, resume: Option<&str>) -> Option<PathBuf> {
        let dir = self.root.join(&self.spec.dir);
        let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
        let mut stack = vec![dir];
        let since = self.since.checked_sub(SLACK).unwrap_or(SystemTime::UNIX_EPOCH);
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for e in entries.flatten() {
                let p = e.path();
                let Ok(meta) = e.metadata() else { continue };
                if meta.is_dir() {
                    stack.push(p);
                } else if p.to_string_lossy().ends_with(&self.spec.suffix) {
                    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    let named = resume.is_some_and(|id| p.file_name().is_some_and(|n| n.to_string_lossy().contains(id)));
                    if modified >= since || named {
                        found.push((modified, p));
                    }
                }
            }
        }
        found.sort_by(|a, b| b.0.cmp(&a.0));
        found.into_iter().map(|(_, p)| p).find(|p| self.is_ours(p, resume))
    }

    /// Its meta line names our folder (or, resumed, our conversation id).
    fn is_ours(&self, path: &Path, resume: Option<&str>) -> bool {
        let Ok(mut f) = std::fs::File::open(path) else { return false };
        let mut head = vec![0u8; 64 * 1024];
        let n = f.read(&mut head).unwrap_or(0);
        head.truncate(n);
        for line in head.split(|b| *b == b'\n') {
            let Ok(v) = serde_json::from_slice::<Value>(line) else { continue };
            if !matches(&v, &self.spec.meta) {
                continue;
            }
            let id = v.pointer(&self.spec.id).and_then(Value::as_str);
            if resume.is_some() && id == resume {
                return true;
            }
            let cwd = v.pointer(&self.spec.cwd).and_then(Value::as_str).map(|c| canon(Path::new(c)));
            return cwd.as_deref() == Some(self.cwd.as_path());
        }
        false
    }

    /// What was appended since the last look (finding the file first).
    pub fn poll(&mut self) -> Vec<Found> {
        self.poll_for(None)
    }

    /// As [`poll`](Self::poll), for a launch that resumes conversation `id`.
    pub fn poll_for(&mut self, resume: Option<&str>) -> Vec<Found> {
        let mut out = Vec::new();
        if self.file.is_none() {
            let Some(path) = self.find(resume) else { return out };
            self.file = Some(path);
            self.offset = 0;
            self.partial.clear();
        }
        let Some(path) = self.file.clone() else { return out };
        let Ok(mut f) = std::fs::File::open(&path) else { return out };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.offset {
            // Rewritten: start over.
            self.offset = 0;
            self.partial.clear();
        }
        if f.seek(SeekFrom::Start(self.offset)).is_err() {
            return out;
        }
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_err() {
            return out;
        }
        self.offset += buf.len() as u64;
        self.partial.extend_from_slice(&buf);
        // Only whole lines: the last one may still be being written.
        let Some(last_nl) = self.partial.iter().rposition(|b| *b == b'\n') else { return out };
        let whole: Vec<u8> = self.partial.drain(..=last_nl).collect();
        for line in whole.split(|b| *b == b'\n') {
            let Ok(v) = serde_json::from_slice::<Value>(line) else { continue };
            if matches(&v, &self.spec.meta) {
                if let Some(id) = v.pointer(&self.spec.id).and_then(Value::as_str) {
                    out.push(Found::SessionId(id.to_string()));
                }
            } else if matches(&v, &self.spec.end) {
                let reply = v.pointer(&self.spec.reply).and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(String::from);
                out.push(Found::TurnEnded(reply));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    /// Recorded from codex-cli 0.160.1 (`codex exec`, then `codex exec
    /// resume`) against a local mock of the model API, trimmed to the
    /// lines that matter, paths replaced.
    const ROLLOUT: &str = include_str!("fixtures/codex-rollout.jsonl");

    fn codex() -> SessionFiles {
        let kinds = crate::kind::load_kinds(Path::new("/nonexistent"));
        let codex = kinds.into_iter().find(|k| k.id == "codex").unwrap();
        codex.share.unwrap().session_files.expect("codex reads its session files")
    }

    fn lines(text: &str) -> Vec<&str> {
        text.lines().collect()
    }

    #[test]
    fn a_codex_rollout_gives_the_session_id_and_each_turns_reply() {
        let dir = TempDir::new("codex-home");
        let work = dir.path().join("pw/collab/shares/api-expert/s/c_1/work");
        std::fs::create_dir_all(&work).unwrap();
        let day = dir.path().join("sessions/2026/10/16");
        std::fs::create_dir_all(&day).unwrap();
        // Another conversation in the same account, in another folder.
        let other = ROLLOUT.lines().next().unwrap().replace("/pw/collab/shares/api-expert/s/c_1/work", "/elsewhere");
        std::fs::write(day.join("rollout-2026-10-16T22-50-00-other.jsonl"), format!("{other}\n")).unwrap();
        let rollout = ROLLOUT.replace("/pw/collab/shares/api-expert/s/c_1/work", &work.to_string_lossy());
        let path = day.join("rollout-2026-10-16T22-53-27-01a1460f-dcd9-7cd2-9dcc-336889d84012.jsonl");
        let all = lines(&rollout);
        // The first turn, written in two pieces (a line cut in the middle).
        let first_end = all.iter().position(|l| l.contains("task_complete")).unwrap();
        let first: String = all[..=first_end].iter().map(|l| format!("{l}\n")).collect();
        let (a, b) = first.split_at(first.len() - 40);
        std::fs::write(&path, a).unwrap();
        let mut r = SessionFileReader::new(codex(), dir.path(), &work, SystemTime::now() - Duration::from_secs(1));
        assert_eq!(r.poll(), [Found::SessionId("01a1460f-dcd9-7cd2-9dcc-336889d84012".into())]);
        assert_eq!(r.file(), Some(path.as_path()));
        std::fs::OpenOptions::new().append(true).open(&path).and_then(|mut f| std::io::Write::write_all(&mut f, b.as_bytes())).unwrap();
        assert_eq!(r.poll(), [Found::TurnEnded(Some("Mock reply 1: the entry point is src/lib.rs.".into()))]);
        assert!(r.poll().is_empty(), "nothing new");
        // The resumed second turn is appended to the same file.
        let second: String = all[first_end + 1..].iter().map(|l| format!("{l}\n")).collect();
        std::fs::OpenOptions::new().append(true).open(&path).and_then(|mut f| std::io::Write::write_all(&mut f, second.as_bytes())).unwrap();
        assert_eq!(r.poll(), [Found::TurnEnded(Some("Mock reply 2: the entry point is src/lib.rs.".into()))]);
    }

    #[test]
    fn a_resumed_launch_finds_its_older_file_by_id_and_skips_what_was_there() {
        let dir = TempDir::new("codex-home");
        let work = dir.path().join("w");
        std::fs::create_dir_all(&work).unwrap();
        let day = dir.path().join("sessions/2026/10/15");
        std::fs::create_dir_all(&day).unwrap();
        let rollout = ROLLOUT.replace("/pw/collab/shares/api-expert/s/c_1/work", "/the/old/copy");
        std::fs::write(day.join("rollout-2026-10-15T09-00-00-01a1460f-dcd9-7cd2-9dcc-336889d84012.jsonl"), &rollout).unwrap();
        let mut r = SessionFileReader::new(codex(), dir.path(), &work, SystemTime::now() + Duration::from_secs(3600));
        assert!(r.poll().is_empty(), "not ours by folder, and too old");
        let mut r = SessionFileReader::new(codex(), dir.path(), &work, SystemTime::now() + Duration::from_secs(3600));
        let found = r.poll_for(Some("01a1460f-dcd9-7cd2-9dcc-336889d84012"));
        assert_eq!(found.iter().filter(|f| matches!(f, Found::TurnEnded(_))).count(), 2, "{found:?}");
        // Before the next message: only what comes after counts.
        r.catch_up();
        assert!(r.poll().is_empty());
    }

    #[test]
    fn nothing_there_finds_nothing() {
        let dir = TempDir::new("codex-home");
        let mut r = SessionFileReader::new(codex(), dir.path(), dir.path(), SystemTime::now());
        assert!(r.poll().is_empty());
        assert!(r.file().is_none());
    }
}
