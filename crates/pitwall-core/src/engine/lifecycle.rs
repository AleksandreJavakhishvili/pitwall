//! Creating, starting, stopping and removing agents, through their
//! provider. These do blocking work (git, starting and ending sessions);
//! call them off the main thread.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use super::{tasks, worktree, Agent, Engine, Shared};
use crate::clock::unix_ms;
use crate::exec::{self, Exec};
use crate::hooks;
use crate::kind::{self, AgentKind};
use crate::model::{size_of, AdoptSessionRequest, AgentRecord, AgentView, CreateAgentRequest, KindCaps};
use crate::onboarding::project_list;
use crate::paths::expand_tilde_in;
use crate::provider::{
    CreateSpec, HookTransport, HookWiring, LaunchIntent, LaunchSpec, Locator, Machine, MachineId, NativeState, Provider,
    ProviderId, Started, TermSize,
};
use crate::rules;
use crate::slug::slug;
use crate::term::TermHost;
use crate::vcs::git::Git;

pub(crate) const STOP_GRACE: Duration = Duration::from_millis(1500);

fn kind_for(engine: &Engine, rec_kind: &str, custom: Option<&str>) -> Result<AgentKind, String> {
    if rec_kind == kind::CUSTOM {
        let cmd = custom.map(str::trim).filter(|c| !c.is_empty()).ok_or("custom command is empty")?;
        return Ok(kind::custom_kind(cmd));
    }
    engine.kinds().find(rec_kind).ok_or_else(|| format!("unknown agent kind \"{rec_kind}\""))
}

/// The Race Engineer's launch: its kind with its per-launch flags, the
/// environment it needs and its own folder (crate::engineer); other agents
/// as they are.
type EngineerLaunch = (AgentKind, Vec<(String, String)>, Option<String>);

fn engineer_launch(engine: &Engine, engineer: bool, kind: AgentKind) -> Result<EngineerLaunch, String> {
    if !engineer {
        return Ok((kind, vec![], None));
    }
    let kit = engine.engineer_kit().ok_or("this Pitwall can't start the Race Engineer: its files aren't installed")?;
    let l = kit.launch(engine.paths(), &kind, crate::shell::spawn_path())?;
    Ok((l.kind, l.env, Some(l.cwd.to_string_lossy().into_owned())))
}

/// Before a launch with the kind's worktree flag: the name to pass, and what
/// to compare against to spot the worktree the agent makes.
fn watch_for(exec: &dyn Exec, kind: &AgentKind, rec: &AgentRecord, now: u64) -> (String, worktree::Watch) {
    let named = kind.worktree_args.iter().any(|a| a.contains("{name}"));
    let base = slug(&rec.name);
    let name = if named { worktree::pick_name(exec, &rec.project, &base) } else { base };
    let watch = worktree::Watch::new(worktree::roots(exec, &rec.project), named.then(|| name.clone()), now);
    (name, watch)
}

/// How hooks reach Pitwall from an agent of `p`, if they can.
fn hook_wiring(engine: &Engine, p: &dyn Provider) -> Option<HookWiring> {
    (p.caps().hooks != HookTransport::None).then(|| HookWiring { command: hooks::hook_command(engine.paths()) })
}

/// The terminal of a session `p` just started (its own, or by attaching).
fn host_of(engine: &Engine, p: &dyn Provider, started: &mut Started, size: TermSize) -> Result<Arc<TermHost>, String> {
    let io = match started.term.take() {
        Some(io) => io,
        None => p.attach(&started.locator, size)?,
    };
    Ok(engine.host_for(io, size))
}

/// The project an agent is shown under when it differs from where it runs.
fn display_project(exec: &dyn Exec, p: Option<&str>) -> Result<Option<String>, String> {
    let Some(p) = p.map(str::trim).filter(|p| !p.is_empty()) else { return Ok(None) };
    let path = expand_tilde_in(exec.home().ok().as_deref(), p);
    if !exec::is_dir(exec, &path) {
        return Err(format!("{path} is not a folder"));
    }
    let path = project_list::normalize(&path);
    Ok(Some(Git::new(exec, &path).repo_root().unwrap_or(path)))
}

/// Start a new agent where `req` says (default: where new agents go). A
/// machine whose form has no folder (a platform such as agw) makes it its
/// own way: [`create_on_platform`]. Blocking.
pub fn create(core: &Shared, req: CreateAgentRequest) -> Result<AgentView, String> {
    crate::exec::assert_off_ui("lifecycle::create");
    let (provider, machine) = core.providers().target(req.provider.as_deref(), req.machine.as_deref())?;
    if !provider.caps().create {
        return Err(format!("new agents can't be started on {}", machine.label));
    }
    let form = provider.create_form(&machine.id)?;
    let options = form.values(&req.options)?;
    if !form.folder {
        return create_on_platform(core, provider, &machine, &req, &options);
    }
    // Without commands on that machine there is nothing to check here: the
    // provider's own create reports a bad folder.
    let exec: Option<Arc<dyn Exec>> = if provider.caps().exec { Some(provider.exec(&machine.id)?) } else { None };
    let exec = exec.as_deref();
    let home = exec.and_then(|x| x.home().ok());
    let kind = kind_for(core, &req.kind, req.custom_command.as_deref())?;
    // The Race Engineer works in its own folder, which its launch prepares.
    let (kind, env, engineer_cwd) = engineer_launch(core, req.engineer, kind)?;
    let project_path = engineer_cwd.unwrap_or_else(|| expand_tilde_in(home.as_deref(), req.project_path.trim()));
    if exec.is_some_and(|x| !exec::is_dir(x, &project_path)) {
        return Err(format!("{project_path} is not a folder"));
    }
    let name = match req.name.trim() {
        "" if req.engineer => crate::engineer::NAME.to_string(),
        "" => kind.name.clone(),
        n => n.to_string(),
    };
    let repo = exec.and_then(|x| Git::new(x, &project_path).repo_root());

    let resume = req.resume_session_id.as_deref().filter(|s| !s.is_empty());
    // A separate worktree is the agent's own feature: Pitwall only passes its
    // flag and finds out afterwards where it works (worktree.rs).
    let wants_worktree = req.worktree && resume.is_none() && !req.engineer;
    let project = if wants_worktree {
        if !KindCaps::of(&kind, &provider.caps()).worktree {
            return Err(format!("{} can't make its own worktree.", kind.name));
        }
        repo.ok_or("A separate worktree needs a git repository.")?
    } else {
        let shown = match exec {
            Some(x) => display_project(x, req.display_project.as_deref())?,
            None => None,
        };
        match shown {
            Some(p) => p,
            None => repo.unwrap_or_else(|| project_path.clone()),
        }
    };
    let cwd = project_path;

    let mut rec = AgentRecord {
        id: uuid::Uuid::new_v4().to_string(),
        locator: None,
        name,
        kind: kind.id.clone(),
        kind_name: kind.name.clone(),
        custom_command: (kind.id == kind::CUSTOM).then(|| kind.command.clone()),
        base_commit: exec.and_then(|x| Git::new(x, &cwd).head()),
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
        adopted: false,
        inner_agent: None,
        engineer: req.engineer,
    };
    let size = size_of(req.cols, req.rows);
    if let Some(size) = size {
        rec.set_term_size(size);
    }
    // Rules go in before the first launch (never fails creation; see rules).
    // Not into the Race Engineer's folder: its instruction files are Pitwall's.
    let dirs = rules::Dirs::of(core.paths());
    if let Some(x) = exec.filter(|_| !rec.engineer) {
        rules::on_create(&dirs, core.kinds(), x, &rec, &kind, req.rule_set_id.clone(), req.apply_to_main_checkout);
    }
    let wt = match exec {
        Some(x) if rec.worktree_pending => Some(watch_for(x, &kind, &rec, core.now())),
        _ => None,
    };
    let size = TermSize::from_pair(size);
    let intent = match resume {
        Some(id) if provider.caps().resume => LaunchIntent::Resume(id.to_string()),
        _ => LaunchIntent::Fresh,
    };
    let mut started = provider.create(&CreateSpec {
        machine: &machine.id,
        name: &rec.name,
        options: &options,
        launch: LaunchSpec {
            agent: &rec.id,
            kind: &kind,
            cwd: &rec.cwd,
            intent,
            worktree: wt.as_ref().map(|(n, _)| n.as_str()),
            size,
            hooks: hook_wiring(core, &*provider),
            env: &env,
        },
    })?;
    rec.locator = Some(started.locator.clone());
    rec.session_id = started.conversation_id.clone();
    rec.has_conversation = started.resumed;
    let host = match host_of(core, &*provider, &mut started, size) {
        Ok(h) => h,
        Err(e) => {
            let _ = provider.stop(&started.locator);
            return Err(e);
        }
    };
    let loc = started.locator.clone();
    let mut agent = Agent::new(rec, Some(host), core.now());
    agent.facts = core.facts(&loc, &agent.rec);
    agent.watch = wt.map(|(_, w)| w);
    let view = agent.view();
    core.agents().push(agent);
    core.changed(true);
    Ok(view)
}

/// A new session on a platform that decides what runs and where (a form
/// without a folder, e.g. agw): the provider makes it from the form's
/// values, then Pitwall attaches to it as it does to an added session. Like
/// an added session it belongs to the platform: removing it from Pitwall,
/// or "Quit and Stop Agents", only ends the attachment (deleting it there is
/// not offered yet). The caller has checked `options` against the form.
fn create_on_platform(
    core: &Shared,
    provider: Arc<dyn Provider>,
    machine: &Machine,
    req: &CreateAgentRequest,
    options: &BTreeMap<String, String>,
) -> Result<AgentView, String> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err("a name is required".into());
    }
    let taken = core.agents().iter().any(|a| {
        let l = a.rec.locator();
        l.provider == *provider.id() && l.machine == machine.id && l.native == name
    });
    if taken {
        return Err(format!("Pitwall already has a session \"{name}\" on {}", machine.label));
    }
    let id = uuid::Uuid::new_v4().to_string();
    // The platform picks the program; the kind only matters to providers
    // that launch one themselves.
    let kind = core.kinds().find(&req.kind).or_else(|| core.kinds().find(super::terminals::TERMINAL_KIND)).unwrap_or_else(|| kind::custom_kind(""));
    let size = size_of(req.cols, req.rows);
    let term_size = TermSize::from_pair(size);
    let mut started = provider.create(&CreateSpec {
        machine: &machine.id,
        name,
        options,
        launch: LaunchSpec {
            agent: &id,
            kind: &kind,
            cwd: "",
            intent: LaunchIntent::Fresh,
            worktree: None,
            size: term_size,
            hooks: hook_wiring(core, &*provider),
            env: &[],
        },
    })?;
    let program = started.kind.clone().unwrap_or_else(|| kind.id.clone());
    let mut rec = session_record(core, id, started.locator.clone(), &program, started.cwd.clone(), name.to_string());
    if let Some(size) = size {
        rec.set_term_size(size);
    }
    let loc = started.locator.clone();
    let attached = host_of(core, &*provider, &mut started, term_size);
    let mut agent = Agent::new(rec, attached.as_ref().ok().cloned(), core.now());
    agent.facts = core.facts(&loc, &agent.rec);
    let view = agent.view();
    core.agents().push(agent);
    core.changed(true);
    match attached {
        Ok(_) => Ok(view),
        Err(e) => Err(format!(
            "\"{name}\" was created on {} and added to Pitwall, but Pitwall couldn't attach to it: {e}",
            machine.label
        )),
    }
}

/// The record of a platform's session Pitwall tracks without owning it
/// (`adopted`): what runs there (`program`, the platform's word) as one of
/// Pitwall's kinds, else a terminal — screen and activity status still work.
fn session_record(core: &Shared, id: String, loc: Locator, program: &str, cwd: String, name: String) -> AgentRecord {
    let (kind, kind_name) = match core.kinds().resolve(program) {
        Some(k) => (k.id, k.name),
        None => (super::terminals::TERMINAL_KIND.to_string(), if program.is_empty() { "Shell".into() } else { program.to_string() }),
    };
    AgentRecord {
        id,
        locator: Some(loc),
        name,
        kind,
        kind_name,
        custom_command: None,
        project: cwd.clone(),
        cwd,
        worktree: None,
        worktree_pending: false,
        base_commit: None,
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
        adopted: true,
        inner_agent: None,
        engineer: false,
    }
}

/// "Add to Pitwall": track a session that already exists on another machine
/// (one its provider's `discover` lists). Nothing changes there: Pitwall
/// attaches to it if it runs, and otherwise shows it stopped. Adopting the
/// same session again returns the agent Pitwall already has (§2.1: the
/// locator is unique). Blocking.
pub fn adopt(core: &Shared, req: AdoptSessionRequest) -> Result<AgentView, String> {
    let provider = core.providers().get(&ProviderId::new(&req.provider))?;
    let loc = Locator::new(provider.id(), &MachineId::new(&req.machine), &req.native);
    if let Some(v) = core.agents().iter().find(|a| a.rec.locator() == loc).map(Agent::view) {
        return Ok(v);
    }
    if !provider.caps().attach_existing {
        return Err(format!("sessions on {} can't be added to Pitwall", provider.label()));
    }
    let found = provider
        .discover(&loc.machine)?
        .into_iter()
        .find(|d| d.locator == loc)
        .ok_or_else(|| format!("{} has no session \"{}\"", core.providers().machine_label(&loc), loc.native))?;
    let name = found.title.clone().unwrap_or_else(|| loc.native.clone());
    let mut rec = session_record(core, uuid::Uuid::new_v4().to_string(), loc.clone(), &found.kind, found.cwd.clone().unwrap_or_default(), name);
    let size = size_of(req.cols, req.rows);
    if let Some(size) = size {
        rec.set_term_size(size);
    }
    let size = TermSize::from_pair(size);
    let host = match found.state {
        Some(NativeState::Running) => Some(core.host_for(provider.attach(&loc, size)?, size)),
        _ => None,
    };
    let mut agent = Agent::new(rec, host, core.now());
    agent.facts = core.facts(&loc, &agent.rec);
    let view = {
        let mut agents = core.agents();
        // Adopted meanwhile (two clicks): keep the first, drop this attachment.
        if let Some(v) = agents.iter().find(|a| a.rec.locator() == loc).map(Agent::view) {
            drop(agents);
            if let Some(h) = agent.host {
                h.close(STOP_GRACE);
            }
            return Ok(v);
        }
        let view = agent.view();
        agents.push(agent);
        view
    };
    core.changed(true);
    Ok(view)
}

/// End agent `loc`: closing its terminal ends a local agent; a remote one
/// is stopped through its provider and only the attachment closed.
fn end(p: Option<&Arc<dyn Provider>>, loc: &Locator, host: Option<&Arc<TermHost>>) {
    match host {
        Some(h) if h.eof_is_exit() => h.close(STOP_GRACE),
        _ => {
            if let Some(p) = p {
                if let Err(e) = p.stop(loc) {
                    eprintln!("pitwall: could not stop {loc}: {e}");
                }
            }
            if let Some(h) = host {
                h.close(STOP_GRACE);
            }
        }
    }
}

pub fn stop(core: &Shared, id: &str) -> Result<(), String> {
    let (loc, host) = core.with(id, |a| (a.rec.locator(), a.host.clone()))?;
    let p = core.providers().for_locator(&loc).ok();
    std::thread::spawn(move || end(p.as_ref(), &loc, host.as_ref()));
    Ok(())
}

/// Stop (if running) and start again, resuming the session when possible.
/// `size` (cols, rows) is what the UI will show; else the last known size.
pub fn restart(core: &Shared, id: &str, size: Option<(u16, u16)>) -> Result<AgentView, String> {
    crate::exec::assert_off_ui("lifecycle::restart");
    // It may still attach to the session it has: not two of them. A pending
    // retry (machine unreachable) is called off, and taken up again if this
    // fails.
    let cancelled = core.with(id, |a| match &a.connect {
        Some(c) if c.busy() => Err("still connecting to its session; try again in a moment".to_string()),
        Some(_) => Ok(a.connect.take().is_some()),
        None => Ok(false),
    })??;
    let res = restart_now(core, id, size);
    if res.is_err() && cancelled {
        super::connect::retry_later(core, id);
    }
    res
}

fn restart_now(core: &Shared, id: &str, size: Option<(u16, u16)>) -> Result<AgentView, String> {
    let (loc, old) = core.with(id, |a| {
        if let Some(size) = size {
            a.rec.set_term_size(size);
        }
        (a.rec.locator(), a.host.clone())
    })?;
    let provider = core.providers().for_locator(&loc)?;
    if old.as_ref().is_some_and(|h| !h.ended()) {
        end(Some(&provider), &loc, old.as_ref());
    }
    // Read after the stop: resizes that arrived meanwhile are in the record.
    let rec = core.with(id, |a| a.rec.clone())?;
    let size = TermSize::from_pair(rec.term_size());
    let kind = kind_for(core, &rec.kind, rec.custom_command.as_deref())?;
    let (kind, env, _) = engineer_launch(core, rec.engineer, kind)?;
    let caps = provider.caps();
    let exec = if caps.exec { Some(provider.exec_at(&loc)?) } else { None };
    if exec.as_deref().is_some_and(|x| !exec::is_dir(x, &rec.cwd)) {
        return Err(format!("{} no longer exists", rec.cwd));
    }
    let intent = match &rec.session_id {
        Some(id) if rec.has_conversation && caps.resume => LaunchIntent::Resume(id.clone()),
        _ => LaunchIntent::Fresh,
    };
    let hooks = hook_wiring(core, &*provider);
    // A terminal that last ran an agent the user started in it: the shell
    // starts and continues that agent inside it (docs/spec/terminals.md §3).
    let inner = match &rec.inner_agent {
        Some(m) if caps.custom_command && rec.kind == super::terminals::TERMINAL_KIND => core.kinds().find(&m.kind).map(|agent| {
            let hook = hooks.as_ref().map_or("", |h| h.command.as_str());
            kind::launch::then_shell(&kind, &agent, m.session_id.as_deref(), caps.resume, hook)
        }),
        _ => None,
    };
    let (kind, inner_plan) = match inner {
        Some((k, p)) => (k, Some(p)),
        None => (kind, None),
    };
    // Still waiting for its own worktree: ask for it again on a fresh start.
    let wt = match exec.as_deref() {
        Some(x) if rec.worktree_pending => Some(watch_for(x, &kind, &rec, core.now())),
        _ => None,
    };
    let launch = LaunchSpec {
        agent: &rec.id,
        kind: &kind,
        cwd: &rec.cwd,
        intent: if inner_plan.is_some() { LaunchIntent::Fresh } else { intent },
        worktree: wt.as_ref().map(|(n, _)| n.as_str()),
        size,
        hooks,
        env: &env,
    };
    let mut started = provider.start(&loc, &launch)?;
    let host = host_of(core, &*provider, &mut started, size)?;
    let now = core.now();
    let facts = core.facts(&started.locator, &rec);
    let attached = core.with(id, |a| {
        if !started.resumed {
            a.rec.has_conversation = false;
            a.rec.session_id = started.conversation_id.clone();
        }
        a.rec.locator = Some(started.locator.clone());
        if let (Some(p), Some(m)) = (&inner_plan, &mut a.rec.inner_agent) {
            // Running again (the poller notices its new process).
            m.session_id = p.session_id.clone().or(m.session_id.take());
            m.pid = None;
            m.left_at = None;
        }
        a.attach_host(host.clone(), now);
        a.facts = facts;
        a.watch = wt.map(|(_, w)| w);
        a.rec.term_size()
    });
    // A resize that landed between starting and attaching went to the record only.
    if let Ok(Some((cols, rows))) = attached {
        host.resize(cols, rows);
    }
    let view = core.with(id, |a| a.view());
    if view.is_err() {
        // Removed while we were starting it.
        end(Some(&provider), &started.locator, Some(&host));
    }
    core.changed(true);
    view
}

pub fn remove(core: &Shared, id: &str, delete_worktree: bool) -> Result<(), String> {
    crate::exec::assert_off_ui("lifecycle::remove");
    let exec = core.exec_for(id);
    let agent = {
        let mut agents = core.agents();
        let pos = agents
            .iter()
            .position(|a| a.rec.id == id)
            .ok_or_else(|| format!("no agent {id}"))?;
        agents.remove(pos)
    };
    core.changed(true);
    let loc = agent.rec.locator();
    let provider = core.providers().for_locator(&loc).ok();
    match (&agent.host, agent.rec.adopted) {
        // Pitwall only stops tracking an adopted session: it keeps running.
        (Some(h), true) if !h.eof_is_exit() => h.close(STOP_GRACE),
        (_, true) => {}
        _ => end(provider.as_ref(), &loc, agent.host.as_ref()),
    }
    if let Some(p) = &provider {
        if let Err(e) = p.remove(&loc) {
            eprintln!("pitwall: could not remove {loc}: {e}");
        }
    }
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

/// "Quit and Stop Agents": the user explicitly ends every agent Pitwall
/// started. A plain quit never does this; agents keep running (local: in
/// their holders) and the next start re-attaches to them (architecture.md
/// §9, decision 1). Adopted sessions were there before Pitwall: only their
/// attachment is closed.
pub fn stop_all(core: &Shared) {
    let running: Vec<_> = core
        .agents()
        .iter()
        .filter_map(|a| {
            let h = a.host.clone()?;
            let loc = a.rec.locator();
            let p = if a.rec.adopted { None } else { core.providers().for_locator(&loc).ok() };
            Some((p, loc, h))
        })
        .collect();
    std::thread::scope(|scope| {
        for (p, loc, h) in &running {
            scope.spawn(move || match p {
                Some(p) => end(Some(p), loc, Some(h)),
                None if !h.eof_is_exit() => h.close(STOP_GRACE),
                None => {}
            });
        }
    });
}

/// A dropped attachment (`eof_is_exit` false): attach again while the
/// provider says the session runs; else the agent has ended. Blocking.
/// When its machine can't be asked, it is tried again later (connect.rs).
pub(crate) fn relink(core: &Shared, id: &str) {
    crate::exec::assert_off_ui("lifecycle::relink");
    let Ok((loc, p)) = core.provider_of(id) else { return };
    let size = core.with(id, |a| TermSize::from_pair(a.rec.term_size())).unwrap_or(TermSize::DEFAULT);
    let unreachable = |e: &crate::error::PwError| matches!(e.code, crate::error::ErrorCode::Unreachable | crate::error::ErrorCode::Other);
    let attached = match p.state(&loc) {
        Ok(crate::provider::NativeState::Running) => p.attach(&loc, size).map(Some),
        Ok(_) => Ok(None),
        Err(e) => Err(e),
    };
    let host = match attached {
        Ok(io) => io.map(|io| core.host_for(io, size)),
        Err(e) if unreachable(&e) => {
            let _ = core.with(id, |a| a.relinking = false);
            super::connect::retry_later(core, id);
            return;
        }
        Err(_) => None,
    };
    let _ = core.with(id, |a| {
        a.relinking = false;
        match host {
            Some(h) => a.host = Some(h),
            None => {
                a.status = crate::model::Status::Exited;
                a.detail = None;
                a.host = None;
            }
        }
    });
    core.changed(false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::PwError;
    use crate::exec::LocalExec;
    use crate::model::Status;
    use crate::provider::{NativeState, ProviderCaps};
    use crate::testing::{Harness, TempDir};
    use std::time::Instant;

    fn req(kind: &str, path: &str, worktree: bool) -> CreateAgentRequest {
        CreateAgentRequest {
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
            ..Default::default()
        }
    }

    fn wait_until(f: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !f() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        f()
    }

    #[test]
    fn display_project_override() {
        let display_project = |p| display_project(&LocalExec, p);
        assert_eq!(display_project(None).unwrap(), None);
        assert_eq!(display_project(Some("  ")).unwrap(), None);
        assert!(display_project(Some("/definitely/not/here")).is_err());
        let dir = TempDir::new("display");
        let got = display_project(Some(&format!("{}/", dir.path().display()))).unwrap().unwrap();
        assert!(!got.ends_with('/'));
        assert!(std::path::Path::new(&got).is_dir());
    }

    #[test]
    fn create_checks_its_input_before_starting_anything() {
        let h = Harness::new(vec![]);
        let dir = h.dir.path().to_string_lossy().into_owned();
        assert!(create(&h.engine, req("shell", "/definitely/not/here", false)).unwrap_err().contains("is not a folder"));
        assert!(create(&h.engine, req("nope", &dir, false)).unwrap_err().contains("unknown agent kind"));
        assert_eq!(create(&h.engine, req("custom", &dir, false)).unwrap_err(), "custom command is empty");
        assert!(create(&h.engine, req("shell", &dir, true)).unwrap_err().contains("can't make its own worktree"));
        assert!(create(&h.engine, req("claude", &dir, true)).unwrap_err().contains("needs a git repository"));
        // The provider refuses: reported, nothing added.
        h.provider.fail_next(PwError::unreachable("machine is asleep"));
        assert_eq!(create(&h.engine, req("shell", &dir, false)).unwrap_err(), "machine is asleep");
        assert!(h.engine.views().is_empty());
        assert!(h.provider.launched().is_empty());
        assert!(stop(&h.engine, "ghost").is_err());
        assert!(remove(&h.engine, "ghost", false).is_err());
    }

    #[test]
    fn agents_are_started_stopped_and_resumed_through_their_provider() {
        let h = Harness::new(vec![]);
        let dir = h.dir.path().to_string_lossy().into_owned();
        let mut r = req("claude", &dir, false);
        r.cols = Some(90);
        r.rows = Some(30);
        let v = create(&h.engine, r).unwrap();
        let rec = h.engine.records().remove(0);
        let loc = rec.locator.clone().expect("locator stored");
        assert_eq!((loc.provider.as_str(), loc.machine.as_str(), loc.native.as_str()), ("local", "this-mac", v.id.as_str()));
        assert_eq!((v.location.as_str(), v.machine.label.as_str()), ("local", "Fake this-mac"));
        let l = h.provider.launched().remove(0);
        assert_eq!((l.cwd.as_str(), l.size, l.resumed), (dir.as_str(), TermSize::new(90, 30), false));
        let sid = rec.session_id.clone().expect("claude gets its session id up front");
        assert!(l.command_line.starts_with(&format!("claude --session-id {sid} --settings ")), "hooks wired: {}", l.command_line);
        assert!(v.running && v.caps.input && v.caps.stop && v.caps.restart);
        assert!(!v.caps.resume, "nothing to resume before a prompt");

        // Input reaches the agent's terminal.
        crate::engine::input::write_input(&h.engine, &v.id, "echo hi\r").unwrap();
        let ctl = h.provider.ctl(&v.id).unwrap();
        assert!(wait_until(|| ctl.input() == b"echo hi\r"), "input is written in order, off the caller's thread");
        assert!(h.engine.views()[0].caps.resume, "after a prompt it can resume");

        stop(&h.engine, &v.id).unwrap();
        assert!(wait_until(|| !ctl.running()));
        assert!(wait_until(|| !h.engine.views()[0].running));
        assert_eq!(h.provider.state(&loc).unwrap(), NativeState::Stopped);

        let v2 = restart(&h.engine, &v.id, Some((100, 40))).unwrap();
        let l = h.provider.launched().pop().unwrap();
        assert!(l.resumed && l.command_line.starts_with(&format!("claude --resume {sid}")));
        assert_eq!(l.size, TermSize::new(100, 40));
        assert!(v2.running);
        assert_eq!(h.engine.records()[0].session_id.as_deref(), Some(sid.as_str()));

        remove(&h.engine, &v.id, false).unwrap();
        assert_eq!(h.provider.state(&loc).unwrap(), NativeState::Gone);
        assert!(h.engine.views().is_empty());
    }

    /// The Race Engineer: its own folder, its know-how and the CLI on PATH
    /// on every launch; nothing without the app's files.
    #[test]
    fn the_race_engineer_is_launched_with_its_kit() {
        let h = Harness::new(vec![]);
        let project = h.dir.path().to_string_lossy().into_owned();
        let mut r = req("claude", &project, false);
        r.engineer = true;
        assert!(create(&h.engine, r.clone()).unwrap_err().contains("files aren't installed"));
        assert!(h.provider.launched().is_empty());

        let skills = h.dir.path().join("skills");
        std::fs::create_dir_all(skills.join("pitwall")).unwrap();
        std::fs::create_dir_all(skills.join("race-engineer")).unwrap();
        std::fs::write(skills.join(crate::engineer::SKILL), "# skill\n").unwrap();
        std::fs::write(skills.join(crate::engineer::PERSONA), "# persona\n").unwrap();
        let cli = h.dir.path().join("pitwall-cli");
        std::fs::write(&cli, "").unwrap();
        h.engine.set_engineer_kit(Some(crate::engineer::Kit { skills, cli: Some(cli) }));

        r.display_project = Some(project.clone());
        let v = create(&h.engine, r).unwrap();
        assert!(v.engineer);
        assert_eq!(v.name, "Race Engineer");
        let workdir = crate::engineer::workdir(h.engine.paths());
        assert_eq!(v.cwd, workdir.to_string_lossy());
        assert!(workdir.join("CLAUDE.md").is_file() && workdir.join("AGENTS.md").is_file());
        let l = h.provider.launched().remove(0);
        assert_eq!(l.cwd, workdir.to_string_lossy());
        assert!(l.command_line.starts_with("claude --session-id ") && l.command_line.contains(" --append-system-prompt "), "{}", l.command_line);
        let bin = crate::engineer::cli_dir(h.engine.paths()).to_string_lossy().into_owned();
        assert!(l.env.iter().any(|(k, v)| k == "PATH" && std::env::split_paths(v).next().is_some_and(|p| p.to_string_lossy() == bin)), "{:?}", l.env);
        let rec = h.engine.records().remove(0);
        assert!(rec.engineer);

        // A restart launches it the same way again.
        restart(&h.engine, &v.id, None).unwrap();
        let again = h.provider.launched().pop().unwrap();
        assert!(again.command_line.contains(" --append-system-prompt ") && again.env == l.env, "{}", again.command_line);

        // Other agents get none of it.
        create(&h.engine, req("claude", &project, false)).unwrap();
        let plain = h.provider.launched().pop().unwrap();
        assert!(!plain.command_line.contains("--append-system-prompt") && plain.env.is_empty());
    }

    #[test]
    fn caps_follow_the_provider_and_the_workspace() {
        let h = Harness::new(vec![]);
        let dir = h.dir.path().to_string_lossy().into_owned();
        let v = create(&h.engine, req("claude", &dir, false)).unwrap();
        assert!(v.caps.diff && v.caps.review, "not known yet: offered");
        // Not a git repository: no diffs, and no more git polling.
        assert!(crate::engine::changes::changes(&h.engine, &v.id).is_err());
        let v = &h.engine.views()[0];
        assert!(!v.caps.diff && !v.caps.review && !v.caps.merge);
        assert!(v.caps.rules && v.caps.hooks);

        // A provider without resume/rules/hooks/exec.
        h.provider.set_caps(ProviderCaps { create: true, survives_detach: true, ..Default::default() });
        let kinds = h.engine.list_kinds().unwrap();
        let claude = kinds.iter().find(|k| k.id == "claude").unwrap();
        assert!(!claude.caps.worktree && !claude.caps.resume && !claude.caps.rules && !claude.caps.hooks);
        assert!(!kinds.iter().any(|k| k.caps.custom_command), "no custom commands on this provider");
        let v = create(&h.engine, req("claude", &dir, false)).unwrap();
        assert!(!v.caps.diff && !v.caps.rules && !v.caps.hooks && !v.caps.remove_worktree);
        assert!(!h.provider.launched().pop().unwrap().command_line.contains("--settings"), "no hooks to wire");
    }

    #[test]
    fn kinds_carry_their_caps() {
        let h = Harness::new(vec![]);
        let kinds = h.engine.list_kinds().unwrap();
        let by = |id: &str| kinds.iter().find(|k| k.id == id).unwrap().clone();
        let (claude, shell, custom) = (by("claude"), by("shell"), by("custom"));
        assert!(claude.caps.worktree && claude.worktree && claude.caps.resume && claude.caps.rules && claude.caps.hooks);
        assert!(!shell.caps.worktree && !shell.caps.resume && !shell.caps.rules && !shell.caps.hooks);
        assert!(custom.caps.custom_command && !claude.caps.custom_command);
    }

    #[test]
    fn saved_agents_reattach_when_their_session_survived() {
        let h = Harness::new(vec![]);
        let dir = h.dir.path().to_string_lossy().into_owned();
        let v = create(&h.engine, req("shell", &dir, false)).unwrap();
        let w = create(&h.engine, req("shell", &dir, false)).unwrap();
        h.provider.ctl(&w.id).unwrap().exit(0);
        h.engine.save().unwrap();
        // Pitwall restarts: the same provider still has the sessions.
        let again = crate::engine::Engine::open(crate::engine::Deps {
            paths: crate::paths::Paths::new(h.dir.path().join("pitwall")),
            events: h.events.clone(),
            clock: h.clock.clone(),
            store: h.store.clone(),
            providers: vec![h.provider.clone()],
        });
        assert!(again.wait_connected(Duration::from_secs(5)), "re-attached in the background");
        let views = again.views();
        let find = |id: &str| views.iter().find(|x| x.id == id).unwrap();
        assert!(find(&v.id).running, "re-attached, no restart");
        assert_eq!(find(&w.id).status, Status::Stopped);
        assert_eq!(h.provider.launched().len(), 2, "nothing was started again");
    }

    /// A platform's machine (agw-like): its form decides the options, the
    /// provider makes the session by name, Pitwall attaches; it belongs to
    /// the platform, so removing it only detaches.
    #[test]
    fn a_platform_session_is_created_from_its_form_then_attached() {
        use crate::engine::{Deps, Engine};
        use crate::provider::{CreateChoice, CreateField, CreateForm};
        use crate::testing::{FakeProvider, ManualClock, MemStore, RecordingSink};
        let dir = TempDir::new("platform");
        let local = FakeProvider::named("local", "this-mac", Arc::new(LocalExec));
        let vm = FakeProvider::named("vmhost", "box", Arc::new(LocalExec));
        vm.set_caps(ProviderCaps { create: true, start: true, attach_existing: true, survives_detach: true, ..Default::default() });
        vm.set_remote(true);
        let pick = |v: &str| CreateChoice { value: v.into(), label: v.into(), detail: None, phrase: Some(format!("in {v}")), creates: None };
        vm.set_form(CreateForm {
            folder: false,
            fields: vec![
                CreateField::select("workspace", "Workspace", vec![pick("work"), pick("api-session")], Some("work".into())),
                CreateField::select("template", "Template", vec![pick("claude-code"), pick("login-shell")], Some("login-shell".into())),
            ],
            submit: "Create".into(),
            ..CreateForm::folder("", "", "")
        });
        let engine = Engine::open(Deps {
            paths: crate::paths::Paths::new(dir.path().join("pitwall")),
            events: RecordingSink::new(),
            clock: ManualClock::new(1),
            store: MemStore::with(vec![]),
            providers: vec![local.clone(), vm.clone()],
        });
        let form = engine.create_form(None, Some("box")).unwrap();
        assert_eq!((form.provider.as_str(), form.machine.as_str(), form.machine_label.as_str(), form.folder), ("vmhost", "box", "Fake box", false));
        assert!(engine.create_form(Some("local"), None).unwrap().folder, "this Mac: Pitwall's own form");
        assert!(engine.create_form(None, Some("nowhere")).unwrap_err().contains("no machine \"nowhere\""));

        let req = |name: &str, opts: &[(&str, &str)]| CreateAgentRequest {
            name: name.into(),
            machine: Some("box".into()),
            options: opts.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            cols: Some(90),
            rows: Some(30),
            ..Default::default()
        };
        // Options are checked against the form before anything is made.
        assert!(create(&engine, req("x", &[("workspace", "nope")])).unwrap_err().contains("isn't a choice for Workspace"));
        assert!(create(&engine, req("x", &[("color", "red")])).unwrap_err().contains("unknown option"));
        assert!(create(&engine, req(" ", &[])).unwrap_err().contains("name is required"));
        assert!(vm.created().is_empty());

        let v = create(&engine, req("api-fix", &[("template", "claude-code")])).unwrap();
        let (name, opts) = vm.created().remove(0);
        assert_eq!(name, "api-fix");
        assert_eq!(opts.into_iter().collect::<Vec<_>>(), [("template".into(), "claude-code".into()), ("workspace".into(), "work".into())], "defaults filled in");
        assert_eq!((v.name.as_str(), v.kind.as_str(), v.cwd.as_str()), ("api-fix", "claude", "/srv/work"), "kind from what the platform runs");
        assert_eq!((v.machine.provider.as_str(), v.machine.id.as_str()), ("vmhost", "box"));
        assert!(v.running && v.caps.input && v.caps.remove_keeps_session);
        assert!(!v.machine.can_create, "no \"new agent in this folder\" on a platform's machine");
        let rec = engine.records().into_iter().find(|r| r.id == v.id).unwrap();
        assert_eq!(rec.locator.as_ref().unwrap().to_string(), "vmhost:box/api-fix");
        assert_eq!(rec.term_size(), Some((90, 30)));
        let ctl = vm.ctl("api-fix").unwrap();
        crate::engine::input::write_input(&engine, &v.id, "hi\r").unwrap();
        assert!(wait_until(|| ctl.input() == b"hi\r"), "attached: input reaches the session");

        // The same name again: Pitwall already has it.
        assert!(create(&engine, req("api-fix", &[])).unwrap_err().contains("already has a session \"api-fix\""));
        // The platform refuses: its message, nothing added.
        let other = create(&engine, req("other", &[]));
        assert!(other.is_ok());
        vm.fail_next(PwError::other("agw couldn't create \"third\": VM is stopped"));
        assert_eq!(create(&engine, req("third", &[])).unwrap_err(), "agw couldn't create \"third\": VM is stopped");
        assert_eq!(engine.views().len(), 2);

        // Quitting with "stop agents" or removing it leaves it running there.
        stop_all(&engine);
        assert!(ctl.running(), "Quit and Stop Agents only detaches");
        remove(&engine, &v.id, false).unwrap();
        assert!(ctl.running(), "removing it from Pitwall only detaches");
        assert_eq!(vm.state(&Locator::new(&ProviderId::new("vmhost"), &MachineId::new("box"), "api-fix")).unwrap(), NativeState::Running);
        // This Mac's form takes no platform options.
        let mut mac = req("m", &[("workspace", "work")]);
        mac.machine = Some("this-mac".into());
        mac.kind = "shell".into();
        mac.project_path = dir.path().to_string_lossy().into_owned();
        assert!(create(&engine, mac).unwrap_err().contains("takes no options"));
    }

    #[test]
    fn a_dropped_attachment_is_relinked_while_the_session_runs() {
        let h = Harness::new(vec![]);
        h.provider.set_remote(true);
        let dir = h.dir.path().to_string_lossy().into_owned();
        let v = create(&h.engine, req("shell", &dir, false)).unwrap();
        let ctl = h.provider.ctl(&v.id).unwrap();
        let first = h.engine.host(&v.id).unwrap();
        assert!(!first.eof_is_exit());
        ctl.drop_links();
        assert!(wait_until(|| first.ended()));
        relink(&h.engine, &v.id);
        let second = h.engine.host(&v.id).unwrap();
        assert!(!Arc::ptr_eq(&first, &second) && !second.ended());
        assert!(h.engine.views()[0].running);
        // The session itself ended: the agent has exited.
        ctl.exit(0);
        assert!(wait_until(|| second.ended()));
        relink(&h.engine, &v.id);
        let v = &h.engine.views()[0];
        assert_eq!((v.status, v.running), (Status::Exited, false));
    }
}
