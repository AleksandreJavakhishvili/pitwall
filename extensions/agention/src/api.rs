//! Everything the tile knows about Agention's operator API, version 1:
//! routes, scopes, the shapes it answers, and what the operator writes.
//! The API shipped in Agention v0.5.0 (WayfarerLabs/agention#41), read here
//! at v0.7.0 (`docs/api/openapi.json` in Agention's repository); when it
//! changes, this is the file to change (and the fake in `tests/fake`).
//!
//! No SDK here: it is plain Rust, tested natively. Answers are read
//! leniently (a field it doesn't know is skipped) but bounded: text is cut
//! to the limits below, lists over theirs are cut too, and a name that
//! could reach a request path is checked.

use serde_json::{json, Value};

/// The operator API version this tile speaks.
pub const API_VERSION: u64 = 1;
/// Where the server lists its operator API versions (no key needed).
pub const VERSIONS_PATH: &str = "/api/operator";
const BASE: &str = "/api/operator/v1";

/// Scopes the key needs (a write scope includes its read).
pub const THREADS_WRITE: &str = "threads:write";
pub const SESSIONS_READ: &str = "sessions:read";

/// How much is read from the server.
pub mod limits {
    /// Threads per page (the API's 1 to 100).
    pub const PAGE: usize = 100;
    /// Threads read across pages.
    pub const THREADS: usize = 500;
    /// Messages read for one thread.
    pub const MESSAGES: usize = 200;
    pub const TITLE: usize = 300;
    pub const TEXT: usize = 8000;
    pub const DETAIL: usize = 40000;
    pub const QUESTION: usize = 2000;
    pub const LABEL: usize = 200;
    pub const DESCRIPTION: usize = 500;
    pub const ANSWER_TEXT: usize = 8000;
    pub const ASKS: usize = 20;
    pub const SUGGESTIONS: usize = 20;
    pub const NAME: usize = 100;
    pub const ERROR: usize = 300;
}

/// What the operator writes, held to the API's bounds before sending.
pub mod input {
    /// A message's text (its summary).
    pub const TEXT: usize = 1000;
    /// A message's detail.
    pub const DETAIL: usize = 20000;
    /// An answer's words.
    pub const ANSWER_TEXT: usize = 4000;
    /// Picks per answer, and the highest suggestion index.
    pub const PICKS: usize = 10;
    /// What Pitwall's reply box takes (split into text and detail).
    pub const REPLY: usize = 4000;
}

/// Agent, session and topic names (Agention's `names.ts`): these go into
/// request paths, so anything else is refused.
pub fn is_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 63 && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

/// Whether `key` looks like an Agention API key: `agention_` and 43
/// characters of unpadded base64url (256 bits).
pub fn is_key(key: &str) -> bool {
    let Some(rest) = key.strip_prefix("agention_") else { return false };
    rest.len() == 43 && rest.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') && b"AEIMQUYcgkosw048".contains(&rest.as_bytes()[42])
}

/// `s` with anything shaped like a key taken out (for words from the
/// server that are shown or logged).
pub fn redact(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find("agention_") {
        out.push_str(&rest[..at]);
        let after = &rest[at + "agention_".len()..];
        let n = after.bytes().take_while(|c| c.is_ascii_alphanumeric() || *c == b'-' || *c == b'_').count();
        if n >= 8 {
            out.push_str("[key]");
        } else {
            out.push_str(&rest[at..at + "agention_".len() + n]);
        }
        rest = &after[n..];
    }
    out.push_str(rest);
    out
}

/// The server's base address from the `server` setting: an `https://`
/// origin (plain `http://` only to this computer), with no user, query or
/// fragment; any path kept, without its trailing `/`.
pub fn base_url(server: &str) -> Result<String, String> {
    let s = server.trim();
    if s.is_empty() {
        return Err("Set your Agention server's address in Settings.".into());
    }
    let (https, rest) = if let Some(r) = s.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = s.strip_prefix("http://") {
        (false, r)
    } else {
        return Err("The server address isn't a web address; it should look like https://agention.example.ts.net.".into());
    };
    if rest.contains(['?', '#', '@', ' ']) {
        return Err("The server address should be just the address you open Agention at, like https://agention.example.ts.net.".into());
    }
    let host = rest.split('/').next().unwrap_or("");
    let name = host.rsplit_once(':').map_or(host, |(h, _)| h).trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    if name.is_empty() {
        return Err("The server address has no host name.".into());
    }
    if !https && !matches!(name.as_str(), "localhost" | "127.0.0.1" | "::1") {
        return Err("The server address must start with https:// (plain http:// only for this computer), so the API key isn't sent in the clear.".into());
    }
    Ok(s.trim_end_matches('/').to_string())
}

// ---- Routes ----------------------------------------------------------------

/// A thread's address: agent, session and number.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadRef {
    pub agent: String,
    pub session: String,
    pub number: u64,
}

impl ThreadRef {
    /// `agent/session/number`.
    pub fn id(&self) -> String {
        format!("{}/{}/{}", self.agent, self.session, self.number)
    }

    /// From an id made by [`ThreadRef::id`].
    pub fn parse(id: &str) -> Option<ThreadRef> {
        let mut parts = id.split('/');
        let (agent, session, number) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || !is_name(agent) || !is_name(session) || number.len() > 15 || !number.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(ThreadRef { agent: agent.into(), session: session.into(), number: number.parse().ok()? })
    }

    fn path(&self) -> String {
        format!("{BASE}/s/{}/{}/threads/{}", self.agent, self.session, self.number)
    }
}

/// A request: method, path (with its query), the scope it needs (`None`:
/// sent without the key), its JSON body, and whether a 404 means the
/// thread is gone (else: a missing scope).
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub method: &'static str,
    pub path: String,
    pub scope: Option<&'static str>,
    pub body: Option<Value>,
    pub thread: bool,
}

fn get(path: String, scope: Option<&'static str>) -> Route {
    Route { method: "GET", path, scope, body: None, thread: false }
}

fn query(pairs: &[(&str, Option<String>)]) -> String {
    let q: Vec<String> = pairs.iter().filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}={}", encode(v)))).collect();
    if q.is_empty() {
        String::new()
    } else {
        format!("?{}", q.join("&"))
    }
}

/// Percent-encoding for a query value (cursors are opaque).
fn encode(v: &str) -> String {
    let mut out = String::new();
    for b in v.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn versions() -> Route {
    get(VERSIONS_PATH.into(), None)
}

/// A page of threads, newest activity first (archived ones aren't listed).
pub fn list_threads(cursor: Option<&str>) -> Route {
    get(format!("{BASE}/threads{}", query(&[("sort", Some("latest".into())), ("limit", Some(limits::PAGE.to_string())), ("cursor", cursor.map(String::from))])), Some("threads:read"))
}

/// A thread and its messages after sequence `after`.
pub fn read_thread(t: &ThreadRef, after: u64, limit: usize) -> Route {
    Route { thread: true, ..get(format!("{}{}", t.path(), query(&[("after", Some(after.to_string())), ("limit", Some(limit.to_string()))])), Some("threads:read")) }
}

/// The sessions, with their status titles.
pub fn sessions(cursor: Option<&str>) -> Route {
    get(format!("{BASE}/sessions{}", query(&[("limit", Some(limits::PAGE.to_string())), ("cursor", cursor.map(String::from))])), Some(SESSIONS_READ))
}

/// Wait up to `seconds` for threads with news since `since`.
pub fn wait(since: Option<&str>, seconds: u32) -> Route {
    get(format!("{BASE}/wait{}", query(&[("wait", Some(seconds.to_string())), ("limit", Some(limits::PAGE.to_string())), ("since", since.map(String::from))])), Some("threads:read"))
}

/// Probes `threads:write` without changing anything: the server checks the
/// scope before it reads the body, so an empty bulk mark-read is refused as
/// invalid input (400) with the scope and as not found (404) without.
pub fn probe_threads_write() -> Route {
    Route { method: "POST", path: format!("{BASE}/reads"), scope: Some(THREADS_WRITE), body: Some(json!({})), thread: false }
}

fn post(t: &ThreadRef, what: &str, body: Value) -> Route {
    Route { method: "POST", path: format!("{}/{what}", t.path()), scope: Some(THREADS_WRITE), body: Some(body), thread: true }
}

/// Mark the thread read through sequence `through`.
pub fn mark_read(t: &ThreadRef, through: u64) -> Route {
    post(t, "read", json!({ "through": through }))
}

/// Archive the thread through sequence `through` (it comes back when the
/// agent writes again).
pub fn archive(t: &ThreadRef, through: u64) -> Route {
    post(t, "archive", json!({ "through": through }))
}

/// Send a message (`text`, maybe `detail`) or answers, having seen the
/// thread through `seen_through`; `request_id` makes it safe to repeat.
pub fn send(t: &ThreadRef, body: Value) -> Route {
    post(t, "messages", body)
}

// ---- What the server answers -------------------------------------------------

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn text_of(v: &Value, max: usize) -> String {
    v.as_str().map(|s| cut(s, max)).unwrap_or_default()
}

/// One line: control characters and line breaks as spaces.
fn line_of(v: &Value, max: usize) -> String {
    let s: String = v.as_str().unwrap_or("").chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    cut(s.trim(), max)
}

fn uint(v: &Value) -> u64 {
    v.as_u64().unwrap_or(0)
}

fn name_of(v: &Value, what: &str) -> Result<String, String> {
    match v.as_str() {
        Some(s) if is_name(s) => Ok(s.to_string()),
        _ => Err(format!("{what} isn't a name")),
    }
}

/// A thread as the list, a thread read and the wait give it.
#[derive(Debug, Clone, PartialEq)]
pub struct Thread {
    pub at: ThreadRef,
    pub title: String,
    pub unread: u32,
    /// Its open asks: how many, and the sequences of the messages they are in.
    pub open_asks: u32,
    pub open_ask_seqs: Vec<u64>,
    pub blocking: bool,
    pub archived: bool,
    /// Its latest activity, in milliseconds since 1970.
    pub updated_ms: i64,
    pub last_seq: u64,
}

impl Thread {
    pub fn id(&self) -> String {
        self.at.id()
    }
}

pub fn thread(v: &Value) -> Result<Thread, String> {
    if !v.is_object() {
        return Err("a thread isn't an object".into());
    }
    let at = ThreadRef { agent: name_of(&v["agent"], "a thread's agent")?, session: name_of(&v["session"], "a thread's session")?, number: v["number"].as_u64().ok_or("a thread has no number")? };
    let open = &v["open_asks"];
    let mut seqs: Vec<u64> = open["asks"].as_array().map(|a| a.iter().take(50).filter_map(|x| x["seq"].as_u64()).collect()).unwrap_or_default();
    seqs.sort_unstable();
    seqs.dedup();
    Ok(Thread {
        at,
        title: line_of(&v["title"], limits::TITLE),
        unread: uint(&v["unread"]).min(u32::MAX as u64) as u32,
        open_asks: uint(&open["count"]).min(u32::MAX as u64) as u32,
        open_ask_seqs: seqs,
        blocking: open["blocking"].as_bool().unwrap_or(false),
        archived: v["archived"].as_bool().unwrap_or(false),
        updated_ms: v["updated_at"].as_str().and_then(parse_time).unwrap_or(0),
        last_seq: uint(&v["last_seq"]),
    })
}

/// A page of threads and the cursor of the next.
pub fn thread_page(v: &Value) -> Result<(Vec<Thread>, Option<String>), String> {
    let list = v["threads"].as_array().ok_or("the answer has no threads")?;
    let threads = list.iter().take(limits::PAGE).map(thread).collect::<Result<Vec<_>, _>>()?;
    Ok((threads, cursor(&v["next"])))
}

fn cursor(v: &Value) -> Option<String> {
    v.as_str().filter(|c| !c.is_empty() && c.len() <= 1024).map(String::from)
}

/// The operator API versions the server serves.
pub fn version_list(v: &Value) -> Result<Vec<u64>, String> {
    v["versions"].as_array().map(|a| a.iter().take(50).filter_map(Value::as_u64).collect()).ok_or_else(|| "its answer isn't a list of versions".into())
}

/// Who wrote a message.
#[derive(Debug, Clone, PartialEq)]
pub enum Author {
    /// An agent's session.
    Agent { agent: String, session: String },
    /// The operator: whoever holds the passkey, or a key acting for them.
    Operator { name: String },
}

/// A suggestion an ask offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub label: String,
    pub description: String,
}

/// An answer as the server keeps it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Answer {
    pub declined: bool,
    pub picks: Vec<u32>,
    pub labels: Vec<String>,
    pub text: String,
}

/// An ask's state.
#[derive(Debug, Clone, PartialEq)]
pub enum AskState {
    Open,
    Answered(Answer),
    Declined,
    /// Withdrawn by the agent, maybe with why.
    Withdrawn(String),
    /// Its message was replaced (and asked again under new numbers).
    Closed,
}

/// A question in a message.
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    pub number: u64,
    pub question: String,
    pub suggestions: Vec<Suggestion>,
    pub multiple: bool,
    pub blocking: bool,
    pub state: AskState,
}

/// An answer a message gives, as words that read without the ask.
#[derive(Debug, Clone, PartialEq)]
pub struct Answered {
    pub ask: u64,
    pub question: String,
    pub answer: Answer,
}

/// A message in a thread.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub seq: u64,
    pub author: Author,
    pub time_ms: i64,
    pub text: String,
    pub detail: String,
    pub asks: Vec<Ask>,
    pub answers: Vec<Answered>,
}

fn answer_of(v: &Value) -> Answer {
    Answer {
        declined: v["declined"].as_bool().unwrap_or(false),
        picks: v["picks"].as_array().map(|a| a.iter().take(limits::SUGGESTIONS).filter_map(|p| p.as_u64().filter(|p| *p < 1000).map(|p| p as u32)).collect()).unwrap_or_default(),
        labels: v["labels"].as_array().map(|a| a.iter().take(limits::SUGGESTIONS).map(|l| line_of(l, limits::LABEL)).collect()).unwrap_or_default(),
        text: text_of(&v["text"], limits::ANSWER_TEXT),
    }
}

fn ask(v: &Value) -> Option<Ask> {
    let number = v["number"].as_u64().filter(|n| *n >= 1)?;
    let answer = v.get("answer").filter(|a| a.is_object()).map(answer_of);
    let state = if v["open"].as_bool().unwrap_or(false) {
        AskState::Open
    } else if let Some(a) = answer {
        if a.declined {
            AskState::Declined
        } else {
            AskState::Answered(a)
        }
    } else if let Some(w) = v.get("withdrawal").filter(|w| w.is_object()) {
        AskState::Withdrawn(line_of(&w["reason"], 500))
    } else {
        AskState::Closed
    };
    Some(Ask {
        number,
        question: line_of(&v["question"], limits::QUESTION),
        suggestions: v["suggestions"].as_array().map(|a| a.iter().take(limits::SUGGESTIONS).map(|s| Suggestion { label: line_of(&s["label"], limits::LABEL), description: line_of(&s["description"], limits::DESCRIPTION) }).collect()).unwrap_or_default(),
        multiple: v["multiple"].as_bool().unwrap_or(false),
        blocking: v["impact"].as_str() == Some("blocking"),
        state,
    })
}

/// A message; `None` for one replaced by a later one (not live), which the
/// thread no longer counts.
pub fn message(v: &Value) -> Result<Option<Message>, String> {
    if v["live"].as_bool() == Some(false) {
        return Ok(None);
    }
    let a = &v["author"];
    let author = match a["kind"].as_str() {
        Some("session") => Author::Agent { agent: name_of(&a["agent"], "a message's agent")?, session: name_of(&a["session"], "a message's session")? },
        Some("person") => Author::Operator { name: line_of(&a["name"], limits::NAME) },
        _ => return Err("a message's author isn't a session or a person".into()),
    };
    Ok(Some(Message {
        seq: v["seq"].as_u64().ok_or("a message has no sequence")?,
        author,
        time_ms: v["created_at"].as_str().and_then(parse_time).unwrap_or(0),
        text: text_of(&v["text"], limits::TEXT),
        detail: text_of(&v["detail"], limits::DETAIL),
        asks: v["asks"].as_array().map(|a| a.iter().take(limits::ASKS).filter_map(ask).collect()).unwrap_or_default(),
        answers: v["answers"].as_array().map(|a| a.iter().take(limits::ASKS).filter_map(|x| Some(Answered { ask: x["ask"].as_u64()?, question: line_of(&x["question"], limits::QUESTION), answer: answer_of(x) })).collect()).unwrap_or_default(),
    }))
}

/// A thread read: the thread, its live messages on this page, and the
/// highest sequence on the page (live or not) to read on from.
#[derive(Debug, Clone, PartialEq)]
pub struct ThreadRead {
    pub thread: Thread,
    pub messages: Vec<Message>,
    pub page_end: u64,
    pub page_count: usize,
}

pub fn thread_read(v: &Value) -> Result<ThreadRead, String> {
    let all = v["messages"].as_array().ok_or("the answer has no messages")?;
    let mut messages = Vec::new();
    let mut page_end = 0;
    for m in all.iter().take(limits::PAGE) {
        page_end = page_end.max(uint(&m["seq"]));
        if let Some(m) = message(m)? {
            messages.push(m);
        }
    }
    Ok(ThreadRead { thread: thread(&v["thread"])?, messages, page_end, page_count: all.len().min(limits::PAGE) })
}

/// A session's line: its agent and name, and its status title if any.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub agent: String,
    pub session: String,
    pub title: String,
}

pub fn session_page(v: &Value) -> Result<(Vec<Session>, Option<String>), String> {
    let agents = v["agents"].as_array().ok_or("the answer has no agents")?;
    let mut out = Vec::new();
    for a in agents.iter().take(limits::PAGE) {
        let agent = name_of(&a["agent"], "a session's agent")?;
        for s in a["sessions"].as_array().map(|s| s.iter().take(500).collect::<Vec<_>>()).unwrap_or_default() {
            out.push(Session { agent: agent.clone(), session: name_of(&s["session"], "a session")?, title: line_of(&s["status"]["title"], limits::TITLE) });
        }
    }
    Ok((out, cursor(&v["next"])))
}

/// The wait's answer: threads with news, and where to wait from next.
pub fn waited(v: &Value) -> Result<(Vec<Thread>, Option<String>), String> {
    let list = v["threads"].as_array().ok_or("the answer has no threads")?;
    Ok((list.iter().take(limits::PAGE).map(thread).collect::<Result<Vec<_>, _>>()?, cursor(&v["since"])))
}

/// The shared error shape: its code and message (bounded, keys taken out).
pub fn error_of(v: &Value) -> Option<(String, String)> {
    let e = v.get("error")?;
    let code = e["code"].as_str()?;
    Some((cut(code, 40), redact(&line_of(&e["message"], limits::ERROR))))
}

// ---- What the operator writes ------------------------------------------------

/// One answer to ask `ask`: suggestions picked (by index), words, or a
/// decline.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reply {
    pub ask: u64,
    pub picks: Vec<u32>,
    pub text: String,
    pub declined: bool,
}

/// The body of answers, checked against the API's bounds.
pub fn answers_body(request_id: &str, seen_through: u64, answers: &[Reply]) -> Result<Value, String> {
    if answers.is_empty() || answers.len() > 10 {
        return Err("send 1 to 10 answers".into());
    }
    let mut out = Vec::new();
    for a in answers {
        if a.declined {
            out.push(json!({ "ask": a.ask, "declined": true }));
            continue;
        }
        let mut o = json!({ "ask": a.ask });
        if !a.picks.is_empty() {
            let mut picks = a.picks.clone();
            picks.sort_unstable();
            picks.dedup();
            if picks.len() > input::PICKS || picks.iter().any(|p| *p as usize >= input::PICKS) {
                return Err("pick at most 10 of the first 10 suggestions".into());
            }
            o["picks"] = json!(picks);
        }
        let words = a.text.trim();
        if words.chars().count() > input::ANSWER_TEXT {
            return Err(format!("an answer has at most {} characters", input::ANSWER_TEXT));
        }
        if !words.is_empty() {
            o["text"] = json!(words);
        }
        if a.picks.is_empty() && words.is_empty() {
            return Err("pick a suggestion or write an answer".into());
        }
        out.push(o);
    }
    Ok(json!({ "request_id": request_id, "seen_through": seen_through, "answers": out }))
}

/// A message's body. Agention's text is a summary of at most 1 000
/// characters, with the rest in detail; Pitwall's reply box allows 4 000. A
/// longer reply keeps its first paragraph (cut) as the summary and goes
/// whole into detail.
pub fn message_body(request_id: &str, seen_through: u64, reply: &str) -> Result<Value, String> {
    let words = reply.trim();
    if words.is_empty() {
        return Err("the message is empty".into());
    }
    if words.chars().count() > input::REPLY {
        return Err(format!("a message has at most {} characters", input::REPLY));
    }
    let mut body = json!({ "request_id": request_id, "seen_through": seen_through });
    if words.chars().count() <= input::TEXT {
        body["text"] = json!(words);
        return Ok(body);
    }
    let first = words.split("\n\n").next().unwrap_or(words);
    let mut summary: String = first.chars().take(input::TEXT - 1).collect();
    if summary.len() < first.len() {
        if let Some(space) = summary.rfind(' ').filter(|s| *s > summary.len() / 2) {
            summary.truncate(space);
        }
        summary.push('…');
    }
    body["text"] = json!(summary);
    body["detail"] = json!(words);
    Ok(body)
}

// ---- Time --------------------------------------------------------------------

/// An RFC 3339 time in UTC (`2026-11-20T10:00:00.000Z`, or with an offset)
/// as milliseconds since 1970.
pub fn parse_time(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse().ok() };
    let (y, mo, d, h, mi, sec) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    let mut i = 19;
    let mut ms = 0i64;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let frac = &s[start..i];
        ms = format!("{:0<3}", &frac[..frac.len().min(3)]).parse().ok()?;
    }
    let offset_min = match b.get(i)? {
        b'Z' | b'z' => 0,
        b'+' | b'-' => {
            let sign = if b[i] == b'-' { -1 } else { 1 };
            sign * (num(i + 1..i + 3)? * 60 + num(i + 4..i + 6)?)
        }
        _ => return None,
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    // Days from the civil date (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3600 + mi * 60 + sec) - offset_min * 60) * 1000 + ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_keys_and_addresses() {
        assert!(is_name("my-app") && is_name("s1") && !is_name("My-App") && !is_name("-a") && !is_name("a/b") && !is_name(""));
        let key = format!("agention_{}A", "A".repeat(42));
        assert!(is_key(&key));
        assert!(!is_key("agention_short") && !is_key(&format!("other_{}A", "A".repeat(42))) && !is_key(&format!("agention_{}B", "A".repeat(42))));
        assert_eq!(redact(&format!("refused {key} here")), "refused [key] here");
        assert_eq!(redact("agention_x stays"), "agention_x stays");
        assert_eq!(base_url(" https://agention.example.ts.net/ ").unwrap(), "https://agention.example.ts.net");
        assert_eq!(base_url("http://127.0.0.1:8787").unwrap(), "http://127.0.0.1:8787");
        assert!(base_url("http://agention.example.ts.net").unwrap_err().contains("https://"));
        assert!(base_url("").is_err() && base_url("agention.example.ts.net").is_err() && base_url("https://a.example?x=1").is_err());
    }

    #[test]
    fn routes() {
        let t = ThreadRef::parse("my-app/s1/3").unwrap();
        assert_eq!(t.id(), "my-app/s1/3");
        assert!(ThreadRef::parse("a/b").is_none() && ThreadRef::parse("A/b/1").is_none() && ThreadRef::parse("a/b/x").is_none());
        assert_eq!(read_thread(&t, 4, 100).path, "/api/operator/v1/s/my-app/s1/threads/3?after=4&limit=100");
        assert_eq!(list_threads(Some("a b")).path, "/api/operator/v1/threads?sort=latest&limit=100&cursor=a%20b");
        assert_eq!(wait(None, 25).path, "/api/operator/v1/wait?wait=25&limit=100");
        assert_eq!(archive(&t, 7).body, Some(json!({"through": 7})));
        assert!(versions().scope.is_none() && mark_read(&t, 1).thread);
    }

    #[test]
    fn a_thread_and_its_messages() {
        let v = json!({
            "thread": {"agent": "my-app", "session": "s1", "number": 1, "title": "Which\ndatabase?", "unread": 2, "last_seq": 2,
                       "archived": false, "updated_at": "2026-11-20T10:00:02.500Z",
                       "open_asks": {"count": 1, "blocking": true, "asks": [{"number": 1, "seq": 1}]}},
            "messages": [
                {"seq": 1, "live": true, "author": {"kind": "session", "agent": "my-app", "session": "s1"}, "created_at": "2026-11-20T10:00:01Z",
                 "text": "Which one?", "asks": [{"number": 1, "question": "SQLite or Postgres?", "suggestions": [{"label": "Postgres"}, {"label": "SQLite", "description": "simple"}], "multiple": false, "impact": "blocking", "open": true}]},
                {"seq": 2, "live": false, "author": {"kind": "session", "agent": "my-app", "session": "s1"}, "created_at": "2026-11-20T10:00:02Z"}
            ],
            "marks": {"read_through": 0}
        });
        let r = thread_read(&v).unwrap();
        assert_eq!((r.thread.title.as_str(), r.thread.unread, r.thread.open_asks, r.thread.blocking), ("Which database?", 2, 1, true));
        assert_eq!(r.thread.updated_ms, parse_time("2026-11-20T10:00:02.500Z").unwrap());
        assert_eq!((r.messages.len(), r.page_end, r.page_count), (1, 2, 2), "the replaced message is left out");
        let a = &r.messages[0].asks[0];
        assert_eq!((a.state.clone(), a.blocking, a.suggestions[1].description.as_str()), (AskState::Open, true, "simple"));
        assert!(thread(&json!({"agent": "../x", "session": "s", "number": 1})).is_err(), "a name that could reach a path");
    }

    #[test]
    fn bodies() {
        let b = answers_body("r1", 3, &[Reply { ask: 1, picks: vec![1, 1], text: " in the data dir ".into(), declined: false }]).unwrap();
        assert_eq!(b, json!({"request_id": "r1", "seen_through": 3, "answers": [{"ask": 1, "picks": [1], "text": "in the data dir"}]}));
        assert_eq!(answers_body("r", 0, &[Reply { ask: 2, declined: true, ..Default::default() }]).unwrap()["answers"][0], json!({"ask": 2, "declined": true}));
        assert!(answers_body("r", 0, &[Reply { ask: 1, ..Default::default() }]).is_err());
        assert_eq!(message_body("r", 1, " hi ").unwrap()["text"], "hi");
        let long = format!("{}\n\n{}", "word ".repeat(300), "more");
        let m = message_body("r", 1, &long).unwrap();
        assert!(m["text"].as_str().unwrap().ends_with('…') && m["text"].as_str().unwrap().chars().count() <= 1000);
        assert_eq!(m["detail"], long.trim());
        assert!(message_body("r", 1, &"x".repeat(4001)).is_err() && message_body("r", 1, "  ").is_err());
    }

    #[test]
    fn times() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2000-03-01T00:00:00.1Z"), Some(951_868_800_100));
        assert_eq!(parse_time("2026-11-20T12:00:00+02:00"), parse_time("2026-11-20T10:00:00Z"));
        assert_eq!(parse_time("yesterday"), None);
    }
}
