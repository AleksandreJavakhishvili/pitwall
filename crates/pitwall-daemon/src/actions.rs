//! What an extension may do to agents, with the user's permission
//! (docs/extensions/actions.md; docs/spec/extension-api.md, phase 5):
//! put text in an agent's Next up (`agents.queue`) and ask to start an
//! agent (`agents.start`). [`crate::methods::dispatch`] has checked the
//! grant before either runs here; these are the rules on top of it.
//!
//! - **Queued text** lands in Next up as the extension's item (its name on
//!   it, [`QueueOrigin`]) and is never sent by Auto-send unless the user
//!   lets that extension send without asking (the engine's
//!   `set_auto_senders`, from Settings). At most [`MAX_QUEUE_TEXT`]
//!   characters, one item per agent per [`QUEUE_EVERY`] and at most
//!   [`MAX_WAITING`] of its items waiting in one agent's Next up. Only
//!   Pitwall's own agents (not a peer's, shown here read-only).
//! - **Starting an agent** asks the user every time through the approval
//!   dialog (high risk: never remembered), naming the extension, the
//!   project, the tool and the first prompt in full; no answer in time is
//!   a no. Only on this computer, in one of Pitwall's projects, with the
//!   project's own accounts; one request waiting at a time and at most
//!   [`MAX_START_ASKS`] in [`START_WINDOW`].
//!
//! Nothing here changes what an agent may do: an agent's own calls never
//! come here (it isn't an extension caller), and what the extension queues
//! reaches the agent only as the user's own typing would.

use std::time::Duration;

use pitwall_core::engine::lifecycle;
use pitwall_core::model::CreateAgentRequest;
use pitwall_core::onboarding;
use pitwall_proto::{code, method, AgentCreate, ErrorBody, QueueAdd, QueueOrigin, Risk};
use serde_json::Value;

use crate::approvals::{Ask, Decision};
use crate::identity::Caller;
use crate::server::Server;

/// The longest text an extension queues (the protocol's reply limit).
pub const MAX_QUEUE_TEXT: usize = 4000;
/// One item per agent per extension this often.
pub const QUEUE_EVERY: Duration = Duration::from_secs(10);
/// The most of an extension's items waiting in one agent's Next up.
pub const MAX_WAITING: usize = 3;
/// The most start requests an extension makes in [`START_WINDOW`] (asked
/// or not answered alike).
pub const MAX_START_ASKS: usize = 3;
pub const START_WINDOW: Duration = Duration::from_secs(10 * 60);

type Res = Result<Value, ErrorBody>;

fn bad(m: impl Into<String>) -> ErrorBody {
    ErrorBody::new(code::BAD_PARAMS, m)
}

/// `queue.add` from extension `(id, name)`.
pub fn queue_add(s: &Server, ext: &(String, String), r: QueueAdd) -> Res {
    let (id, name) = ext;
    let text = r.text.trim_end().to_string();
    if text.trim().is_empty() {
        return Err(bad("the text is empty"));
    }
    let n = text.chars().count();
    if n > MAX_QUEUE_TEXT {
        return Err(bad(format!("The text is {n} characters; an extension queues at most {MAX_QUEUE_TEXT}.")));
    }
    let agent = crate::manage::agent(s, &r.agent_id)?;
    let waiting = s.engine.queued_by(&agent.id, id);
    if waiting >= MAX_WAITING {
        return Err(ErrorBody::new(crate::extensions::BUSY, format!("{MAX_WAITING} of {name}'s items already wait in {}'s Next up: the user sends or removes them first.", agent.name)));
    }
    s.extensions.may_queue(id, &agent.id)?;
    let origin = QueueOrigin { extension: id.clone(), name: name.clone() };
    let view = s.engine.queue_add_from(&agent.id, text, Some(origin)).map_err(|e| ErrorBody::new(code::OTHER, e))?;
    serde_json::to_value(view).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

/// `agent.create` from extension `caller`: asked of the user every time.
pub fn start(s: &Server, caller: &Caller, r: AgentCreate) -> Res {
    let Some((id, name)) = caller.extension.clone() else { return Err(ErrorBody::new(code::DENIED, "not an extension")) };
    if r.provider.is_some() || r.machine.is_some() || !r.options.is_empty() {
        return Err(bad("An extension starts agents on this computer only."));
    }
    if r.account.is_some() || r.resume.is_some() {
        return Err(bad("An extension starts a new conversation with the project's own accounts: no account or resume."));
    }
    if r.kind.trim().is_empty() || r.project.trim().is_empty() {
        return Err(bad("kind and project are required"));
    }
    let project = onboarding::project_list::normalize(&r.project);
    let Some(known) = s.engine.projects().list().into_iter().find(|p| p.path == project) else {
        return Err(ErrorBody::new(code::NOT_FOUND, format!("{} isn't one of Pitwall's projects: an extension starts agents only there.", r.project)));
    };
    let Some(kind) = s.engine.kinds().find(r.kind.trim()) else {
        return Err(ErrorBody::new(code::NOT_FOUND, format!("Pitwall has no agent kind \"{}\".", r.kind)));
    };
    let prompt = r.prompt.as_deref().map(str::trim_end).filter(|p| !p.trim().is_empty()).map(String::from);
    if let Some(p) = &prompt {
        let n = p.chars().count();
        if n > MAX_QUEUE_TEXT {
            return Err(bad(format!("The prompt is {n} characters; at most {MAX_QUEUE_TEXT}.")));
        }
    }
    let agent_name = r.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(String::from);
    if let Some(n) = &agent_name {
        pitwall_core::engine::check_name(n).map_err(bad)?;
    }
    let _asking = s.extensions.start_asking(&id)?;
    let place = if known.display.is_empty() { project.clone() } else { known.display.clone() };
    let mut details = vec![
        format!("Asked by the extension “{name}”, not by you."),
        format!("Project: {project}"),
        format!("Tool: {}", kind.name),
    ];
    if let Some(n) = &agent_name {
        details.push(format!("Name: {n}"));
    }
    details.push(match &prompt {
        Some(p) => format!("Its first prompt, put in its Next up marked “{name}” (sent only when you press Send, unless you let {name} send without asking):\n{p}"),
        None => "No first prompt.".into(),
    });
    details.push("It runs as you, in this project's folder, with the project's accounts and its own permissions there.".into());
    let decision = s.approvals.ask(Ask {
        action: method::AGENT_CREATE.into(),
        summary: format!("start a {} agent in {place}", kind.name),
        details,
        requester: caller.requester(),
        caller_key: caller.key(),
        // Never remembered: asked every time.
        risk: Risk::High,
    });
    match decision {
        Decision::Allowed | Decision::Remembered => {}
        Decision::Denied => return Err(ErrorBody::new(code::DENIED, format!("The user didn't allow starting a {} agent in {place}. Nothing was started.", kind.name))),
        Decision::TimedOut => return Err(ErrorBody::new(code::APPROVAL_TIMEOUT, format!("Nobody answered the request to start a {} agent in {place} in time. Nothing was started.", kind.name))),
    }
    let view = lifecycle::create(
        &s.engine,
        CreateAgentRequest { name: agent_name.unwrap_or_default(), kind: kind.id.clone(), project_path: project, cols: r.cols, rows: r.rows, ..Default::default() },
    )
    .map_err(|e| ErrorBody::new(code::OTHER, e))?;
    onboarding::add_agent_project(&s.engine, &view);
    let view = match prompt {
        Some(p) => s.engine.queue_add_from(&view.id, p, Some(QueueOrigin { extension: id, name })).unwrap_or(view),
        None => view,
    };
    serde_json::to_value(view).map_err(|e| ErrorBody::new(code::OTHER, e.to_string()))
}

/// What a call that acts asked for, in a few words, for the activity log
/// (which agent, the text, the project).
pub fn detail(s: &Server, name: &str, p: &Value) -> Option<String> {
    let agent = |key: &str| {
        let id = p.get(key)?.as_str()?;
        Some(s.engine.views().into_iter().find(|v| v.id == id).map_or_else(|| format!("agent {id}"), |v| format!("“{}”", v.name)))
    };
    let text = |key: &str| p.get(key).and_then(Value::as_str).map(|t| format!("“{t}”"));
    match name {
        method::QUEUE_ADD => Some(format!("to {}: {}", agent("agentId")?, text("text").unwrap_or_default())),
        method::AGENT_CREATE => {
            let mut d = format!("a {} agent in {}", p.get("kind").and_then(Value::as_str).unwrap_or("?"), p.get("project").and_then(Value::as_str).unwrap_or("?"));
            if let Some(t) = text("prompt") {
                d.push_str(&format!(" with the prompt {t}"));
            }
            Some(d)
        }
        method::VIEWER_OPEN => {
            let files: Vec<&str> = p.get("files").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            let what = if files.is_empty() { p.get("folder").and_then(Value::as_str).unwrap_or_default().to_string() } else { files.join(", ") };
            Some(format!("{what} for {}", agent("agentId").unwrap_or_else(|| "no agent".into())))
        }
        method::WEB_OPEN => Some(format!("{} for {}", p.get("url").and_then(Value::as_str).unwrap_or_default(), agent("agentId").unwrap_or_else(|| "no agent".into()))),
        method::AGENT_FOCUS => agent("agentId"),
        _ => None,
    }
}
