//! The collaboration handler (§8.1): one [`Handler`] with three groups of
//! routes, handed to every enabled network backend's `serve`:
//!
//! - `/a2a/<slug>/…`: the A2A front (HTTP+JSON binding, A2A 1.0), [`a2a`];
//! - `/mcp`: the MCP front (Streamable HTTP), [`mcp`];
//! - `/pitwall/v1/…`: the Pitwall team endpoints (§8.4), [`pitwall`].
//!
//! Plus `/.well-known/oauth-protected-resource[/mcp]` (RFC 9728) when the
//! network's plain clients sign in with OAuth (§15.3).
//!
//! The fronts only translate: audience, limits, conversations and turns
//! are the core's ([`Conversations`], [`Teams`]). Here: who is asking (the
//! [`Caller`] the network proved plus the `Pitwall-Agent` / `Pitwall-Chain`
//! labels, which never widen access), the loop check (§8.6), the body size
//! limit, requests per second and open streams per person (§8.1), and the
//! `Origin` check against DNS rebinding.
//!
//! A backend's listener may enforce the same limits before the handler
//! (the Tailscale one does); they are kept here too so every network,
//! including the Fake one, gets them, counted per person across networks.
//! Every stream sends a `:` comment every [`Options::heartbeat`] (15 s), so
//! a server finds a gone peer when the write fails.
//!
//! [`client`] is the other side: Pitwall's A2A client for the daemon's
//! `ask.*`, over a network's [`HttpClient`](crate::net::HttpClient).

pub mod a2a;
pub mod client;
pub mod mcp;
pub mod pitwall;
mod wire;

#[cfg(test)]
mod stub;
#[cfg(test)]
pub(crate) mod tests;

use std::collections::HashMap;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::turn::Front;
use super::turn::{Asker, Refusal, ShareCard, Turn, TurnId, TurnStatus, Who};
use super::{Conversations, RefusalReason, Teams, AGENT_HEADER, CHAIN_HEADER, MAX_STREAMS};
use crate::error::Result;
use crate::net::http::{header, Method, StatusCode};
use crate::net::{Caller, Endpoint, Handler, Network, NetworkId, PersonId, Request, Response};

pub use wire::{SseEvent, SseReader};

/// Requests per second per person, sustained (§8.1), and the burst on top.
/// **Provisional**: the spec gives no numbers; these are a first guess
/// (decided 2026-10-16 to keep them for now), to be tuned once real
/// teams use collaboration. A Pitwall caller's Ask panel and an MCP
/// client's `tools/list` + `tools/call` + `check_turn` stay far below
/// them; a script hammering the endpoint doesn't.
pub const REQUESTS_PER_SECOND: u32 = 10;
pub const BURST: u32 = 40;

/// Tunables; [`Options::default`] is what Pitwall ships.
#[derive(Debug, Clone)]
pub struct Options {
    /// Requests per second per person, sustained, and the burst on top
    /// (§8.1 "requests per second"). Over it: `429`.
    pub requests_per_second: u32,
    pub burst: u32,
    /// How often a quiet stream sends a keep-alive (and finds out whether
    /// the peer is still there).
    pub heartbeat: Duration,
    /// How long A2A `message:send` waits for a turn to settle unless the
    /// caller asked for `returnImmediately`.
    pub send_wait: Duration,
    /// This Pitwall's version (MCP `serverInfo`, A2A card provider).
    pub version: String,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            requests_per_second: REQUESTS_PER_SECOND,
            burst: BURST,
            heartbeat: Duration::from_secs(15),
            send_wait: Duration::from_secs(u64::from(super::WAIT_SECONDS_MAX)),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

type Endpoints = dyn Fn(&NetworkId) -> Option<Endpoint> + Send + Sync;

/// The handler. Cheap to clone; serve the same one on every network.
#[derive(Clone)]
pub struct Fronts {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    conversations: Arc<dyn Conversations>,
    teams: Arc<dyn Teams>,
    endpoints: Box<Endpoints>,
    options: Options,
    streams: Mutex<HashMap<PersonId, usize>>,
    buckets: Mutex<HashMap<PersonId, Bucket>>,
    mcp: mcp::State,
}

impl Fronts {
    /// `endpoints` says how each network reaches this device (card URLs,
    /// `securitySchemes`, protected resource metadata); asked per request,
    /// so a changed endpoint shows at once.
    pub fn new(
        conversations: Arc<dyn Conversations>,
        teams: Arc<dyn Teams>,
        endpoints: impl Fn(&NetworkId) -> Option<Endpoint> + Send + Sync + 'static,
        options: Options,
    ) -> Fronts {
        Fronts {
            inner: Arc::new(Inner {
                conversations,
                teams,
                endpoints: Box::new(endpoints),
                options,
                streams: Mutex::default(),
                buckets: Mutex::default(),
                mcp: mcp::State::default(),
            }),
        }
    }

    /// Endpoints from the networks this handler is served on.
    pub fn for_networks(
        conversations: Arc<dyn Conversations>,
        teams: Arc<dyn Teams>,
        networks: Vec<Arc<dyn Network>>,
        options: Options,
    ) -> Fronts {
        Fronts::new(
            conversations,
            teams,
            move |id| networks.iter().find(|n| &n.id() == id).map(|n| n.endpoint()),
            options,
        )
    }
}

impl Handler for Fronts {
    fn handle(&self, req: Request, caller: Caller) -> Response {
        self.inner.handle(req, caller)
    }
}

impl Inner {
    fn handle(self: &Arc<Self>, req: Request, caller: Caller) -> Response {
        let endpoint = (self.endpoints)(caller.network());
        if !origin_allowed(&req, endpoint.as_ref()) {
            return wire::json(StatusCode::FORBIDDEN, &serde_json::json!({"error": {"message": "origin not allowed"}}));
        }
        if !self.take_token(caller.person()) {
            let mut res = wire::json(
                StatusCode::TOO_MANY_REQUESTS,
                &serde_json::json!({"error": {"message": "too many requests"}}),
            );
            wire::set(&mut res, "retry-after", "1");
            return res;
        }
        let path = req.uri().path().to_string();
        let ctx = Ctx { inner: self.clone(), caller, endpoint };
        if path == "/mcp" || path == "/mcp/" {
            mcp::handle(&ctx, req)
        } else if let Some(rest) = path.strip_prefix("/a2a/") {
            a2a::handle(&ctx, req, rest)
        } else if let Some(rest) = path.strip_prefix("/pitwall/v1/") {
            pitwall::handle(&ctx, req, rest)
        } else if path == mcp::RESOURCE_METADATA_PATH || path == mcp::RESOURCE_METADATA_ROOT {
            match (req.method() == Method::GET, ctx.endpoint.as_ref().and_then(mcp::protected_resource_metadata)) {
                (true, Some(doc)) => wire::json(StatusCode::OK, &doc),
                _ => wire::empty(StatusCode::NOT_FOUND),
            }
        } else {
            wire::empty(StatusCode::NOT_FOUND)
        }
    }

    fn take_token(&self, person: &PersonId) -> bool {
        let (rate, burst) = (f64::from(self.options.requests_per_second.max(1)), f64::from(self.options.burst.max(1)));
        let mut buckets = lock(&self.buckets);
        let now = Instant::now();
        if buckets.len() > 10_000 {
            buckets.retain(|_, b| now.duration_since(b.at) < Duration::from_secs(60));
        }
        let b = buckets.entry(person.clone()).or_insert(Bucket { tokens: burst, at: now });
        b.tokens = (b.tokens + now.duration_since(b.at).as_secs_f64() * rate).min(burst);
        b.at = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

/// One request's context: the handler, the proven caller, and the
/// endpoint of the network it came over.
#[derive(Clone)]
pub(crate) struct Ctx {
    inner: Arc<Inner>,
    caller: Caller,
    endpoint: Option<Endpoint>,
}

impl Ctx {
    fn conversations(&self) -> &dyn Conversations {
        &*self.inner.conversations
    }

    fn teams(&self) -> &dyn Teams {
        &*self.inner.teams
    }

    fn options(&self) -> &Options {
        &self.inner.options
    }

    /// The base URL of this device on the caller's network (no trailing
    /// slash); empty when unknown, so URLs come out relative.
    fn base_url(&self) -> &str {
        self.endpoint.as_ref().map_or("", |e| e.base_url.trim_end_matches('/'))
    }

    /// Who is asking: the network's proof plus the request's labels.
    fn asker(&self, headers: &crate::net::http::HeaderMap, front: Front, client: Option<String>) -> Asker {
        let agent = wire::header_str(headers, AGENT_HEADER).and_then(|v| label(v, 128));
        let chain = wire::header_str(headers, CHAIN_HEADER)
            .map(|v| v.split(',').filter_map(|e| label(e.trim(), 128)).take(16).collect())
            .unwrap_or_default();
        Asker { who: Who::Network(Box::new(self.caller.clone())), agent, client, chain, front }
    }

    /// One more open stream (or long wait) for this person, or `None` over
    /// [`MAX_STREAMS`]. The slot frees when the guard drops.
    fn open_stream(&self) -> Option<StreamSlot> {
        let person = self.caller.person().clone();
        let mut streams = lock(&self.inner.streams);
        let open = streams.entry(person.clone()).or_insert(0);
        if *open >= MAX_STREAMS {
            return None;
        }
        *open += 1;
        Some(StreamSlot { inner: self.inner.clone(), person })
    }

    /// The share a slug names, if this asker may use it.
    fn share(&self, asker: &Asker, slug: &str) -> Option<ShareCard> {
        self.conversations().shares(asker).into_iter().find(|s| s.slug == slug)
    }

    /// Follow a turn until `settled` says so, it is final, or `deadline`.
    /// `on_turn` sees every change (`false`: the client is gone), `beat` is
    /// called on quiet stretches (`false`: the client is gone). Returns the
    /// latest turn and whether the client went away.
    fn follow(
        &self,
        asker: &Asker,
        id: &TurnId,
        deadline: Option<Instant>,
        settled: impl Fn(&Turn) -> bool,
        mut on_turn: impl FnMut(&Turn) -> bool,
        mut beat: impl FnMut() -> bool,
    ) -> Result<(Turn, bool)> {
        let rx = self.conversations().watch(asker, id)?;
        let mut last: Option<Turn> = None;
        let heartbeat = self.options().heartbeat.max(Duration::from_millis(1));
        loop {
            let wait = match deadline {
                Some(d) => match d.checked_duration_since(Instant::now()) {
                    Some(left) if !left.is_zero() => left.min(heartbeat),
                    _ => break,
                },
                None => heartbeat,
            };
            match rx.recv_timeout(wait) {
                Ok(turn) => {
                    let done = turn.status.is_final() || settled(&turn);
                    if !on_turn(&turn) {
                        return Ok((turn, true));
                    }
                    last = Some(turn);
                    if done {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if !beat() {
                        let turn = match last {
                            Some(t) => t,
                            None => self.conversations().turn(asker, id)?,
                        };
                        return Ok((turn, true));
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    last = None;
                    break;
                }
            }
        }
        let turn = match last {
            Some(t) => t,
            None => self.conversations().turn(asker, id)?,
        };
        Ok((turn, false))
    }

    /// The loop check (§8.6) for a message to `share`, before the core
    /// sees it, with the one rule ([`super::turn::chain_loop`]): a chain
    /// through more than `MAX_CHAIN_DEPTH` shares, or one that already went
    /// through this share, is refused with `loop`. The core applies the
    /// same rule again (the chain is on the [`Asker`]); a plain client sends
    /// no chain and limits are the real bound.
    fn loop_refusal(&self, asker: &Asker, share: &ShareCard, conversation: Option<&str>) -> Option<Turn> {
        let found = super::turn::chain_loop(&asker.chain, &share.slug, &share.owner)?;
        let owner = short_name(&share.owner);
        let message = match found {
            super::turn::ChainLoop::Cycle => format!("{owner}/{} is already in this message's chain; not asking it again", share.slug),
            super::turn::ChainLoop::TooDeep(n) => {
                format!("This message already went through {n} shared agents; not asking {owner}/{}", share.slug)
            }
        };
        // Never reaches the core, so it has no turn there: a one-off id.
        let conversation = conversation.map_or_else(|| format!("x_{}", uuid::Uuid::new_v4().simple()), str::to_string);
        let conversation = super::ConversationId(conversation);
        Some(Turn {
            id: TurnId::new(&conversation, 0),
            conversation,
            share: share.slug.clone(),
            status: TurnStatus::Refused(Refusal { reason: RefusalReason::Loop, message, resets_at: None, until: None }),
            status_message: None,
            reply: None,
            changes: None,
            left: None,
            at: String::new(),
            done_at: None,
        })
    }
}

/// A turn id from a URL: percent-decoded.
fn turn_id(raw: &str) -> Option<TurnId> {
    wire::percent_decode(raw).filter(|s| !s.is_empty()).map(TurnId)
}

/// `alice` for `alice@example.com`.
pub(crate) use super::turn::short_name;

/// How a person is shown: email, else subject.
pub(crate) fn display_name(identity: &crate::net::Identity) -> String {
    identity.email.clone().unwrap_or_else(|| identity.subject.clone())
}

/// A request label, if it is short printable text.
fn label(value: &str, max: usize) -> Option<String> {
    let ok = !value.is_empty() && value.len() <= max && value.chars().all(|c| !c.is_control());
    ok.then(|| value.to_string())
}

/// No `Origin`, or the device's own: a web page elsewhere can't use the
/// caller's network identity (DNS rebinding, MCP's transport rules).
fn origin_allowed(req: &Request, endpoint: Option<&Endpoint>) -> bool {
    let Some(origin) = req.headers().get(header::ORIGIN) else { return true };
    let Ok(origin) = origin.to_str() else { return false };
    let Some(endpoint) = endpoint else { return false };
    let own = origin_of(&endpoint.base_url);
    !own.is_empty() && origin.trim_end_matches('/').eq_ignore_ascii_case(&own)
}

/// `scheme://host[:port]` of a URL.
fn origin_of(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else { return String::new() };
    let host = rest.split('/').next().unwrap_or("");
    format!("{scheme}://{host}")
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// An open stream's place in the per-person count.
pub(crate) struct StreamSlot {
    inner: Arc<Inner>,
    person: PersonId,
}

impl Drop for StreamSlot {
    fn drop(&mut self) {
        let mut streams = lock(&self.inner.streams);
        if let Some(n) = streams.get_mut(&self.person) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                streams.remove(&self.person);
            }
        }
    }
}

/// The JSON-ish name of a turn's state (`waiting_for_owner`).
pub(crate) fn state_name(status: &TurnStatus) -> &'static str {
    match status {
        TurnStatus::Queued { .. } => "queued",
        TurnStatus::Working => "working",
        TurnStatus::WaitingForOwner => "waiting_for_owner",
        TurnStatus::ChangesHeld => "changes_held",
        TurnStatus::Done => "done",
        TurnStatus::Failed { .. } => "failed",
        TurnStatus::Canceled => "canceled",
        TurnStatus::Refused(_) => "refused",
    }
}

/// A short line for people when the core gave none.
pub(crate) fn status_line(turn: &Turn, owner: &str) -> String {
    if let Some(m) = &turn.status_message {
        return m.clone();
    }
    match &turn.status {
        TurnStatus::Queued { position } => format!("{} in line", wire::ordinal(*position)),
        TurnStatus::Working => "working".into(),
        TurnStatus::WaitingForOwner => format!("waiting for {owner}"),
        TurnStatus::ChangesHeld => format!("changes waiting for {owner}'s OK"),
        TurnStatus::Done => "done".into(),
        TurnStatus::Failed { reason } => match reason {
            super::Failure::NoAnswer => "failed: the agent gave no answer".into(),
            super::Failure::Timeout => "failed: the turn took too long".into(),
            super::Failure::Crashed => "failed: the agent stopped".into(),
        },
        TurnStatus::Canceled => "canceled".into(),
        TurnStatus::Refused(r) => r.message.clone(),
    }
}
