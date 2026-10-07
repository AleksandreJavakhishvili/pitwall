//! Agent records survive restarts. The engine talks to a [`Store`]; the
//! app uses [`FileStore`] (`~/.pitwall/state.json`), tests use
//! `testing::MemStore`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::AgentRecord;

/// What persists.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub agents: Vec<AgentRecord>,
}

pub trait Store: Send + Sync {
    /// The saved state; empty when there is none or it can't be read.
    fn load(&self) -> Snapshot;
    fn save(&self, snapshot: &Snapshot) -> Result<(), String>;
}

#[derive(Serialize, Deserialize, Default)]
struct StateFile {
    version: u32,
    agents: Vec<AgentRecord>,
}

/// Current `state.json` format. v2 (architecture.md §5): records carry their
/// `locator`; v1 records mean `local:this-mac/<id>` and load unchanged.
pub const STATE_VERSION: u32 = 2;

/// `state.json`, written atomically (temp file + rename). An unreadable file
/// is kept aside as `state.json.corrupt` and treated as empty. Loading a v1
/// file keeps a one-time copy as `state.v1.json` before v2 is written.
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(path: impl Into<PathBuf>) -> FileStore {
        FileStore { path: path.into() }
    }
}

impl Store for FileStore {
    fn load(&self) -> Snapshot {
        let path = &self.path;
        let Ok(src) = std::fs::read_to_string(path) else { return Snapshot::default() };
        match serde_json::from_str::<StateFile>(&src) {
            Ok(state) => {
                let backup = path.with_file_name("state.v1.json");
                if state.version < STATE_VERSION && !backup.exists() {
                    let _ = std::fs::copy(path, &backup);
                }
                Snapshot { agents: state.agents }
            }
            Err(err) => {
                eprintln!("pitwall: ignoring unreadable {}: {err}", path.display());
                let _ = std::fs::copy(path, path.with_extension("json.corrupt"));
                Snapshot::default()
            }
        }
    }

    fn save(&self, snapshot: &Snapshot) -> Result<(), String> {
        let path = &self.path;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let file = StateFile { version: STATE_VERSION, agents: snapshot.agents.clone() };
        let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_records_get_defaults() {
        let src = r#"{"version":1,"agents":[{"id":"a","name":"n","kind":"claude","kindName":"Claude Code",
            "cwd":"/tmp","project":"/tmp","createdAt":1}]}"#;
        let state: StateFile = serde_json::from_str(src).unwrap();
        let a = &state.agents[0];
        assert!(a.auto_send);
        assert!(a.queue.is_empty());
        assert_eq!(a.session_id, None);
        assert!(!a.worktree_pending);
    }

    #[test]
    fn worktree_agents_made_by_older_versions_load() {
        let src = r#"{"version":1,"agents":[{"id":"a","name":"n","kind":"claude","kindName":"Claude Code",
            "cwd":"/h/.pitwall/worktrees/r/n","project":"/r","createdAt":1,
            "worktree":{"repo":"/r","path":"/h/.pitwall/worktrees/r/n","branch":"pitwall/n"}}]}"#;
        let state: StateFile = serde_json::from_str(src).unwrap();
        let a = &state.agents[0];
        let wt = a.worktree.as_ref().unwrap();
        assert_eq!((wt.path.as_str(), wt.branch.as_str()), ("/h/.pitwall/worktrees/r/n", "pitwall/n"));
        assert_eq!(a.cwd, wt.path);
        assert!(!a.worktree_pending);
    }

    #[test]
    fn v1_state_is_backed_up_once_and_means_local() {
        let dir = crate::testing::TempDir::new("store-v1");
        let path = dir.path().join("state.json");
        let v1 = r#"{"version":1,"agents":[{"id":"a","name":"n","kind":"shell","kindName":"Shell","cwd":"/tmp","project":"/tmp","createdAt":1}]}"#;
        std::fs::write(&path, v1).unwrap();
        let store = FileStore::new(&path);
        let snap = store.load();
        assert_eq!(std::fs::read_to_string(dir.path().join("state.v1.json")).unwrap(), v1);
        let loc = snap.agents[0].locator();
        assert_eq!(snap.agents[0].locator, None);
        assert_eq!(loc.to_string(), "local:this-mac/a");
        let mut snap = snap;
        snap.agents[0].locator = Some(loc);
        store.save(&snap).unwrap();
        let back = store.load();
        assert_eq!(back.agents[0].locator.as_ref().map(|l| l.to_string()).as_deref(), Some("local:this-mac/a"));
        assert_eq!(std::fs::read_to_string(dir.path().join("state.v1.json")).unwrap(), v1, "backup kept as it was");
    }

    #[test]
    fn file_store_round_trips_and_survives_garbage() {
        let dir = crate::testing::TempDir::new("store");
        let store = FileStore::new(dir.path().join("sub").join("state.json"));
        assert!(store.load().agents.is_empty(), "missing file = empty");
        let rec = crate::testing::record("a1", "/tmp");
        store.save(&Snapshot { agents: vec![rec] }).unwrap();
        let back = store.load();
        assert_eq!(back.agents.len(), 1);
        assert_eq!(back.agents[0].id, "a1");

        let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("sub").join("state.json")).unwrap()).unwrap();
        assert_eq!(saved["version"], 2);

        std::fs::write(dir.path().join("sub").join("state.json"), "{nope").unwrap();
        assert!(store.load().agents.is_empty());
        assert!(dir.path().join("sub").join("state.json.corrupt").exists());
    }
}
