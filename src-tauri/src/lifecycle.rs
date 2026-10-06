//! Creating, starting, stopping and removing agents. These do blocking work
//! (git, process spawn/teardown); call them off the main thread.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::agents::{self, AgentKind};
use crate::model::{unix_ms, AgentRecord, AgentView, CreateAgentRequest};
use crate::registry::{Agent, Shared};
use crate::session::{Session, Spawn};
use crate::{git, hooks, kinds, launch, paths, worktree};

const STOP_GRACE: Duration = Duration::from_millis(1500);

fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            paths::home().join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(p),
    }
}

/// Lowercase ASCII words joined by '-', safe for branch and directory names.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(40).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "agent".into() } else { out }
}

fn kind_for(rec_kind: &str, custom: Option<&str>) -> Result<AgentKind, String> {
    if rec_kind == "custom" {
        let cmd = custom.map(str::trim).filter(|c| !c.is_empty()).ok_or("custom command is empty")?;
        return Ok(agents::custom_kind(cmd));
    }
    kinds::find(rec_kind).ok_or_else(|| format!("unknown agent kind \"{rec_kind}\""))
}

/// Before a launch with the kind's worktree flag: the name to pass, and what
/// to compare against to spot the worktree the agent makes.
fn watch_for(kind: &AgentKind, rec: &AgentRecord) -> (String, worktree::Watch) {
    let named = kind.worktree_args.iter().any(|a| a.contains("{name}"));
    let base = slug(&rec.name);
    let name = if named { worktree::pick_name(&rec.project, &base) } else { base };
    let watch = worktree::Watch::new(worktree::roots(&rec.project), named.then(|| name.clone()));
    (name, watch)
}

/// The project an agent is shown under when it differs from where it runs.
fn display_project(p: Option<&str>) -> Result<Option<String>, String> {
    let Some(p) = p.map(str::trim).filter(|p| !p.is_empty()) else { return Ok(None) };
    let path = expand_tilde(p);
    if !path.is_dir() {
        return Err(format!("{} is not a folder", path.display()));
    }
    let path = crate::project_list::normalize(&path.to_string_lossy());
    Ok(Some(git::repo_root(&path).unwrap_or(path)))
}

pub fn create(core: &Shared, req: CreateAgentRequest) -> Result<AgentView, String> {
    let project_path = expand_tilde(req.project_path.trim());
    if !project_path.is_dir() {
        return Err(format!("{} is not a folder", project_path.display()));
    }
    let project_path = project_path.to_string_lossy().into_owned();
    let kind = kind_for(&req.kind, req.custom_command.as_deref())?;
    let name = match req.name.trim() {
        "" => kind.name.clone(),
        n => n.to_string(),
    };
    let repo = git::repo_root(&project_path);

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
        match display_project(req.display_project.as_deref())? {
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
        base_commit: git::head(&cwd),
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
    let size = crate::model::size_of(req.cols, req.rows);
    if let Some(size) = size {
        rec.set_term_size(size);
    }
    // Rules go in before the first launch (never fails creation; see rules.rs).
    crate::rules::on_create(&rec, &kind, req.rule_set_id.clone(), req.apply_to_main_checkout);
    let wt = rec.worktree_pending.then(|| watch_for(&kind, &rec));
    let plan = launch::plan(&kind, resume, resume.is_some(), &hooks::hook_command(), wt.as_ref().map(|(n, _)| n.as_str()));
    rec.session_id = plan.session_id;
    rec.has_conversation = plan.resumed;
    let session = Session::spawn(Spawn {
        agent_id: &rec.id,
        cwd: &rec.cwd,
        command_line: plan.command_line,
        size,
        hold_dir: &paths::hold_dir(),
    })?;
    let mut agent = Agent::new(rec, Some(session));
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
    let kind = kind_for(&rec.kind, rec.custom_command.as_deref())?;
    if !Path::new(&rec.cwd).is_dir() {
        return Err(format!("{} no longer exists", rec.cwd));
    }
    let resume = rec.has_conversation;
    // Still waiting for its own worktree: ask for it again on a fresh start.
    let wt = rec.worktree_pending.then(|| watch_for(&kind, &rec));
    let plan = launch::plan(&kind, rec.session_id.as_deref(), resume, &hooks::hook_command(), wt.as_ref().map(|(n, _)| n.as_str()));
    let session = Session::spawn(Spawn {
        agent_id: &rec.id,
        cwd: &rec.cwd,
        command_line: plan.command_line,
        size,
        hold_dir: &paths::hold_dir(),
    })?;
    let attached = core.with(id, |a| {
        if !plan.resumed {
            a.rec.has_conversation = false;
            a.rec.session_id = plan.session_id.clone();
        }
        a.attach_session(session.clone());
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
    crate::tasks::forget(&agent.rec);
    // Only when asked (the UI confirms first); never `--force`, never the branch.
    let res = match (&agent.rec.worktree, delete_worktree) {
        (Some(wt), true) => git::worktree_remove(&wt.repo, &wt.path)
            .map_err(|e| format!("Agent removed, but its worktree was kept: {e}")),
        _ => Ok(()),
    };
    // After the worktree is gone: drop generated rule files + exclude entries.
    crate::rules::forget_agent(&agent.rec.id);
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

/// App start: the terminal of every agent whose holder outlived the last run.
pub fn reattach(rec: &AgentRecord) -> Option<std::sync::Arc<Session>> {
    let session = Session::reattach(&paths::hold_dir(), &rec.id)?;
    if let Some((cols, rows)) = rec.term_size() {
        session.resize(cols, rows);
    }
    Some(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Fix the Login bug!"), "fix-the-login-bug");
        assert_eq!(slug("  ---  "), "agent");
        assert_eq!(slug("ünï"), "n");
        assert_eq!(slug("ü"), "agent");
        assert!(slug(&"ab ".repeat(50)).len() <= 40);
        assert!(!slug(&"ab ".repeat(50)).ends_with('-'));
    }

    #[test]
    fn display_project_override() {
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
    fn tilde_expansion() {
        assert_eq!(expand_tilde("~/x"), paths::home().join("x"));
        assert_eq!(expand_tilde("~"), paths::home());
        assert_eq!(expand_tilde("/a/~b"), PathBuf::from("/a/~b"));
    }
}
