//! The method table (architecture.md §4: every method's risk in one place)
//! and what each method does, in terms of `pitwall-core`.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use pitwall_core::engine::lifecycle;
use pitwall_core::model::{AdoptSessionRequest, CreateAgentRequest};
use pitwall_core::onboarding::{self, places};
use pitwall_core::provider::{Locator, MachineId, ProviderId};
use pitwall_proto::{
    code, method, AgentCreate, ApprovalAnswer, ErrorBody, MachineEntry, ProviderMachines, Risk, SessionAdd, SessionAdded, SessionFilter,
};

use crate::approvals::{Ask, Decision};
use crate::identity::Caller;
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
    (method::AGENT_CREATE, Access::Open),
    (method::MACHINE_LIST, Access::Open),
    (method::SESSION_LIST, Access::Open),
    (method::SESSION_ADD, Access::AskWhen("`start`: starts a stopped session on its machine", Risk::Low)),
    (method::APPROVAL_LIST, Access::Ui),
    (method::APPROVAL_ANSWER, Access::Ui),
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
        method::AGENT_CREATE => agent_create(s, required(p)?),
        method::MACHINE_LIST => ok(machines(s)),
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
        _ => Err(ErrorBody::new(code::UNKNOWN_METHOD, format!("unknown method \"{name}\""))),
    }
}

fn agent_create(s: &Server, req: AgentCreate) -> Res {
    if req.kind.trim().is_empty() || req.project.trim().is_empty() {
        return Err(ErrorBody::new(code::BAD_PARAMS, "kind and project are required"));
    }
    let view = lifecycle::create(
        &s.engine,
        CreateAgentRequest {
            name: req.name.unwrap_or_default(),
            kind: req.kind,
            project_path: req.project,
            custom_command: None,
            worktree: false,
            resume_session_id: req.resume,
            rule_set_id: None,
            apply_to_main_checkout: false,
            display_project: None,
            cols: req.cols,
            rows: req.rows,
        },
    )
    .map_err(other)?;
    onboarding::add_agent_project(&s.engine, &view);
    ok(view)
}

fn machines(s: &Server) -> Vec<ProviderMachines> {
    let providers = s.engine.providers();
    providers
        .all()
        .iter()
        .map(|p| {
            let caps = p.caps();
            let (machines, error) = match providers.machines(p) {
                Ok(ms) => (
                    ms.into_iter().map(|m| MachineEntry { id: m.id.to_string(), label: m.label, detail: m.detail }).collect(),
                    None,
                ),
                Err(e) => (vec![], Some(e.to_string())),
            };
            ProviderMachines {
                provider: p.id().to_string(),
                label: p.label(),
                version: p.version(),
                can_create: caps.create,
                can_add_sessions: caps.attach_existing,
                machines,
                error,
            }
        })
        .collect()
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
        assert!(matches!(access(method::SESSION_ADD), Some(Access::AskWhen(_, Risk::Low))));
        assert_eq!(access(method::APPROVAL_ANSWER), Some(Access::Ui));
        assert_eq!(access("agent.remove"), None);
    }
}
