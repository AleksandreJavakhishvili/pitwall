//! The Pitwall endpoints beside the fronts (§8.4), under `/pitwall/v1/`.
//! Identity is the [`Caller`](crate::net::Caller), as for the fronts; team
//! records carry their own signatures and the core decides who may ask.
//!
//! | route | answer |
//! |---|---|
//! | `GET info` | [`PitwallInfo`](crate::collab::team::PitwallInfo) |
//! | `GET teams:discover` | `{"records": [TeamRecord]}` |
//! | `POST teams/{t}/join` (JoinRequest) | `202 {}` |
//! | `POST teams/{t}/invites:redeem` (InviteRedeem) | `200 {}` |
//! | `POST teams/{t}/heads` (TeamHead) | `{"heads": [TeamHead]}`: ours for that team |
//! | `GET teams/{t}/records?after=n` | `{"records": [TeamRecord]}` |
//! | `POST teams/{t}/announce` (TeamHead) | `202 {}` |
//! | `GET teams/{t}/requests` | `{"requests": [PendingJoin]}` |
//! | `GET directory` | `{"agents": [DirectoryEntry]}`: the Agent Cards this caller may use |
//! | `GET sessions` | `{"sessions": [ConversationSummary]}` |
//! | `DELETE sessions/{conversation}` | `204` |
//! | `GET results/{task}` | the patch series, `text/x-patch` |
//! | `GET events` | SSE, see below |
//!
//! Errors: `{"error": {"code": "not_found", "message": "…"}}` with the
//! HTTP status; not found, not allowed to see it and someone else's are
//! all `404 not_found`.
//!
//! **`GET events`** (member devices; others `404`): an SSE stream of
//! - `event: team.announce`, `data: {"team": "t_…", "version": 8, "hash": "sha256:…"}`
//!   (a [`TeamHead`]) when an admin changed a team the caller is in: pull
//!   `teams/{t}/records?after=<your version>`;
//! - `event: directory.changed`, `data: {}` when the caller's own
//!   directory changed (a share went live, paused, or its audience
//!   changed): fetch `directory` again.
//!
//! Quiet streams get a `:` comment line every heartbeat.

use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::wire::{self, SseWriter};
use super::{display_name, turn_id, Ctx};
use crate::collab::team::{InviteRedeem, JoinRequest, TeamHead, TeamId};
use crate::collab::turn::Front;
use crate::collab::{ConversationId, MAX_BODY};
use crate::error::{ErrorCode, PwError};
use crate::net::http::{Method, StatusCode};
use crate::net::{Body, Request, Response};
use pitwall_proto::collab::DirectoryEntry;

pub(super) fn handle(ctx: &Ctx, req: Request, rest: &str) -> Response {
    let method = req.method().clone();
    let segments: Vec<&str> = rest.split('/').collect();
    let caller = ctx.caller.clone();
    let teams = ctx.teams();
    let team = |raw: &str| wire::percent_decode(raw).map(TeamId);
    match (method.as_str(), segments.as_slice()) {
        ("GET", ["info"]) => ok(&teams.info(&caller)),
        ("GET", ["teams:discover"]) => ok(&json!({"records": teams.listed(&caller)})),
        ("POST", ["teams", t, "join"]) => {
            let (Some(t), Ok(body)) = (team(t), body::<JoinRequest>(req)) else { return bad_request() };
            done(teams.request_join(&caller, &t, body), StatusCode::ACCEPTED)
        }
        ("POST", ["teams", t, "invites:redeem"]) => {
            let (Some(t), Ok(body)) = (team(t), body::<InviteRedeem>(req)) else { return bad_request() };
            done(teams.redeem_invite(&caller, &t, body), StatusCode::OK)
        }
        ("POST", ["teams", t, "heads"]) => {
            let (Some(t), Ok(head)) = (team(t), body::<TeamHead>(req)) else { return bad_request() };
            if head.team != t {
                return bad_request();
            }
            match teams.heads(&caller, &[head]) {
                Ok(ours) => ok(&json!({"heads": ours.into_iter().filter(|h| h.team == t).collect::<Vec<_>>()})),
                Err(e) => error(e),
            }
        }
        ("GET", ["teams", t, "records"]) => {
            let Some(t) = team(t) else { return bad_request() };
            let after = wire::query_get(req.uri().query(), "after").and_then(|a| a.parse().ok()).unwrap_or(0);
            match teams.records(&caller, &t, after) {
                Ok(records) => ok(&json!({"records": records})),
                Err(e) => error(e),
            }
        }
        ("POST", ["teams", t, "announce"]) => {
            let (Some(t), Ok(head)) = (team(t), body::<TeamHead>(req)) else { return bad_request() };
            if head.team != t {
                return bad_request();
            }
            done(teams.announce(&caller, head), StatusCode::ACCEPTED)
        }
        ("GET", ["teams", t, "requests"]) => {
            let Some(t) = team(t) else { return bad_request() };
            match teams.join_requests(&caller, &t) {
                Ok(requests) => ok(&json!({"requests": requests})),
                Err(e) => error(e),
            }
        }
        ("GET", ["directory"]) => {
            let asker = ctx.asker(req.headers(), Front::A2a, None);
            ok(&json!({"agents": directory(ctx, &asker)}))
        }
        ("GET", ["sessions"]) => {
            let asker = ctx.asker(req.headers(), Front::A2a, None);
            match ctx.conversations().conversations(&asker) {
                Ok(list) => ok(&json!({"sessions": list})),
                Err(e) => error(e),
            }
        }
        ("DELETE", ["sessions", c]) => {
            let Some(c) = wire::percent_decode(c).filter(|c| !c.is_empty()) else { return bad_request() };
            let asker = ctx.asker(req.headers(), Front::A2a, None);
            match ctx.conversations().forget(&asker, &ConversationId(c)) {
                Ok(()) => wire::empty(StatusCode::NO_CONTENT),
                Err(e) => error(e),
            }
        }
        ("GET", ["results", ..]) => {
            let Some(id) = turn_id(&rest["results/".len()..]) else { return not_found() };
            let asker = ctx.asker(req.headers(), Front::A2a, None);
            match ctx.conversations().patch(&asker, &id) {
                Ok(patch) => wire::text(StatusCode::OK, wire::PATCH, patch),
                Err(e) => error(e),
            }
        }
        ("GET", ["events"]) => events(ctx, req),
        (_, _) if method != Method::GET && method != Method::POST && method != Method::DELETE => {
            wire::empty(StatusCode::METHOD_NOT_ALLOWED)
        }
        _ => not_found(),
    }
}

/// The Agent Cards this caller may use, as directory entries.
fn directory(ctx: &Ctx, asker: &crate::collab::Asker) -> Vec<DirectoryEntry> {
    let base = ctx.base_url();
    ctx.conversations()
        .shares(asker)
        .iter()
        .map(|share| DirectoryEntry {
            card_url: format!("{base}/a2a/{}/card", wire::percent_encode(&share.slug)),
            card: super::a2a::agent_card(share, ctx.endpoint.as_ref()),
            owner: display_name(&share.owner),
            slug: share.slug.clone(),
            capability: share.capability,
            kind: share.kind.clone(),
            left: Some(share.left),
            online: true,
            last_seen: None,
            // Why the caller may use it is the core's to say; ShareCard
            // doesn't carry it yet.
            via: Vec::new(),
        })
        .collect()
}

fn events(ctx: &Ctx, req: Request) -> Response {
    let announcements = match ctx.teams().announcements(&ctx.caller) {
        Ok(rx) => rx,
        Err(e) => return error(e),
    };
    let Some(slot) = ctx.open_stream() else {
        return json_error(StatusCode::TOO_MANY_REQUESTS, "busy", "too many open streams");
    };
    let asker = ctx.asker(req.headers(), Front::A2a, None);
    let (tx, body) = Body::channel();
    let ctx = ctx.clone();
    thread::spawn(move || {
        let _slot = slot;
        let w = SseWriter(tx);
        let shares = ctx.conversations().shares_changed();
        let mut seen = super::mcp::fingerprint(&ctx.conversations().shares(&asker));
        let heartbeat = ctx.options().heartbeat.max(Duration::from_millis(1));
        let poll = heartbeat.min(Duration::from_millis(250));
        let mut quiet_since = Instant::now();
        let mut announcements: Option<Receiver<TeamHead>> = Some(announcements);
        let mut shares = Some(shares);
        loop {
            let mut sent = false;
            if let Some(rx) = &announcements {
                match rx.recv_timeout(poll) {
                    Ok(head) => {
                        if !w.json(Some("team.announce"), &serde_json::to_value(&head).unwrap_or(Value::Null)) {
                            return;
                        }
                        sent = true;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => announcements = None,
                }
            } else {
                thread::sleep(poll);
            }
            if let Some(rx) = &shares {
                let mut ticked = false;
                loop {
                    match rx.try_recv() {
                        Ok(()) => ticked = true,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            shares = None;
                            break;
                        }
                    }
                }
                if ticked {
                    let now = super::mcp::fingerprint(&ctx.conversations().shares(&asker));
                    if now != seen {
                        seen = now;
                        if !w.json(Some("directory.changed"), &json!({})) {
                            return;
                        }
                        sent = true;
                    }
                }
            }
            if announcements.is_none() && shares.is_none() {
                return;
            }
            if sent {
                quiet_since = Instant::now();
            } else if quiet_since.elapsed() >= heartbeat {
                if !w.keepalive() {
                    return;
                }
                quiet_since = Instant::now();
            }
        }
    });
    wire::sse(body)
}

// ---------------------------------------------------------------- bits

fn body<T: DeserializeOwned>(req: Request) -> Result<T, ()> {
    let bytes = req.into_body().read_all(MAX_BODY).map_err(|_| ())?;
    serde_json::from_slice(&bytes).map_err(|_| ())
}

fn ok<T: serde::Serialize>(value: &T) -> Response {
    wire::json(StatusCode::OK, &serde_json::to_value(value).unwrap_or(Value::Null))
}

fn done(result: crate::error::Result<()>, status: StatusCode) -> Response {
    match result {
        Ok(()) => wire::json(status, &json!({})),
        Err(e) => error(e),
    }
}

fn json_error(status: StatusCode, code: &str, message: &str) -> Response {
    wire::json(status, &json!({"error": {"code": code, "message": message}}))
}

fn not_found() -> Response {
    json_error(StatusCode::NOT_FOUND, "not_found", "not found")
}

fn bad_request() -> Response {
    json_error(StatusCode::BAD_REQUEST, "bad_request", "malformed request")
}

/// A core error as HTTP. Not-found messages are never passed on: they'd
/// say what exists.
fn error(e: PwError) -> Response {
    match e.code {
        ErrorCode::NotFound => not_found(),
        ErrorCode::Denied => json_error(StatusCode::FORBIDDEN, "forbidden", &e.message),
        ErrorCode::Conflict => json_error(StatusCode::CONFLICT, "conflict", &e.message),
        ErrorCode::Unsupported => json_error(StatusCode::BAD_REQUEST, "unsupported", &e.message),
        ErrorCode::Unreachable | ErrorCode::NotRunning => {
            json_error(StatusCode::SERVICE_UNAVAILABLE, "unavailable", &e.message)
        }
        ErrorCode::Other => json_error(StatusCode::INTERNAL_SERVER_ERROR, "error", &e.message),
    }
}
