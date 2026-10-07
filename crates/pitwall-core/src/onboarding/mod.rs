//! First-launch auto-detect and the project list (docs/spec/onboarding.md).
//! Events: `ScanProgress`, `ProjectsChanged`.

pub mod elsewhere;
pub mod latest;
pub mod places;
pub mod project_list;
pub mod recent;
pub mod scan;

use crate::engine::{lifecycle, Engine, Shared};
use crate::events::Event;
use crate::hooks;
use crate::model::{AdoptSessionRequest, AgentView, CreateAgentRequest};
use project_list::Project;
use scan::ScanResult;

fn projects_changed(engine: &Engine) -> Vec<Project> {
    let list = engine.projects().list();
    engine.emit(Event::ProjectsChanged(list.clone()));
    list
}

/// Run the read-only scan, reporting each step as an event. Blocking.
pub fn scan_environment(engine: &Engine) -> ScanResult {
    let views = engine.list_kinds().unwrap_or_default();
    let owned = engine.records().iter().map(|r| r.locator()).collect();
    let places = || places::places(engine.providers(), engine.kinds(), &owned);
    let mut result = scan::scan(engine.paths(), engine.kinds(), &views, engine.projects(), &places, &|p| {
        engine.emit(Event::ScanProgress(p));
    });
    // "Scan again" must not offer sessions Pitwall agents already run,
    // including agents started by hand in Pitwall terminals.
    scan::mark_in_pitwall(&mut result, &engine.owned_sessions());
    let pids = engine.owned_pids();
    for r in result.running.iter_mut().filter(|r| pids.contains(&r.pid)) {
        r.in_pitwall = true;
    }
    result
}

pub fn add_project(engine: &Engine, path: String) -> Result<Vec<Project>, String> {
    engine.projects().add(&[path])?;
    Ok(projects_changed(engine))
}

/// Drops the project from Pitwall's list. Never deletes files.
pub fn remove_project(engine: &Engine, path: &str) -> Result<Vec<Project>, String> {
    engine.projects().remove(path)?;
    Ok(projects_changed(engine))
}

/// Save the chosen projects and finish onboarding. Blocking.
pub fn complete(engine: &Engine, projects: &[String], install_codex_hooks: bool) -> Result<Vec<Project>, String> {
    // Folders that vanished since the scan are skipped, not fatal.
    for p in projects {
        if let Err(e) = engine.projects().add(std::slice::from_ref(p)) {
            eprintln!("pitwall: not adding project: {e}");
        }
    }
    engine.projects().set_onboarded()?;
    let list = projects_changed(engine);
    if install_codex_hooks {
        let paths = engine.paths();
        hooks::install_script(paths)
            .and_then(|_| hooks::install_codex_hooks_at(&hooks::codex_hooks_path(), &hooks::hook_command(paths)))
            .map_err(|e| format!("Projects were saved, but Codex hooks could not be installed: {e}"))?;
    }
    Ok(list)
}

/// "Add to Pitwall" on a session the scan found on another machine
/// (`ScanResult.places`). Nothing changes on that machine. Blocking.
pub fn adopt_session(engine: &Shared, req: AdoptSessionRequest) -> Result<AgentView, String> {
    lifecycle::adopt(engine, req)
}

/// "Continue" on a conversation found by the scan.
pub struct ContinueRequest {
    pub kind: String,
    pub session_id: String,
    pub project_path: String,
    pub name: String,
    pub display_project: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

/// Start an agent that resumes an existing conversation. Blocking.
pub fn continue_conversation(engine: &Shared, req: ContinueRequest) -> Result<AgentView, String> {
    if req.session_id.trim().is_empty() {
        return Err("no session to continue".into());
    }
    let display_project = req.display_project.filter(|p| !p.trim().is_empty());
    let remember = (req.session_id.clone(), display_project.clone());
    let view = lifecycle::create(
        engine,
        CreateAgentRequest {
            name: req.name,
            kind: req.kind,
            project_path: req.project_path,
            custom_command: None,
            worktree: false,
            resume_session_id: Some(req.session_id),
            rule_set_id: None,
            apply_to_main_checkout: false,
            display_project,
            cols: req.cols,
            rows: req.rows,
            provider: None,
            machine: None,
            options: Default::default(),
        },
    )?;
    // "Scan again" keeps the user's choice of project for this conversation.
    if let (id, Some(p)) = &remember {
        if let Err(e) = engine.projects().remember_conversation_project(id, Some(p)) {
            eprintln!("pitwall: not remembering the project for {id}: {e}");
        }
    }
    add_agent_project(engine, &view);
    Ok(view)
}

/// The agent's project joins the project list if it wasn't there yet (not
/// `~` etc.), so it shows in the sidebar even after its agents are removed.
pub fn add_agent_project(engine: &Engine, view: &AgentView) {
    // A platform's session (added or created there) works in the
    // platform's folder, not one of this Mac's projects.
    if view.caps.remove_keeps_session {
        return;
    }
    if !scan::is_project_folder(engine.paths(), &view.project) {
        return;
    }
    match engine.projects().add(std::slice::from_ref(&view.project)) {
        Ok(n) if n > 0 => {
            projects_changed(engine);
        }
        Ok(_) => {}
        Err(e) => eprintln!("pitwall: not adding project {}: {e}", view.project),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Harness;

    fn project_events(h: &Harness) -> Vec<Vec<Project>> {
        h.events
            .take()
            .into_iter()
            .filter_map(|e| match e {
                Event::ProjectsChanged(list) => Some(list),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn project_list_changes_are_announced() {
        let h = Harness::new(vec![]);
        let folder = h.dir.path().join("my-app");
        std::fs::create_dir_all(&folder).unwrap();
        let folder = folder.to_string_lossy().into_owned();

        let list = add_project(&h.engine, folder.clone()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(project_events(&h), vec![list]);

        assert!(add_project(&h.engine, "/definitely/not/here".into()).is_err());
        assert!(project_events(&h).is_empty(), "nothing changed, nothing sent");

        assert!(remove_project(&h.engine, &folder).unwrap().is_empty());
        assert_eq!(project_events(&h), vec![vec![]]);
        assert!(std::path::Path::new(&folder).is_dir(), "files are never touched");
    }

    #[test]
    fn completing_onboarding_skips_vanished_folders() {
        let h = Harness::new(vec![]);
        let folder = h.dir.path().join("kept");
        std::fs::create_dir_all(&folder).unwrap();
        let list = complete(&h.engine, &[folder.to_string_lossy().into_owned(), "/gone/away".into()], false).unwrap();
        assert_eq!(list.len(), 1);
        assert!(h.engine.projects().onboarded());
        assert_eq!(project_events(&h).len(), 1);
    }

    #[test]
    fn continuing_needs_a_session() {
        let h = Harness::new(vec![]);
        let req = ContinueRequest {
            kind: "claude".into(),
            session_id: "  ".into(),
            project_path: "/tmp".into(),
            name: "x".into(),
            display_project: None,
            cols: None,
            rows: None,
        };
        assert_eq!(continue_conversation(&h.engine, req).unwrap_err(), "no session to continue");
    }
}
