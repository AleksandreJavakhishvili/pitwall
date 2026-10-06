//! Creating, starting, stopping and removing agents. These do blocking work
//! (git, process spawn/teardown); call them off the main thread.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use super::{tasks, worktree, Agent, Engine, Shared};
use crate::clock::{unix_ms, Clock};
use crate::kind::{self, launch, AgentKind};
use crate::model::{size_of, AgentRecord, AgentView, CreateAgentRequest};
use crate::onboarding::project_list;
use crate::paths::{expand_tilde, Paths};
use crate::rules;
use crate::session::{Session, Spawn};
use crate::slug::slug;
use crate::vcs::git::Git;
use crate::hooks;

const STOP_GRACE: Duration = Duration::from_millis(1500);

fn kind_for(engine: &Engine, rec_kind: &str, custom: Option<&str>) -> Result<AgentKind, String> {
    if rec_kind == "custom" {
        let cmd = custom.map(str::trim).filter(|c| !c.is_empty()).ok_or("custom command is empty")?;
        return Ok(kind::custom_kind(cmd));
    }
    engine.kinds().find(rec_kind).ok_or_else(|| format!("unknown agent kind \"{rec_kind}\""))
}

/// Before a launch with the kind's worktree flag: the name to pass, and what
/// to compare against to spot the worktree the agent makes.
fn watch_for(core: &Engine, kind: &AgentKind, rec: &AgentRecord, now: u64) -> (String, worktree::Watch) {
    let exec = core.exec_for(&rec.id);
    let named = kind.worktree_args.iter().any(|a| a.contains("{name}"));
    let base = slug(&rec.name);
    let name = if named { worktree::pick_name(&*exec, &rec.project, &base) } else { base };
    let watch = worktree::Watch::new(worktree::roots(&*exec, &rec.project), named.then(|| name.clone()), now);
    (name, watch)
}

/// Start `command_line` for agent `id` in a new terminal holder.
fn spawn(engine: &Engine, id: &str, cwd: &str, command_line: String, size: Option<(u16, u16)>) -> Result<Arc<Session>, String> {
    let paths = engine.paths();
    Session::spawn(Spawn {
        agent_id: id,
        cwd,
        command_line,
        size,
        hold_dir: &paths.hold_dir(),
        holder: engine.holder_bin()?,
        hook_socket: &paths.hook_socket(),
        clock: engine.clock().clone(),
    })
}

/// The project an agent is shown under when it differs from where it runs.
fn display_project(core: &Engine, p: Option<&str>) -> Result<Option<String>, String> {
    let Some(p) = p.map(str::trim).filter(|p| !p.is_empty()) else { return Ok(None) };
    let path = expand_tilde(p);
    if !path.is_dir() {
        return Err(format!("{} is not a folder", path.display()));
    }
    let path = project_list::normalize(&path.to_string_lossy());
    Ok(Some(Git::new(core.exec(), &path).repo_root().unwrap_or(path)))
}

pub fn create(core: &Shared, req: CreateAgentRequest) -> Result<AgentView, String> {
    let project_path = expand_tilde(req.project_path.trim());
    if !project_path.is_dir() {
        return Err(format!("{} is not a folder", project_path.display()));
    }
    let project_path = project_path.to_string_lossy().into_owned();
    let kind = kind_for(core, &req.kind, req.custom_command.as_deref())?;
    let name = match req.name.trim() {
        "" => kind.name.clone(),
        n => n.to_string(),
    };
    let repo = Git::new(core.exec(), &project_path).repo_root();

    let resume = req.resume_session_id.as_deref().filter(|s| !s.is_empty());
    // A separate worktree is the agent's own feature: Pitwall only passes its
    // flag and finds out afterwards where it works (worktree.rs).
    let wants_worktree = req.worktree && resume.is_none();
    let project = if wants_worktree {
        if kind.worktree_args.is_empty() {
            return Err(format!("{} can't make its own worktree.", kind.name));
        }
        repo.ok_or("A separate worktree needs a git repository.")?
    } else {
        match display_project(core, req.display_project.as_deref())? {
            Some(p) => p,
            None => repo.unwrap_or_else(|| project_path.clone()),
        }
    };
    let cwd = project_path;

    let mut rec = AgentRecord {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        kind: kind.id.clone(),
        kind_name: kind.name.clone(),
        custom_command: (kind.id == "custom").then(|| kind.command.clone()),
        base_commit: Git::new(core.exec(), &cwd).head(),
        cwd,
        project,
        worktree: None,
        worktree_pending: wants_worktree,
        session_id: None,
        has_conversation: false,
        queue: vec![],
        auto_send: true,
        last_sent: None,
        last_sent_at: None,
        created_at: unix_ms(),
        tasks: vec![],
        cols: None,
        rows: None,
    };
    let size = size_of(req.cols, req.rows);
    if let Some(size) = size {
        rec.set_term_size(size);
    }
    // Rules go in before the first launch (never fails creation; see rules).
    let dirs = rules::Dirs::of(core.paths());
    let exec = core.exec_for(&rec.id);
    rules::on_create(&dirs, core.kinds(), &*exec, &rec, &kind, req.rule_set_id.clone(), req.apply_to_main_checkout);
    let wt = rec.worktree_pending.then(|| watch_for(core, &kind, &rec, core.now()));
    let hook_command = hooks::hook_command(core.paths());
    let plan = launch::plan(&kind, resume, resume.is_some(), &hook_command, wt.as_ref().map(|(n, _)| n.as_str()));
    rec.session_id = plan.session_id;
    rec.has_conversation = plan.resumed;
    let session = spawn(core, &rec.id, &rec.cwd, plan.command_line, size)?;
    let mut agent = Agent::new(rec, Some(session), core.now());
    agent.watch = wt.map(|(_, w)| w);
    let view = agent.view();
    core.agents().push(agent);
    core.changed(true);
    Ok(view)
}

pub fn stop(core: &Shared, id: &str) -> Result<(), String> {
    if let Some(session) = core.with(id, |a| a.session.clone())? {
        std::thread::spawn(move || session.terminate(STOP_GRACE));
    }
    Ok(())
}

/// Stop (if running) and start again, resuming the session when possible.
/// `size` (cols, rows) is what the UI will show; else the last known size.
pub fn restart(core: &Shared, id: &str, size: Option<(u16, u16)>) -> Result<AgentView, String> {
    let old = core.with(id, |a| {
        if let Some(size) = size {
            a.rec.set_term_size(size);
        }
        a.session.clone()
    })?;
    if let Some(old) = old {
        old.terminate(STOP_GRACE);
    }
    // Read after the stop: resizes that arrived meanwhile are in the record.
    let rec = core.with(id, |a| a.rec.clone())?;
    let size = rec.term_size();
    let kind = kind_for(core, &rec.kind, rec.custom_command.as_deref())?;
    if !Path::new(&rec.cwd).is_dir() {
        return Err(format!("{} no longer exists", rec.cwd));
    }
    let resume = rec.has_conversation;
    // Still waiting for its own worktree: ask for it again on a fresh start.
    let wt = rec.worktree_pending.then(|| watch_for(core, &kind, &rec, core.now()));
    let hook_command = hooks::hook_command(core.paths());
    let plan = launch::plan(&kind, rec.session_id.as_deref(), resume, &hook_command, wt.as_ref().map(|(n, _)| n.as_str()));
    let session = spawn(core, &rec.id, &rec.cwd, plan.command_line, size)?;
    let now = core.now();
    let attached = core.with(id, |a| {
        if !plan.resumed {
            a.rec.has_conversation = false;
            a.rec.session_id = plan.session_id.clone();
        }
        a.attach_session(session.clone(), now);
        a.watch = wt.map(|(_, w)| w);
        a.rec.term_size()
    });
    // A resize that landed between spawning and attaching went to the record only.
    if let Ok(Some((cols, rows))) = attached {
        session.resize(cols, rows);
    }
    let view = core.with(id, |a| a.view());
    if view.is_err() {
        // Removed while we were starting it.
        session.terminate(STOP_GRACE);
    }
    core.changed(true);
    view
}

pub fn remove(core: &Shared, id: &str, delete_worktree: bool) -> Result<(), String> {
    let agent = {
        let mut agents = core.agents();
        let pos = agents
            .iter()
            .position(|a| a.rec.id == id)
            .ok_or_else(|| format!("no agent {id}"))?;
        agents.remove(pos)
    };
    core.changed(true);
    if let Some(session) = &agent.session {
        session.terminate(STOP_GRACE);
    }
    let exec = core.exec_for(id);
    tasks::forget(&*exec, &agent.rec);
    // Only when asked (the UI confirms first); never `--force`, never the branch.
    let res = match (&agent.rec.worktree, delete_worktree) {
        (Some(wt), true) => Git::new(&*exec, &wt.repo).worktree_remove(&wt.path)
            .map_err(|e| format!("Agent removed, but its worktree was kept: {e}")),
        _ => Ok(()),
    };
    // After the worktree is gone: drop generated rule files + exclude entries.
    rules::forget_agent(&rules::Dirs::of(core.paths()), &*exec, &agent.rec.id);
    res
}

/// "Quit and Stop Agents": the user explicitly ends every agent. A plain
/// quit never does this; agents keep running in their holders and the next
/// start re-attaches to them (architecture.md §9, decision 1).
pub fn stop_all(core: &Shared) {
    let sessions: Vec<_> = core.agents().iter().filter_map(|a| a.session.clone()).collect();
    std::thread::scope(|scope| {
        for s in &sessions {
            scope.spawn(move || s.terminate(STOP_GRACE));
        }
    });
}

/// Engine start: the terminal of an agent whose holder outlived the last run.
pub(crate) fn reattach(paths: &Paths, clock: &Arc<dyn Clock>, rec: &AgentRecord) -> Option<Arc<Session>> {
    let session = Session::reattach(&paths.hold_dir(), &rec.id, clock.clone())?;
    if let Some((cols, rows)) = rec.term_size() {
        session.resize(cols, rows);
    }
    Some(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_project_override() {
        let h = crate::testing::Harness::new(vec![]);
        let display_project = |p| display_project(&h.engine, p);
        assert_eq!(display_project(None).unwrap(), None);
        assert_eq!(display_project(Some("  ")).unwrap(), None);
        assert!(display_project(Some("/definitely/not/here")).is_err());
        let dir = std::env::temp_dir().join(format!("pitwall-display-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let got = display_project(Some(&format!("{}/", dir.display()))).unwrap().unwrap();
        assert!(!got.ends_with('/'));
        assert!(Path::new(&got).is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_checks_its_input_before_starting_anything() {
        let h = crate::testing::Harness::new(vec![]);
        let req = |kind: &str, path: &str, worktree: bool| CreateAgentRequest {
            name: String::new(),
            kind: kind.into(),
            project_path: path.into(),
            custom_command: None,
            worktree,
            resume_session_id: None,
            rule_set_id: None,
            apply_to_main_checkout: false,
            display_project: None,
            cols: None,
            rows: None,
        };
        let dir = h.dir.path().to_string_lossy().into_owned();
        assert!(create(&h.engine, req("shell", "/definitely/not/here", false)).unwrap_err().contains("is not a folder"));
        assert!(create(&h.engine, req("nope", &dir, false)).unwrap_err().contains("unknown agent kind"));
        assert_eq!(create(&h.engine, req("custom", &dir, false)).unwrap_err(), "custom command is empty");
        assert!(create(&h.engine, req("shell", &dir, true)).unwrap_err().contains("can't make its own worktree"));
        assert!(create(&h.engine, req("claude", &dir, true)).unwrap_err().contains("needs a git repository"));
        // Valid request, but the host gave no terminal holder: reported, nothing added.
        assert_eq!(create(&h.engine, req("shell", &dir, false)).unwrap_err(), "no terminal holder in tests");
        assert!(h.engine.views().is_empty());
        assert!(stop(&h.engine, "ghost").is_err());
        assert!(remove(&h.engine, "ghost", false).is_err());
    }
}
