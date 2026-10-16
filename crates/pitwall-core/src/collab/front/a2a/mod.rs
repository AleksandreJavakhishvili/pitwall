//! The A2A front (§8.3.2): A2A 1.0 over the HTTP+JSON binding, one base URL
//! per share, `<endpoint>/a2a/<slug>`.
//!
//! | operation | route |
//! |---|---|
//! | Agent Card | `GET /a2a/<slug>/card` (also `…/.well-known/agent-card.json`) |
//! | `SendMessage` | `POST /a2a/<slug>/message:send` |
//! | `SendStreamingMessage` | `POST /a2a/<slug>/message:stream` (SSE) |
//! | `GetTask` | `GET /a2a/<slug>/tasks/{id}` |
//! | `ListTasks` | `GET /a2a/<slug>/tasks?contextId=…` |
//! | `CancelTask` | `POST /a2a/<slug>/tasks/{id}:cancel` |
//! | `SubscribeToTask` | `POST` or `GET /a2a/<slug>/tasks/{id}:subscribe` (SSE) |
//!
//! A task id is a turn id (`c_8f2k/7`); clients may send its `/` as is or
//! as `%2F`. A `contextId` is a conversation id. Every task carries the
//! whole core [`Turn`] under `metadata["pitwall/turn"]`, so Pitwall's own
//! client reads turns back without loss; other clients use the standard
//! fields.

pub mod model;

use std::thread;
use std::time::Instant;

use serde_json::{json, Map, Value};

use self::model::*;
use super::wire::{self, A2A_JSON, PATCH};
use super::{display_name, short_name, state_name, status_line, turn_id, Ctx};
use crate::collab::turn::{Ask, Asker, ShareCard, Turn, TurnStatus};
use crate::collab::turn::{ChangesState, Front};
use crate::collab::{Capability, ConversationId, A2A_VERSION, MAX_BODY, NEW_CONVERSATION_META};
use crate::error::{ErrorCode, PwError};
use crate::net::http::{Method, StatusCode};
use crate::net::{Body, ClientAuth, Endpoint, Request, Response};

/// The header (or query parameter) naming the A2A version.
pub const VERSION_HEADER: &str = "A2A-Version";
/// Where the whole core turn travels in a task's metadata.
pub const TURN_META: &str = "pitwall/turn";
/// The (informational) extension in a card's capabilities that carries
/// the share's Pitwall data: A2A 1.0's `AgentCard` has no `metadata`.
/// Under the project's domain ([`pitwall_proto::DOMAIN`]).
pub const SHARE_EXTENSION: &str = concat!("https://", pitwall_proto::project_domain!(), "/pitwall/a2a/share/v1");
/// The security scheme name in cards on networks with OAuth.
pub const SECURITY_SCHEME: &str = "pitwall";

pub(super) fn handle(ctx: &Ctx, req: Request, rest: &str) -> Response {
    let (slug, sub) = rest.split_once('/').unwrap_or((rest, ""));
    let Some(slug) = wire::percent_decode(slug).filter(|s| !s.is_empty()) else { return not_found() };
    let method = req.method().clone();
    let asker = ctx.asker(req.headers(), Front::A2a, None);

    if method == Method::GET && (sub == "card" || sub == ".well-known/agent-card.json") {
        return match ctx.share(&asker, &slug) {
            Some(share) => respond(StatusCode::OK, &agent_card(&share, ctx.endpoint.as_ref())),
            None => not_found(),
        };
    }
    if !version_ok(&req) {
        return error(
            StatusCode::BAD_REQUEST,
            "VERSION_NOT_SUPPORTED",
            &format!("A2A version not supported; this agent speaks {A2A_VERSION}"),
            json!({"supportedVersions": A2A_VERSION}),
        );
    }
    match (method.as_str(), sub) {
        ("POST", "message:send") => send(ctx, req, asker, &slug, false),
        ("POST", "message:stream") => send(ctx, req, asker, &slug, true),
        ("GET", "tasks") => list(ctx, &req, &asker, &slug),
        (m, sub) if sub.starts_with("tasks/") => {
            let raw = &sub["tasks/".len()..];
            let (raw, op) = match raw.rsplit_once(':') {
                Some((id, op @ ("cancel" | "subscribe"))) => (id, op),
                _ => (raw, ""),
            };
            let Some(id) = turn_id(raw) else { return task_not_found() };
            let turn = match ctx.conversations().turn(&asker, &id) {
                Ok(t) if t.share == slug => t,
                Ok(_) => return task_not_found(),
                Err(e) => return core_error(e, true),
            };
            match (m, op) {
                ("GET", "") => respond(StatusCode::OK, &to_value(&task_of(ctx, &asker, &turn, true))),
                ("POST", "cancel") => cancel(ctx, &asker, turn),
                ("POST" | "GET", "subscribe") => stream(ctx, asker, turn),
                _ => error(StatusCode::METHOD_NOT_ALLOWED, "UNSUPPORTED_OPERATION", "method not allowed", Value::Null),
            }
        }
        _ => not_found(),
    }
}

/// `A2A-Version` (header, else query parameter) is 1.0. Only major.minor
/// counts; an empty value means 0.3, which isn't served.
fn version_ok(req: &Request) -> bool {
    let v = wire::header_str(req.headers(), VERSION_HEADER)
        .map(str::to_string)
        .or_else(|| wire::query_get(req.uri().query(), VERSION_HEADER))
        .unwrap_or_default();
    let mut parts = v.trim().split('.');
    matches!((parts.next(), parts.next()), (Some("1"), Some("0")))
}

// ------------------------------------------------------------- cards

/// A share's Agent Card (§8.7) as this caller's network reaches it.
pub fn agent_card(share: &ShareCard, endpoint: Option<&Endpoint>) -> Value {
    let base = endpoint.map_or("", |e| e.base_url.trim_end_matches('/'));
    let mut card = json!({
        "name": share.name,
        "description": share.description,
        "version": share.version.to_string(),
        "supportedInterfaces": [{
            "url": format!("{base}/a2a/{}", wire::percent_encode(&share.slug)),
            "protocolBinding": "HTTP+JSON",
            "protocolVersion": A2A_VERSION,
        }],
        "capabilities": {
            "streaming": true,
            "pushNotifications": false,
            "extensions": [{
                "uri": SHARE_EXTENSION,
                "description": "Pitwall shared agent: owner, capability, agent kind, messages left today",
                "required": false,
                "params": share_meta(share),
            }],
        },
        "defaultInputModes": ["text/plain", PATCH],
        "defaultOutputModes": ["text/plain", PATCH],
        "skills": [{
            "id": "ask",
            "name": "Ask",
            "description": skill_description(share.capability),
            "tags": ["code", share.kind.as_str()],
        }],
    });
    if let Some(Endpoint { client_auth: ClientAuth::OAuth { issuer, scopes, .. }, .. }) = endpoint {
        let issuer = issuer.trim_end_matches('/');
        let scope_map: Map<String, Value> = scopes.iter().map(|s| (s.clone(), Value::String(s.clone()))).collect();
        card["securitySchemes"] = json!({
            SECURITY_SCHEME: {
                "oauth2SecurityScheme": {
                    "description": "Sign in with the network this agent is shared on",
                    "flows": {"authorizationCode": {
                        "authorizationUrl": format!("{issuer}/authorize"),
                        "tokenUrl": format!("{issuer}/token"),
                        "scopes": scope_map,
                        "pkceRequired": true,
                    }},
                    "oauth2MetadataUrl": format!("{issuer}/.well-known/oauth-authorization-server"),
                }
            }
        });
        card["securityRequirements"] = json!([{"schemes": {SECURITY_SCHEME: {"list": scopes}}}]);
    }
    card
}

/// `pitwall/share` (§8.7): the share as Pitwall describes it to callers.
pub fn share_meta(share: &ShareCard) -> Value {
    json!({
        "owner": display_name(&share.owner),
        "slug": share.slug,
        "capability": share.capability,
        "kind": share.kind,
        "messagesPerDay": share.messages_per_day,
        "left": share.left,
    })
}

fn skill_description(capability: Capability) -> &'static str {
    match capability {
        Capability::Read => "Send any message; it replies (read only)",
        Capability::Write | Capability::WriteApproval => "Send any message; replies, and patches when it changed code",
    }
}

// ------------------------------------------------------------ sending

fn send(ctx: &Ctx, req: Request, asker: Asker, slug: &str, streaming: bool) -> Response {
    let body = match req.into_body().read_all(MAX_BODY) {
        Ok(b) => b,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "INVALID_ARGUMENT", "message too large", Value::Null),
    };
    let request: SendMessageRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return invalid(&format!("not a SendMessageRequest: {e}")),
    };
    let ask = match ask_of(&request, slug) {
        Ok(ask) => ask,
        Err(resp) => return *resp,
    };
    let refused = ctx
        .share(&asker, slug)
        .and_then(|share| ctx.loop_refusal(&asker, &share, ask.conversation.as_ref().map(|c| c.as_str())));
    let turn = match refused {
        Some(turn) => turn,
        None => match ctx.conversations().ask(&asker, ask) {
            Ok(turn) => turn,
            Err(e) => return core_error(e, false),
        },
    };
    if streaming {
        return stream(ctx, asker, turn);
    }
    let immediately = request.configuration.as_ref().is_some_and(|c| c.return_immediately);
    let mut turn = turn;
    if !immediately && !turn.status.is_final() {
        if let Some(_slot) = ctx.open_stream() {
            let deadline = Instant::now() + ctx.options().send_wait;
            let settled = |t: &Turn| matches!(t.status, TurnStatus::WaitingForOwner | TurnStatus::ChangesHeld);
            if let Ok((t, _)) = ctx.follow(&asker, &turn.id, Some(deadline), settled, |_| true, || true) {
                turn = t;
            }
        }
    }
    let response = SendMessageResponse { task: Some(task_of(ctx, &asker, &turn, true)), message: None };
    respond(StatusCode::OK, &to_value(&response))
}

/// The core's [`Ask`] from an A2A message: text parts are the caller's
/// text, verbatim; `text/x-patch` file parts are "redo on my code" (§7.5);
/// anything else is refused.
fn ask_of(request: &SendMessageRequest, slug: &str) -> std::result::Result<Ask, Box<Response>> {
    let message = &request.message;
    if message.task_id.as_deref().is_some_and(|t| !t.is_empty()) {
        return Err(Box::new(error(
            StatusCode::BAD_REQUEST,
            "UNSUPPORTED_OPERATION",
            "each message starts a new task; send it without taskId",
            Value::Null,
        )));
    }
    let mut texts = Vec::new();
    let mut patches = Vec::new();
    for part in &message.parts {
        let is_patch = part.media_type.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(PATCH));
        match (part, is_patch) {
            (Part { raw: Some(raw), .. }, true) => {
                match wire::b64_decode(raw).and_then(|b| String::from_utf8(b).ok()) {
                    Some(p) => patches.push(p),
                    None => return Err(Box::new(invalid("a text/x-patch part is not base64 UTF-8 text"))),
                }
            }
            (Part { text: Some(t), .. }, true) => patches.push(t.clone()),
            (Part { text: Some(t), .. }, false) => texts.push(t.clone()),
            _ => {
                return Err(Box::new(error(
                    StatusCode::BAD_REQUEST,
                    "CONTENT_TYPE_NOT_SUPPORTED",
                    "only text parts and text/x-patch file parts are accepted",
                    Value::Null,
                )))
            }
        }
    }
    let text = texts.join("\n");
    if text.trim().is_empty() && patches.is_empty() {
        return Err(Box::new(invalid("the message has no text")));
    }
    let flag = |m: &Option<Map<String, Value>>| {
        m.as_ref().and_then(|m| m.get(NEW_CONVERSATION_META)).and_then(Value::as_bool).unwrap_or(false)
    };
    Ok(Ask {
        share: slug.to_string(),
        message: text,
        conversation: message.context_id.clone().filter(|c| !c.is_empty()).map(ConversationId),
        new_conversation: flag(&message.metadata) || flag(&request.metadata),
        my_changes: (!patches.is_empty()).then(|| patches.join("")),
    })
}

fn cancel(ctx: &Ctx, asker: &Asker, turn: Turn) -> Response {
    if turn.status.is_final() {
        return error(StatusCode::BAD_REQUEST, "TASK_NOT_CANCELABLE", "the task has already ended", Value::Null);
    }
    match ctx.conversations().cancel(asker, &turn.id) {
        Ok(turn) => respond(StatusCode::OK, &to_value(&task_of(ctx, asker, &turn, true))),
        Err(e) => core_error(e, true),
    }
}

fn list(ctx: &Ctx, req: &Request, asker: &Asker, slug: &str) -> Response {
    let q = req.uri().query();
    let context = wire::query_get(q, "contextId").filter(|c| !c.is_empty()).map(ConversationId);
    let mut turns = match ctx.conversations().turns(asker, context.as_ref()) {
        Ok(t) => t,
        Err(e) => return core_error(e, false),
    };
    turns.retain(|t| t.share == slug);
    if let Some(state) = wire::query_get(q, "status").filter(|s| !s.is_empty()) {
        turns.retain(|t| task_state(&t.status).name() == state);
    }
    turns.reverse(); // newest first
    let total = turns.len();
    let size = wire::query_get(q, "pageSize").and_then(|s| s.parse::<usize>().ok()).unwrap_or(50).clamp(1, 100);
    let from = wire::query_get(q, "pageToken").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0).min(total);
    let artifacts = wire::query_get(q, "includeArtifacts").is_some_and(|v| v == "true");
    let page: Vec<Task> = turns.iter().skip(from).take(size).map(|t| task_of(ctx, asker, t, artifacts)).collect();
    let next = if from + page.len() < total { (from + page.len()).to_string() } else { String::new() };
    let response = ListTasksResponse {
        page_size: page.len() as i32,
        tasks: page,
        next_page_token: next,
        total_size: total as i32,
    };
    respond(StatusCode::OK, &to_value(&response))
}

// ---------------------------------------------------------- streaming

/// `message:stream` and `SubscribeToTask`: the task first, then status and
/// artifact updates until the turn is final or the caller goes away.
fn stream(ctx: &Ctx, asker: Asker, turn: Turn) -> Response {
    let Some(slot) = ctx.open_stream() else {
        return error(StatusCode::TOO_MANY_REQUESTS, "RESOURCE_EXHAUSTED", "too many open streams", Value::Null);
    };
    let (tx, body) = Body::channel();
    let ctx = ctx.clone();
    thread::spawn(move || {
        let _slot = slot;
        let w = wire::SseWriter(tx);
        let task = task_of(&ctx, &asker, &turn, true);
        let mut sent = Sent::of(&task, &turn);
        if !w.json(None, &to_value(&StreamResponse { task: Some(task), ..StreamResponse::default() })) {
            return;
        }
        if turn.status.is_final() {
            return;
        }
        let _ = ctx.follow(&asker, &turn.id, None, |_| false, |t| sent.emit(&ctx, &asker, t, &w), || w.keepalive());
    });
    let mut res = wire::sse(body);
    wire::set(&mut res, VERSION_HEADER, A2A_VERSION);
    res
}

/// What a stream already told the caller.
struct Sent {
    reply: bool,
    changes: bool,
    status: (TaskState, String),
}

impl Sent {
    fn of(task: &Task, turn: &Turn) -> Sent {
        let has = |id: &str| task.artifacts.iter().any(|a| a.artifact_id == id);
        Sent { reply: has("reply"), changes: has("changes"), status: (task.status.state, status_key(turn)) }
    }

    /// Updates for one change of the turn: new artifacts, then the status.
    fn emit(&mut self, ctx: &Ctx, asker: &Asker, turn: &Turn, w: &wire::SseWriter) -> bool {
        let (task_id, context_id) = (turn.id.0.clone(), turn.conversation.0.clone());
        let mut artifacts = Vec::new();
        if !self.reply {
            if let Some(a) = reply_artifact(turn) {
                self.reply = true;
                artifacts.push(a);
            }
        }
        if !self.changes {
            if let Some(a) = changes_artifact(ctx, asker, turn) {
                self.changes = true;
                artifacts.push(a);
            }
        }
        for artifact in artifacts {
            let update = TaskArtifactUpdateEvent {
                task_id: task_id.clone(),
                context_id: context_id.clone(),
                artifact,
                append: false,
                last_chunk: true,
                metadata: None,
            };
            let event = StreamResponse { artifact_update: Some(update), ..StreamResponse::default() };
            if !w.json(None, &to_value(&event)) {
                return false;
            }
        }
        let status = (task_state(&turn.status), status_key(turn));
        if status != self.status {
            self.status = status;
            let update = TaskStatusUpdateEvent {
                task_id,
                context_id,
                status: task_status(ctx, asker, turn),
                metadata: Some(turn_meta(turn)),
            };
            let event = StreamResponse { status_update: Some(update), ..StreamResponse::default() };
            return w.json(None, &to_value(&event));
        }
        true
    }
}

/// What makes a status worth an update: the state, its details and line.
fn status_key(turn: &Turn) -> String {
    format!("{}|{:?}|{:?}", state_name(&turn.status), turn.status, turn.status_message)
}

// ------------------------------------------------------- turn → task

/// The A2A state of a turn (the table on [`TurnStatus`]).
pub fn task_state(status: &TurnStatus) -> TaskState {
    match status {
        TurnStatus::Queued { .. } => TaskState::Submitted,
        TurnStatus::Working | TurnStatus::WaitingForOwner | TurnStatus::ChangesHeld => TaskState::Working,
        TurnStatus::Done => TaskState::Completed,
        TurnStatus::Failed { .. } => TaskState::Failed,
        TurnStatus::Canceled => TaskState::Canceled,
        TurnStatus::Refused(_) => TaskState::Rejected,
    }
}

fn owner_name(ctx: &Ctx, asker: &Asker, slug: &str) -> String {
    ctx.share(asker, slug).map_or_else(|| "the owner".to_string(), |s| short_name(&s.owner))
}

fn task_status(ctx: &Ctx, asker: &Asker, turn: &Turn) -> TaskStatus {
    let line = status_line(turn, &owner_name(ctx, asker, &turn.share));
    let data = serde_json::to_value(&turn.status).unwrap_or(Value::Null);
    let timestamp = turn.done_at.clone().or_else(|| Some(turn.at.clone())).filter(|t| !t.is_empty());
    TaskStatus {
        state: task_state(&turn.status),
        message: Some(Message {
            message_id: format!("{}#{}", turn.id.0, state_name(&turn.status)),
            context_id: Some(turn.conversation.0.clone()),
            task_id: Some(turn.id.0.clone()),
            role: Role::Agent,
            parts: vec![Part::text(line), Part::data(data)],
            metadata: None,
            extensions: Vec::new(),
            reference_task_ids: Vec::new(),
        }),
        timestamp,
    }
}

fn turn_meta(turn: &Turn) -> Map<String, Value> {
    let mut meta = Map::new();
    meta.insert(TURN_META.into(), serde_json::to_value(turn).unwrap_or(Value::Null));
    meta
}

/// A turn as an A2A task.
pub(super) fn task_of(ctx: &Ctx, asker: &Asker, turn: &Turn, artifacts: bool) -> Task {
    let mut list = Vec::new();
    if artifacts {
        list.extend(reply_artifact(turn));
        list.extend(changes_artifact(ctx, asker, turn));
    }
    Task {
        id: turn.id.0.clone(),
        context_id: turn.conversation.0.clone(),
        status: task_status(ctx, asker, turn),
        artifacts: list,
        history: Vec::new(),
        metadata: Some(turn_meta(turn)),
    }
}

/// `reply`: what the agent said, and where it was read from.
fn reply_artifact(turn: &Turn) -> Option<Artifact> {
    let reply = turn.reply.as_ref()?;
    Some(Artifact {
        artifact_id: "reply".into(),
        name: Some("reply".into()),
        description: None,
        parts: vec![Part::text(reply.text.clone()), Part::data(json!({"source": reply.source}))],
        metadata: None,
    })
}

/// `changes` (only once sent): the patch series as a file (`raw` up to
/// 2 MiB, else a `url` to `/pitwall/v1/results/{task}`) and its summary.
fn changes_artifact(ctx: &Ctx, asker: &Asker, turn: &Turn) -> Option<Artifact> {
    let changes = turn.changes.as_ref().filter(|c| c.state == ChangesState::Sent)?;
    let patch = ctx.conversations().patch(asker, &turn.id).ok()?;
    let filename = format!("turn-{}.patch", turn.id.number().unwrap_or(0));
    let mut file = Part { media_type: Some(PATCH.into()), filename: Some(filename), ..Part::default() };
    if patch.len() <= MAX_BODY {
        file.raw = Some(wire::b64_encode(patch.as_bytes()));
    } else {
        file.url = Some(results_url(ctx.base_url(), &turn.id.0));
    }
    let summary = json!({
        "baseCommit": changes.base_commit,
        "files": changes.files,
        "added": changes.added,
        "removed": changes.removed,
        "patches": changes.patches,
    });
    Some(Artifact {
        artifact_id: "changes".into(),
        name: Some("changes".into()),
        description: None,
        parts: vec![file, Part::data(summary)],
        metadata: None,
    })
}

/// Where a large patch series is fetched.
pub fn results_url(base: &str, task: &str) -> String {
    format!("{}/pitwall/v1/results/{}", base.trim_end_matches('/'), wire::percent_encode(task))
}

// ------------------------------------------------------------ errors

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn respond(status: StatusCode, value: &Value) -> Response {
    let mut res = wire::json_as(status, A2A_JSON, value);
    wire::set(&mut res, VERSION_HEADER, A2A_VERSION);
    res
}

/// An A2A error in the HTTP+JSON binding's shape (`google.rpc.Status`
/// with an `ErrorInfo`).
fn error(status: StatusCode, reason: &str, message: &str, metadata: Value) -> Response {
    let mut info = json!({
        "@type": "type.googleapis.com/google.rpc.ErrorInfo",
        "reason": reason,
        "domain": "a2a-protocol.org",
    });
    if let Value::Object(m) = metadata {
        info["metadata"] =
            // `ErrorInfo.metadata` is a map of strings.
            Value::Object(
                m.into_iter()
                    .map(|(k, v)| (k, Value::String(v.as_str().map_or_else(|| v.to_string(), str::to_string))))
                    .collect::<Map<_, _>>(),
            );
    }
    let body = json!({"error": {
        "code": status.as_u16(),
        "status": rpc_status(status),
        "message": message,
        "details": [info],
    }});
    respond(status, &body)
}

fn rpc_status(status: StatusCode) -> &'static str {
    match status.as_u16() {
        400 | 413 => "INVALID_ARGUMENT",
        401 => "UNAUTHENTICATED",
        403 => "PERMISSION_DENIED",
        404 => "NOT_FOUND",
        405 | 501 => "UNIMPLEMENTED",
        409 => "ABORTED",
        429 => "RESOURCE_EXHAUSTED",
        503 => "UNAVAILABLE",
        _ => "INTERNAL",
    }
}

fn invalid(message: &str) -> Response {
    error(StatusCode::BAD_REQUEST, "INVALID_ARGUMENT", message, Value::Null)
}

/// Not in the audience, no such share, no such context: one answer.
fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "NOT_FOUND", "not found", Value::Null)
}

fn task_not_found() -> Response {
    error(StatusCode::NOT_FOUND, "TASK_NOT_FOUND", "task not found", Value::Null)
}

fn core_error(e: PwError, task: bool) -> Response {
    match e.code {
        ErrorCode::NotFound if task => task_not_found(),
        ErrorCode::NotFound => not_found(),
        ErrorCode::Unreachable | ErrorCode::NotRunning => {
            error(StatusCode::SERVICE_UNAVAILABLE, "UNAVAILABLE", &e.message, Value::Null)
        }
        ErrorCode::Unsupported => error(StatusCode::BAD_REQUEST, "UNSUPPORTED_OPERATION", &e.message, Value::Null),
        ErrorCode::Denied => error(StatusCode::FORBIDDEN, "PERMISSION_DENIED", &e.message, Value::Null),
        ErrorCode::Conflict => error(StatusCode::CONFLICT, "ABORTED", &e.message, Value::Null),
        ErrorCode::Other => error(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL", &e.message, Value::Null),
    }
}
