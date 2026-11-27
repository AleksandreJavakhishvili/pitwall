//! Extensions' skills for agents (docs/spec/tile-sdk/agent-tools.md
//! §11.11): an extension's `agents/SKILL.md` reaches agents the way the
//! library's skills do, through rulesync, as a skill of its own staged
//! with the agent's rules. Read-only: never in the library or a set.
//!
//! The app keeps what is given in step ([`sync`]): each skill the user
//! gave to some projects, while its extension is on and its tools are
//! offered to agents, is in `<data>/run/ext-skills/<id>/SKILL.md`, with
//! the projects in `index.json`. An agent of one of those projects gets
//! it at its next apply; one turned off, or a project taken off, makes
//! the agents' rules stale, and the next apply removes it (as for the
//! built-in show-files rule). Agents rulesync has no skills for (Gemini
//! CLI, Aider) get its description through the MCP server instead.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::Dirs;

/// The list of skills and their projects, in the folder.
const INDEX: &str = "index.json";

/// An extension's skill as Pitwall gives it to agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtSkill {
    /// The extension's id (also the skill's name).
    pub id: String,
    /// The extension's name, as Settings says where it is from.
    pub name: String,
    pub description: String,
    /// The `SKILL.md` written for agents (see
    /// `extensions::agent_skill::Skill::for_agents`).
    #[serde(skip)]
    pub text: String,
    /// The projects (folders) whose agents get it.
    pub projects: Vec<String>,
}

impl Dirs {
    /// Where extensions' skills wait for agents (`<data>/run/ext-skills`).
    pub fn ext_skills(&self) -> PathBuf {
        self.base.join("run").join("ext-skills")
    }
}

/// The skills as they are now (none when nothing was given).
fn index(dirs: &Dirs) -> BTreeMap<String, ExtSkill> {
    std::fs::read_to_string(dirs.ext_skills().join(INDEX)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Keep the folder in step with `list`: each skill with a project written
/// (only what changed), the others removed. Agents' rules that include one
/// go stale and are reapplied as usual.
pub fn sync(dirs: &Dirs, list: &[ExtSkill]) {
    let root = dirs.ext_skills();
    let keep: BTreeMap<String, ExtSkill> = list.iter().filter(|s| !s.projects.is_empty() && is_name(&s.id)).map(|s| (s.id.clone(), s.clone())).collect();
    if keep.is_empty() {
        if root.exists() {
            let _ = std::fs::remove_dir_all(&root);
        }
        return;
    }
    for s in keep.values() {
        let file = root.join(&s.id).join("SKILL.md");
        if std::fs::read_to_string(&file).ok().as_deref() != Some(s.text.as_str()) {
            let _ = std::fs::create_dir_all(root.join(&s.id));
            let _ = std::fs::write(&file, &s.text);
        }
    }
    if let Ok(entries) = std::fs::read_dir(&root) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() && !keep.contains_key(&name) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    if index(dirs) != keep {
        if let Ok(json) = serde_json::to_string_pretty(&keep) {
            let _ = std::fs::write(root.join(INDEX), json);
        }
    }
}

/// A skill's name a folder can take (an extension id always can).
fn is_name(id: &str) -> bool {
    !id.is_empty() && id != "." && id != ".." && !id.contains(['/', '\\'])
}

fn same_folder(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// The extensions' skills agents of `project` get, with their folders.
pub fn for_project(dirs: &Dirs, project: &str) -> Vec<(ExtSkill, PathBuf)> {
    let root = dirs.ext_skills();
    index(dirs)
        .into_values()
        .filter(|s| s.projects.iter().any(|p| same_folder(p, project)))
        .map(|s| {
            let dir = root.join(&s.id);
            (s, dir)
        })
        .filter(|(_, dir)| dir.join("SKILL.md").is_file())
        .collect()
}
