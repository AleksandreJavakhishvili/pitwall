//! An extension's activity log (docs/spec/extension-api.md, "How the user
//! sees and stops it"): a record of what it asked Pitwall for — each call,
//! from its adapter's stdio or the socket, with when, the method and how
//! it went (answered, refused and why) — and what its limits cut. Shown
//! in Settings next to its protocol log; kept in
//! `<data>/extensions/.log/<id>.jsonl` (one entry a line), at most
//! [`MAX_ENTRIES`] and [`KEEP_DAYS`] days, so it survives a restart and a
//! turn off.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// The most entries kept (shown and in the file).
pub const MAX_ENTRIES: usize = 500;
/// Entries older than this go.
pub const KEEP_DAYS: u64 = 30;

/// Where a call came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Via {
    /// Its adapter's stdio (protocol 2).
    Adapter,
    /// The socket, from a process its adapter started.
    Socket,
    /// Pitwall itself (a limit it applied).
    Pitwall,
    /// A button of its view the user pressed that Pitwall ran itself
    /// (open a file or a page, focus an agent), checked as its own call.
    View,
    /// Its tiles' component (docs/spec/tile-sdk/, the `agents`
    /// interface).
    Tile,
    /// An agent's call of one of its tiles' tools (agent-tools.md §11.10).
    Agent,
}

/// How a call went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Outcome {
    /// Answered.
    Ok,
    /// Refused: not granted, not available, over a limit (its code and
    /// words).
    Refused { code: String, message: String },
    /// Allowed, but it failed (bad params, not found, …).
    Failed { code: String, message: String },
}

/// One entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// Unix milliseconds.
    pub at: u64,
    pub via: Via,
    /// The method called (or what Pitwall did, for [`Via::Pitwall`]).
    pub method: String,
    pub outcome: Outcome,
    /// What a call that acts asked for, in a few words (which agent, the
    /// text it queued, the agent it asked to start).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The longest [`Entry::detail`] kept.
pub const MAX_DETAIL: usize = 600;

impl Entry {
    /// An entry for now.
    pub fn now(via: Via, method: impl Into<String>, outcome: Outcome) -> Entry {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        Entry { at, via, method: method.into(), outcome, detail: None }
    }

    /// With `detail` (cut to [`MAX_DETAIL`] characters).
    pub fn with_detail(mut self, detail: Option<String>) -> Entry {
        self.detail = detail.map(|d| {
            if d.chars().count() > MAX_DETAIL {
                format!("{}…", d.chars().take(MAX_DETAIL).collect::<String>())
            } else {
                d
            }
        });
        self
    }

    /// Refused or failed.
    pub fn refused(&self) -> bool {
        matches!(self.outcome, Outcome::Refused { .. })
    }
}

/// An extension's log: its latest entries, and the file they are kept in.
#[derive(Debug, Default)]
pub struct Activity {
    entries: VecDeque<Entry>,
    file: Option<PathBuf>,
    /// Lines in the file (it is rewritten when it doubles).
    lines: usize,
}

pub type Shared = Arc<Mutex<Activity>>;

/// The file extension `id`'s log is kept in, under `log_dir`
/// (`<data>/extensions/.log`).
pub fn file_of(log_dir: &Path, id: &str) -> PathBuf {
    log_dir.join(format!("{id}.jsonl"))
}

impl Activity {
    /// Its log kept in `file` (`None`: in memory only), the entries there
    /// read back (the latest [`MAX_ENTRIES`] of the last [`KEEP_DAYS`]
    /// days; unreadable lines skipped).
    pub fn open(file: Option<PathBuf>, now_ms: u64) -> Activity {
        let mut a = Activity { entries: VecDeque::new(), file, lines: 0 };
        let Some(f) = &a.file else { return a };
        let text = std::fs::read_to_string(f).unwrap_or_default();
        let oldest = now_ms.saturating_sub(KEEP_DAYS * 86_400_000);
        for l in text.lines() {
            a.lines += 1;
            if let Ok(e) = serde_json::from_str::<Entry>(l) {
                if e.at >= oldest {
                    a.entries.push_back(e);
                }
            }
        }
        while a.entries.len() > MAX_ENTRIES {
            a.entries.pop_front();
        }
        if a.lines > a.entries.len() {
            a.rewrite();
        }
        a
    }

    /// Add `e` (the oldest goes past [`MAX_ENTRIES`]) and keep it.
    pub fn push(&mut self, e: Entry) {
        let line = serde_json::to_string(&e).unwrap_or_default();
        self.entries.push_back(e);
        while self.entries.len() > MAX_ENTRIES {
            self.entries.pop_front();
        }
        let Some(f) = &self.file else { return };
        if self.lines >= MAX_ENTRIES * 2 {
            self.rewrite();
            return;
        }
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let appended = std::fs::OpenOptions::new().create(true).append(true).open(f).and_then(|mut w| w.write_all(format!("{line}\n").as_bytes()));
        if appended.is_ok() {
            self.lines += 1;
        }
    }

    /// The entries, oldest first.
    pub fn entries(&self) -> Vec<Entry> {
        self.entries.iter().cloned().collect()
    }

    /// Forget it all, the file too (the extension is removed).
    pub fn clear(&mut self) {
        self.entries.clear();
        if let Some(f) = &self.file {
            let _ = std::fs::remove_file(f);
        }
        self.lines = 0;
    }

    /// The file holds only what is kept now.
    fn rewrite(&mut self) {
        let Some(f) = &self.file else { return };
        let text: String = self.entries.iter().filter_map(|e| serde_json::to_string(e).ok()).map(|l| l + "\n").collect();
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = f.with_extension("jsonl.tmp");
        if std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, f)).is_ok() {
            self.lines = self.entries.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(at: u64, method: &str) -> Entry {
        Entry { at, via: Via::Adapter, method: method.into(), outcome: Outcome::Ok, detail: None }
    }

    /// Kept in its file across a restart, bounded, old entries gone.
    #[test]
    fn a_log_is_kept_bounded_and_read_back() {
        let t = crate::testing::TempDir::new("activity");
        let f = file_of(&t.path().join(".log"), "ci");
        let day = 86_400_000;
        let now = 100 * day;
        let mut a = Activity::open(Some(f.clone()), now);
        assert!(a.entries().is_empty());
        a.push(entry(now - 40 * day, "agent.list"));
        a.push(Entry { at: now, via: Via::Socket, method: "queue.add".into(), outcome: Outcome::Refused { code: "unsupported".into(), message: "not yet".into() }, detail: None });
        let back = Activity::open(Some(f.clone()), now);
        assert_eq!(back.entries().len(), 1, "older than 30 days: gone");
        assert!(back.entries()[0].refused());
        assert_eq!(std::fs::read_to_string(&f).unwrap().lines().count(), 1, "the file holds what is kept");
        let mut a = back;
        for i in 0..MAX_ENTRIES * 2 + 5 {
            a.push(entry(now + i as u64, "agent.list"));
        }
        assert_eq!(a.entries().len(), MAX_ENTRIES);
        assert!(std::fs::read_to_string(&f).unwrap().lines().count() <= MAX_ENTRIES * 2 + 1, "the file stays bounded");
        let back = Activity::open(Some(f.clone()), now);
        assert_eq!(back.entries().len(), MAX_ENTRIES);
        assert_eq!(back.entries().last().unwrap().at, now + (MAX_ENTRIES * 2 + 4) as u64);
        let mut a = back;
        a.clear();
        assert!(!f.exists());
        let wire = serde_json::to_value(Entry { at: 1, via: Via::Adapter, method: "m".into(), outcome: Outcome::Failed { code: "bad_params".into(), message: "x".into() }, detail: None }).unwrap();
        assert_eq!(wire, serde_json::json!({"at": 1, "via": "adapter", "method": "m", "outcome": {"kind": "failed", "code": "bad_params", "message": "x"}}));
        let e = Entry::now(Via::View, "queue.add", Outcome::Ok).with_detail(Some("y".repeat(MAX_DETAIL + 5)));
        assert_eq!(e.detail.as_ref().unwrap().chars().count(), MAX_DETAIL + 1, "cut, with …");
        assert_eq!(serde_json::to_value(&e).unwrap()["via"], "view");
    }
}
