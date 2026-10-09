//! The method table (architecture.md §4: every method's risk in one place)
//! and what each method does, in terms of `pitwall-core`.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use pitwall_core::engine::{changes, lifecycle};
use pitwall_core::model::{AdoptSessionRequest, CreateAgentRequest};
use pitwall_core::onboarding::{self, places};
use pitwall_core::provider::{Locator, MachineId, ProviderId};
use pitwall_proto::{code, method, AgentCreate, AgentRef, ApprovalAnswer, CreateForm, ErrorBody, FormRequest, Risk, SessionAdd, SessionAdded, SessionFilter};

use crate::approvals::{Ask, Decision};
use crate::identity::Caller;
use crate::{manage, workspace};
use crate::server::Server;

/// Who may call a method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Anyone: read-only, or only adds to Pitwall (nothing changes
    /// anywhere else).
    Open,
    /// Anyone, but the user approves in Pitwall first when the request does
    /// what the note says.
    AskWhen(&'static str, Risk),
    /// Pitwall's own (verified) UI only.
    Ui,
}

pub const METHODS: &[(&str, Access)] = &[
    (method::AGENT_LIST, Access::Open),
    // Read-only: git reads on the agent's machine.
    (method::AGENT_REFRESH, Access::Open),
    (
        method::AGENT_CREATE,
        // Low; High (asked every time) when it also makes a workspace or an
        // agent user there.
        Access::AskWhen("`machine` is a platform's (agw): creates and starts a session on that machine", Risk::Low),
    ),
    (method::MACHINE_LIST, Access::Open),
    (method::MACHINE_FORM, Access::Open),
    (method::SESSION_LIST, Access::Open),
    (method::SESSION_ADD, Access::AskWhen("`start`: starts a stopped session on its machine", Risk::Low)),
    (method::APPROVAL_LIST, Access::Ui),
    (method::APPROVAL_ANSWER, Access::Ui),
    (method::SETTINGS_LIST, Access::Open),
    (method::SETTINGS_GET, Access::Open),
    // `safe` settings (Pitwall's own look) apply directly.
    (method::SETTINGS_SET, Access::AskWhen("the setting is `approval` (changes something outside Pitwall)", Risk::High)),
    (method::SETTINGS_RESET, Access::AskWhen("the setting is `approval` (changes something outside Pitwall)", Risk::High)),
    // Managing agents (manage.rs). Stop/restart: Low on this Mac, High
    // (asked every time) on another machine.
    (method::AGENT_STOP, Access::AskWhen("always: ends the agent's process", Risk::Low)),
    (method::AGENT_RESTART, Access::AskWhen("always: ends and starts the agent's process", Risk::Low)),
    (method::AGENT_REMOVE, Access::AskWhen("always; `deleteWorktree` also deletes its worktree", Risk::High)),
    (method::AGENT_RENAME, Access::Open),
    (method::AGENT_STATUS, Access::Open),
    (method::AGENT_WAIT, Access::Open),
    // Queued prompts go to agents already in Pitwall, as the UI's queue.
    (method::QUEUE_ADD, Access::Open),
    (method::QUEUE_LIST, Access::Open),
    (method::QUEUE_REMOVE, Access::Open),
    (method::QUEUE_SEND, Access::Open),
    // Pitwall's project list; never touches the folders.
    (method::PROJECT_LIST, Access::Open),
    (method::PROJECT_ADD, Access::Open),
    (method::PROJECT_REMOVE, Access::Open),
    (method::RULES_LIST, Access::Open),
    (method::RULES_SETS, Access::Open),
    (method::RULES_APPLY, Access::AskWhen("the agent works in the main checkout: rule files are written there", Risk::High)),
    (method::RULES_DEFAULT, Access::Open),
    (method::REVIEW_CHANGES, Access::Open),
    // Pitwall's own windows (workspace.rs).
    (method::SPACE_LIST, Access::Open),
    (method::SPACE_CREATE, Access::Open),
    (method::SPACE_RENAME, Access::Open),
    (method::SPACE_MOVE, Access::Open),
    (method::AGENT_MOVE, Access::Open),
];

pub fn access(name: &str) -> Option<Access> {
    METHODS.iter().find(|(m, _)| *m == name).map(|(_, a)| *a)
}

type Res = Result<Value, ErrorBody>;

fn params<T: DeserializeOwned + Default>(v: Value) -> Result<T, ErrorBody> {
    if v.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(v).map_err(|e| ErrorBody::new(code::BAD_PARAMS, e.to_string()))
}

fn required<T: DeserializeOwned>(v: Value) -> Result<T, ErrorBody> {
    serde_json::from_value(v).map_err(|e| ErrorBody::new(code::BAD_PARAMS, e.to_string()))
}

fn ok(v: impl Serialize) -> Res {
    serde_json::to_value(v).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

fn other(e: impl Into<String>) -> ErrorBody {
    ErrorBody::new(code::OTHER, e)
}

pub fn dispatch(s: &Server, caller: &Caller, name: &str, p: Value) -> Res {
    match access(name) {
        None => return Err(ErrorBody::new(code::UNKNOWN_METHOD, format!("unknown method \"{name}\""))),
        Some(Access::Ui) if !caller.ui => {
            return Err(ErrorBody::new(code::DENIED, "only Pitwall's own window can do this"));
        }
        _ => {}
    }
    match name {
        method::AGENT_LIST => ok(s.engine.views()),
        method::AGENT_REFRESH => {
            let r: AgentRef = required(p)?;
            changes::refresh(&s.engine, &r.agent_id).map_err(other)?;
            let view = s.engine.views().into_iter().find(|v| v.id == r.agent_id);
            ok(view.ok_or_else(|| ErrorBody::new(code::NOT_FOUND, format!("no agent {}", r.agent_id)))?)
        }
        method::AGENT_CREATE => agent_create(s, caller, required(p)?),
        method::MACHINE_LIST => ok(s.engine.machine_list()),
        method::MACHINE_FORM => {
            let r: FormRequest = required(p)?;
            ok(s.engine.create_form(Some(&r.provider), Some(&r.machine)).map_err(other)?)
        }
        method::SESSION_LIST => {
            let filter: SessionFilter = params(p)?;
            let owned = s.engine.records().iter().map(|r| r.locator()).collect();
            ok(filter.apply(places::places(s.engine.providers(), s.engine.kinds(), &owned)))
        }
        method::SESSION_ADD => session_add(s, caller, required(p)?),
        method::APPROVAL_LIST => ok(s.approvals.pending()),
        method::APPROVAL_ANSWER => {
            let a: ApprovalAnswer = required(p)?;
            s.approvals.answer(&a).map_err(|e| ErrorBody::new(code::NOT_FOUND, e))?;
            Ok(Value::Null)
        }
        method::SETTINGS_LIST => crate::settings::list(s),
        method::SETTINGS_GET => crate::settings::get(s, required(p)?),
        method::SETTINGS_SET => crate::settings::set(s, caller, required(p)?),
        method::SETTINGS_RESET => crate::settings::reset(s, caller, required(p)?),
        method::AGENT_STOP => manage::stop(s, caller, required(p)?),
        method::AGENT_RESTART => manage::restart(s, caller, required(p)?),
        method::AGENT_REMOVE => manage::remove(s, caller, required(p)?),
        method::AGENT_RENAME => manage::rename(s, required(p)?),
        method::AGENT_STATUS => manage::status(s, required(p)?),
        method::AGENT_WAIT => manage::wait(s, required(p)?),
        method::QUEUE_ADD => manage::queue_add(s, required(p)?),
        method::QUEUE_LIST => manage::queue_list(s, params(p)?),
        method::QUEUE_REMOVE => manage::queue_remove(s, required(p)?),
        method::QUEUE_SEND => manage::queue_send(s, required(p)?),
        method::PROJECT_LIST => manage::project_list(s),
        method::PROJECT_ADD => manage::project_add(s, required(p)?),
        method::PROJECT_REMOVE => manage::project_remove(s, required(p)?),
        method::RULES_LIST => manage::rules_list(s),
        method::RULES_SETS => manage::rules_sets(s),
        method::RULES_APPLY => manage::rules_apply(s, caller, required(p)?),
        method::RULES_DEFAULT => manage::rules_default(s, required(p)?),
        method::REVIEW_CHANGES => manage::review_changes(s, required(p)?),
        method::SPACE_LIST => workspace::list(s),
        method::SPACE_CREATE => workspace::create(s, required(p)?),
        method::SPACE_RENAME => workspace::rename(s, required(p)?),
        method::SPACE_MOVE => workspace::move_to_window(s, required(p)?),
        method::AGENT_MOVE => workspace::move_agent(s, required(p)?),
        _ => Err(ErrorBody::new(code::UNKNOWN_METHOD, format!("unknown method \"{name}\""))),
    }
}

fn agent_create(s: &Server, caller: &Caller, req: AgentCreate) -> Res {
    if req.provider.is_some() || req.machine.is_some() {
        let form = s.engine.create_form(req.provider.as_deref(), req.machine.as_deref()).map_err(other)?;
        if !form.folder {
            return create_on_platform(s, caller, req, form);
        }
    }
    if req.kind.trim().is_empty() || req.project.trim().is_empty() {
        return Err(ErrorBody::new(code::BAD_PARAMS, "kind and project are required"));
    }
    let view = lifecycle::create(
        &s.engine,
        CreateAgentRequest {
            name: req.name.unwrap_or_default(),
            kind: req.kind,
            project_path: req.project,
            provider: req.provider,
            machine: req.machine,
            cols: req.cols,
            rows: req.rows,
            resume_session_id: req.resume,
            ..Default::default()
        },
    )
    .map_err(other)?;
    onboarding::add_agent_project(&s.engine, &view);
    ok(view)
}

/// A new session on a platform's machine (agw): it changes that machine,
/// so the user approves what the form's summary says first — every time
/// when it also makes a workspace or an agent user there.
fn create_on_platform(s: &Server, caller: &Caller, req: AgentCreate, form: CreateForm) -> Res {
    let name = req.name.as_deref().map(str::trim).unwrap_or_default().to_string();
    if name.is_empty() {
        return Err(ErrorBody::new(code::BAD_PARAMS, format!("a name is required: the session's name on {}", form.machine_label)));
    }
    let values = form.values(&req.options).map_err(|e| ErrorBody::new(code::BAD_PARAMS, e))?;
    let summary = form.summarize(&name, &values);
    let place = format!("{} ({})", form.machine_label, form.provider);
    let mut details = vec![summary.text.clone()];
    details.extend(summary.creates.iter().map(|c| format!("Also creates {c} on {}.", form.machine_label)));
    details.push("It keeps running there after Pitwall quits; removing it from Pitwall leaves it there.".into());
    let risk = if summary.creates.is_empty() { Risk::Low } else { Risk::High };
    let decision = s.approvals.ask(Ask {
        action: method::AGENT_CREATE.into(),
        summary: format!("create the session \"{name}\" on {place}"),
        details,
        requester: caller.requester(),
        caller_key: caller.key(),
        risk,
    });
    match decision {
        Decision::Allowed | Decision::Remembered => {}
        Decision::Denied => return Err(ErrorBody::new(code::DENIED, format!("The user didn't allow creating \"{name}\" on {place}. Nothing was created."))),
        Decision::TimedOut => {
            return Err(ErrorBody::new(
                code::APPROVAL_TIMEOUT,
                format!("Nobody answered the approval to create \"{name}\" on {place} in time. Nothing was created."),
            ))
        }
    }
    let view = lifecycle::create(
        &s.engine,
        CreateAgentRequest {
            name,
            kind: req.kind,
            provider: Some(form.provider),
            machine: Some(form.machine),
            options: req.options,
            cols: req.cols,
            rows: req.rows,
            ..Default::default()
        },
    )
    .map_err(other)?;
    ok(view)
}

/// Adopt a session (only tracks it: nothing changes there), then — when
/// asked and it is stopped — start it there after the user approves.
fn session_add(s: &Server, caller: &Caller, req: SessionAdd) -> Res {
    let known: Vec<String> = s.engine.views().into_iter().map(|v| v.id).collect();
    let view = lifecycle::adopt(
        &s.engine,
        AdoptSessionRequest {
            provider: req.provider.clone(),
            machine: req.machine.clone(),
            native: req.native.clone(),
            cols: req.cols,
            rows: req.rows,
        },
    )
    .map_err(other)?;
    let already = known.contains(&view.id);
    if !req.start || view.running {
        return ok(SessionAdded { agent: view, already, started: false });
    }
    if !view.caps.restart {
        return Err(ErrorBody::new(code::UNSUPPORTED, format!("\"{}\" was added, but Pitwall can't start it", req.native)));
    }
    let loc = Locator::new(&ProviderId::new(&req.provider), &MachineId::new(&req.machine), &req.native);
    let place = format!("{} ({})", s.engine.providers().machine_label(&loc), view.machine.provider);
    let decision = s.approvals.ask(Ask {
        action: method::SESSION_ADD.into(),
        summary: format!("start the session \"{}\" on {place}", req.native),
        details: vec![
            "It is stopped there. Starting it runs it on that machine, where it keeps running after Pitwall quits.".into(),
            "It has already been added to Pitwall (that needs no approval).".into(),
        ],
        requester: caller.requester(),
        caller_key: caller.key(),
        risk: Risk::Low,
    });
    match decision {
        Decision::Allowed | Decision::Remembered => {
            let size = pitwall_core::model::size_of(req.cols, req.rows);
            let agent = lifecycle::restart(&s.engine, &view.id, size).map_err(other)?;
            ok(SessionAdded { agent, already, started: true })
        }
        Decision::Denied => Err(ErrorBody::new(
            code::DENIED,
            format!("The user didn't allow starting \"{}\". It was added to Pitwall and is still stopped.", req.native),
        )),
        Decision::TimedOut => Err(ErrorBody::new(
            code::APPROVAL_TIMEOUT,
            format!("Nobody answered the approval to start \"{}\" in time. It was added to Pitwall and is still stopped.", req.native),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_method_has_one_row() {
        for (i, (m, _)) in METHODS.iter().enumerate() {
            assert!(METHODS[i + 1..].iter().all(|(n, _)| n != m), "{m} twice");
        }
        assert_eq!(access(method::AGENT_LIST), Some(Access::Open));
        assert_eq!(access(method::MACHINE_FORM), Some(Access::Open));
        assert_eq!(access(method::AGENT_REFRESH), Some(Access::Open));
        assert!(matches!(access(method::AGENT_CREATE), Some(Access::AskWhen(_, Risk::Low))));
        assert!(matches!(access(method::SESSION_ADD), Some(Access::AskWhen(_, Risk::Low))));
        assert_eq!(access(method::APPROVAL_ANSWER), Some(Access::Ui));
        assert_eq!(access(method::SETTINGS_LIST), Some(Access::Open));
        assert!(matches!(access(method::SETTINGS_SET), Some(Access::AskWhen(_, Risk::High))));
        assert!(matches!(access(method::SETTINGS_RESET), Some(Access::AskWhen(_, Risk::High))));
        assert!(matches!(access(method::AGENT_REMOVE), Some(Access::AskWhen(_, Risk::High))));
        assert!(matches!(access(method::AGENT_STOP), Some(Access::AskWhen(..))));
        assert!(matches!(access(method::AGENT_RESTART), Some(Access::AskWhen(..))));
        assert!(matches!(access(method::RULES_APPLY), Some(Access::AskWhen(_, Risk::High))));
        // Read-only methods never ask.
        for m in [method::AGENT_STATUS, method::AGENT_WAIT, method::QUEUE_LIST, method::PROJECT_LIST, method::RULES_LIST, method::RULES_SETS, method::REVIEW_CHANGES, method::SPACE_LIST] {
            assert_eq!(access(m), Some(Access::Open), "{m}");
        }
        assert_eq!(access("agent.prompt"), None);
    }
}
