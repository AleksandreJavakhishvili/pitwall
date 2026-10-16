//! Where collaboration keeps its files (§8.8), and the small JSON helpers
//! they share. Everything is under `<PITWALL_HOME>/collab/`:
//!
//! ```text
//! shares/<share>.json          share settings (owner)
//! shares/<share>/base/         instructions, approved summary
//! shares/<share>/s/<conv>/     conversation.json, work/ (snapshot), home/, tmp/
//! sandbox/<share>/<conv>/      the sandbox backend's state (outside what sessions read)
//! people.json                  owner's identity links and block list (§15.7)
//! usage.jsonl                  per share, per caller, per day (§5.8)
//! audit.jsonl                  every turn, refusal, share change, join (§8.8)
//! teams/<team>/chain.jsonl     signed records (§4.2)
//! teams/<team>/invites.json    admin only: invite code hashes
//! teams/<team>/requests.json   join requests waiting for an admin
//! discovered.json              listed teams seen on other Pitwalls (§3)
//! ```

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{PwError, Result};

/// The collaboration folder.
#[derive(Debug, Clone)]
pub struct CollabPaths {
    root: PathBuf,
}

impl CollabPaths {
    /// `root` is `<PITWALL_HOME>/collab` ([`CollabPaths::of`]).
    pub fn new(root: impl Into<PathBuf>) -> CollabPaths {
        CollabPaths { root: root.into() }
    }

    pub fn of(paths: &crate::paths::Paths) -> CollabPaths {
        CollabPaths::new(paths.root().join("collab"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn shares_dir(&self) -> PathBuf {
        self.root.join("shares")
    }

    pub fn share_file(&self, slug: &str) -> PathBuf {
        self.shares_dir().join(format!("{slug}.json"))
    }

    pub fn share_dir(&self, slug: &str) -> PathBuf {
        self.shares_dir().join(slug)
    }

    pub fn base_dir(&self, slug: &str) -> PathBuf {
        self.share_dir(slug).join("base")
    }

    pub fn session_dir(&self, slug: &str, conversation: &str) -> PathBuf {
        self.share_dir(slug).join("s").join(conversation)
    }

    /// The sandbox backend's own files for one session (its profile):
    /// outside the session folder, so the session can't read them.
    pub fn sandbox_state(&self, slug: &str, conversation: &str) -> PathBuf {
        self.root.join("sandbox").join(slug).join(conversation)
    }

    pub fn people_file(&self) -> PathBuf {
        self.root.join("people.json")
    }

    pub fn usage_file(&self) -> PathBuf {
        self.root.join("usage.jsonl")
    }

    pub fn audit_file(&self) -> PathBuf {
        self.root.join("audit.jsonl")
    }

    pub fn team_dir(&self, team: &str) -> PathBuf {
        self.root.join("teams").join(team)
    }

    pub fn teams_dir(&self) -> PathBuf {
        self.root.join("teams")
    }

    pub fn discovered_file(&self) -> PathBuf {
        self.root.join("discovered.json")
    }
}

fn io(path: &Path, e: std::io::Error) -> PwError {
    PwError::other(format!("{}: {e}", path.display()))
}

/// A JSON file, or `None` when it isn't there. A damaged file is an error.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| PwError::other(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(path, e)),
    }
}

/// Write `value` as pretty JSON, atomically (a temp file renamed over it).
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| PwError::other(e.to_string()))?;
    write_atomic(path, &bytes)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    }
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, bytes).map_err(|e| io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io(path, e)
    })
}

/// Append one JSON line.
pub fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    }
    let mut line = serde_json::to_vec(value).map_err(|e| PwError::other(e.to_string()))?;
    line.push(b'\n');
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| io(path, e))?;
    f.write_all(&line).map_err(|e| io(path, e))
}

/// Every line of a JSON-lines file that parses (others are skipped: a line
/// cut short by a crash must not lose the rest).
pub fn read_jsonl<T: DeserializeOwned>(path: &Path) -> Vec<T> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    text.lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// A slug as a file name: `[a-z0-9-]`, 1–48 characters, not starting with `-`.
pub fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 48
        && !slug.starts_with('-')
        && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// An id we made (`c_…`, `t_…`) used as a folder name.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
