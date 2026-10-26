//! What peers keep under `<PITWALL_HOME>/peers/` (tailnet-peers.md §5):
//!
//! - `settings.json`: the two switches and the port ([`PeerSettings`]);
//! - `peers.json`: the machines this Pitwall shows (`peers`) and the ones
//!   that may see it (`viewers`), each with its Tailscale node, user and
//!   Pitwall key ([`Paired`]);
//! - `audit.jsonl`: connections, pairings and refusals, one JSON line
//!   each, rotated at 1 MiB keeping 3 files ([`AuditEntry`]);
//! - `identity.key`: the instance key ([`crate::key`]).
//!
//! Files are read on every query (they are tiny), so a change the CLI made
//! is seen by the app too. Writes replace the file whole.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use pitwall_proto::peer::PEER_PORT;
use pitwall_proto::{code, ErrorBody};

use crate::lock;

/// The two switches (both off by default) and the port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PeerSettings {
    /// "Let my other machines see this Pitwall's agents": listen on this
    /// node's Tailscale addresses. Off: nothing listens.
    pub share: bool,
    /// "Show agents from my other machines": discover, pair and keep links
    /// to paired machines. Off: nothing is probed or dialed.
    pub show: bool,
    /// Where this Pitwall listens, and where it reaches peers.
    pub port: u16,
}

impl Default for PeerSettings {
    fn default() -> Self {
        PeerSettings { share: false, show: false, port: PEER_PORT }
    }
}

/// A paired machine: one this Pitwall shows (`peers`) or one that may see
/// it (`viewers`). The node, its user and the Pitwall key must all match on
/// every connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paired {
    /// This Pitwall's name for it: a short slug of its label for peers
    /// (`studio`, ids of mirrored agents use it), the node's StableID for
    /// viewers.
    pub id: String,
    /// Its machine's name (`studio`).
    pub label: String,
    /// Tailscale node `StableID`.
    pub stable_id: String,
    /// Its owner's Tailscale login.
    pub login: String,
    /// Its Pitwall's key, `ed25519:…`.
    pub public_key: String,
    /// ms since the epoch.
    pub added_at: u64,
    /// The last time it connected (ms), if ever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PeersFile {
    /// Machines whose agents this Pitwall shows.
    pub peers: Vec<Paired>,
    /// Machines that may see this Pitwall's agents.
    pub viewers: Vec<Paired>,
}

/// One line of `audit.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    /// ms since the epoch.
    pub at: u64,
    /// `connect`, `pair`, `repair`, `unpair`, `revoke`, `denied`,
    /// `refused`, `identity_changed`, `listen`, `stop`.
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl AuditEntry {
    pub fn new(event: &str) -> AuditEntry {
        AuditEntry { at: crate::now_ms(), event: event.into(), peer: None, login: None, node: None, addr: None, method: None, detail: None }
    }

    pub fn peer(mut self, v: impl Into<String>) -> Self {
        self.peer = Some(v.into());
        self
    }

    pub fn login(mut self, v: impl Into<String>) -> Self {
        self.login = Some(v.into()).filter(|s: &String| !s.is_empty());
        self
    }

    pub fn node(mut self, v: impl Into<String>) -> Self {
        self.node = Some(v.into()).filter(|s: &String| !s.is_empty());
        self
    }

    pub fn addr(mut self, v: impl ToString) -> Self {
        self.addr = Some(v.to_string());
        self
    }

    pub fn method(mut self, v: impl Into<String>) -> Self {
        self.method = Some(v.into());
        self
    }

    pub fn detail(mut self, v: impl Into<String>) -> Self {
        self.detail = Some(v.into());
        self
    }
}

/// The audit log rotates at this size…
pub const AUDIT_MAX: u64 = 1024 * 1024;
/// …keeping this many files (`audit.jsonl`, `.1`, `.2`).
pub const AUDIT_FILES: usize = 3;

/// `<root>/peers/`.
pub struct Store {
    dir: PathBuf,
    /// One writer at a time in this process.
    write: Mutex<()>,
}

fn io_err(what: &str, e: impl std::fmt::Display) -> ErrorBody {
    ErrorBody::new(code::OTHER, format!("{what}: {e}"))
}

impl Store {
    /// The store under a Pitwall data folder (`Paths::root`).
    pub fn new(root: &Path) -> Store {
        Store { dir: root.join("peers"), write: Mutex::new(()) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn read<T: Default + for<'de> Deserialize<'de>>(&self, name: &str) -> T {
        // A missing or damaged file reads as empty (off, nothing paired).
        std::fs::read(self.dir.join(name)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn write<T: Serialize>(&self, name: &str, v: &T) -> Result<(), ErrorBody> {
        std::fs::create_dir_all(&self.dir).map_err(|e| io_err("peers folder", e))?;
        let path = self.dir.join(name);
        let tmp = path.with_extension("tmp");
        let mut bytes = serde_json::to_vec_pretty(v).map_err(|e| io_err(name, e))?;
        bytes.push(b'\n');
        std::fs::write(&tmp, bytes).map_err(|e| io_err(name, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| io_err(name, e))
    }

    pub fn settings(&self) -> PeerSettings {
        self.read("settings.json")
    }

    pub fn set_settings(&self, s: &PeerSettings) -> Result<(), ErrorBody> {
        let _w = lock(&self.write);
        self.write("settings.json", s)
    }

    pub fn pairings(&self) -> PeersFile {
        self.read("peers.json")
    }

    /// Change `peers.json` under the write lock.
    pub fn update<R>(&self, f: impl FnOnce(&mut PeersFile) -> R) -> Result<R, ErrorBody> {
        let _w = lock(&self.write);
        let mut file = self.pairings();
        let r = f(&mut file);
        self.write("peers.json", &file)?;
        Ok(r)
    }

    /// Add a line to the audit log (failures are ignored: the log never
    /// stops a connection).
    pub fn audit(&self, entry: AuditEntry) {
        let _w = lock(&self.write);
        let _ = self.append_audit(&entry);
    }

    fn append_audit(&self, entry: &AuditEntry) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join("audit.jsonl");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() >= AUDIT_MAX) {
            for i in (1..AUDIT_FILES).rev() {
                let from = if i == 1 { path.clone() } else { self.dir.join(format!("audit.jsonl.{}", i - 1)) };
                let _ = std::fs::rename(from, self.dir.join(format!("audit.jsonl.{i}")));
            }
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        let mut line = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
        line.push(b'\n');
        f.write_all(&line)
    }

    /// The newest `limit` lines of the current log, oldest first.
    pub fn audit_tail(&self, limit: usize) -> Vec<AuditEntry> {
        let text = std::fs::read_to_string(self.dir.join("audit.jsonl")).unwrap_or_default();
        let all: Vec<AuditEntry> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        all[all.len().saturating_sub(limit)..].to_vec()
    }
}

/// A short id for a peer from its label, unique among `taken`
/// (`studio`, `studio-2`).
pub fn slug(label: &str, taken: &[&str]) -> String {
    let mut base: String = label.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    base = base.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    if base.is_empty() {
        base = "peer".into();
    }
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id.as_str()) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use pitwall_core::testing::TempDir;

    #[test]
    fn settings_default_off_and_round_trip() {
        let dir = TempDir::new("peer-store");
        let store = Store::new(dir.path());
        assert_eq!(store.settings(), PeerSettings { share: false, show: false, port: 47420 });
        let on = PeerSettings { share: true, show: true, port: 5000 };
        store.set_settings(&on).unwrap();
        assert_eq!(store.settings(), on);
        assert_eq!(serde_json::from_str::<PeerSettings>(r#"{"show":true}"#).unwrap(), PeerSettings { show: true, ..PeerSettings::default() });
    }

    #[test]
    fn pairings_are_kept() {
        let dir = TempDir::new("peer-store");
        let store = Store::new(dir.path());
        let p = Paired {
            id: "studio".into(),
            label: "studio".into(),
            stable_id: "nSTUDIO".into(),
            login: "alice@example.com".into(),
            public_key: "ed25519:k".into(),
            added_at: 1,
            last_seen: None,
        };
        store.update(|f| f.peers.push(p.clone())).unwrap();
        assert_eq!(store.pairings().peers, [p]);
        let json = std::fs::read_to_string(dir.path().join("peers/peers.json")).unwrap();
        assert!(json.contains("\"stableId\": \"nSTUDIO\"") && json.contains("\"publicKey\""), "{json}");
    }

    #[test]
    fn the_audit_log_rotates() {
        let dir = TempDir::new("peer-store");
        let store = Store::new(dir.path());
        store.audit(AuditEntry::new("connect").peer("studio").login("alice@example.com").node(""));
        let tail = store.audit_tail(10);
        assert_eq!(tail.len(), 1);
        assert_eq!((tail[0].event.as_str(), tail[0].node.as_deref()), ("connect", None));
        std::fs::write(dir.path().join("peers/audit.jsonl"), vec![b' '; AUDIT_MAX as usize]).unwrap();
        store.audit(AuditEntry::new("denied"));
        assert!(dir.path().join("peers/audit.jsonl.1").is_file());
        assert_eq!(store.audit_tail(10)[0].event, "denied");
    }

    #[test]
    fn slugs_are_short_and_unique() {
        assert_eq!(slug("Studio", &[]), "studio");
        assert_eq!(slug("Alice's MacBook Pro", &[]), "alice-s-macbook-pro");
        assert_eq!(slug("studio", &["studio", "studio-2"]), "studio-3");
        assert_eq!(slug("···", &[]), "peer");
    }
}
