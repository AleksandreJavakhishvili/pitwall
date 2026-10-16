//! The MCP front (§8.3.3): one endpoint per owner and network, `/mcp`,
//! Streamable HTTP, one tool per share this caller may use plus
//! `check_turn` and `get_changes`.
//!
//! Both MCP eras are served on the same endpoint ("dual-era", MCP
//! 2026-07-28 "Versioning"):
//! - **legacy** (2025-03-26, 2025-06-18, 2025-11-25): an `initialize`
//!   handshake (versions negotiated there), an `Mcp-Session-Id` minted at
//!   `initialize` (transport only: conversations are keyed by the caller,
//!   §6.1), `GET /mcp` for `notifications/tools/list_changed`,
//!   `notifications/cancelled` to stop a running call;
//! - **modern** (2026-07-28): no handshake, every request carries
//!   `_meta["io.modelcontextprotocol/protocolVersion"]` mirrored in the
//!   `MCP-Protocol-Version` / `Mcp-Method` / `Mcp-Name` headers,
//!   `server/discover`, `subscriptions/listen` for list changes, and
//!   closing a call's SSE stream cancels it.
//!
//! A tool call answers as JSON, or as an SSE stream when the client gave a
//! `progressToken` (progress while waiting: "2nd in line", "working",
//! "waiting for alice") and always in the modern era (so closing the
//! stream can cancel). Calls wait up to `wait_seconds` (default 120, max
//! 240: below Claude Code's 5-minute idle timeout for HTTP tools).
//!
//! Not in the audience and unknown tool look the same: the tool isn't
//! listed, and calling it is the JSON-RPC error "Unknown tool".

use std::collections::HashMap;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use super::wire::{self, SseWriter};
use super::{display_name, lock, short_name, status_line, Ctx};
use crate::collab::turn::{Ask, Asker, ChangesState, Failure, Front, ShareCard, Turn, TurnId, TurnStatus};
use crate::collab::{Capability, CHANGES_PART_CHARS, MAX_BODY, WAIT_SECONDS_DEFAULT, WAIT_SECONDS_MAX};
use crate::error::ErrorCode;
use crate::net::http::{header, Method, StatusCode};
use crate::net::{Body, ClientAuth, Endpoint, PersonId, Request, Response};

/// Initialization-based versions, newest first (the one offered when a
/// client asks for something else).
pub const LEGACY_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
/// Per-request-metadata versions.
pub const MODERN_VERSIONS: [&str; 1] = ["2026-07-28"];

pub const SESSION_HEADER: &str = "Mcp-Session-Id";
pub const VERSION_HEADER: &str = "MCP-Protocol-Version";
pub const METHOD_HEADER: &str = "Mcp-Method";
pub const NAME_HEADER: &str = "Mcp-Name";
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT: &str = "io.modelcontextprotocol/clientInfo";
const META_SERVER: &str = "io.modelcontextprotocol/serverInfo";
const META_SUBSCRIPTION: &str = "io.modelcontextprotocol/subscriptionId";

/// RFC 9728 protected resource metadata, at the root and for `/mcp`.
pub const RESOURCE_METADATA_ROOT: &str = "/.well-known/oauth-protected-resource";
pub const RESOURCE_METADATA_PATH: &str = "/.well-known/oauth-protected-resource/mcp";

/// JSON-RPC and MCP error codes used here.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
const HEADER_MISMATCH: i64 = -32020;
const UNSUPPORTED_VERSION: i64 = -32022;

/// Freshness hints for modern clients' caches.
const DISCOVER_TTL_MS: u64 = 300_000;
const TOOLS_TTL_MS: u64 = 30_000;

/// Sessions kept per person and in all (legacy clients).
const SESSIONS_PER_PERSON: usize = 16;
const SESSIONS_MAX: usize = 4096;

#[derive(Default)]
pub(crate) struct State {
    sessions: Mutex<HashMap<String, Session>>,
    /// Legacy calls still waiting, for `notifications/cancelled`: (person,
    /// session or "", JSON-RPC id) → the turn and who asked.
    running: Mutex<HashMap<RunningKey, (TurnId, Asker)>>,
}

/// (person, session or "", JSON-RPC id as JSON text).
type RunningKey = (PersonId, String, String);

struct Session {
    person: PersonId,
    client: Option<String>,
    version: String,
    at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Era {
    Legacy { version: String, session: Option<String> },
    Modern { version: String },
}

impl Era {
    fn modern(&self) -> bool {
        matches!(self, Era::Modern { .. })
    }
}

pub(super) fn handle(ctx: &Ctx, req: Request) -> Response {
    match *req.method() {
        Method::POST => post(ctx, req),
        Method::GET => {
            let accepts = wire::header_str(req.headers(), header::ACCEPT.as_str())
                .is_some_and(|a| a.contains(wire::EVENT_STREAM));
            if !accepts {
                return wire::empty(StatusCode::METHOD_NOT_ALLOWED);
            }
            if let Some(id) = wire::header_str(req.headers(), SESSION_HEADER) {
                if session(ctx, id).is_none() {
                    return rpc_http(StatusCode::NOT_FOUND, error(Value::Null, INVALID_REQUEST, "unknown session"));
                }
            }
            let asker = ctx.asker(req.headers(), Front::Mcp, None);
            listen(ctx, asker, None, true)
        }
        Method::DELETE => {
            let Some(id) = wire::header_str(req.headers(), SESSION_HEADER) else {
                return wire::empty(StatusCode::METHOD_NOT_ALLOWED);
            };
            let mut sessions = lock(&ctx.inner.mcp.sessions);
            match sessions.get(id) {
                Some(s) if &s.person == ctx.caller.person() => {
                    sessions.remove(id);
                    wire::empty(StatusCode::NO_CONTENT)
                }
                _ => wire::empty(StatusCode::NOT_FOUND),
            }
        }
        _ => wire::empty(StatusCode::METHOD_NOT_ALLOWED),
    }
}

/// The client's name and version of a live legacy session of this person.
fn session(ctx: &Ctx, id: &str) -> Option<(Option<String>, String)> {
    let sessions = lock(&ctx.inner.mcp.sessions);
    sessions.get(id).filter(|s| &s.person == ctx.caller.person()).map(|s| (s.client.clone(), s.version.clone()))
}

fn post(ctx: &Ctx, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let headers = parts.headers;
    let body = match body.read_all(MAX_BODY) {
        Ok(b) => b,
        Err(_) => {
            return rpc_http(StatusCode::PAYLOAD_TOO_LARGE, error(Value::Null, INVALID_REQUEST, "request too large"))
        }
    };
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return rpc_http(StatusCode::BAD_REQUEST, error(Value::Null, PARSE_ERROR, "parse error")),
    };
    let Some(obj) = msg.as_object() else {
        return rpc_http(StatusCode::BAD_REQUEST, error(Value::Null, INVALID_REQUEST, "batches are not supported"));
    };
    let Some(method) = obj.get("method").and_then(Value::as_str).map(str::to_string) else {
        // A response from the client: Pitwall never asks clients anything.
        return wire::empty(StatusCode::ACCEPTED);
    };
    let id = obj.get("id").cloned();
    let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));
    let meta = params.get("_meta").and_then(Value::as_object);
    let header = |name: &str| wire::header_str(&headers, name).map(str::to_string);

    // Which era, and is the request well formed for it?
    let modern = meta.and_then(|m| m.get(META_VERSION)).and_then(Value::as_str).map(str::to_string);
    let era = if method == "initialize" {
        Era::Legacy { version: String::new(), session: None }
    } else if let Some(version) = modern {
        if !MODERN_VERSIONS.contains(&version.as_str()) {
            return unsupported_version(id, &version);
        }
        if header(VERSION_HEADER).as_deref() != Some(version.as_str()) {
            return mismatch(id, "MCP-Protocol-Version header missing or not the body's protocol version");
        }
        if header(METHOD_HEADER).as_deref() != Some(method.as_str()) {
            return mismatch(id, "Mcp-Method header missing or not the body's method");
        }
        if method == "tools/call" {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            if header(NAME_HEADER).map(|h| decode_sentinel(&h)).as_deref() != Some(name) {
                return mismatch(id, "Mcp-Name header missing or not the body's tool name");
            }
        }
        Era::Modern { version }
    } else {
        let mut version = header(VERSION_HEADER);
        let session = header(SESSION_HEADER);
        if let Some(sid) = &session {
            match self::session(ctx, sid) {
                Some((_, v)) => version = version.or(Some(v)),
                None => {
                    return rpc_http(
                        StatusCode::NOT_FOUND,
                        error(
                            id.unwrap_or(Value::Null),
                            INVALID_REQUEST,
                            "unknown or expired session; initialize again",
                        ),
                    )
                }
            }
        }
        let version = version.unwrap_or_else(|| "2025-03-26".to_string());
        if MODERN_VERSIONS.contains(&version.as_str()) {
            return mismatch(id, "requests at this protocol version carry _meta with the version");
        }
        if !LEGACY_VERSIONS.contains(&version.as_str()) {
            return unsupported_version(id, &version);
        }
        Era::Legacy { version, session }
    };

    let client = match &era {
        Era::Modern { .. } => meta
            .and_then(|m| m.get(META_CLIENT))
            .and_then(|c| c.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        Era::Legacy { session: Some(sid), .. } => self::session(ctx, sid).and_then(|(c, _)| c),
        Era::Legacy { .. } => None,
    };

    let Some(id) = id else {
        // A notification.
        if method == "notifications/cancelled" {
            if let Era::Legacy { session, .. } = &era {
                cancel_running(ctx, session.as_deref(), params.get("requestId"));
            }
        }
        return wire::empty(StatusCode::ACCEPTED);
    };

    let asker = ctx.asker(&headers, Front::Mcp, client);
    match method.as_str() {
        "initialize" => initialize(ctx, id, &params),
        "server/discover" if era.modern() => {
            let result = json!({
                "supportedVersions": all_versions(),
                "capabilities": capabilities(),
                "_meta": {META_SERVER: server_info(ctx)},
                "instructions": instructions(ctx),
                // Caching hints, required on modern list results (MCP
                // 2026-07-28 "Caching"); Claude Code rejects the result
                // without them.
                "ttlMs": DISCOVER_TTL_MS,
                "cacheScope": "private",
            });
            ok(&era, id, result)
        }
        "ping" => ok(&era, id, json!({})),
        "tools/list" => {
            let mut result = json!({"tools": tools(ctx, &asker)});
            if era.modern() {
                // Per caller (private); short-lived because descriptions
                // carry "N messages left today". list_changed invalidates.
                result["ttlMs"] = json!(TOOLS_TTL_MS);
                result["cacheScope"] = json!("private");
            }
            ok(&era, id, result)
        }
        "tools/call" => call(ctx, era, id, &params, asker),
        "subscriptions/listen" if era.modern() => {
            let wanted = params.pointer("/notifications/toolsListChanged").and_then(Value::as_bool).unwrap_or(false);
            listen(ctx, asker, Some(id), wanted)
        }
        _ => {
            let status = if era.modern() { StatusCode::NOT_FOUND } else { StatusCode::OK };
            rpc_http(status, error(id, METHOD_NOT_FOUND, &format!("Method not found: {method}")))
        }
    }
}

fn all_versions() -> Vec<&'static str> {
    MODERN_VERSIONS.iter().chain(LEGACY_VERSIONS.iter()).copied().collect()
}

fn capabilities() -> Value {
    json!({"tools": {"listChanged": true}})
}

fn server_info(ctx: &Ctx) -> Value {
    json!({"name": "pitwall", "title": "Pitwall", "version": ctx.options().version})
}

fn instructions(ctx: &Ctx) -> String {
    let owner = display_name(&ctx.teams().info(&ctx.caller).person);
    format!(
        "Agents {owner} shares with you, one ask_<agent> tool each. A message goes into your conversation with \
         that agent, which remembers earlier messages. A long turn comes back with status \"working\" and a turn \
         id: call check_turn with it to wait more. Changes come back as a git format-patch series (apply with \
         `git am -3`); big ones through get_changes."
    )
}

fn initialize(ctx: &Ctx, id: Value, params: &Value) -> Response {
    let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
    let version = if LEGACY_VERSIONS.contains(&requested) { requested } else { LEGACY_VERSIONS[0] };
    let client = params.pointer("/clientInfo/name").and_then(Value::as_str).map(str::to_string);
    let sid = uuid::Uuid::new_v4().simple().to_string();
    {
        let mut sessions = lock(&ctx.inner.mcp.sessions);
        let person = ctx.caller.person().clone();
        let mine: Vec<(String, Instant)> =
            sessions.iter().filter(|(_, s)| s.person == person).map(|(k, s)| (k.clone(), s.at)).collect();
        if mine.len() >= SESSIONS_PER_PERSON {
            if let Some((oldest, _)) = mine.iter().min_by_key(|(_, at)| *at) {
                sessions.remove(oldest);
            }
        }
        if sessions.len() >= SESSIONS_MAX {
            if let Some(oldest) = sessions.iter().min_by_key(|(_, s)| s.at).map(|(k, _)| k.clone()) {
                sessions.remove(&oldest);
            }
        }
        sessions.insert(sid.clone(), Session { person, client, version: version.to_string(), at: Instant::now() });
    }
    let result = json!({
        "protocolVersion": version,
        "capabilities": capabilities(),
        "serverInfo": server_info(ctx),
        "instructions": instructions(ctx),
    });
    let mut res = rpc_http(StatusCode::OK, json!({"jsonrpc": "2.0", "id": id, "result": result}));
    wire::set(&mut res, SESSION_HEADER, &sid);
    res
}

// --------------------------------------------------------------- tools

/// `ask_payments_expert` for `payments-expert`.
pub fn tool_name(slug: &str) -> String {
    let name: String =
        slug.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    format!("ask_{name}")
}

/// The tools this caller sees: one per share, plus the two helpers when
/// there is at least one.
fn tools(ctx: &Ctx, asker: &Asker) -> Vec<Value> {
    let shares = ctx.conversations().shares(asker);
    if shares.is_empty() {
        return Vec::new();
    }
    let mut tools: Vec<Value> = shares.iter().map(share_tool).collect();
    tools.push(json!({
        "name": "check_turn",
        "title": "Check a turn",
        "description": "The state of a turn that was still working, waiting for its owner, or holding changes; \
                        waits up to wait_seconds for it to finish, like ask_<agent>.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "turn": {"type": "string", "description": "The turn id an earlier result gave"},
                "wait_seconds": wait_schema(),
            },
            "required": ["turn"],
        },
        "outputSchema": turn_output_schema(),
        "annotations": {"readOnlyHint": true, "openWorldHint": false},
    }));
    tools.push(json!({
        "name": "get_changes",
        "title": "Get a turn's changes",
        "description": format!(
            "A turn's patch series in parts of {CHANGES_PART_CHARS} characters, for series too big to come \
             inline. Concatenate the parts in order and apply with `git am -3`."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "turn": {"type": "string", "description": "The turn id"},
                "part": {"type": "integer", "minimum": 1, "default": 1, "description": "Which part, from 1"},
            },
            "required": ["turn"],
        },
        "outputSchema": {
            "type": "object",
            "properties": {
                "turn": {"type": "string"},
                "part": {"type": "integer"},
                "parts": {"type": "integer"},
                "patch": {"type": "string"},
                "error": {"type": "string"},
            },
        },
        "annotations": {"readOnlyHint": true, "openWorldHint": false},
    }));
    tools
}

fn wait_schema() -> Value {
    json!({
        "type": "integer", "minimum": 0, "maximum": WAIT_SECONDS_MAX, "default": WAIT_SECONDS_DEFAULT,
        "description": "How long to wait for the turn before returning with status \"working\"",
    })
}

/// A share as an MCP tool (§8.7).
pub fn share_tool(share: &ShareCard) -> Value {
    let read_only = share.capability == Capability::Read;
    let mut annotations = json!({"readOnlyHint": read_only, "openWorldHint": false});
    if !read_only {
        // It changes its own copy; nothing on the caller's side.
        annotations["destructiveHint"] = json!(false);
    }
    json!({
        "name": tool_name(&share.slug),
        "title": format!("{}/{}", short_name(&share.owner), share.slug),
        "description": tool_description(share),
        "inputSchema": {
            "type": "object",
            "properties": {
                "message": {"type": "string", "description": "Your message to the agent, in your own words"},
                "new_conversation": {"type": "boolean", "description": "Start a new conversation instead of continuing yours"},
                "wait_seconds": wait_schema(),
                "my_changes": {"type": "string", "description": "a git format-patch series to apply to the agent's copy first"},
            },
            "required": ["message"],
        },
        "outputSchema": turn_output_schema(),
        "annotations": annotations,
        "_meta": {"pitwall/share": super::a2a::share_meta(share)},
    })
}

/// The `ask_<slug>` description, from the share (§8.3.3).
pub fn tool_description(share: &ShareCard) -> String {
    let mut about = share.description.trim().trim_end_matches('.').to_string();
    if about.is_empty() {
        about = "a shared agent".into();
    }
    let capability = match share.capability {
        Capability::Read => "Read only.",
        Capability::Write => "Can change code; changes come back as a patch.",
        Capability::WriteApproval => "Can change code; changes come back as a patch after the owner approves them.",
    };
    format!(
        "{} ({}): {about}. {capability} Runs on {}. One conversation per caller; it remembers earlier messages. \
         {} of {} messages left today.",
        share.name,
        display_name(&share.owner),
        share.kind_name,
        share.left,
        share.messages_per_day,
    )
}

fn turn_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "conversation": {"type": "string"},
            "turn": {"type": "string"},
            "status": {"type": "string", "enum": ["done", "working", "waiting_for_owner", "failed"]},
            "statusMessage": {"type": "string"},
            "reply": {"type": "string"},
            "replySource": {"type": "string", "enum": ["hook", "session-file", "non-interactive", "terminal"]},
            "failure": {"type": "string", "enum": ["no_answer", "timeout", "crashed", "canceled"]},
            "changes": {
                "type": "object",
                "properties": {
                    "state": {"type": "string", "enum": ["sent", "held", "discarded"]},
                    "baseCommit": {"type": "string"},
                    "files": {"type": "integer"},
                    "added": {"type": "integer"},
                    "removed": {"type": "integer"},
                    "patches": {"type": "integer"},
                    "patch": {"type": "string"},
                    "parts": {"type": "integer"},
                },
            },
            "left": {
                "type": "object",
                "properties": {"messagesToday": {"type": "integer"}, "resetsAt": {"type": "string"}},
            },
            "refused": {"type": "string", "enum": ["quota", "busy", "paused", "share_total", "agent_limit", "loop"]},
            "message": {"type": "string"},
            "resetsAt": {"type": "string"},
            "until": {"type": "string"},
            "error": {"type": "string"},
        },
        "required": ["status"],
    })
}

// --------------------------------------------------------------- calls

fn call(ctx: &Ctx, era: Era, id: Value, params: &Value, asker: Asker) -> Response {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let token = params.pointer("/_meta/progressToken").filter(|t| t.is_string() || t.is_i64() || t.is_u64()).cloned();
    let wait = match args.get("wait_seconds") {
        None | Some(Value::Null) => WAIT_SECONDS_DEFAULT,
        Some(v) => match v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)) {
            Some(n) => n.clamp(0, i64::from(WAIT_SECONDS_MAX)) as u32,
            None => return rpc_http(StatusCode::OK, error(id, INVALID_PARAMS, "wait_seconds must be an integer")),
        },
    };
    let unknown = |id: Value| rpc_http(StatusCode::OK, error(id, INVALID_PARAMS, &format!("Unknown tool: {name}")));

    match name.as_str() {
        "check_turn" | "get_changes" => {
            if ctx.conversations().shares(&asker).is_empty() {
                return unknown(id);
            }
            let Some(turn) = args.get("turn").and_then(Value::as_str).filter(|t| !t.is_empty()) else {
                return rpc_http(StatusCode::OK, error(id, INVALID_PARAMS, "turn is required"));
            };
            let turn_id = TurnId(turn.to_string());
            if name == "get_changes" {
                let part = args.get("part").and_then(Value::as_u64).unwrap_or(1).max(1) as u32;
                return ok(&era, id, changes_part(ctx, &asker, &turn_id, part));
            }
            match ctx.conversations().turn(&asker, &turn_id) {
                Ok(turn) => wait_and_answer(ctx, era, id, asker, turn, wait, token),
                Err(e) if e.is(ErrorCode::NotFound) => ok(&era, id, error_result(&format!("No such turn: {turn}"))),
                Err(e) => rpc_http(StatusCode::OK, error(id, INTERNAL_ERROR, &e.message)),
            }
        }
        _ => {
            let Some(share) = ctx.conversations().shares(&asker).into_iter().find(|s| tool_name(&s.slug) == name)
            else {
                return unknown(id);
            };
            let Some(message) = args.get("message").and_then(Value::as_str) else {
                return rpc_http(StatusCode::OK, error(id, INVALID_PARAMS, "message is required"));
            };
            if let Some(turn) = ctx.loop_refusal(&asker, &share, None) {
                return ok(&era, id, turn_result(ctx, &asker, &turn, &short_name(&share.owner)));
            }
            let ask = Ask {
                share: share.slug.clone(),
                message: message.to_string(),
                conversation: None,
                new_conversation: args.get("new_conversation").and_then(Value::as_bool).unwrap_or(false),
                my_changes: args
                    .get("my_changes")
                    .and_then(Value::as_str)
                    .filter(|p| !p.is_empty())
                    .map(str::to_string),
            };
            match ctx.conversations().ask(&asker, ask) {
                Ok(turn) => wait_and_answer(ctx, era, id, asker, turn, wait, token),
                Err(e) if e.is(ErrorCode::NotFound) => unknown(id),
                Err(e) => rpc_http(StatusCode::OK, error(id, INTERNAL_ERROR, &e.message)),
            }
        }
    }
}

/// Wait for the turn (up to `wait` seconds), then answer: as JSON, or as
/// SSE with progress when there is a progress token or the era cancels by
/// closing the stream.
fn wait_and_answer(
    ctx: &Ctx,
    era: Era,
    id: Value,
    asker: Asker,
    turn: Turn,
    wait: u32,
    token: Option<Value>,
) -> Response {
    let owner = ctx.share(&asker, &turn.share).map_or_else(|| "the owner".to_string(), |s| short_name(&s.owner));
    if turn.status.is_final() || (wait == 0 && token.is_none() && !era.modern()) {
        return ok(&era, id, turn_result(ctx, &asker, &turn, &owner));
    }
    let Some(slot) = ctx.open_stream() else {
        // Too many open calls: answer with where the turn is now.
        return ok(&era, id, turn_result(ctx, &asker, &turn, &owner));
    };
    let deadline = Instant::now() + Duration::from_secs(u64::from(wait));
    let running_key = match &era {
        Era::Legacy { session, .. } => {
            let key = (ctx.caller.person().clone(), session.clone().unwrap_or_default(), id.to_string());
            lock(&ctx.inner.mcp.running).insert(key.clone(), (turn.id.clone(), asker.clone()));
            Some(key)
        }
        Era::Modern { .. } => None,
    };
    let finish = {
        let ctx = ctx.clone();
        move || {
            if let Some(key) = running_key {
                lock(&ctx.inner.mcp.running).remove(&key);
            }
        }
    };

    if token.is_none() && !era.modern() {
        let _slot = slot;
        let latest = ctx.follow(&asker, &turn.id, Some(deadline), |_| false, |_| true, || true);
        finish();
        return match latest {
            Ok((t, _)) => ok(&era, id, turn_result(ctx, &asker, &t, &owner)),
            Err(e) => rpc_http(StatusCode::OK, error(id, INTERNAL_ERROR, &e.message)),
        };
    }

    let (tx, body) = Body::channel();
    let ctx = ctx.clone();
    thread::spawn(move || {
        let _slot = slot;
        let w = SseWriter(tx);
        let mut progress = 0u64;
        let mut last_line = String::new();
        let followed = ctx.follow(
            &asker,
            &turn.id,
            Some(deadline),
            |_| false,
            |t| {
                let Some(token) = &token else { return true };
                if t.status.is_final() {
                    return true; // the result follows; progress stops
                }
                let line = status_line(t, &owner);
                if line == last_line {
                    return true;
                }
                last_line = line.clone();
                progress += 1;
                let note = json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/progress",
                    "params": {"progressToken": token, "progress": progress, "message": line},
                });
                w.json(None, &note)
            },
            || w.keepalive(),
        );
        finish();
        match followed {
            Ok((t, false)) => {
                let _ = w.json(None, &result_message(&era, id, turn_result(&ctx, &asker, &t, &owner)));
            }
            Ok((t, true)) => {
                // The client closed the stream. In the modern era that is
                // cancellation of the call, and so of its turn (§8.3.3).
                if era.modern() && !t.status.is_final() {
                    let _ = ctx.conversations().cancel(&asker, &t.id);
                }
            }
            Err(e) => {
                let _ = w.json(None, &error(id, INTERNAL_ERROR, &e.message));
            }
        }
    });
    wire::sse(body)
}

/// `notifications/cancelled` for a legacy call that is still waiting:
/// stop its turn (as Stop) or drop it from the queue.
fn cancel_running(ctx: &Ctx, session: Option<&str>, request: Option<&Value>) {
    let Some(request) = request else { return };
    let key = (ctx.caller.person().clone(), session.unwrap_or("").to_string(), request.to_string());
    let found = lock(&ctx.inner.mcp.running).get(&key).cloned();
    if let Some((turn, asker)) = found {
        let _ = ctx.conversations().cancel(&asker, &turn);
    }
}

fn changes_part(ctx: &Ctx, asker: &Asker, turn: &TurnId, part: u32) -> Value {
    match ctx.conversations().patch_part(asker, turn, part) {
        Ok(p) => {
            let mut text = p.text.clone();
            if p.part < p.parts {
                text.push_str(&format!(
                    "\n\n(part {} of {}; next: get_changes with turn \"{}\" and part {})",
                    p.part,
                    p.parts,
                    turn.0,
                    p.part + 1
                ));
            }
            json!({
                "content": [{"type": "text", "text": text}],
                "structuredContent": {"turn": p.turn.0, "part": p.part, "parts": p.parts, "patch": p.text},
                "isError": false,
            })
        }
        Err(_) => error_result(&format!("No changes to send for turn {} part {part}", turn.0)),
    }
}

fn error_result(message: &str) -> Value {
    json!({
        "content": [{"type": "text", "text": message}],
        "structuredContent": {"status": "failed", "error": message},
        "isError": true,
    })
}

/// A turn as a tool result: `structuredContent` (§8.3.3) and the same as
/// text (reply first, then a status line, then the patch when inlined).
pub(super) fn turn_result(ctx: &Ctx, asker: &Asker, turn: &Turn, owner: &str) -> Value {
    let line = status_line(turn, owner);
    let status = match &turn.status {
        TurnStatus::Queued { .. } | TurnStatus::Working => "working",
        TurnStatus::WaitingForOwner | TurnStatus::ChangesHeld => "waiting_for_owner",
        TurnStatus::Done => "done",
        TurnStatus::Failed { .. } | TurnStatus::Canceled | TurnStatus::Refused(_) => "failed",
    };
    let mut s = Map::new();
    s.insert("conversation".into(), json!(turn.conversation.0));
    s.insert("turn".into(), json!(turn.id.0));
    s.insert("status".into(), json!(status));
    s.insert("statusMessage".into(), json!(line));
    if let Some(reply) = &turn.reply {
        s.insert("reply".into(), json!(reply.text));
        s.insert("replySource".into(), json!(reply.source));
    }
    let id = &turn.id.0;
    let mut sentence = match &turn.status {
        TurnStatus::Queued { .. } | TurnStatus::Working => {
            format!("Turn {id} is still running ({line}). Call check_turn with turn \"{id}\" to wait for the reply.")
        }
        TurnStatus::WaitingForOwner | TurnStatus::ChangesHeld => {
            format!("Turn {id}: {line}. Call check_turn with turn \"{id}\" later.")
        }
        TurnStatus::Done => format!("Turn {id} is done (conversation {}).", turn.conversation.0),
        TurnStatus::Failed { reason } => {
            s.insert("failure".into(), json!(reason));
            format!("Turn {id} failed: {}.", failure_words(*reason))
        }
        TurnStatus::Canceled => {
            s.insert("failure".into(), json!("canceled"));
            format!("Turn {id} was canceled.")
        }
        TurnStatus::Refused(r) => {
            s.insert("refused".into(), json!(r.reason));
            s.insert("message".into(), json!(r.message));
            if let Some(at) = &r.resets_at {
                s.insert("resetsAt".into(), json!(at));
            }
            if let Some(until) = &r.until {
                s.insert("until".into(), json!(until));
            }
            r.message.clone()
        }
    };
    let mut text = Vec::new();
    if let Some(reply) = &turn.reply {
        text.push(reply.text.clone());
    }
    let mut patch_text = None;
    if let Some(c) = &turn.changes {
        let mut changes = json!({
            "state": c.state,
            "baseCommit": c.base_commit,
            "files": c.files,
            "added": c.added,
            "removed": c.removed,
            "patches": c.patches,
        });
        let summary = format!(
            "{} file{}, +{} -{}, {} patch{} on {}",
            c.files,
            if c.files == 1 { "" } else { "s" },
            c.added,
            c.removed,
            c.patches,
            if c.patches == 1 { "" } else { "es" },
            short_commit(&c.base_commit),
        );
        let words = match c.state {
            ChangesState::Held => format!("Changes ({summary}) are waiting for {owner}'s OK."),
            ChangesState::Discarded => format!("{owner} discarded this turn's changes ({summary})."),
            ChangesState::Sent => match ctx.conversations().patch(asker, &turn.id) {
                Ok(patch) if patch.chars().count() <= CHANGES_PART_CHARS => {
                    changes["patch"] = json!(patch);
                    changes["parts"] = json!(1);
                    patch_text = Some(patch);
                    format!("Changes: {summary}. The patch series follows; apply it with `git am -3` (or `git apply`).")
                }
                Ok(patch) => {
                    let parts = c.parts.max(patch.chars().count().div_ceil(CHANGES_PART_CHARS) as u32);
                    changes["parts"] = json!(parts);
                    format!(
                        "Changes: {summary}. Too big to show here: call get_changes with turn \"{id}\" and part 1 \
                         to {parts}, join the parts in order and apply with `git am -3`."
                    )
                }
                Err(_) => {
                    changes["parts"] = json!(c.parts);
                    format!("Changes: {summary}. Call get_changes with turn \"{id}\" to fetch them.")
                }
            },
        };
        s.insert("changes".into(), changes);
        sentence = format!("{sentence} {words}");
    }
    if let Some(left) = &turn.left {
        s.insert("left".into(), json!(left));
        if turn.status == TurnStatus::Done {
            sentence = format!("{sentence} {} messages left today.", left.messages_today);
        }
    }
    text.push(sentence);
    if let Some(p) = patch_text {
        text.push(p);
    }
    json!({
        "content": [{"type": "text", "text": text.join("\n\n")}],
        "structuredContent": Value::Object(s),
        "isError": matches!(turn.status, TurnStatus::Refused(_)),
    })
}

fn failure_words(f: Failure) -> &'static str {
    match f {
        Failure::NoAnswer => "the agent gave no answer",
        Failure::Timeout => "the turn took too long and was stopped",
        Failure::Crashed => "the agent stopped",
    }
}

fn short_commit(c: &str) -> &str {
    c.get(..7).unwrap_or(c)
}

// ------------------------------------------------------ list changes

/// What makes the tool list change for a caller (not `left`).
pub(super) fn fingerprint(shares: &[ShareCard]) -> String {
    let mut keys: Vec<String> = shares
        .iter()
        .map(|s| {
            format!(
                "{}\u{1}{}\u{1}{}\u{1}{:?}\u{1}{}\u{1}{}\u{1}{}",
                s.slug, s.name, s.description, s.capability, s.kind, s.messages_per_day, s.version
            )
        })
        .collect();
    keys.sort();
    keys.join("\u{2}")
}

/// A stream of `notifications/tools/list_changed` for this caller: the
/// legacy `GET /mcp` stream (`subscription` `None`) or a modern
/// `subscriptions/listen`. Sent only when this caller's own list changed.
fn listen(ctx: &Ctx, asker: Asker, subscription: Option<Value>, wanted: bool) -> Response {
    let Some(slot) = ctx.open_stream() else {
        return wire::json(StatusCode::TOO_MANY_REQUESTS, &json!({"error": {"message": "too many open streams"}}));
    };
    let (tx, body) = Body::channel();
    let ctx = ctx.clone();
    thread::spawn(move || {
        let _slot = slot;
        let w = SseWriter(tx);
        let ticks = ctx.conversations().shares_changed();
        let meta = |sub: &Value| json!({META_SUBSCRIPTION: sub});
        if let Some(sub) = &subscription {
            let mut filter = Map::new();
            if wanted {
                filter.insert("toolsListChanged".into(), json!(true));
            }
            let ack = json!({
                "jsonrpc": "2.0",
                "method": "notifications/subscriptions/acknowledged",
                "params": {"_meta": meta(sub), "notifications": filter},
            });
            if !w.json(None, &ack) {
                return;
            }
        }
        let mut seen = fingerprint(&ctx.conversations().shares(&asker));
        let heartbeat = ctx.options().heartbeat.max(Duration::from_millis(1));
        loop {
            match ticks.recv_timeout(heartbeat) {
                Ok(()) => {
                    let now = fingerprint(&ctx.conversations().shares(&asker));
                    if now == seen || !wanted {
                        continue;
                    }
                    seen = now;
                    let mut note = json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"});
                    if let Some(sub) = &subscription {
                        note["params"] = json!({"_meta": meta(sub)});
                    }
                    if !w.json(None, &note) {
                        return;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if !w.keepalive() {
                        return;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    if let Some(sub) = &subscription {
                        let done = json!({
                            "jsonrpc": "2.0", "id": sub,
                            "result": {"resultType": "complete", "_meta": meta(sub)},
                        });
                        let _ = w.json(None, &done);
                    }
                    return;
                }
            }
        }
    });
    wire::sse(body)
}

// ---------------------------------------------- OAuth (RFC 9728) metadata

/// The MCP endpoint's protected resource metadata, when plain clients
/// sign in with OAuth on this network (§15.3); `None` on networks that
/// prove the caller themselves.
pub fn protected_resource_metadata(endpoint: &Endpoint) -> Option<Value> {
    match &endpoint.client_auth {
        ClientAuth::None => None,
        ClientAuth::OAuth { issuer, resource, scopes } => Some(json!({
            "resource": resource,
            "authorization_servers": [issuer],
            "scopes_supported": scopes,
            "bearer_methods_supported": ["header"],
            "resource_name": "Pitwall shared agents",
        })),
    }
}

/// The `WWW-Authenticate` value a backend sends with a `401` for `/mcp`
/// on an OAuth network (it authenticates before this handler runs).
pub fn www_authenticate(endpoint: &Endpoint) -> Option<String> {
    match &endpoint.client_auth {
        ClientAuth::None => None,
        ClientAuth::OAuth { scopes, .. } => {
            let base = endpoint.base_url.trim_end_matches('/');
            let mut v = format!("Bearer resource_metadata=\"{base}{RESOURCE_METADATA_PATH}\"");
            if !scopes.is_empty() {
                v.push_str(&format!(", scope=\"{}\"", scopes.join(" ")));
            }
            Some(v)
        }
    }
}

// ------------------------------------------------------------ JSON-RPC

fn result_message(era: &Era, id: Value, mut result: Value) -> Value {
    if era.modern() {
        if let Value::Object(m) = &mut result {
            m.insert("resultType".into(), json!("complete"));
        }
    }
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn ok(era: &Era, id: Value, result: Value) -> Response {
    rpc_http(StatusCode::OK, result_message(era, id, result))
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn rpc_http(status: StatusCode, body: Value) -> Response {
    wire::json(status, &body)
}

fn mismatch(id: Option<Value>, message: &str) -> Response {
    rpc_http(StatusCode::BAD_REQUEST, error(id.unwrap_or(Value::Null), HEADER_MISMATCH, message))
}

fn unsupported_version(id: Option<Value>, requested: &str) -> Response {
    let mut body = error(id.unwrap_or(Value::Null), UNSUPPORTED_VERSION, "Unsupported protocol version");
    body["error"]["data"] = json!({"supported": all_versions(), "requested": requested});
    rpc_http(StatusCode::BAD_REQUEST, body)
}

/// `=?base64?…?=` header values decoded (MCP 2026-07-28 "Value Encoding").
fn decode_sentinel(value: &str) -> String {
    value
        .strip_prefix("=?base64?")
        .and_then(|v| v.strip_suffix("?="))
        .and_then(wire::b64_decode)
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_else(|| value.to_string())
}
