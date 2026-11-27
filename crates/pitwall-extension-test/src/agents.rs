//! The agents' questions group in the harness: a fake method table with
//! the agents you give ([`Harness::agent`]) that records what the tile
//! queues, and what the tile shows in Pitwall's sidebar and floating
//! control (its newest badges and questions) with ways to answer.

use std::sync::{Arc, Mutex};

use pitwall_tile_host::ui::{self, AgentState, AskAnswer, Event};
use pitwall_tile_host::{MethodError, Methods, UiRequest};
use serde_json::{json, Value};

use crate::{Harness, Session};

/// An agent for [`Harness::agent`] and [`Session::set_agents`]: idle,
/// Claude Code, in `/projects/<name>` on no branch, nothing queued. Change
/// its fields for more (`state`, `waiting_on_you`, `project`…).
pub fn agent(id: &str, name: &str) -> ui::Agent {
    ui::Agent { id: id.into(), name: name.into(), kind: "claude".into(), kind_name: "Claude Code".into(), project: format!("/projects/{name}"), branch: None, state: AgentState::Idle, waiting_on_you: false, queued: 0 }
}

/// [`agent`], blocked on a question: waiting on the user.
pub fn waiting_agent(id: &str, name: &str) -> ui::Agent {
    ui::Agent { state: AgentState::Blocked, waiting_on_you: true, ..agent(id, name) }
}

/// An item a tile put in an agent's Next up (`agents::queue_add`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    /// The agent's id.
    pub agent: String,
    /// The text, as Pitwall keeps it (trailing white space trimmed).
    pub text: String,
}

/// The most of one extension's items that wait in an agent's Next up, as
/// in Pitwall.
const MAX_WAITING: u32 = 3;

/// The fake method table: `agent.list` and `queue.add` as Pitwall answers
/// an extension (the same limits, except the one item per agent every 10
/// seconds, which a test on a virtual clock would only trip over).
#[derive(Default)]
pub(crate) struct FakeMethods {
    pub agents: Mutex<Vec<ui::Agent>>,
    pub queued: Mutex<Vec<Queued>>,
}

fn status(s: AgentState) -> &'static str {
    match s {
        AgentState::Working => "working",
        AgentState::Blocked => "blocked",
        AgentState::Idle => "idle",
        AgentState::Done => "done",
        AgentState::Stopped => "stopped",
        AgentState::Unknown => "unknown",
    }
}

impl Methods for FakeMethods {
    fn call(&self, _extension: &str, method: &str, params: Value) -> Result<Value, MethodError> {
        let mut agents = self.agents.lock().unwrap_or_else(|e| e.into_inner());
        match method {
            "agent.list" => Ok(Value::Array(
                agents
                    .iter()
                    .map(|a| {
                        let queue: Vec<Value> = (0..a.queued).map(|i| json!({"id": format!("q{i}"), "text": ""})).collect();
                        json!({"id": a.id, "name": a.name, "kind": a.kind, "kindName": a.kind_name, "project": a.project, "branch": a.branch, "status": status(a.state), "queue": queue})
                    })
                    .collect(),
            )),
            "queue.add" => {
                let id = params["agentId"].as_str().unwrap_or("");
                let text = params["text"].as_str().unwrap_or("").trim_end().to_string();
                let Some(a) = agents.iter_mut().find(|a| a.id == id) else {
                    return Err(MethodError::new("not_found", format!("no agent “{id}”")));
                };
                let mut queued = self.queued.lock().unwrap_or_else(|e| e.into_inner());
                if queued.iter().filter(|q| q.agent == id).count() as u32 >= MAX_WAITING {
                    return Err(MethodError::new("busy", format!("{MAX_WAITING} of its items already wait in {}'s Next up", a.name)));
                }
                a.queued += 1;
                queued.push(Queued { agent: id.to_string(), text });
                Ok(json!({"id": id}))
            }
            other => Err(MethodError::new("unsupported", format!("the harness doesn't answer {other}"))),
        }
    }
}

impl Harness {
    /// Pitwall shows `agent` (made with [`agent`] or [`waiting_agent`]):
    /// `agents::list` gives the agents added, in order, once the tile is
    /// granted `agents.read` ([`Harness::grant`]).
    pub fn agent(self, agent: ui::Agent) -> Harness {
        self.methods.agents.lock().unwrap_or_else(|e| e.into_inner()).push(agent);
        self
    }
}

impl Session {
    /// Pitwall's agents are now `agents`: the tile hears `agents-changed`
    /// (when it is granted `agents.read`, as in Pitwall).
    pub fn set_agents(&mut self, agents: Vec<ui::Agent>) -> &mut Self {
        *self.methods.agents.lock().unwrap_or_else(|e| e.into_inner()) = agents;
        if self.grants.iter().any(|g| g == "agents.read") {
            self.send(Event::AgentsChanged);
        }
        self.settle()
    }

    /// The agents as Pitwall shows them now (with what the tile queued
    /// counted in `queued`).
    pub fn agents(&self) -> Vec<ui::Agent> {
        self.methods.agents.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// What the tile put in agents' Next up, oldest first.
    pub fn queued(&self) -> Vec<Queued> {
        self.methods.queued.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The user sent or removed what waits in agent `agent`'s Next up.
    pub fn clear_queue(&mut self, agent: &str) -> &mut Self {
        self.methods.queued.lock().unwrap_or_else(|e| e.into_inner()).retain(|q| q.agent != agent);
        if let Some(a) = self.methods.agents.lock().unwrap_or_else(|e| e.into_inner()).iter_mut().find(|a| a.id == agent) {
            a.queued = 0;
        }
        self
    }

    /// The badges it shows on sidebar rows now (its newest set, as checked
    /// by Pitwall: rows it may not name are dropped).
    pub fn badges(&self) -> Vec<ui::Badge> {
        self.ui
            .iter()
            .rev()
            .find_map(|r| match r {
                UiRequest::Badges(b) => Some(b.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The questions it shows in the floating control now (its newest set,
    /// as checked by Pitwall).
    pub fn asks(&self) -> Vec<ui::Ask> {
        self.ui
            .iter()
            .rev()
            .find_map(|r| match r {
                UiRequest::Asks(a) => Some(a.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn answer_with(&mut self, id: &str, picks: Vec<u32>, text: Option<&str>, declined: bool) -> &mut Self {
        assert!(self.asks().iter().any(|a| a.id == id), "no open question “{id}”: {:?}", self.asks().iter().map(|a| &a.id).collect::<Vec<_>>());
        self.send(Event::AskAnswered(AskAnswer { id: id.into(), picks, text: text.map(str::to_string), declined }));
        self.settle()
    }

    /// The user answers question `id` in the floating control by picking
    /// suggestions `picks` (by place, from 0).
    pub fn answer(&mut self, id: &str, picks: &[u32]) -> &mut Self {
        self.answer_with(id, picks.to_vec(), None, false)
    }

    /// The user answers question `id` in their own words.
    pub fn answer_text(&mut self, id: &str, text: &str) -> &mut Self {
        self.answer_with(id, vec![], Some(text), false)
    }

    /// The user declines question `id`.
    pub fn decline(&mut self, id: &str) -> &mut Self {
        self.answer_with(id, vec![], None, true)
    }

    /// What the tile replied to the latest answer to question `id`
    /// (`asks::reply`): `Some(None)` taken, `Some(Some(why))` refused,
    /// `None` no reply yet.
    pub fn reply(&self, id: &str) -> Option<Option<String>> {
        self.ui.iter().rev().find_map(|r| match r {
            UiRequest::AskResult { id: i, refused } if i == id => Some(refused.clone()),
            _ => None,
        })
    }
}

/// The method table a session's host uses.
pub(crate) fn methods() -> Arc<FakeMethods> {
    Arc::new(FakeMethods::default())
}
