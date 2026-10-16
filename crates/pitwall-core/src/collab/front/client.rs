//! Pitwall's client for another Pitwall's collaboration handler: the A2A
//! front per share and the Pitwall endpoints, over a network's
//! [`HttpClient`] (`Network::client(&device)`). The daemon's `ask.*` uses
//! it; the network picks the route and proves who is calling, this adds
//! only the labels (`Pitwall-Agent`, `Pitwall-Chain`), never credentials.
//!
//! ```ignore
//! let owner = PitwallClient::new(network.client(&alice_studio)?).with_labels(Labels {
//!     agent: Some("client-fix@bob-mbp".into()),
//!     chain: vec![],
//! });
//! let agents = owner.directory()?;
//! let task = owner.agent("api-expert").send(&Outgoing::text("where are refresh tokens rotated?"))?;
//! let turn = turn_of(&task); // the core's Turn, as the owner's Pitwall saw it
//! ```

use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use super::a2a::model::{
    ListTasksResponse, Message, Part, Role, SendMessageConfiguration, SendMessageRequest, SendMessageResponse,
    StreamResponse, Task,
};
use super::a2a::{TURN_META, VERSION_HEADER};
use super::wire::{self, SseReader};
use crate::collab::team::{PitwallInfo, TeamHead};
use crate::collab::turn::{ConversationId, ConversationSummary, Turn, TurnId};
use crate::collab::{A2A_VERSION, AGENT_HEADER, CHAIN_HEADER, NEW_CONVERSATION_META};
use crate::error::{ErrorCode, PwError, Result};
use crate::net::http::{header, Method, StatusCode};
use crate::net::{Body, HttpClient, Request, Response, Timeouts};
use pitwall_proto::collab::DirectoryEntry;

/// Labels a Pitwall caller adds (§8.2): who is asking on this device, and
/// the chain of turns and shares the message came through (§8.6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Labels {
    /// `client-fix@bob-mbp`, from the process tree, never from an argument.
    pub agent: Option<String>,
    /// `c_17`, `alice/api-expert`, … (the asking turn and share, when an
    /// agent in a shared conversation asks on).
    pub chain: Vec<String>,
}

/// One owner's Pitwall, reached over one network.
pub struct PitwallClient {
    http: Box<dyn HttpClient>,
    labels: Labels,
}

/// One message to send.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outgoing {
    pub text: String,
    /// One of my conversations with this share; `None`: my current one.
    pub conversation: Option<ConversationId>,
    pub new_conversation: bool,
    /// "Redo on my code" (§7.5): my commits as a patch series.
    pub my_changes: Option<String>,
    /// Answer as soon as the task exists (else the owner's Pitwall waits a
    /// while for the turn to settle).
    pub return_immediately: bool,
}

impl Outgoing {
    pub fn text(text: impl Into<String>) -> Outgoing {
        Outgoing { text: text.into(), ..Outgoing::default() }
    }
}

/// An event of `GET /pitwall/v1/events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamEvent {
    Announce(TeamHead),
    DirectoryChanged,
}

impl PitwallClient {
    pub fn new(http: Box<dyn HttpClient>) -> PitwallClient {
        PitwallClient { http, labels: Labels::default() }
    }

    pub fn with_labels(mut self, labels: Labels) -> PitwallClient {
        self.labels = labels;
        self
    }

    /// One share's A2A front.
    pub fn agent(&self, slug: &str) -> Agent<'_> {
        Agent { client: self, base: format!("/a2a/{}", wire::percent_encode(slug)) }
    }

    // ----------------------------------------------- Pitwall endpoints

    /// `GET /pitwall/v1/info`: the discovery probe.
    pub fn info(&self) -> Result<PitwallInfo> {
        self.get_json("/pitwall/v1/info")
    }

    /// `GET /pitwall/v1/directory`: the shared agents I may use here.
    pub fn directory(&self) -> Result<Vec<DirectoryEntry>> {
        #[derive(serde::Deserialize)]
        struct Directory {
            agents: Vec<DirectoryEntry>,
        }
        Ok(self.get_json::<Directory>("/pitwall/v1/directory")?.agents)
    }

    /// `GET /pitwall/v1/sessions`: my conversations with this owner.
    pub fn sessions(&self) -> Result<Vec<ConversationSummary>> {
        #[derive(serde::Deserialize)]
        struct Sessions {
            sessions: Vec<ConversationSummary>,
        }
        Ok(self.get_json::<Sessions>("/pitwall/v1/sessions")?.sessions)
    }

    /// `DELETE /pitwall/v1/sessions/{conversation}`.
    pub fn forget(&self, conversation: &ConversationId) -> Result<()> {
        let path = format!("/pitwall/v1/sessions/{}", wire::percent_encode(conversation.as_str()));
        self.call(Method::DELETE, &path, None, false).map(|_| ())
    }

    /// `GET /pitwall/v1/results/{task}`: a large patch series.
    pub fn result(&self, turn: &TurnId) -> Result<String> {
        let path = format!("/pitwall/v1/results/{}", wire::percent_encode(turn.as_str()));
        let res = self.call(Method::GET, &path, None, false)?;
        String::from_utf8(res.into_body().read_all(64 * 1024 * 1024)?).map_err(|_| PwError::other("patch is not UTF-8"))
    }

    /// Fetch what a `changes` artifact's `url` part points at (it names this
    /// device's `/pitwall/v1/results/{task}`; the network picks the host).
    pub fn fetch(&self, url: &str) -> Result<String> {
        let path = url.find("/pitwall/v1/").map_or(url, |at| &url[at..]);
        let res = self.call(Method::GET, path, None, false)?;
        String::from_utf8(res.into_body().read_all(64 * 1024 * 1024)?).map_err(|_| PwError::other("patch is not UTF-8"))
    }

    /// `GET /pitwall/v1/events` (member devices).
    pub fn events(&self) -> Result<impl Iterator<Item = Result<TeamEvent>>> {
        let res = self.call(Method::GET, "/pitwall/v1/events", None, true)?;
        Ok(SseReader::new(res.into_body()).filter_map(|e| match e {
            Err(e) => Some(Err(PwError::unreachable(e.to_string()))),
            Ok(e) if e.event == "team.announce" => {
                Some(serde_json::from_str(&e.data).map(TeamEvent::Announce).map_err(|e| PwError::other(e.to_string())))
            }
            Ok(e) if e.event == "directory.changed" => Some(Ok(TeamEvent::DirectoryChanged)),
            Ok(_) => None,
        }))
    }

    // ------------------------------------------------------- plumbing

    fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let res = self.call(Method::GET, path, None, false)?;
        parse(res)
    }

    /// One request with the labels and the A2A version; an HTTP error
    /// status is an error with the server's message.
    fn call(&self, method: Method, path: &str, body: Option<Value>, stream: bool) -> Result<Response> {
        let mut builder = http_request(method, path).header(VERSION_HEADER, A2A_VERSION);
        if let Some(agent) = &self.labels.agent {
            builder = builder.header(AGENT_HEADER, agent.as_str());
        }
        if !self.labels.chain.is_empty() {
            builder = builder.header(CHAIN_HEADER, self.labels.chain.join(","));
        }
        if stream {
            builder = builder.header(header::ACCEPT, wire::EVENT_STREAM);
        } else {
            builder = builder.header(header::ACCEPT, wire::A2A_JSON);
        }
        let body = match body {
            Some(v) => {
                builder = builder.header(header::CONTENT_TYPE, wire::A2A_JSON);
                Body::json(&v)
            }
            None => Body::Empty,
        };
        let mut req: Request = builder.body(body).map_err(|e| PwError::other(e.to_string()))?;
        if stream {
            req.extensions_mut().insert(Timeouts { idle: None, ..Timeouts::default() });
        }
        let res = self.http.send(req)?;
        if res.status().is_success() {
            Ok(res)
        } else {
            Err(status_error(res))
        }
    }
}

fn http_request(method: Method, path: &str) -> crate::net::http::request::Builder {
    crate::net::http::Request::builder().method(method).uri(path)
}

fn parse<T: DeserializeOwned>(res: Response) -> Result<T> {
    let bytes = res.into_body().read_all(crate::collab::MAX_BODY * 8)?;
    serde_json::from_slice(&bytes).map_err(|e| PwError::other(format!("unexpected answer: {e}")))
}

/// An error answer as a [`PwError`]: `404` is `NotFound` (no such share,
/// or not mine to use: the same thing), `429`/`503` `Unreachable` (try
/// later), `403` `Denied`.
fn status_error(res: Response) -> PwError {
    let status = res.status();
    let body: Value =
        res.into_body().read_all(64 * 1024).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    let message = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .map_or_else(|| status.canonical_reason().unwrap_or("error").to_string(), str::to_string);
    let code = match status {
        StatusCode::NOT_FOUND => ErrorCode::NotFound,
        StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED => ErrorCode::Denied,
        StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE => ErrorCode::Unreachable,
        StatusCode::CONFLICT => ErrorCode::Conflict,
        _ => ErrorCode::Other,
    };
    PwError::new(code, message)
}

/// One share's A2A front on that Pitwall.
pub struct Agent<'a> {
    client: &'a PitwallClient,
    base: String,
}

impl Agent<'_> {
    /// The share's Agent Card (`404` / `NotFound` when I'm not in its
    /// audience).
    pub fn card(&self) -> Result<Value> {
        self.client.get_json(&format!("{}/card", self.base))
    }

    /// `SendMessage`: the task as it stands when the owner's Pitwall
    /// answers.
    pub fn send(&self, msg: &Outgoing) -> Result<Task> {
        let res = self.client.call(Method::POST, &format!("{}/message:send", self.base), Some(request(msg)), false)?;
        parse::<SendMessageResponse>(res)?.task.ok_or_else(|| PwError::other("the agent answered without a task"))
    }

    /// `SendStreamingMessage`: the task, then its updates until it ends.
    pub fn stream(&self, msg: &Outgoing) -> Result<TaskEvents> {
        let res = self.client.call(Method::POST, &format!("{}/message:stream", self.base), Some(request(msg)), true)?;
        Ok(TaskEvents { reader: SseReader::new(res.into_body()) })
    }

    /// `GetTask`.
    pub fn get(&self, task: &TurnId) -> Result<Task> {
        self.client.get_json(&format!("{}/tasks/{}", self.base, wire::percent_encode(task.as_str())))
    }

    /// `ListTasks`: my tasks with this share, newest first, all pages.
    pub fn list(&self, conversation: Option<&ConversationId>) -> Result<Vec<Task>> {
        let mut tasks = Vec::new();
        let mut token = String::new();
        loop {
            let mut path = format!("{}/tasks?pageSize=100&includeArtifacts=true", self.base);
            if let Some(c) = conversation {
                path.push_str(&format!("&contextId={}", wire::percent_encode(c.as_str())));
            }
            if !token.is_empty() {
                path.push_str(&format!("&pageToken={}", wire::percent_encode(&token)));
            }
            let page: ListTasksResponse = self.client.get_json(&path)?;
            tasks.extend(page.tasks);
            if page.next_page_token.is_empty() {
                return Ok(tasks);
            }
            token = page.next_page_token;
        }
    }

    /// `CancelTask`.
    pub fn cancel(&self, task: &TurnId) -> Result<Task> {
        let path = format!("{}/tasks/{}:cancel", self.base, wire::percent_encode(task.as_str()));
        parse(self.client.call(Method::POST, &path, Some(json!({"id": task.as_str()})), false)?)
    }

    /// `SubscribeToTask`: re-attach to a running task's stream.
    pub fn subscribe(&self, task: &TurnId) -> Result<TaskEvents> {
        let path = format!("{}/tasks/{}:subscribe", self.base, wire::percent_encode(task.as_str()));
        let res = self.client.call(Method::POST, &path, Some(json!({"id": task.as_str()})), true)?;
        Ok(TaskEvents { reader: SseReader::new(res.into_body()) })
    }
}

fn request(msg: &Outgoing) -> Value {
    let mut parts = vec![Part::text(msg.text.clone())];
    if let Some(patch) = &msg.my_changes {
        parts.push(Part {
            raw: Some(wire::b64_encode(patch.as_bytes())),
            media_type: Some(wire::PATCH.into()),
            filename: Some("my-changes.patch".into()),
            ..Part::default()
        });
    }
    let mut metadata = Map::new();
    if msg.new_conversation {
        metadata.insert(NEW_CONVERSATION_META.into(), Value::Bool(true));
    }
    let request = SendMessageRequest {
        tenant: None,
        message: Message {
            message_id: uuid::Uuid::new_v4().to_string(),
            context_id: msg.conversation.as_ref().map(|c| c.0.clone()),
            task_id: None,
            role: Role::User,
            parts,
            metadata: (!metadata.is_empty()).then_some(metadata),
            extensions: Vec::new(),
            reference_task_ids: Vec::new(),
        },
        configuration: msg
            .return_immediately
            .then(|| SendMessageConfiguration { return_immediately: true, ..SendMessageConfiguration::default() }),
        metadata: None,
    };
    serde_json::to_value(request).unwrap_or(Value::Null)
}

/// The events of a task stream, as they arrive.
pub struct TaskEvents {
    reader: SseReader,
}

impl Iterator for TaskEvents {
    type Item = Result<StreamResponse>;

    fn next(&mut self) -> Option<Self::Item> {
        let event = match self.reader.next()? {
            Ok(e) => e,
            Err(e) => return Some(Err(PwError::unreachable(e.to_string()))),
        };
        Some(serde_json::from_str(&event.data).map_err(|e| PwError::other(format!("unexpected event: {e}"))))
    }
}

/// The owner's core turn behind a task (or a status update's metadata),
/// when the other side is a Pitwall.
pub fn turn_of(task: &Task) -> Option<Turn> {
    turn_in(task.metadata.as_ref())
}

/// The same, from any A2A object's `metadata`.
pub fn turn_in(metadata: Option<&Map<String, Value>>) -> Option<Turn> {
    serde_json::from_value(metadata?.get(TURN_META)?.clone()).ok()
}

/// A task's patch series when it came inline (`raw`); `None` when there is
/// none or it is behind a `url` ([`PitwallClient::fetch`]).
pub fn inline_patch(task: &Task) -> Option<String> {
    let artifact = task.artifacts.iter().find(|a| a.artifact_id == "changes")?;
    let raw = artifact.parts.iter().find_map(|p| p.raw.as_deref())?;
    String::from_utf8(wire::b64_decode(raw)?).ok()
}

/// A task's patch series `url`, when it was too large to come inline.
pub fn patch_url(task: &Task) -> Option<&str> {
    let artifact = task.artifacts.iter().find(|a| a.artifact_id == "changes")?;
    artifact.parts.iter().find_map(|p| p.url.as_deref())
}
