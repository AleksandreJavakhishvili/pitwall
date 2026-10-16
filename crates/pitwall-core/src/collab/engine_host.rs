//! Shared sessions as ordinary engine agents (§6.1): [`EngineHost`] runs
//! each one through the engine (`lifecycle::launch_shared`, with
//! `AgentRecord.shared`), types messages in like any prompt, and turns what
//! the engine already knows (status from hooks, screen rules or activity;
//! hook payloads; the screen) into [`SessionEvent`]s. It reads replies by
//! the kind's tier (§7.2): (a) hooks, else (c) the turn's terminal output.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::session::{
    limit_on_screen, reply_from_hook, reply_from_screen, share_kind, HostLaunch, HostSink, OwnerAgents, SessionEvent, SessionHost,
    SharedAgent,
};
use super::session_file::{Found, SessionFileReader};
use super::turn::Reply;
use pitwall_proto::collab::ReplySource;
use crate::engine::{input, lifecycle, Shared};
use crate::error::{PwError, Result};
use crate::kind::ReplyTier;
use crate::kind::AgentKind;
use crate::model::Status;

/// How often the engine's view of shared sessions is read.
const POLL: Duration = Duration::from_millis(250);
/// After the turn ends by status, how long to wait for a hook's reply.
const HOOK_GRACE: Duration = Duration::from_secs(2);
/// … and for the turn's end in the kind's session file.
const FILE_GRACE: Duration = Duration::from_secs(5);

struct Watch {
    kind: AgentKind,
    ready: bool,
    /// A message was sent and its turn hasn't ended.
    open: bool,
    /// The agent was seen working since the message.
    worked: bool,
    /// The screen when the message was sent (tier c), and the message.
    before: String,
    sent: String,
    /// The turn's end by status, waiting for a hook's reply.
    idle_since: Option<Instant>,
    hook_reply: Option<Reply>,
    blocked: bool,
    limited: Option<Option<String>>,
    exited: bool,
    /// Being (re)started: its old state isn't an exit.
    launching: bool,
    /// Tier (a) from the kind's own session files (Codex).
    files: Option<SessionFileReader>,
    /// The conversation a resumed launch continues (its file is older).
    resume_id: Option<String>,
    /// The id the files gave, once told to the core.
    told_id: Option<String>,
}

impl Watch {
    /// New lines of the session file: the id (once) and a turn's end.
    fn read_files(&mut self) -> (Vec<SessionEvent>, Option<Option<Reply>>) {
        let Some(r) = self.files.as_mut() else { return (Vec::new(), None) };
        let mut events = Vec::new();
        let mut ended = None;
        for f in r.poll_for(self.resume_id.as_deref()) {
            match f {
                Found::SessionId(id) => {
                    if self.told_id.as_deref() != Some(id.as_str()) {
                        self.told_id = Some(id.clone());
                        events.push(SessionEvent::SessionId(id));
                    }
                }
                Found::TurnEnded(text) => ended = Some(text.map(|text| Reply { text, source: ReplySource::SessionFile })),
            }
        }
        (events, ended)
    }
}

/// The engine as the place shared sessions run.
pub struct EngineHost {
    engine: Shared,
    sink: RwLock<Option<HostSink>>,
    watches: Mutex<HashMap<String, Watch>>,
}

fn other(e: String) -> PwError {
    PwError::other(e)
}

impl EngineHost {
    /// Taps the engine's hooks and polls its view of shared sessions in the
    /// background (the engine's own ticker must run: `Engine::start`).
    pub fn new(engine: Shared) -> Arc<EngineHost> {
        let host = Arc::new(EngineHost { engine: engine.clone(), sink: RwLock::default(), watches: Mutex::default() });
        let weak: Weak<EngineHost> = Arc::downgrade(&host);
        let tap = weak.clone();
        engine.tap_hooks(Box::new(move |id, payload| {
            if let Some(h) = tap.upgrade() {
                h.on_hook(id, payload);
            }
        }));
        std::thread::spawn(move || loop {
            std::thread::sleep(POLL);
            match weak.upgrade() {
                Some(h) => h.poll(),
                None => return,
            }
        });
        host
    }

    fn watches(&self) -> MutexGuard<'_, HashMap<String, Watch>> {
        self.watches.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, agent: &str, event: SessionEvent) {
        let sink = self.sink.read().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(sink) = sink {
            sink(agent, event);
        }
    }

    fn screen(&self, agent: &str) -> String {
        self.engine.host(agent).map(|h| h.screen_text().0).unwrap_or_default()
    }

    fn on_hook(&self, agent: &str, payload: &Value) {
        let event = payload.get("hook_event_name").and_then(Value::as_str).unwrap_or("");
        let mut out = Vec::new();
        {
            let mut w = self.watches();
            let Some(watch) = w.get_mut(agent) else { return };
            if let Some(id) = payload.get("session_id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                if event == "SessionStart" {
                    out.push(SessionEvent::SessionId(id.to_string()));
                }
            }
            match event {
                "UserPromptSubmit" | "PreToolUse" | "PostToolUse" if watch.open => {
                    watch.worked = true;
                    if watch.blocked {
                        watch.blocked = false;
                        out.push(SessionEvent::Working);
                    }
                }
                "Stop" if watch.open => {
                    watch.hook_reply = reply_from_hook(payload);
                    watch.worked = true;
                    if watch.hook_reply.is_some() {
                        watch.open = false;
                        out.push(SessionEvent::TurnEnded(watch.hook_reply.take()));
                    }
                }
                _ => {}
            }
        }
        for e in out {
            self.emit(agent, e);
        }
    }

    /// One look at every shared session the engine runs.
    fn poll(&self) {
        let views: HashMap<String, (Status, Option<String>, bool)> =
            self.engine.views().into_iter().map(|v| (v.id, (v.status, v.status_detail, v.running))).collect();
        let ids: Vec<String> = self.watches().keys().cloned().collect();
        for id in ids {
            let screen = self.screen(&id);
            let mut out = Vec::new();
            {
                let mut w = self.watches();
                let Some(watch) = w.get_mut(&id) else { continue };
                let Some((status, detail, running)) = views.get(&id).cloned() else { continue };
                if watch.launching {
                    continue;
                }
                if !running || matches!(status, Status::Exited | Status::Stopped) {
                    if !watch.exited {
                        watch.exited = true;
                        watch.open = false;
                        out.push(SessionEvent::Exited);
                    }
                } else {
                    // The agent's own usage limit, as its screen shows it (§5.8).
                    match (limit_on_screen(&share_kind(&watch.kind), &screen), &watch.limited) {
                        (Some(until), None) => {
                            watch.limited = Some(until.clone());
                            out.push(SessionEvent::Limited { until });
                        }
                        (None, Some(_)) => {
                            watch.limited = None;
                            out.push(SessionEvent::LimitCleared);
                        }
                        _ => {}
                    }
                    // Tier (a), session files: the turn's end and its reply.
                    if watch.files.is_some() {
                        let (events, ended) = watch.read_files();
                        out.extend(events);
                        if let (true, Some(reply)) = (watch.open, ended) {
                            watch.open = false;
                            watch.idle_since = None;
                            let reply = reply.or_else(|| reply_from_screen(&watch.before, &screen, &watch.sent));
                            out.push(SessionEvent::TurnEnded(reply));
                        }
                    }
                    match status {
                        Status::Idle | Status::Done if !watch.ready => {
                            watch.ready = true;
                            out.push(SessionEvent::Ready);
                        }
                        Status::Working => {
                            watch.worked |= watch.open;
                            if watch.blocked {
                                watch.blocked = false;
                                out.push(SessionEvent::Working);
                            }
                        }
                        Status::Blocked if watch.open && !watch.blocked => {
                            watch.blocked = true;
                            out.push(SessionEvent::Blocked { detail });
                        }
                        Status::Idle | Status::Done if watch.open && watch.worked => {
                            // Ended by status; a hook's reply may still be on its way.
                            let since = *watch.idle_since.get_or_insert_with(Instant::now);
                            let tier = share_kind(&watch.kind).reply;
                            let grace = match tier {
                                ReplyTier::Hook => HOOK_GRACE,
                                ReplyTier::SessionFile if watch.files.is_some() => FILE_GRACE,
                                _ => Duration::ZERO,
                            };
                            if since.elapsed() >= grace {
                                watch.open = false;
                                watch.idle_since = None;
                                let reply = watch.hook_reply.take().or_else(|| reply_from_screen(&watch.before, &screen, &watch.sent));
                                out.push(SessionEvent::TurnEnded(reply));
                            }
                        }
                        _ => watch.idle_since = None,
                    }
                }
            }
            for e in out {
                self.emit(&id, e);
            }
        }
    }
}

impl SessionHost for EngineHost {
    fn subscribe(&self, sink: HostSink) {
        *self.sink.write().unwrap_or_else(|e| e.into_inner()) = Some(sink);
    }

    fn launch(&self, launch: &HostLaunch) -> Result<()> {
        let kind = self.engine.kinds().find(&launch.kind).ok_or_else(|| PwError::not_found(format!("unknown agent kind {}", launch.kind)))?;
        // `env -i` first: the session gets exactly the environment the
        // sandbox policy built, nothing of Pitwall's (§6.3).
        // (A backend that already starts with `env -i` is run as it is.)
        let mut parts = Vec::new();
        if launch.wrapped.argv.first().map(String::as_str) != Some("/usr/bin/env") {
            parts.extend(["/usr/bin/env".to_string(), "-i".to_string()]);
            parts.extend(launch.wrapped.env.iter().map(|(k, v)| format!("{k}={v}")));
        }
        parts.extend(launch.wrapped.argv.iter().cloned());
        let command_line = parts.iter().map(|p| crate::shell::quote(p)).collect::<Vec<_>>().join(" ");
        let files = session_files(&kind, launch);
        let resume_id = launch.resumed.then(|| launch.session_id.clone()).flatten();
        self.watches().insert(
            launch.agent.clone(),
            Watch {
                kind,
                ready: false,
                open: false,
                worked: false,
                before: String::new(),
                sent: String::new(),
                idle_since: None,
                hook_reply: None,
                blocked: false,
                limited: None,
                exited: false,
                launching: true,
                files,
                resume_id,
                told_id: None,
            },
        );
        let cwd = launch.cwd.to_string_lossy().into_owned();
        let started = lifecycle::launch_shared(
            &self.engine,
            lifecycle::SharedLaunch {
                id: &launch.agent,
                name: &launch.name,
                kind: &launch.kind,
                kind_name: &launch.kind_name,
                cwd: &cwd,
                project: &launch.project,
                command_line,
                shared: launch.shared.clone(),
                session_id: launch.session_id.clone(),
                resumed: launch.resumed,
            },
        )
        .map_err(other);
        if let Some(w) = self.watches().get_mut(&launch.agent) {
            w.launching = false;
        }
        started.map(|_| ())
    }

    fn send(&self, agent: &str, text: &str) -> Result<()> {
        let before = self.screen(agent);
        let mut told = Vec::new();
        if let Some(w) = self.watches().get_mut(agent) {
            // Only what the file gets after this message is this turn's.
            told = w.read_files().0;
        }
        for e in told {
            self.emit(agent, e);
        }
        if let Some(w) = self.watches().get_mut(agent) {
            w.open = true;
            w.worked = false;
            w.before = before;
            w.sent = text.to_string();
            w.hook_reply = None;
            w.idle_since = None;
            w.blocked = false;
        }
        input::send_prompt(&self.engine, agent, text.to_string()).map_err(other)
    }

    fn interrupt(&self, agent: &str) -> Result<()> {
        if let Some(w) = self.watches().get_mut(agent) {
            w.open = false;
        }
        input::write_input_bytes(&self.engine, agent, b"\x1b").map_err(other)
    }

    fn stop(&self, agent: &str) -> Result<()> {
        if let Some(w) = self.watches().get_mut(agent) {
            w.open = false;
            w.ready = false;
            w.exited = true;
        }
        match lifecycle::stop(&self.engine, agent) {
            Ok(()) => Ok(()),
            // Never started, or already gone.
            Err(_) => Ok(()),
        }
    }

    fn remove(&self, agent: &str) -> Result<()> {
        self.watches().remove(agent);
        let engine = self.engine.clone();
        let id = agent.to_string();
        // Removing waits for the process to end: off the caller's thread.
        std::thread::spawn(move || {
            let _ = lifecycle::remove(&engine, &id, false);
        });
        Ok(())
    }

    fn partial_reply(&self, agent: &str) -> Option<Reply> {
        let screen = self.screen(agent);
        let w = self.watches();
        let watch = w.get(agent)?;
        watch.hook_reply.clone().or_else(|| reply_from_screen(&watch.before, &screen, &watch.sent))
    }
}

impl OwnerAgents for EngineHost {
    fn agent(&self, id: &str) -> Option<SharedAgent> {
        let rec = self.engine.records().into_iter().find(|r| r.id == id && r.shared.is_none())?;
        let kind = self.engine.kinds().for_record(&rec.kind, rec.custom_command.as_deref())?;
        Some(SharedAgent { kind, project: rec.project, account: rec.account })
    }

    fn account(&self, kind: &AgentKind, key: &str) -> Option<super::live::AccountInfo> {
        let accounts = crate::accounts::Accounts::load(self.engine.paths());
        let id = accounts.find(&kind.id, key).filter(|id| id != crate::accounts::DEFAULT)?;
        let a = accounts.get(&id)?;
        Some(super::live::AccountInfo { id: a.id.clone(), name: a.name.clone(), dir: crate::paths::expand_tilde(&a.dir) })
    }

    fn env_value(&self, name: &str) -> Option<String> {
        owner_env_value(name)
    }

    fn account_env(&self, kind: &AgentKind, account: Option<&str>) -> Vec<(String, String)> {
        let accounts = crate::accounts::Accounts::load(self.engine.paths());
        match account.filter(|a| *a != crate::accounts::DEFAULT).and_then(|a| accounts.get(a)) {
            Some(acc) => crate::accounts::env(kind, Some(acc)),
            // The owner's own login: its usual folder, named explicitly.
            None => match (&kind.account_env, &kind.account_home) {
                (Some(var), Some(home)) => vec![(var.clone(), crate::paths::expand_tilde(home).to_string_lossy().into_owned())],
                _ => vec![],
            },
        }
    }
}

/// A reader of the kind's session files for this launch (tier (a), §7.2),
/// under the agent account's folder the launch's environment names.
fn session_files(kind: &AgentKind, launch: &HostLaunch) -> Option<SessionFileReader> {
    let sk = share_kind(kind);
    if sk.reply != ReplyTier::SessionFile {
        return None;
    }
    let spec = sk.session_files?;
    let root = kind.account_env.as_ref().and_then(|v| launch.wrapped.env.get(v)).or_else(|| launch.wrapped.env.get("HOME"))?;
    Some(SessionFileReader::new(spec, std::path::Path::new(root), &launch.cwd, std::time::SystemTime::now()))
}

/// How long a variable read from the login shell is reused.
const SHELL_VALUE_TTL: Duration = Duration::from_secs(60);

/// The owner's variable `name` (passed explicitly by a share): this
/// process's environment, else the login shell's (`$SHELL -l -i -c`,
/// time-boxed, cached a minute). Only a valid variable name is looked up.
fn owner_env_value(name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(name) {
        return Some(v);
    }
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || name.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    static CACHE: Mutex<Option<HashMap<String, (Instant, Option<String>)>>> = Mutex::new(None);
    if let Some((at, v)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).get(name) {
        if at.elapsed() < SHELL_VALUE_TTL {
            return v.clone();
        }
    }
    const MARK: &str = "__pitwall_value__";
    let shell = crate::shell::LoginShell::current();
    let value = (shell.flavor == crate::shell::Flavor::Posix)
        .then(|| shell.output(&format!("printf '\\n{MARK}%s' \"${{{name}:-}}\""), Duration::from_secs(5)))
        .flatten()
        .and_then(|out| out.rsplit_once(MARK).map(|(_, v)| v.trim_end_matches(['\r', '\n']).to_string()))
        .filter(|v| !v.is_empty());
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(name.to_string(), (Instant::now(), value.clone()));
    value
}

#[cfg(test)]
mod tests;
