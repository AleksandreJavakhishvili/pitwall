//! The owner's collaboration audit log, `collab/audit.jsonl` (§8.8): every
//! turn (with its front), refusal, result decision, share change (with its
//! diff) and join. tailnet-peers.md §5's line format
//! (`{ at, peer, login, node, method, agent, outcome, approvedOn }`) with
//! `kind: "collab"` and a few collaboration fields; rotates at 1 MiB,
//! keeping 3 files (`audit.jsonl`, `.1`, `.2`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::store::{append_jsonl, read_jsonl};

/// Rotate when the file reaches this size.
pub const ROTATE_BYTES: u64 = 1024 * 1024;
/// Files kept, the live one included.
pub const KEEP_FILES: usize = 3;

/// One line.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLine {
    pub at: String,
    /// Always `collab`.
    pub kind: String,
    /// The network the request came over (`tailscale:example.ts.net`);
    /// `local` for the owner's own changes and guest sessions.
    pub peer: String,
    /// Who: the caller's identity (`provider:subject`), or `owner`.
    pub login: String,
    /// The caller's device on that network, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// What: `ask`, `cancel`, `forget`, `share.create`, `share.set`,
    /// `share.go_live`, `team.join`, …
    pub method: String,
    /// The share (its slug) or team it was about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// `ok`, `refused:quota`, `not_found`, `done`, `failed:timeout`, …
    pub outcome: String,
    /// Set when an approval decided it (`approved on studio`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    /// A share change's diff, a refusal's sentence, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl AuditLine {
    pub fn new(at: String, method: &str, outcome: impl Into<String>) -> AuditLine {
        AuditLine {
            at,
            kind: "collab".into(),
            peer: "local".into(),
            login: "owner".into(),
            method: method.into(),
            outcome: outcome.into(),
            ..AuditLine::default()
        }
    }
}

/// The log. Writes are serialized; a write that fails is reported on stderr
/// and never fails the request it records.
pub struct AuditLog {
    path: PathBuf,
    lock: Mutex<()>,
    rotate_at: u64,
}

impl AuditLog {
    pub fn new(path: impl Into<PathBuf>) -> AuditLog {
        AuditLog { path: path.into(), lock: Mutex::new(()), rotate_at: ROTATE_BYTES }
    }

    /// For tests: rotate at a smaller size.
    pub fn with_rotation(path: impl Into<PathBuf>, rotate_at: u64) -> AuditLog {
        AuditLog { path: path.into(), lock: Mutex::new(()), rotate_at }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&self, line: &AuditLine) {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if std::fs::metadata(&self.path).is_ok_and(|m| m.len() >= self.rotate_at) {
            self.rotate();
        }
        if let Err(e) = append_jsonl(&self.path, line) {
            eprintln!("pitwall: collaboration audit log: {e}");
        }
    }

    fn rotated(&self, n: usize) -> PathBuf {
        let mut name = self.path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".{n}"));
        self.path.with_file_name(name)
    }

    fn rotate(&self) {
        for n in (1..KEEP_FILES).rev() {
            let from = if n == 1 { self.path.clone() } else { self.rotated(n - 1) };
            let _ = std::fs::rename(&from, self.rotated(n));
        }
    }

    /// Every line kept, oldest first (the owner view, Settings).
    pub fn lines(&self) -> Vec<AuditLine> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for n in (1..KEEP_FILES).rev() {
            out.extend(read_jsonl::<AuditLine>(&self.rotated(n)));
        }
        out.extend(read_jsonl::<AuditLine>(&self.path));
        out
    }
}
