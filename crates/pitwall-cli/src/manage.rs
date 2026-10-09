//! `pitwall agent stop|restart|remove|rename|status|move`, `queue`,
//! `space`, `project`, `rules`, `review` and `wait`: the socket's managing
//! methods (`pitwall_proto::manage`), with agents and spaces named by id or
//! name.

use std::fmt::Write;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use pitwall_client::{Client, Error};
use pitwall_proto::{
    caps, code, method, AgentDiagnosis, AgentMove, AgentQueue, AgentRef, AgentRemove, AgentRename, AgentView, AgentWait, ErrorBody, ProjectPath,
    ProjectView, QueueAdd, QueueFilter, QueueItemRef, ReviewChanges, ReviewRequest, RuleSets, RulesApplied, RulesApply, RulesDefault,
    RulesOverview, SpaceCreate, SpaceMove, SpaceRename, SpaceView, Status, WaitResult,
};

use crate::args::{AgentCmd, ProjectCmd, QueueCmd, ReviewCmd, RulesCmd, SpaceCmd, WaitArgs};
use crate::Output;

fn err(code: &str, message: impl Into<String>) -> Error {
    Error::Server(ErrorBody::new(code, message))
}

fn out<T: Serialize>(v: &T, human: String) -> Output {
    Output { json: serde_json::to_value(v).unwrap_or(Value::Null), human }
}

fn manage<R: DeserializeOwned>(c: &mut Client, method: &str, params: impl Serialize) -> Result<R, Error> {
    c.call_cap(caps::MANAGE, method, params)
}

fn spaces<R: DeserializeOwned>(c: &mut Client, method: &str, params: impl Serialize) -> Result<R, Error> {
    c.call_cap(caps::SPACES, method, params)
}

fn word(s: Status) -> String {
    serde_json::to_value(s).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

fn parse_status(s: &str) -> Result<Status, Error> {
    serde_json::from_value(Value::String(s.trim().to_lowercase()))
        .map_err(|_| err(code::BAD_PARAMS, format!("{s}: expected idle, blocked, done, working, unknown, stopped or exited")))
}

/// An agent by id, else by name (ignoring case; it must be the only one).
pub fn resolve(list: &[AgentView], key: &str) -> Result<AgentView, Error> {
    let key = key.trim();
    if let Some(a) = list.iter().find(|a| a.id == key) {
        return Ok(a.clone());
    }
    let named: Vec<&AgentView> = list.iter().filter(|a| a.name.eq_ignore_ascii_case(key)).collect();
    match named.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(err(code::NOT_FOUND, format!("no agent \"{key}\" in Pitwall (see `pitwall agent list`)"))),
        more => Err(err(
            code::CONFLICT,
            format!("{} agents are named \"{key}\": use an id ({})", more.len(), more.iter().map(|a| a.id.as_str()).collect::<Vec<_>>().join(", ")),
        )),
    }
}

fn agent(c: &mut Client, key: &str) -> Result<AgentView, Error> {
    let list = c.agents()?;
    resolve(&list, key)
}

/// The queued item at 1-based `n`.
fn nth(a: &AgentView, n: usize) -> Result<String, Error> {
    if n == 0 {
        return Err(err(code::BAD_PARAMS, "items are numbered from 1"));
    }
    a.queue
        .get(n - 1)
        .map(|q| q.id.clone())
        .ok_or_else(|| err(code::NOT_FOUND, format!("\"{}\" has {} queued prompt(s), not {n}", a.name, a.queue.len())))
}

pub fn agent_cmd(cmd: &AgentCmd, c: &mut Client) -> Result<Output, Error> {
    let key = match cmd {
        AgentCmd::Stop { agent }
        | AgentCmd::Restart { agent }
        | AgentCmd::Remove { agent, .. }
        | AgentCmd::Rename { agent, .. }
        | AgentCmd::Status { agent }
        | AgentCmd::Move { agent, .. } => agent,
        AgentCmd::List | AgentCmd::New(_) => unreachable!("main.rs runs these"),
    };
    let a = agent(c, key)?;
    let r = AgentRef { agent_id: a.id.clone() };
    Ok(match cmd {
        AgentCmd::Stop { .. } => {
            let v: AgentView = manage(c, method::AGENT_STOP, &r)?;
            out(&v, format!("Stopped {} — id {}\n", v.name, v.id))
        }
        AgentCmd::Restart { .. } => {
            let v: AgentView = manage(c, method::AGENT_RESTART, &r)?;
            out(&v, format!("Restarted {} ({}) — id {}\n", v.name, word(v.status), v.id))
        }
        AgentCmd::Remove { worktree, .. } => {
            manage::<Value>(c, method::AGENT_REMOVE, AgentRemove { agent_id: a.id.clone(), delete_worktree: *worktree })?;
            let v = serde_json::json!({ "removed": a.id, "deletedWorktree": worktree });
            let extra = if *worktree { " and deleted its worktree" } else { "" };
            Output { json: v, human: format!("Removed {}{extra}\n", a.name) }
        }
        AgentCmd::Rename { name, .. } => {
            let v: AgentView = manage(c, method::AGENT_RENAME, AgentRename { agent_id: a.id.clone(), name: name.clone() })?;
            out(&v, format!("Renamed {} to {}\n", a.name, v.name))
        }
        AgentCmd::Status { .. } => {
            let d: AgentDiagnosis = manage(c, method::AGENT_STATUS, &r)?;
            out(&d, diagnosis(&d))
        }
        AgentCmd::Move { space, .. } => {
            let s: SpaceView = spaces(c, method::AGENT_MOVE, AgentMove { agent_id: a.id.clone(), space: space.clone() })?;
            out(&s, format!("{} is in space {} ({})\n", a.name, s.name, s.window))
        }
        AgentCmd::List | AgentCmd::New(_) => unreachable!(),
    })
}

fn prompt_text(text: &str) -> Result<String, Error> {
    if text != "-" {
        return Ok(text.to_string());
    }
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).map_err(Error::Io)?;
    Ok(s.trim_end_matches('\n').to_string())
}

pub fn queue_cmd(cmd: &QueueCmd, c: &mut Client) -> Result<Output, Error> {
    Ok(match cmd {
        QueueCmd::Add { args, status } => {
            let (targets, text) = match (status, args.as_slice()) {
                (Some(st), [text]) => {
                    let want = parse_status(st)?;
                    let list: Vec<AgentView> = c.agents()?.into_iter().filter(|a| a.status == want).collect();
                    (list, text)
                }
                (None, [key, text]) => (vec![agent(c, key)?], text),
                (Some(_), _) => return Err(err(code::BAD_PARAMS, "with --status give only the prompt: queue add --status idle \"…\"")),
                (None, _) => return Err(err(code::BAD_PARAMS, "give the agent and the prompt: queue add <agent> \"…\"")),
            };
            let text = prompt_text(text)?;
            let mut done = vec![];
            for a in &targets {
                let v: AgentView = manage(c, method::QUEUE_ADD, QueueAdd { agent_id: a.id.clone(), text: text.clone() })?;
                done.push(v);
            }
            let human = match done.as_slice() {
                [] => "No agent has that status: nothing queued.\n".to_string(),
                [one] => format!("Queued for {} ({} in its queue)\n", one.name, one.queue.len()),
                many => format!("Queued for {} agents: {}\n", many.len(), many.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")),
            };
            // One agent: its view (as `queue.add` answers); several: the list.
            match done.as_slice() {
                [one] => out(one, human),
                _ => out(&done, human),
            }
        }
        QueueCmd::List { agent: key } => {
            let id = match key {
                Some(k) => Some(agent(c, k)?.id),
                None => None,
            };
            let list: Vec<AgentQueue> = manage(c, method::QUEUE_LIST, QueueFilter { agent_id: id })?;
            out(&list, queues(&list))
        }
        QueueCmd::Remove { agent: key, n } => {
            let a = agent(c, key)?;
            let item = nth(&a, *n)?;
            let v: AgentView = manage(c, method::QUEUE_REMOVE, QueueItemRef { agent_id: a.id.clone(), item_id: Some(item) })?;
            out(&v, format!("Removed #{n} from {}'s queue ({} left)\n", v.name, v.queue.len()))
        }
        QueueCmd::Send { agent: key, n } => {
            let a = agent(c, key)?;
            let item = n.map(|n| nth(&a, n)).transpose()?;
            let v: AgentView = manage(c, method::QUEUE_SEND, QueueItemRef { agent_id: a.id.clone(), item_id: item })?;
            out(&v, format!("Sent to {} ({} still queued)\n", v.name, v.queue.len()))
        }
    })
}

pub fn space_cmd(cmd: &SpaceCmd, c: &mut Client) -> Result<Output, Error> {
    Ok(match cmd {
        SpaceCmd::List => {
            let list: Vec<SpaceView> = spaces(c, method::SPACE_LIST, Value::Null)?;
            let names = c.agents()?;
            out(&list, space_list(&list, &names))
        }
        SpaceCmd::Create { name } => {
            let s: SpaceView = spaces(c, method::SPACE_CREATE, SpaceCreate { name: name.clone() })?;
            out(&s, format!("Created space {} — id {}\n", s.name, s.id))
        }
        SpaceCmd::Rename { space, name } => {
            let s: SpaceView = spaces(c, method::SPACE_RENAME, SpaceRename { space: space.clone(), name: name.clone() })?;
            out(&s, format!("Renamed space to {} — id {}\n", s.name, s.id))
        }
        SpaceCmd::Move { space, to_window } => {
            let s: SpaceView = spaces(c, method::SPACE_MOVE, SpaceMove { space: space.clone(), window: to_window.clone() })?;
            out(&s, format!("Space {} is in window {}\n", s.name, s.window))
        }
    })
}

/// A folder as the server needs it (absolute; `~` left for the server).
fn folder(p: &str) -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    crate::absolute(Some(p), Path::new(&cwd))
}

pub fn project_cmd(cmd: &ProjectCmd, c: &mut Client) -> Result<Output, Error> {
    let (list, note): (Vec<ProjectView>, String) = match cmd {
        ProjectCmd::List => (manage(c, method::PROJECT_LIST, Value::Null)?, String::new()),
        ProjectCmd::Add { path } => (manage(c, method::PROJECT_ADD, ProjectPath { path: folder(path) })?, format!("Added {path}\n")),
        ProjectCmd::Remove { path } => (manage(c, method::PROJECT_REMOVE, ProjectPath { path: folder(path) })?, format!("Removed {path} from the list\n")),
    };
    Ok(out(&list, note + &projects(&list)))
}

pub fn rules_cmd(cmd: &RulesCmd, c: &mut Client) -> Result<Output, Error> {
    Ok(match cmd {
        RulesCmd::List => {
            let o: RulesOverview = manage(c, method::RULES_LIST, Value::Null)?;
            let names = c.agents()?;
            out(&o, rules_overview(&o, &names))
        }
        RulesCmd::Sets => {
            let s: RuleSets = manage(c, method::RULES_SETS, Value::Null)?;
            out(&s, rule_sets(&s))
        }
        RulesCmd::Apply { agent: key, set, main_checkout } => {
            let a = agent(c, key)?;
            let r: RulesApplied = manage(c, method::RULES_APPLY, RulesApply { agent_id: a.id.clone(), set: set.clone(), main_checkout: *main_checkout })?;
            let mut h = format!("Applied rules to {}: {} file(s)\n", a.name, r.generated.len());
            for g in &r.generated {
                let _ = writeln!(h, "  {g}");
            }
            if !r.log.trim().is_empty() {
                let _ = writeln!(h, "{}", r.log.trim_end());
            }
            out(&r, h)
        }
        RulesCmd::Default { project, set } => {
            let s: RuleSets = manage(c, method::RULES_DEFAULT, RulesDefault { project: folder(project), set: Some(set.clone()) })?;
            out(&s, rule_sets(&s))
        }
    })
}

pub fn review_cmd(cmd: &ReviewCmd, c: &mut Client) -> Result<Output, Error> {
    let ReviewCmd::Changes { agent: key, task } = cmd;
    let a = agent(c, key)?;
    let r: ReviewChanges = manage(c, method::REVIEW_CHANGES, ReviewRequest { agent_id: a.id.clone(), task_id: task.clone() })?;
    Ok(out(&r, review(&a.name, &r)))
}

pub fn wait_cmd(w: &WaitArgs, c: &mut Client) -> Result<Output, Error> {
    let until = w.until.iter().map(|s| parse_status(s)).collect::<Result<Vec<_>, _>>()?;
    let a = agent(c, &w.agent)?;
    let r: WaitResult = manage(
        c,
        method::AGENT_WAIT,
        AgentWait { agent_id: a.id.clone(), until, timeout_ms: w.timeout.map(|s| s.saturating_mul(1000)), fresh: w.fresh },
    )?;
    Ok(out(&r, format!("{} is {} (waited {:.1}s)\n", r.agent.name, word(r.agent.status), r.waited_ms as f64 / 1000.0)))
}

// ------------------------------------------------------------ --human

use crate::render::table;

pub fn diagnosis(d: &AgentDiagnosis) -> String {
    let mut h = format!("{} ({}) — {}", d.name, d.kind, word(d.status));
    if let Some(x) = &d.detail {
        let _ = write!(h, ": {x}");
    }
    let _ = writeln!(h, "  [from {}]", serde_json::to_value(d.source).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default());
    let opt = |o: &Option<String>| o.clone().unwrap_or_else(|| "-".into());
    let _ = writeln!(
        h,
        "  hooks:    {} ({}), last {}{}",
        if d.hooks.supported { "yes" } else { "no" },
        d.hooks.mode,
        opt(&d.hooks.last),
        match d.hooks.installed {
            Some(true) => ", installed",
            Some(false) => ", NOT installed",
            None => "",
        }
    );
    let _ = writeln!(h, "  screen:   rules {}, last {}", if d.screen.rules { "yes" } else { "no" }, opt(&d.screen.last));
    let _ = writeln!(
        h,
        "  activity: {}",
        match d.activity.last_output_ms_ago {
            Some(ms) => format!("last output {:.1}s ago", ms as f64 / 1000.0),
            None => "no output yet".into(),
        }
    );
    for l in &d.explanation {
        let _ = writeln!(h, "- {l}");
    }
    h
}

pub fn queues(list: &[AgentQueue]) -> String {
    if list.iter().all(|q| q.items.is_empty()) {
        return "Nothing queued.\n".into();
    }
    let mut h = String::new();
    for q in list {
        let send = if q.auto_send { "auto-send" } else { "manual" };
        let _ = writeln!(h, "{} ({}, {send}) — id {}", q.name, word(q.status), q.agent_id);
        for (i, it) in q.items.iter().enumerate() {
            let _ = writeln!(h, "  {}. {}", i + 1, it.text.lines().next().unwrap_or(""));
        }
    }
    h
}

fn name_of<'a>(agents: &'a [AgentView], id: &'a str) -> &'a str {
    agents.iter().find(|a| a.id == id).map_or(id, |a| a.name.as_str())
}

pub fn space_list(list: &[SpaceView], agents: &[AgentView]) -> String {
    let mut rows = vec![vec!["NAME".into(), "KIND".into(), "WINDOW".into(), "SHOWN".into(), "ID".into()]];
    rows.extend(list.iter().map(|s| {
        let shown: Vec<&str> = s.shown.iter().map(|a| name_of(agents, a)).collect();
        vec![s.name.clone(), s.kind.clone(), s.window.clone(), if shown.is_empty() { "-".into() } else { shown.join(", ") }, s.id.clone()]
    }));
    table(&rows)
}

pub fn projects(list: &[ProjectView]) -> String {
    if list.is_empty() {
        return "No projects.\n".into();
    }
    let rows: Vec<Vec<String>> = list.iter().map(|p| vec![p.display.clone(), if p.is_git { "git".into() } else { String::new() }]).collect();
    table(&rows)
}

pub fn rule_sets(s: &RuleSets) -> String {
    let mut h = String::new();
    if s.sets.is_empty() {
        h.push_str("No rule sets (make them in Settings → Rules).\n");
    }
    for set in &s.sets {
        let _ = writeln!(h, "{} — id {}: {}", set.name, set.id, set.rule_ids.join(", "));
    }
    for (p, id) in &s.project_defaults {
        let _ = writeln!(h, "default for {p}: {id}");
    }
    h
}

pub fn rules_overview(o: &RulesOverview, agents: &[AgentView]) -> String {
    let mut h = match (&o.via, o.available) {
        (Some(v), true) => format!("rulesync: via {v}\n"),
        _ => "rulesync: not available (install it, or allow npx: settings set rules.allowNpx true)\n".into(),
    };
    for f in &o.files {
        let _ = writeln!(h, "  {}  {}", f.id, f.description.clone().unwrap_or_default());
    }
    for a in &o.agents {
        let set = a.rule_set_id.clone().or(a.project_rule_set_id.clone()).unwrap_or_else(|| "-".into());
        let flags = [(a.stale, "stale"), (a.main_checkout, "main checkout"), (a.error.is_some(), "error")]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, w)| *w)
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(h, "{}: set {set}, {} file(s){}", name_of(agents, &a.agent_id), a.generated.len(), if flags.is_empty() { String::new() } else { format!(" ({flags})") });
    }
    h
}

pub fn review(name: &str, r: &ReviewChanges) -> String {
    if r.files.is_empty() {
        return format!("{name}: no changes\n");
    }
    let mut rows: Vec<Vec<String>> = r
        .files
        .iter()
        .map(|f| {
            let st = f.status.map(|s| format!("{s:?}")).unwrap_or_else(|| if f.untracked { "U".into() } else { "M".into() });
            let nums = if f.binary { "binary".to_string() } else { format!("+{} -{}", f.added, f.removed) };
            vec![st, nums, f.path.clone()]
        })
        .collect();
    rows.push(vec![String::new(), format!("+{} -{}", r.added, r.removed), format!("{} file(s)", r.files.len())]);
    table(&rows)
}
