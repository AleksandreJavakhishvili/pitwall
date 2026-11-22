//! The adapter protocol (docs/spec/extensions.md, "Adapter protocol"): an
//! extension's adapter is a separate process that Pitwall talks to in JSON
//! lines over its stdin and stdout, one object per line, each with a
//! `type`. Pitwall draws everything itself from what the adapter sends;
//! nothing it sends is run, loaded or followed.
//!
//! Everything from an adapter is untrusted: a line longer than
//! [`MAX_LINE`] or more than [`MAX_LINES_PER_SEC`] lines a second stops it
//! ([`LineGuard`]); strings are cut to their limits, control and
//! bidirectional-override characters removed, lists cut to their lengths;
//! an item without a usable id is dropped. Unknown `type`s and keys are
//! ignored, so a newer adapter still works with this Pitwall.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// The protocol version both sides say in `hello`.
pub const PROTOCOL: u64 = 1;
/// The longest line an adapter may send (bytes, without the newline).
pub const MAX_LINE: usize = 1 << 20;
/// More lines than this in one second stop the adapter.
pub const MAX_LINES_PER_SEC: u32 = 200;
/// The most threads a list keeps.
pub const MAX_THREADS: usize = 500;
/// The most messages a thread keeps (its latest).
pub const MAX_MESSAGES: usize = 200;
/// The most asks in a message, suggestions in an ask, picks in an answer.
pub const MAX_ASKS: usize = 10;
pub const MAX_SUGGESTIONS: usize = 10;
/// The longest id (longer ones are dropped, never cut: ids are matched).
pub const MAX_ID: usize = 200;
/// The longest text the user may send in one answer or message.
pub const MAX_REPLY: usize = 4000;

/// The state of the adapter's connection to its service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnState {
    Connecting,
    Ready,
    /// A required credential is missing, wrong, expired or lacks access.
    SignedOut,
    Error,
}

/// The names a thread is from (an agent and its session, in the service's
/// words), matched against Pitwall's agents with the manifest's
/// `matchAgents`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct From {
    pub agent: String,
    pub session: String,
}

/// A thread in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub id: String,
    pub title: String,
    pub from: Option<From>,
    /// Unread messages in it (`true` reads as 1, `false` as 0).
    pub unread: u32,
    /// Asks in it still waiting for the user.
    pub open_asks: u32,
    /// One of its open asks holds its agent up.
    pub blocking: bool,
    /// When something last happened in it (ISO 8601, as the adapter says).
    pub last_activity: String,
}

/// Who wrote a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthorKind {
    /// An agent (its name in `name`).
    Agent,
    /// Someone else.
    Person,
    /// The user.
    You,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    pub kind: AuthorKind,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub description: String,
}

/// How an ask was answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub declined: bool,
    /// The suggestions picked (indexes into `suggestions`).
    pub picks: Vec<u32>,
    pub text: String,
}

/// A question in a message, with its suggested answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    /// Its number in the thread (what an answer names).
    pub number: u32,
    pub question: String,
    /// More than one suggestion may be picked.
    pub multiple: bool,
    /// Still waiting for an answer.
    pub open: bool,
    pub suggestions: Vec<Suggestion>,
    pub answer: Option<Answer>,
    /// Withdrawn by its asker, and why.
    pub withdrawn: Option<String>,
    /// Its agent is held up until it is answered.
    pub blocking: bool,
}

/// An answer a message carries (to an ask in this or an earlier message),
/// with the words of what was picked, so it reads on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answered {
    pub ask: u32,
    pub question: String,
    pub declined: bool,
    pub labels: Vec<String>,
    pub text: String,
}

/// An ask a message withdraws, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Withdrawal {
    pub ask: u32,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub author: Author,
    pub time: String,
    /// Markdown, shown first.
    pub text: String,
    /// Markdown, shown folded under the text.
    pub detail: String,
    pub asks: Vec<Ask>,
    /// Answers it gives to asks.
    pub answers: Vec<Answered>,
    /// Asks it withdraws.
    pub withdrawals: Vec<Withdrawal>,
}

/// A thread with its messages (the answer to `open`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadDetail {
    pub id: String,
    pub title: String,
    pub from: Option<From>,
    pub unread: u32,
    pub messages: Vec<Message>,
    /// Older messages exist that weren't sent.
    pub earlier: bool,
    /// The user may write in it.
    pub can_reply: bool,
}

/// Totals for the whole inbox, when the adapter knows them (else Pitwall
/// counts its list).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    /// Unread threads.
    pub unread: u32,
    /// Open asks.
    pub open_asks: u32,
    /// Open asks holding an agent up.
    pub blocking: u32,
}

/// What an adapter sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromAdapter {
    Hello { protocol: u64 },
    Status { state: ConnState, message: String },
    /// The whole list, replacing the last one.
    Threads(Vec<Thread>),
    Thread(ThreadDetail),
    Counts(Counts),
    /// How request `req` went (`message`: why not, in plain words).
    Result { req: u64, ok: bool, message: String },
    /// A `type` this Pitwall doesn't know.
    Unknown,
}

/// What Pitwall sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToAdapter {
    /// The first line: the protocol, Pitwall's version, the extension's
    /// settings and its secrets (only ever here, never in argv or the
    /// environment).
    Hello { pitwall: String, settings: Map<String, Value>, secrets: Map<String, Value> },
    /// Send this thread (`thread`), and keep it current while it is open.
    Open { thread: String },
    /// Its view closed.
    Close { thread: String },
    /// Answer ask `ask`: picks and/or own words, or a decline.
    Answer { req: u64, thread: String, ask: u32, picks: Vec<u32>, text: Option<String>, declined: bool },
    /// A plain message in the thread.
    Message { req: u64, thread: String, text: String },
    MarkRead { req: u64, thread: String },
    Archive { req: u64, thread: String },
    /// Send the list again now.
    Refresh,
}

impl ToAdapter {
    /// The line to send (no newline).
    pub fn line(&self) -> String {
        let v = match self {
            ToAdapter::Hello { pitwall, settings, secrets } => {
                json!({"type": "hello", "protocol": PROTOCOL, "pitwall": pitwall, "settings": settings, "secrets": secrets})
            }
            ToAdapter::Open { thread } => json!({"type": "open", "thread": thread}),
            ToAdapter::Close { thread } => json!({"type": "close", "thread": thread}),
            ToAdapter::Answer { req, thread, ask, picks, text, declined } => {
                let mut v = json!({"type": "answer", "req": req, "thread": thread, "ask": ask, "picks": picks, "declined": declined});
                if let Some(t) = text {
                    v["text"] = json!(t);
                }
                v
            }
            ToAdapter::Message { req, thread, text } => json!({"type": "message", "req": req, "thread": thread, "text": text}),
            ToAdapter::MarkRead { req, thread } => json!({"type": "markRead", "req": req, "thread": thread}),
            ToAdapter::Archive { req, thread } => json!({"type": "archive", "req": req, "thread": thread}),
            ToAdapter::Refresh => json!({"type": "refresh"}),
        };
        v.to_string()
    }
}

/// `s` safe to show: control characters (but newlines and tabs when
/// `multiline`) and bidirectional overrides gone, at most `max`
/// characters (cut with "…").
pub fn clean(s: &str, max: usize, multiline: bool) -> String {
    let bidi = |c: char| matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}');
    let mut out: String = s
        .chars()
        .filter(|c| !bidi(*c))
        .map(|c| if c.is_control() && !(multiline && (c == '\n' || c == '\t')) { ' ' } else { c })
        .collect();
    if !multiline {
        out = out.trim().to_string();
    }
    if out.chars().count() > max {
        out = out.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
    }
    out
}

fn text(v: &Value, key: &str, max: usize) -> String {
    v.get(key).and_then(Value::as_str).map(|s| clean(s, max, false)).unwrap_or_default()
}

fn long_text(v: &Value, key: &str, max: usize) -> String {
    v.get(key).and_then(Value::as_str).map(|s| clean(s, max, true)).unwrap_or_default()
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn count(v: &Value, key: &str) -> u32 {
    v.get(key).and_then(Value::as_u64).map_or(0, |n| n.min(u32::MAX as u64) as u32)
}

/// An id: a non-empty string of at most [`MAX_ID`] characters without
/// control characters (else the item is dropped).
fn id(v: &Value, key: &str) -> Option<String> {
    let s = v.get(key)?.as_str()?;
    (!s.is_empty() && s.chars().count() <= MAX_ID && !s.chars().any(char::is_control)).then(|| s.to_string())
}

/// A count that may be said as a boolean (`true`: 1).
fn count_or_flag(v: &Value, key: &str) -> u32 {
    match v.get(key) {
        Some(Value::Bool(b)) => u32::from(*b),
        _ => count(v, key),
    }
}

fn number(v: &Value, key: &str) -> Option<u32> {
    v.get(key)?.as_u64().filter(|n| *n <= u32::MAX as u64).map(|n| n as u32)
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

fn from(v: &Value) -> Option<From> {
    let f = v.get("from")?;
    let name = |k: &str| f.get(k).and_then(Value::as_str).filter(|s| !s.is_empty() && s.len() <= 100 && !s.chars().any(char::is_control)).map(String::from);
    Some(From { agent: name("agent")?, session: name("session")? })
}

fn thread(v: &Value) -> Option<Thread> {
    Some(Thread {
        id: id(v, "id")?,
        title: text(v, "title", 300),
        from: from(v),
        unread: count_or_flag(v, "unread"),
        open_asks: count(v, "openAsks"),
        blocking: flag(v, "blocking"),
        last_activity: text(v, "lastActivity", 40),
    })
}

fn ask(v: &Value) -> Option<Ask> {
    let number = number(v, "number")?;
    let suggestions: Vec<Suggestion> = list(v, "suggestions")
        .iter()
        .take(MAX_SUGGESTIONS)
        .map(|s| Suggestion { label: text(s, "label", 200), description: text(s, "description", 500) })
        .collect();
    let answer = v.get("answer").filter(|a| a.is_object()).map(|a| Answer {
        declined: flag(a, "declined"),
        picks: list(a, "picks").iter().filter_map(Value::as_u64).filter(|p| (*p as usize) < suggestions.len()).take(MAX_SUGGESTIONS).map(|p| p as u32).collect(),
        text: long_text(a, "text", 8000),
    });
    Some(Ask {
        number,
        question: long_text(v, "question", 2000),
        multiple: flag(v, "multiple"),
        open: flag(v, "open"),
        suggestions,
        answer,
        withdrawn: v.get("withdrawn").and_then(Value::as_str).map(|s| clean(s, 500, false)),
        blocking: flag(v, "blocking"),
    })
}

fn message(v: &Value) -> Option<Message> {
    let a = v.get("author");
    let kind = a.and_then(|a| a.get("kind")).and_then(|k| serde_json::from_value::<AuthorKind>(k.clone()).ok()).unwrap_or(AuthorKind::Person);
    Some(Message {
        id: id(v, "id")?,
        author: Author { kind, name: a.map(|a| text(a, "name", 100)).unwrap_or_default() },
        time: text(v, "time", 40),
        text: long_text(v, "text", 8000),
        detail: long_text(v, "detail", 40_000),
        asks: list(v, "asks").iter().filter_map(ask).take(MAX_ASKS).collect(),
        answers: list(v, "answers")
            .iter()
            .filter_map(|a| {
                Some(Answered {
                    ask: number(a, "ask")?,
                    question: long_text(a, "question", 2000),
                    declined: flag(a, "declined"),
                    labels: list(a, "labels").iter().filter_map(Value::as_str).take(MAX_SUGGESTIONS).map(|l| clean(l, 200, false)).collect(),
                    text: long_text(a, "text", 8000),
                })
            })
            .take(MAX_ASKS)
            .collect(),
        withdrawals: list(v, "withdrawals").iter().filter_map(|w| Some(Withdrawal { ask: number(w, "ask")?, reason: text(w, "reason", 500) })).take(MAX_ASKS).collect(),
    })
}

fn detail(v: &Value) -> Option<ThreadDetail> {
    let mut all: Vec<Message> = list(v, "messages").iter().filter_map(message).collect();
    let earlier = flag(v, "earlier") || all.len() > MAX_MESSAGES;
    // Too many: keep the ones with open asks, then the latest, in order.
    if all.len() > MAX_MESSAGES {
        let asking = |m: &Message| m.asks.iter().any(|a| a.open);
        let mut keep = vec![false; all.len()];
        let mut left = MAX_MESSAGES;
        for (i, m) in all.iter().enumerate() {
            if left > 0 && asking(m) {
                keep[i] = true;
                left -= 1;
            }
        }
        for i in (0..all.len()).rev() {
            if left == 0 {
                break;
            }
            if !keep[i] {
                keep[i] = true;
                left -= 1;
            }
        }
        let mut k = keep.into_iter();
        all.retain(|_| k.next().unwrap_or(false));
    }
    Some(ThreadDetail {
        id: id(v, "id")?,
        title: text(v, "title", 300),
        from: from(v),
        unread: count_or_flag(v, "unread"),
        messages: all,
        earlier,
        can_reply: v.get("canReply").and_then(Value::as_bool).unwrap_or(true),
    })
}

/// One line from an adapter, read and cut to its limits; why not, when it
/// isn't a protocol message at all.
pub fn parse_line(line: &str) -> Result<FromAdapter, String> {
    if line.len() > MAX_LINE {
        return Err("a line over 1 MiB".into());
    }
    let v: Value = serde_json::from_str(line).map_err(|_| "a line that isn't JSON".to_string())?;
    if !v.is_object() {
        return Err("a line that isn't a JSON object".into());
    }
    let Some(kind) = v.get("type").and_then(Value::as_str) else {
        return Err("a message without a \"type\"".into());
    };
    Ok(match kind {
        "hello" => FromAdapter::Hello { protocol: v.get("protocol").and_then(Value::as_u64).unwrap_or(0) },
        "status" => {
            let state = v.get("state").and_then(|s| serde_json::from_value::<ConnState>(s.clone()).ok()).ok_or("a status with an unknown \"state\"")?;
            FromAdapter::Status { state, message: text(&v, "message", 500) }
        }
        "threads" => FromAdapter::Threads(list(&v, "threads").iter().filter_map(thread).take(MAX_THREADS).collect()),
        "thread" => FromAdapter::Thread(v.get("thread").and_then(detail).ok_or("a thread without a usable \"id\"")?),
        "counts" => FromAdapter::Counts(Counts { unread: count(&v, "unread"), open_asks: count(&v, "openAsks"), blocking: count(&v, "blocking") }),
        "result" => FromAdapter::Result {
            req: v.get("req").and_then(Value::as_u64).ok_or("a result without \"req\"")?,
            ok: flag(&v, "ok"),
            message: text(&v, "message", 500),
        },
        _ => FromAdapter::Unknown,
    })
}

/// Counts an adapter's lines: more than [`MAX_LINES_PER_SEC`] in a second
/// is a flood.
#[derive(Debug, Default)]
pub struct LineGuard {
    window_ms: u64,
    lines: u32,
}

impl LineGuard {
    /// A line came at `now_ms` (a monotonic clock): `Err` when it is one
    /// too many.
    pub fn line(&mut self, now_ms: u64) -> Result<(), String> {
        if now_ms.saturating_sub(self.window_ms) >= 1000 {
            self.window_ms = now_ms;
            self.lines = 0;
        }
        self.lines += 1;
        if self.lines > MAX_LINES_PER_SEC {
            return Err(format!("it sent more than {MAX_LINES_PER_SEC} lines in a second"));
        }
        Ok(())
    }
}

/// Read one line of at most [`MAX_LINE`] bytes from `r` into `buf` (its
/// newline dropped). `Ok(false)`: the end. A longer line is an error and
/// isn't kept.
pub fn read_line(r: &mut impl std::io::BufRead, buf: &mut Vec<u8>) -> std::io::Result<bool> {
    buf.clear();
    loop {
        let (done, used) = {
            let avail = r.fill_buf()?;
            if avail.is_empty() {
                return Ok(!buf.is_empty());
            }
            match avail.iter().position(|b| *b == b'\n') {
                Some(i) => {
                    buf.extend_from_slice(&avail[..i]);
                    (true, i + 1)
                }
                None => {
                    buf.extend_from_slice(avail);
                    (false, avail.len())
                }
            }
        };
        r.consume(used);
        if buf.len() > MAX_LINE {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "a line over 1 MiB"));
        }
        if done {
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            return Ok(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_list_is_read_and_cut_to_its_limits() {
        let long = "t".repeat(400);
        let line = json!({"type": "threads", "threads": [
            {"id": "a/b/1", "title": long, "from": {"agent": "my-app", "session": "s1"}, "unread": true, "openAsks": 2, "lastActivity": "2026-11-21T10:00:00Z", "later": 1},
            {"id": "", "title": "no id"},
            {"title": "no id at all"},
            {"id": "x".repeat(201)},
            {"id": "c", "title": "evil\u{202E}txt\nnext", "from": {"agent": "a"}}
        ]})
        .to_string();
        let FromAdapter::Threads(t) = parse_line(&line).unwrap() else { panic!() };
        assert_eq!(t.len(), 2, "items without a usable id are dropped");
        assert_eq!(t[0].title.chars().count(), 300);
        assert!(t[0].title.ends_with('…'));
        assert_eq!(t[0].from, Some(From { agent: "my-app".into(), session: "s1".into() }));
        assert_eq!((t[0].unread, t[0].open_asks, t[0].blocking), (1, 2, false));
        assert_eq!(t[1].title, "eviltxt next", "no override, no newline");
        assert_eq!(t[1].from, None, "half a from is none");
        let many: Vec<Value> = (0..600).map(|i| json!({"id": i.to_string()})).collect();
        let FromAdapter::Threads(t) = parse_line(&json!({"type": "threads", "threads": many}).to_string()).unwrap() else { panic!() };
        assert_eq!(t.len(), MAX_THREADS);
    }

    #[test]
    fn a_thread_with_asks_is_read() {
        let line = json!({"type": "thread", "thread": {"id": "t1", "title": "Deploy?", "canReply": false, "messages": [
            {"id": "1", "author": {"kind": "agent", "name": "my-app/s1"}, "time": "2026-11-21T10:00:00Z", "text": "**Ship** it?\n<script>x</script>", "detail": "More",
             "asks": [{"number": 1, "question": "Which?", "multiple": true, "open": false,
                       "suggestions": [{"label": "A", "description": "first"}, {"label": "B"}],
                       "answer": {"declined": false, "picks": [1, 7], "text": "and C"}},
                      {"number": "two"}]},
            {"id": "2", "author": {"kind": "robot"}, "text": "?"}
        ]}})
        .to_string();
        let FromAdapter::Thread(d) = parse_line(&line).unwrap() else { panic!() };
        assert!(!d.can_reply);
        assert_eq!(d.messages.len(), 2);
        let m = &d.messages[0];
        assert_eq!(m.author, Author { kind: AuthorKind::Agent, name: "my-app/s1".into() });
        assert_eq!(m.text, "**Ship** it?\n<script>x</script>", "kept as text: it is drawn inert");
        assert_eq!(m.asks.len(), 1, "an ask without a number is dropped");
        let a = &m.asks[0];
        assert_eq!(a.suggestions[1], Suggestion { label: "B".into(), description: String::new() });
        assert_eq!(a.answer.as_ref().unwrap().picks, [1], "a pick past the suggestions is dropped");
        assert_eq!(d.messages[1].author.kind, AuthorKind::Person, "an unknown kind");
        let many: Vec<Value> = (0..250).map(|i| json!({"id": i.to_string()})).collect();
        let FromAdapter::Thread(d) = parse_line(&json!({"type": "thread", "thread": {"id": "t", "messages": many}}).to_string()).unwrap() else { panic!() };
        assert_eq!((d.messages.len(), d.messages[0].id.as_str()), (MAX_MESSAGES, "50"), "the latest are kept");
        assert!(d.can_reply, "replies by default");
        assert!(d.earlier, "older ones were dropped");
        // An old message with an open ask is kept over a newer one.
        let mut many: Vec<Value> = (0..250).map(|i| json!({"id": i.to_string()})).collect();
        many[3] = json!({"id": "3", "asks": [{"number": 1, "open": true}]});
        let FromAdapter::Thread(d) = parse_line(&json!({"type": "thread", "thread": {"id": "t", "messages": many}}).to_string()).unwrap() else { panic!() };
        let ids: Vec<&str> = d.messages.iter().take(2).map(|m| m.id.as_str()).collect();
        assert_eq!((d.messages.len(), ids), (MAX_MESSAGES, vec!["3", "51"]));
    }

    /// The richer parts: counts, unread counts, blocking asks, answers and
    /// withdrawals a message carries.
    #[test]
    fn counts_blocking_and_carried_answers() {
        assert_eq!(
            parse_line(r#"{"type":"counts","unread":3,"openAsks":2,"blocking":1}"#).unwrap(),
            FromAdapter::Counts(Counts { unread: 3, open_asks: 2, blocking: 1 })
        );
        let line = json!({"type": "threads", "threads": [{"id": "a", "unread": 4, "blocking": true}, {"id": "b", "unread": false}]}).to_string();
        let FromAdapter::Threads(t) = parse_line(&line).unwrap() else { panic!() };
        assert_eq!((t[0].unread, t[0].blocking, t[1].unread), (4, true, 0));
        let line = json!({"type": "thread", "thread": {"id": "t", "unread": true, "earlier": true, "messages": [
            {"id": "1", "asks": [{"number": 1, "question": "Which?", "open": true, "blocking": true}]},
            {"id": "2", "author": {"kind": "you"}, "answers": [{"ask": 1, "question": "Which?", "labels": ["Redis", 7], "text": "and \u{202E}fast"}, {"question": "no ask"}],
             "withdrawals": [{"ask": 2, "reason": "moot"}]}
        ]}})
        .to_string();
        let FromAdapter::Thread(d) = parse_line(&line).unwrap() else { panic!() };
        assert_eq!((d.unread, d.earlier), (1, true));
        assert!(d.messages[0].asks[0].blocking);
        let m = &d.messages[1];
        assert_eq!(m.answers, [Answered { ask: 1, question: "Which?".into(), declined: false, labels: vec!["Redis".into()], text: "and fast".into() }]);
        assert_eq!(m.withdrawals, [Withdrawal { ask: 2, reason: "moot".into() }]);
    }

    #[test]
    fn other_messages_and_bad_lines() {
        assert_eq!(parse_line(r#"{"type":"hello","protocol":1}"#).unwrap(), FromAdapter::Hello { protocol: 1 });
        assert_eq!(
            parse_line(r#"{"type":"status","state":"signedOut","message":"The key expired"}"#).unwrap(),
            FromAdapter::Status { state: ConnState::SignedOut, message: "The key expired".into() }
        );
        assert_eq!(parse_line(r#"{"type":"result","req":4,"ok":true}"#).unwrap(), FromAdapter::Result { req: 4, ok: true, message: String::new() });
        assert_eq!(parse_line(r#"{"type":"badge","n":3}"#).unwrap(), FromAdapter::Unknown, "a later type is ignored");
        assert!(parse_line("not json").is_err());
        assert!(parse_line("[1]").is_err());
        assert!(parse_line(r#"{"no":"type"}"#).is_err());
        assert!(parse_line(r#"{"type":"status","state":"great"}"#).is_err());
        assert!(parse_line(r#"{"type":"thread","thread":{"title":"x"}}"#).is_err());
        assert!(parse_line(r#"{"type":"result","ok":true}"#).is_err());
        assert!(parse_line(&format!(r#"{{"type":"x","pad":"{}"}}"#, "p".repeat(MAX_LINE))).is_err());
    }

    #[test]
    fn what_pitwall_sends() {
        let mut settings = Map::new();
        settings.insert("server".into(), json!("https://inbox.example.com"));
        let mut secrets = Map::new();
        secrets.insert("apiKey".into(), json!("k"));
        let hello: Value = serde_json::from_str(&ToAdapter::Hello { pitwall: "0.7.0".into(), settings, secrets }.line()).unwrap();
        assert_eq!(hello, json!({"type": "hello", "protocol": 1, "pitwall": "0.7.0", "settings": {"server": "https://inbox.example.com"}, "secrets": {"apiKey": "k"}}));
        let a = ToAdapter::Answer { req: 3, thread: "t".into(), ask: 2, picks: vec![0, 1], text: None, declined: false }.line();
        assert_eq!(serde_json::from_str::<Value>(&a).unwrap(), json!({"type": "answer", "req": 3, "thread": "t", "ask": 2, "picks": [0, 1], "declined": false}));
        assert!(!ToAdapter::Refresh.line().contains('\n'));
        let m = ToAdapter::Message { req: 1, thread: "t".into(), text: "two\nlines".into() }.line();
        assert!(!m.contains('\n'), "one line on the wire");
    }

    #[test]
    fn floods_and_long_lines_are_caught() {
        let mut g = LineGuard::default();
        for i in 0..MAX_LINES_PER_SEC {
            g.line(5000 + u64::from(i)).unwrap();
        }
        assert!(g.line(5900).is_err());
        let mut g = LineGuard::default();
        for i in 0..1000u64 {
            g.line(i * 10).unwrap();
        }
        let mut r = std::io::BufReader::with_capacity(16, &b"{\"a\":1}\r\nsecond"[..]);
        let mut buf = Vec::new();
        assert!(read_line(&mut r, &mut buf).unwrap());
        assert_eq!(buf, b"{\"a\":1}");
        assert!(read_line(&mut r, &mut buf).unwrap());
        assert_eq!(buf, b"second");
        assert!(!read_line(&mut r, &mut buf).unwrap());
        let big = vec![b'x'; MAX_LINE + 10];
        let mut r = std::io::BufReader::new(&big[..]);
        assert!(read_line(&mut r, &mut buf).is_err());
    }
}
