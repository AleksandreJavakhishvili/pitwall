//! Recent projects, taken from where Claude Code keeps its conversations
//! (~/.claude/projects/<encoded-path>/<session>.jsonl). Read-only.

use std::io::{BufRead, BufReader};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::paths::{self, Paths};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub path: String,
    pub display: String,
    /// Unix milliseconds of the newest conversation.
    pub last_used: u64,
}

/// `pitwall`: to leave out worktrees that old Pitwall versions made.
pub fn recent_projects(pitwall: &Paths, limit: usize) -> Vec<RecentProject> {
    let root = paths::home().join(".claude").join("projects");
    let Ok(dirs) = std::fs::read_dir(root) else { return vec![] };

    let mut found: Vec<RecentProject> = Vec::new();
    for dir in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
        let newest = files
            .flatten()
            .filter(|f| f.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
            .filter_map(|f| {
                let modified = f.metadata().ok()?.modified().ok()?;
                Some((modified, f.path()))
            })
            .max_by_key(|(m, _)| *m);
        let Some((modified, file)) = newest else { continue };
        // The directory name encodes the path lossily; the transcript has the real cwd.
        let Some(cwd) = transcript_cwd(&file) else { continue };
        if !std::path::Path::new(&cwd).is_dir() || found.iter().any(|p| p.path == cwd) {
            continue;
        }
        found.push(RecentProject {
            display: paths::tildify(&cwd),
            path: cwd,
            last_used: modified
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        });
    }
    found.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    // Worktrees older Pitwall versions made show up here too; they aren't projects.
    let worktrees = pitwall.legacy_worktrees_dir().to_string_lossy().into_owned();
    found.retain(|p| !p.path.starts_with(&worktrees));
    found.truncate(limit);
    found
}

fn transcript_cwd(file: &std::path::Path) -> Option<String> {
    let reader = BufReader::new(std::fs::File::open(file).ok()?);
    for line in reader.lines().take(50).map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
        if let Some(cwd) = value.get("cwd").and_then(|c| c.as_str()) {
            return Some(cwd.to_string());
        }
    }
    None
}
