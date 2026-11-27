//! A fake Agention operator API for the harness's `net` (`Harness::serve`),
//! ported from the adapter's `fake-agention.js`, written from the operator
//! API's OpenAPI document: in-memory threads, messages and asks; bearer
//! keys with scopes; the wait as a long poll held until there is news;
//! idempotent writes by request id; and switches that make it misbehave.
//! Nothing leaves the test.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use pitwall_extension_test::{NetHeader, NetReply, NetRequest, NetResponse};
use serde_json::{json, Value};

/// The address the tests set as Server address.
pub const SERVER: &str = "https://agention.example.ts.net";

/// A key of the right form, made of one letter.
pub fn key(c: char) -> String {
    format!("agention_{}A", c.to_string().repeat(42))
}

/// Keys the fake knows: all scopes, read-only, no sessions; and one it
/// doesn't.
pub fn full() -> String {
    key('A')
}
pub fn read_only() -> String {
    key('B')
}
pub fn no_sessions() -> String {
    key('C')
}
pub fn revoked() -> String {
    key('D')
}

/// An ask an agent posts.
#[derive(Clone, Default)]
pub struct AskSpec {
    pub question: String,
    pub suggestions: Vec<String>,
    pub multiple: bool,
    pub blocking: bool,
}

impl AskSpec {
    pub fn new(question: &str, suggestions: &[&str]) -> AskSpec {
        AskSpec { question: question.into(), suggestions: suggestions.iter().map(|s| s.to_string()).collect(), ..AskSpec::default() }
    }
    pub fn blocking(mut self) -> AskSpec {
        self.blocking = true;
        self
    }
    pub fn multiple(mut self) -> AskSpec {
        self.multiple = true;
        self
    }
}

struct FAsk {
    number: u64,
    question: String,
    suggestions: Vec<String>,
    multiple: bool,
    blocking: bool,
    seq: u64,
    answer: Option<Value>,
    withdrawn: Option<String>,
}

struct FMsg {
    seq: u64,
    author: Value,
    at: i64,
    text: Option<String>,
    detail: Option<String>,
    asks: Vec<FAsk>,
    answers: Option<Value>,
}

struct FThread {
    agent: String,
    session: String,
    number: u64,
    title: String,
    created: i64,
    updated: i64,
    messages: Vec<FMsg>,
    read_through: u64,
    archived_through: u64,
    position: u64,
    next_ask: u64,
}

#[derive(Default)]
struct State {
    keys: HashMap<String, Vec<&'static str>>,
    threads: Vec<FThread>,
    clock: i64,
    counter: u64,
    waiters: Vec<(u64, usize, NetRequest, NetReply)>,
    requests: Vec<(String, String, Option<String>, Option<Value>)>,
    writes: HashMap<String, (Vec<u8>, Value)>,
    fail_next: VecDeque<(u16, &'static str)>,
    legacy: bool,
    not_agention: bool,
}

/// The fake server. Share it with `Arc` between the test and the harness.
pub struct Fake {
    s: Mutex<State>,
}

/// An answer to send.
struct Out(u16, Value);

fn refuse(status: u16, code: &str, message: &str) -> Out {
    Out(status, json!({ "error": { "code": code, "message": message } }))
}

fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3600, rem % 3600 / 60, rem % 60, ms.rem_euclid(1000))
}

impl Fake {
    pub fn new() -> Arc<Fake> {
        let mut s = State { clock: 1_795_000_000_000, ..State::default() };
        s.keys.insert(full(), vec!["threads:write", "sessions:read"]);
        s.keys.insert(read_only(), vec!["threads:read", "sessions:read"]);
        s.keys.insert(no_sessions(), vec!["threads:write"]);
        Arc::new(Fake { s: Mutex::new(s) })
    }

    /// What `Harness::serve` takes.
    pub fn server(self: &Arc<Fake>) -> impl Fn(NetRequest, NetReply) + Send + Sync + 'static {
        let f = self.clone();
        move |req, reply| f.handle(req, reply)
    }

    // ---- Switches and records ------------------------------------------------

    /// Answer the next requests with these statuses and error codes.
    pub fn fail_next(&self, status: u16, code: &'static str) {
        self.s.lock().unwrap().fail_next.push_back((status, code));
    }
    /// An Agention from before the operator API.
    pub fn legacy(&self, yes: bool) {
        self.s.lock().unwrap().legacy = yes;
    }
    /// Some other web server.
    pub fn not_agention(&self, yes: bool) {
        self.s.lock().unwrap().not_agention = yes;
    }
    /// The requests it got: method, path with query, Authorization, body.
    pub fn requests(&self) -> Vec<(String, String, Option<String>, Option<Value>)> {
        self.s.lock().unwrap().requests.clone()
    }
    /// The bodies of the messages the operator sent, oldest first.
    pub fn sent(&self) -> Vec<Value> {
        self.requests().into_iter().filter(|(m, p, _, _)| m == "POST" && p.ends_with("/messages")).filter_map(|(_, _, _, b)| b).collect()
    }
    /// Waits held now.
    pub fn held(&self) -> usize {
        self.s.lock().unwrap().waiters.len()
    }

    // ---- Data ------------------------------------------------------------------

    /// A thread an agent opens with a message and its asks; its id.
    pub fn add_thread(&self, agent: &str, session: &str, title: &str, text: &str, asks: Vec<AskSpec>) -> String {
        let id = {
            let mut s = self.s.lock().unwrap();
            let number = s.threads.iter().filter(|t| t.agent == agent && t.session == session).count() as u64 + 1;
            s.clock += 1000;
            let at = s.clock;
            s.threads.push(FThread { agent: agent.into(), session: session.into(), number, title: title.into(), created: at, updated: at, messages: vec![], read_through: 0, archived_through: 0, position: 0, next_ask: 1 });
            format!("{agent}/{session}/{number}")
        };
        self.agent_post(&id, text, asks);
        id
    }

    /// The agent writes in thread `id`; waits with news are answered.
    pub fn agent_post(&self, id: &str, text: &str, asks: Vec<AskSpec>) -> u64 {
        let (seq, ready) = {
            let mut s = self.s.lock().unwrap();
            let i = s.index(id).expect("a thread of the fake");
            let author = json!({ "kind": "session", "agent": s.threads[i].agent, "session": s.threads[i].session });
            let seq = s.append(i, author, Some(text.into()), None, asks, None);
            (seq, s.ready_waiters())
        };
        for (reply, out, url) in ready {
            reply(Ok(response(out, url)));
        }
        seq
    }

    /// The agent withdraws ask `number` of thread `id`.
    pub fn withdraw(&self, id: &str, number: u64, reason: &str) {
        let mut s = self.s.lock().unwrap();
        let i = s.index(id).unwrap();
        for m in &mut s.threads[i].messages {
            for a in &mut m.asks {
                if a.number == number {
                    a.withdrawn = Some(reason.into());
                }
            }
        }
    }

    /// The answer ask `number` of thread `id` holds.
    pub fn answer_of(&self, id: &str, number: u64) -> Option<Value> {
        let s = self.s.lock().unwrap();
        let i = s.index(id)?;
        s.threads[i].messages.iter().flat_map(|m| &m.asks).find(|a| a.number == number).and_then(|a| a.answer.clone())
    }

    /// The operator read thread `id` elsewhere (in Agention's web app).
    pub fn mark_read(&self, id: &str) {
        let mut s = self.s.lock().unwrap();
        let i = s.index(id).unwrap();
        s.threads[i].read_through = s.threads[i].messages.len() as u64;
    }

    /// Thread `id`'s marks: read through, archived through.
    pub fn marks(&self, id: &str) -> (u64, u64) {
        let s = self.s.lock().unwrap();
        let t = &s.threads[s.index(id).unwrap()];
        (t.read_through, t.archived_through)
    }

    // ---- HTTP --------------------------------------------------------------------

    pub fn handle(&self, req: NetRequest, reply: NetReply) {
        let url = req.url.to_string();
        let mut s = self.s.lock().unwrap();
        let path = req.url.path().to_string();
        let full = match req.url.query() {
            Some(q) => format!("{path}?{q}"),
            None => path.clone(),
        };
        let auth = req.headers.iter().find(|h| h.name.eq_ignore_ascii_case("authorization")).map(|h| h.value.clone());
        let body: Option<Value> = req.body.as_ref().and_then(|b| serde_json::from_slice(b).ok());
        s.requests.push((req.method.clone(), full, auth.clone(), body.clone()));
        if s.not_agention {
            drop(s);
            return reply(Ok(NetResponse { status: 200, headers: vec![NetHeader { name: "content-type".into(), value: "text/html".into() }], body: b"<!doctype html><title>Welcome to nginx</title>".to_vec(), url }));
        }
        match s.route(&req, &path, auth.as_deref(), body) {
            Some(out) => {
                // Answered outside the lock: the tile may ask again at once.
                drop(s);
                reply(Ok(response(out, url)));
            }
            None => {
                // The wait, held until there is news.
                let since = query(&req, "since").and_then(|v| v[1..].parse().ok()).unwrap_or(0);
                let limit = query(&req, "limit").and_then(|v| v.parse().ok()).unwrap_or(50);
                s.waiters.push((since, limit, req, reply));
            }
        }
    }
}

fn query(req: &NetRequest, key: &str) -> Option<String> {
    req.url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned())
}

fn response(out: Out, url: String) -> NetResponse {
    NetResponse { status: out.0, headers: vec![NetHeader { name: "content-type".into(), value: "application/json".into() }], body: serde_json::to_vec(&out.1).unwrap(), url }
}

impl State {
    fn index(&self, id: &str) -> Option<usize> {
        self.threads.iter().position(|t| format!("{}/{}/{}", t.agent, t.session, t.number) == id)
    }

    fn append(&mut self, i: usize, author: Value, text: Option<String>, detail: Option<String>, asks: Vec<AskSpec>, answers: Option<Value>) -> u64 {
        self.clock += 1000;
        let at = self.clock;
        let t = &mut self.threads[i];
        let seq = t.messages.len() as u64 + 1;
        let mut out = Vec::new();
        for a in asks {
            out.push(FAsk { number: t.next_ask, question: a.question, suggestions: a.suggestions, multiple: a.multiple, blocking: a.blocking, seq, answer: None, withdrawn: None });
            t.next_ask += 1;
        }
        t.messages.push(FMsg { seq, author, at, text, detail, asks: out, answers });
        t.updated = at;
        self.counter += 1;
        t.position = self.counter;
        seq
    }

    fn unread(t: &FThread) -> u64 {
        let through = t.read_through.max(t.archived_through);
        t.messages.iter().filter(|m| m.seq > through && m.author["kind"] == "session").count() as u64
    }

    fn summary(t: &FThread) -> Value {
        let open: Vec<&FAsk> = t.messages.iter().flat_map(|m| &m.asks).filter(|a| a.answer.is_none() && a.withdrawn.is_none()).collect();
        let last = t.messages.len() as u64;
        json!({
            "agent": t.agent, "session": t.session, "number": t.number, "title": t.title,
            "created_at": iso(t.created), "updated_at": iso(t.updated), "last_seq": last,
            "read_through": t.read_through, "unread": State::unread(t), "message_count": last,
            "archived": t.archived_through >= last,
            "open_asks": { "asks": open.iter().map(|a| json!({"number": a.number, "seq": a.seq})).collect::<Vec<_>>(), "count": open.len(), "blocking": open.iter().any(|a| a.blocking), "overdue": false },
            "status": if open.is_empty() { "none" } else { "open" },
            "credential_active": true, "links": { "pull_requests": 0, "branches": 0 }
        })
    }

    fn message(m: &FMsg) -> Value {
        let mut out = json!({ "seq": m.seq, "author": m.author, "created_at": iso(m.at), "live": true });
        if let Some(t) = &m.text {
            out["text"] = json!(t);
        }
        if let Some(d) = &m.detail {
            out["detail"] = json!(d);
        }
        if !m.asks.is_empty() {
            out["asks"] = m
                .asks
                .iter()
                .map(|a| {
                    let mut o = json!({ "number": a.number, "question": a.question, "suggestions": a.suggestions.iter().map(|s| json!({"label": s, "description": ""})).collect::<Vec<_>>(), "multiple": a.multiple, "impact": if a.blocking { "blocking" } else { "none" }, "priority": "normal", "open": a.answer.is_none() && a.withdrawn.is_none() });
                    if let Some(ans) = &a.answer {
                        o["answer"] = ans.clone();
                    }
                    if let Some(w) = &a.withdrawn {
                        o["withdrawal"] = json!({ "seq": 0, "reason": w });
                    }
                    o
                })
                .collect();
        }
        if let Some(a) = &m.answers {
            out["answers"] = a.clone();
        }
        out
    }

    /// Waits that have news now, answered.
    fn ready_waiters(&mut self) -> Vec<(NetReply, Out, String)> {
        let mut out = Vec::new();
        let waiters = std::mem::take(&mut self.waiters);
        for (since, limit, req, reply) in waiters {
            let found = self.found(since, limit);
            if found.is_empty() {
                self.waiters.push((since, limit, req, reply));
            } else {
                let o = self.wait_answer(since, limit, found);
                out.push((reply, o, req.url.to_string()));
            }
        }
        out
    }

    fn found(&self, since: u64, limit: usize) -> Vec<usize> {
        let mut f: Vec<usize> = (0..self.threads.len()).filter(|&i| self.threads[i].position > since && State::unread(&self.threads[i]) > 0).collect();
        f.sort_by_key(|&i| self.threads[i].position);
        f.truncate(limit);
        f
    }

    fn wait_answer(&self, since: u64, limit: usize, found: Vec<usize>) -> Out {
        let up_to = if found.len() == limit { self.threads[*found.last().unwrap()].position } else { self.counter };
        Out(200, json!({ "threads": found.iter().map(|&i| State::summary(&self.threads[i])).collect::<Vec<_>>(), "since": format!("w{}", up_to.max(since)) }))
    }

    fn authenticate(&self, auth: Option<&str>, scope: &str) -> Result<(), Out> {
        let key = auth.and_then(|a| a.strip_prefix("Bearer "));
        let Some(scopes) = key.and_then(|k| self.keys.get(k)) else { return Err(refuse(401, "unauthenticated", "the API key is missing, invalid, expired or revoked")) };
        let holds = scopes.iter().any(|s| *s == scope || (*s == "threads:write" && scope == "threads:read"));
        // Without the scope, as if the operation did not exist; before any input.
        if holds {
            Ok(())
        } else {
            Err(refuse(404, "not_found", "not found"))
        }
    }

    /// The answer to a request; `None` holds it (a wait with no news).
    fn route(&mut self, req: &NetRequest, p: &str, auth: Option<&str>, body: Option<Value>) -> Option<Out> {
        let r = self.route_inner(req, p, auth, body);
        match r {
            Ok(o) => o,
            Err(e) => Some(e),
        }
    }

    fn route_inner(&mut self, req: &NetRequest, p: &str, auth: Option<&str>, body: Option<Value>) -> Result<Option<Out>, Out> {
        let m = req.method.as_str();
        if self.legacy && p.starts_with("/api/operator") {
            return Err(refuse(404, "not_found", "not found"));
        }
        if let Some((status, code)) = self.fail_next.pop_front() {
            return Err(refuse(status, code, code));
        }
        if m == "GET" && p == "/api/operator" {
            return Ok(Some(Out(200, json!({ "versions": [1] }))));
        }
        let live = |t: &FThread| t.archived_through < t.messages.len() as u64;
        match (m, p) {
            ("GET", "/api/operator/v1/threads") => {
                self.authenticate(auth, "threads:read")?;
                let offset: usize = query(req, "cursor").and_then(|c| c.parse().ok()).unwrap_or(0);
                let limit: usize = query(req, "limit").and_then(|c| c.parse().ok()).unwrap_or(50);
                let mut all: Vec<&FThread> = self.threads.iter().filter(|t| live(t)).collect();
                all.sort_by_key(|t| std::cmp::Reverse(t.updated));
                let page: Vec<Value> = all.iter().skip(offset).take(limit).map(|t| State::summary(t)).collect();
                let mut out = json!({ "threads": page, "total": all.len(), "as_of": "fake" });
                if offset + limit < all.len() {
                    out["next"] = json!((offset + limit).to_string());
                }
                return Ok(Some(Out(200, out)));
            }
            ("GET", "/api/operator/v1/wait") => {
                self.authenticate(auth, "threads:read")?;
                let since: u64 = query(req, "since").and_then(|v| v[1..].parse().ok()).unwrap_or(0);
                let limit: usize = query(req, "limit").and_then(|v| v.parse().ok()).unwrap_or(50);
                let wait: u64 = query(req, "wait").and_then(|v| v.parse().ok()).unwrap_or(0);
                let found = self.found(since, limit);
                if !found.is_empty() || wait == 0 {
                    return Ok(Some(self.wait_answer(since, limit, found)));
                }
                return Ok(None);
            }
            ("GET", "/api/operator/v1/sessions") => {
                self.authenticate(auth, "sessions:read")?;
                let mut agents: BTreeMap<String, Vec<Value>> = BTreeMap::new();
                for t in &self.threads {
                    let list = agents.entry(t.agent.clone()).or_default();
                    if !list.iter().any(|s| s["session"] == t.session.as_str()) {
                        list.push(json!({ "session": t.session, "status": { "state": "working", "title": format!("Working in {}", t.agent) }, "presence": { "quiet": false }, "threads": { "total": 1, "unread": 0 }, "open_asks": { "count": 0 } }));
                    }
                }
                let out: Vec<Value> = agents.into_iter().map(|(a, s)| json!({ "agent": a, "sessions": s, "truncated": false })).collect();
                return Ok(Some(Out(200, json!({ "agents": out }))));
            }
            ("POST", "/api/operator/v1/reads") => {
                self.authenticate(auth, "threads:write")?;
                let ok = body.as_ref().and_then(|b| b["threads"].as_array()).is_some_and(|t| !t.is_empty());
                if !ok {
                    return Err(Out(400, json!({ "error": { "code": "invalid_input", "message": "Required", "field": "threads" } })));
                }
                return Ok(Some(Out(200, json!({ "done": true }))));
            }
            _ => {}
        }
        // A thread's own routes.
        let rest = p.strip_prefix("/api/operator/v1/s/").ok_or_else(|| refuse(404, "not_found", "not found"))?;
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() < 4 || parts[2] != "threads" {
            return Err(refuse(404, "not_found", "not found"));
        }
        let id = format!("{}/{}/{}", parts[0], parts[1], parts[3]);
        let action = parts.get(4).copied();
        let scope = if m == "GET" { "threads:read" } else { "threads:write" };
        self.authenticate(auth, scope)?;
        let i = self.index(&id).ok_or_else(|| refuse(404, "not_found", "no such thread"))?;
        let last = self.threads[i].messages.len() as u64;
        match (m, action) {
            ("GET", None) => {
                let after: u64 = query(req, "after").and_then(|v| v.parse().ok()).unwrap_or(0);
                let limit: usize = query(req, "limit").and_then(|v| v.parse().ok()).unwrap_or(50);
                let t = &self.threads[i];
                let messages: Vec<Value> = t.messages.iter().filter(|m| m.seq > after).take(limit).map(State::message).collect();
                Ok(Some(Out(200, json!({ "thread": State::summary(t), "messages": messages, "marks": { "read_through": t.read_through, "archived_through": t.archived_through } }))))
            }
            ("POST", Some("read" | "archive")) => {
                let through = body.as_ref().and_then(|b| b["through"].as_u64()).ok_or_else(|| refuse(400, "invalid_input", "through"))?.min(last);
                let t = &mut self.threads[i];
                t.read_through = t.read_through.max(through);
                if action == Some("archive") {
                    t.archived_through = t.archived_through.max(through);
                }
                Ok(Some(Out(200, json!({ "marks": { "read_through": t.read_through, "archived_through": t.archived_through } }))))
            }
            ("POST", Some("messages")) => {
                let b = body.ok_or_else(|| refuse(400, "invalid_input", "not JSON"))?;
                let rid = b["request_id"].as_str().filter(|r| !r.is_empty() && r.len() <= 64).ok_or_else(|| refuse(400, "invalid_input", "request_id"))?.to_string();
                let seen = b["seen_through"].as_u64().ok_or_else(|| refuse(400, "invalid_input", "seen_through"))?;
                let key = format!("{id}#{rid}");
                let raw = serde_json::to_vec(&b).unwrap();
                if let Some((prior, result)) = self.writes.get(&key) {
                    if *prior != raw {
                        return Err(refuse(409, "request_id_conflict", "request id reused"));
                    }
                    return Ok(Some(Out(200, result.clone())));
                }
                if seen < last {
                    return Err(refuse(409, "stale", "the thread has messages you have not seen"));
                }
                if let Some(t) = b["text"].as_str() {
                    if t.is_empty() || t.chars().count() > 1000 {
                        return Err(refuse(400, "invalid_input", "text"));
                    }
                }
                let mut answers = Vec::new();
                for a in b["answers"].as_array().cloned().unwrap_or_default() {
                    let n = a["ask"].as_u64().unwrap_or(0);
                    let t = &self.threads[i];
                    let ask = t.messages.iter().flat_map(|m| &m.asks).find(|x| x.number == n && x.answer.is_none() && x.withdrawn.is_none()).ok_or_else(|| refuse(400, "invalid_input", "that ask is not open"))?;
                    let picks: Vec<u64> = a["picks"].as_array().map(|p| p.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
                    if picks.iter().any(|p| *p as usize >= ask.suggestions.len()) {
                        return Err(refuse(400, "invalid_input", "no such suggestion"));
                    }
                    if !ask.multiple && picks.len() > 1 {
                        return Err(refuse(400, "invalid_input", "one pick only"));
                    }
                    let declined = a["declined"].as_bool().unwrap_or(false);
                    if !declined && picks.is_empty() && a["text"].as_str().is_none() {
                        return Err(refuse(400, "invalid_input", "empty answer"));
                    }
                    let labels: Vec<String> = picks.iter().map(|p| ask.suggestions[*p as usize].clone()).collect();
                    let mut held = json!({ "declined": declined, "picks": picks, "labels": labels });
                    if let Some(text) = a["text"].as_str() {
                        held["text"] = json!(text);
                    }
                    answers.push((n, ask.question.clone(), held));
                }
                if b["text"].as_str().is_none() && answers.is_empty() {
                    return Err(refuse(400, "invalid_input", "an empty message"));
                }
                let seq_next = last + 1;
                for (n, _, held) in &answers {
                    let t = &mut self.threads[i];
                    for m in &mut t.messages {
                        for a in &mut m.asks {
                            if a.number == *n {
                                let mut h = held.clone();
                                h["seq"] = json!(seq_next);
                                a.answer = Some(h);
                            }
                        }
                    }
                }
                let answered = if answers.is_empty() { None } else { Some(Value::Array(answers.iter().map(|(n, q, h)| { let mut o = h.clone(); o["ask"] = json!(n); o["question"] = json!(q); o }).collect())) };
                let seq = self.append(i, json!({ "kind": "person", "name": "Operator" }), b["text"].as_str().map(String::from), b["detail"].as_str().map(String::from), vec![], answered);
                self.threads[i].read_through = seq;
                let result = json!({ "number": self.threads[i].number, "seq": seq });
                self.writes.insert(key, (raw, result.clone()));
                Ok(Some(Out(200, result)))
            }
            _ => Err(refuse(404, "not_found", "not found")),
        }
    }
}
