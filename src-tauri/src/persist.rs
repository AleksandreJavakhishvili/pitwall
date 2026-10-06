//! ~/.pitwall/state.json: agent records survive restarts (loaded as `stopped`).

use serde::{Deserialize, Serialize};

use crate::model::AgentRecord;
use crate::paths;

#[derive(Serialize, Deserialize, Default)]
struct StateFile {
    version: u32,
    agents: Vec<AgentRecord>,
}

pub fn load() -> Vec<AgentRecord> {
    let path = paths::state_file();
    let Ok(src) = std::fs::read_to_string(&path) else { return vec![] };
    match serde_json::from_str::<StateFile>(&src) {
        Ok(state) => state.agents,
        Err(err) => {
            eprintln!("pitwall: ignoring unreadable {}: {err}", path.display());
            let _ = std::fs::copy(&path, path.with_extension("json.corrupt"));
            vec![]
        }
    }
}

pub fn save(agents: Vec<AgentRecord>) -> Result<(), String> {
    let path = paths::state_file();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&StateFile { version: 1, agents }).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
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
}
