//! ~/.pitwall/projects.json: the user's project list (shown in the sidebar
//! even without agents) and the "onboarding done" flag. Removing a project
//! only drops it from this list; files are never touched.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::clock::unix_ms;
use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEntry {
    pub path: String,
    pub added_at: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub path: String,
    pub display: String,
    pub is_git: bool,
    pub added_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ProjectsFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    onboarded: bool,
    #[serde(default)]
    projects: Vec<ProjectEntry>,
    /// Session id → project the user chose to show that conversation under
    /// (conversations started outside a project, e.g. in `~`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    conversation_projects: BTreeMap<String, String>,
}

fn load_from(path: &Path) -> ProjectsFile {
    let Ok(src) = std::fs::read_to_string(path) else { return ProjectsFile::default() };
    serde_json::from_str(&src).unwrap_or_else(|err| {
        eprintln!("pitwall: ignoring unreadable {}: {err}", path.display());
        let _ = std::fs::copy(path, path.with_extension("json.corrupt"));
        ProjectsFile::default()
    })
}

fn save_to(path: &Path, file: &ProjectsFile) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&ProjectsFile { version: 1, ..file.clone() }).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// `~/x` → absolute; trailing slashes dropped.
pub fn normalize(path: &str) -> String {
    let p = path.trim();
    let p = match p.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            format!("{}{}", paths::home().to_string_lossy(), rest)
        }
        _ => p.to_string(),
    };
    let trimmed = p.trim_end_matches('/');
    if trimmed.is_empty() { "/".into() } else { trimmed.to_string() }
}

pub fn view(entry: &ProjectEntry) -> Project {
    Project {
        display: paths::tildify(&entry.path),
        is_git: Path::new(&entry.path).join(".git").exists(),
        path: entry.path.clone(),
        added_at: entry.added_at,
    }
}

fn list_at(path: &Path) -> Vec<Project> {
    let mut list: Vec<Project> = load_from(path).projects.iter().map(view).collect();
    list.sort_by_key(|p| p.display.to_lowercase());
    list
}

fn add_at(path: &Path, folders: &[String]) -> Result<usize, String> {
    let mut file = load_from(path);
    let mut added = 0;
    for f in folders {
        let p = normalize(f);
        if !Path::new(&p).is_dir() {
            return Err(format!("{p} is not a folder"));
        }
        if file.projects.iter().any(|e| e.path == p) {
            continue;
        }
        file.projects.push(ProjectEntry { path: p, added_at: unix_ms() });
        added += 1;
    }
    if added > 0 {
        save_to(path, &file)?;
    }
    Ok(added)
}

fn remove_at(path: &Path, folder: &str) -> Result<bool, String> {
    let mut file = load_from(path);
    let p = normalize(folder);
    let before = file.projects.len();
    file.projects.retain(|e| e.path != p);
    let removed = file.projects.len() != before;
    if removed {
        save_to(path, &file)?;
    }
    Ok(removed)
}

fn set_onboarded_at(path: &Path) -> Result<(), String> {
    let mut file = load_from(path);
    if !file.onboarded {
        file.onboarded = true;
        save_to(path, &file)?;
    }
    Ok(())
}

fn remember_conversation_project_at(path: &Path, session_id: &str, project: Option<&str>) -> Result<(), String> {
    let mut file = load_from(path);
    let before = file.conversation_projects.get(session_id).cloned();
    match project.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => file.conversation_projects.insert(session_id.to_string(), normalize(p)),
        None => file.conversation_projects.remove(session_id),
    };
    if file.conversation_projects.get(session_id) != before.as_ref() {
        save_to(path, &file)?;
    }
    Ok(())
}

/// The project list file, with its read-modify-write serialised. Owned by
/// the engine.
pub struct ProjectList {
    file: PathBuf,
    lock: Mutex<()>,
}

impl ProjectList {
    pub fn new(file: PathBuf) -> ProjectList {
        ProjectList { file, lock: Mutex::new(()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn list(&self) -> Vec<Project> {
        let _g = self.lock();
        list_at(&self.file)
    }

    /// Adds folders that exist and aren't listed yet. Returns how many were added.
    pub fn add(&self, folders: &[String]) -> Result<usize, String> {
        let _g = self.lock();
        add_at(&self.file, folders)
    }

    pub fn remove(&self, folder: &str) -> Result<bool, String> {
        let _g = self.lock();
        remove_at(&self.file, folder)
    }

    pub fn onboarded(&self) -> bool {
        let _g = self.lock();
        load_from(&self.file).onboarded
    }

    pub fn set_onboarded(&self) -> Result<(), String> {
        let _g = self.lock();
        set_onboarded_at(&self.file)
    }

    /// Remember (or forget, with `None`) which project a conversation is shown under.
    pub fn remember_conversation_project(&self, session_id: &str, project: Option<&str>) -> Result<(), String> {
        let _g = self.lock();
        remember_conversation_project_at(&self.file, session_id, project)
    }

    /// Session id → chosen display project.
    pub fn conversation_projects(&self) -> BTreeMap<String, String> {
        let _g = self.lock();
        load_from(&self.file).conversation_projects
    }

    /// Paths currently in the list (for marking scan results as already added).
    pub fn known_paths(&self) -> Vec<String> {
        let _g = self.lock();
        load_from(&self.file).projects.into_iter().map(|e| e.path).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pitwall-projects-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn add_list_remove_roundtrip() {
        let dir = tmp("rt");
        let file = dir.join("projects.json");
        let a = dir.join("alpha");
        let b = dir.join("beta");
        std::fs::create_dir_all(a.join(".git")).unwrap();
        std::fs::create_dir_all(&b).unwrap();

        assert!(list_at(&file).is_empty());
        assert!(!load_from(&file).onboarded);
        let paths = vec![
            format!("{}/", b.display()),
            a.to_string_lossy().into_owned(),
            a.to_string_lossy().into_owned(),
        ];
        assert_eq!(add_at(&file, &paths).unwrap(), 2);
        let list = list_at(&file);
        assert_eq!(list.len(), 2);
        let alpha = list.iter().find(|p| p.path.ends_with("alpha")).unwrap();
        assert!(alpha.is_git);
        assert!(!list.iter().find(|p| p.path.ends_with("beta")).unwrap().is_git);

        assert!(remove_at(&file, &b.to_string_lossy()).unwrap());
        assert!(!remove_at(&file, "/nope").unwrap());
        assert_eq!(list_at(&file).len(), 1);
        // Removing never touches the folder itself.
        assert!(b.is_dir());

        set_onboarded_at(&file).unwrap();
        assert!(load_from(&file).onboarded);
        assert_eq!(list_at(&file).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_folders_are_rejected() {
        let dir = tmp("missing");
        let file = dir.join("projects.json");
        assert!(add_at(&file, &["/definitely/not/here".into()]).is_err());
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn conversation_project_choices_persist() {
        let dir = tmp("conv");
        let file = dir.join("projects.json");
        remember_conversation_project_at(&file, "s1", Some("/x/pitwall/")).unwrap();
        remember_conversation_project_at(&file, "s2", Some("/x/other")).unwrap();
        add_at(&file, &[dir.to_string_lossy().into_owned()]).unwrap();
        let got = load_from(&file).conversation_projects;
        assert_eq!(got.get("s1").map(String::as_str), Some("/x/pitwall"));
        assert_eq!(got.len(), 2);
        remember_conversation_project_at(&file, "s2", None).unwrap();
        assert!(!load_from(&file).conversation_projects.contains_key("s2"));
        // Older files without the field still load.
        std::fs::write(&file, r#"{"version":1,"onboarded":true,"projects":[]}"#).unwrap();
        assert!(load_from(&file).onboarded);
        assert!(load_from(&file).conversation_projects.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_paths() {
        assert_eq!(normalize("/a/b/"), "/a/b");
        assert_eq!(normalize("/"), "/");
        assert_eq!(normalize("~/x"), format!("{}/x", paths::home().to_string_lossy()));
    }
}
